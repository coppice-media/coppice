//! The socket.io lane: `GET /socket.io?EIO=4&transport=websocket`.
//!
//! Audiobookshelf pushes state changes over socket.io, and the official app
//! hides its **bookmark button** unless the socket is connected
//! (`components/app/AudioPlayer.vue:68`, `v-if="… && socketConnected"`), so a
//! REST-only server costs that app a feature no REST route can give back.
//! This module is the smallest thing that closes that gap honestly: an
//! Engine.IO v4 handshake, a Socket.IO v5 connect, the app's `auth`
//! round-trip, and the state events it can be told the truth about.
//!
//! It is a **sink**, not a source of state: it holds no reading position, no
//! session and no user list of its own. Everything it emits is translated
//! from an [`AbsEvent`] that some other lane already committed — the server
//! adapter forwards the process-wide `CoreEvent`/`ReadingHeadChanged` buses
//! into it, and the profile's own bookmark routes publish their writes the
//! same way. A socket that is dropped therefore loses notifications, never
//! data.
//!
//! Framing lives in [`protocol`]; it was written from the MIT-licensed
//! `engine.io-protocol` / `socket.io-protocol` specifications and verified
//! frame-for-frame against a live `abs-ref` 2.36.0 container.
pub(crate) mod protocol;
#[cfg(test)]
mod tests;

use std::{
	collections::HashMap,
	sync::{Arc, Mutex},
	time::Duration,
};

use axum::{
	extract::{
		ws::{Message, WebSocket, WebSocketUpgrade},
		Extension,
	},
	response::Response,
	routing::get,
	Router,
};
use chrono::Utc;
use models::entity::user::AuthUser;
use tokio::{sync::broadcast, time::Instant};
use uuid::Uuid;

use crate::{
	auth::verify_token,
	dto::{SocketInitDto, UserItemProgressUpdatedDto, UserOnlineDto},
	mapper,
	model::ItemShape,
	routes::{me, query, AbsBackend},
};

use protocol::{ClientFrame, DEFAULT_NAMESPACE};

/// Engine.IO heartbeat, as abs-ref advertises it: the server pings every
/// 25 s and gives the client 20 s to answer.
pub const PING_INTERVAL: Duration = Duration::from_secs(25);
pub const PING_TIMEOUT: Duration = Duration::from_secs(20);
/// `maxPayload`, the largest single frame the client may send. Only the
/// handshake advertises it; nothing this lane accepts comes close.
const MAX_PAYLOAD: u64 = 1_000_000;
/// How many events the bus buffers per subscriber before a slow socket is
/// told it lagged. A lagging socket skips notifications and keeps running:
/// every one of them is a "refetch this" hint, never the state itself.
const EVENT_BUFFER: usize = 256;

/// A change one of Stump's lanes committed, in the terms this profile can
/// translate. The server adapter fills these from the core event buses; the
/// profile's own writes publish them directly.
#[derive(Debug, Clone, PartialEq)]
pub enum AbsEvent {
	/// One user's listening position for one book moved (or was cleared).
	ProgressChanged { user_id: String, media_id: String },
	/// One user's `/api/me` object changed for a reason other than a
	/// position: a bookmark write, today.
	UserChanged { user_id: String },
	/// Books appeared in a library.
	ItemsAdded { media_ids: Vec<String> },
	/// A series' books were rewritten in place (a provider materialise). The
	/// series is named rather than its books: the sink resolves them anyway
	/// to gate visibility, so the bus never carries an id list that a
	/// subscriber may not see.
	SeriesUpdated { series_id: String },
	/// A playlist mutation, carrying the exact expanded DTO returned by REST.
	/// The app's modal and playlist page consume the same `id`/`items` shape.
	PlaylistAdded {
		user_id: String,
		playlist: crate::dto::PlaylistDto,
	},
	PlaylistUpdated {
		user_id: String,
		playlist: crate::dto::PlaylistDto,
	},
	PlaylistRemoved {
		user_id: String,
		playlist: crate::dto::PlaylistDto,
	},
}

/// The per-server socket bus: the event fan-out plus the presence registry
/// that `usersOnline` reports.
#[derive(Clone)]
pub struct AbsEvents {
	sender: broadcast::Sender<AbsEvent>,
	online: Arc<Mutex<HashMap<String, usize>>>,
}

impl Default for AbsEvents {
	fn default() -> Self {
		Self::new()
	}
}

impl AbsEvents {
	pub fn new() -> Self {
		Self {
			sender: broadcast::channel(EVENT_BUFFER).0,
			online: Arc::new(Mutex::new(HashMap::new())),
		}
	}

	/// Announce a change. A send with no subscribers is not an error: it
	/// means nobody has a socket open.
	pub fn send(&self, event: AbsEvent) {
		let _ = self.sender.send(event);
	}

	pub fn subscribe(&self) -> broadcast::Receiver<AbsEvent> {
		self.sender.subscribe()
	}

	/// Register one authenticated socket and answer how many that user now
	/// holds. A user may have several: the app opens one per web view and
	/// keeps it across screens.
	fn join(&self, user_id: &str) -> usize {
		let mut online = self.online.lock().expect("socket presence registry");
		let count = online.entry(user_id.to_owned()).or_insert(0);
		*count += 1;
		*count
	}

	fn leave(&self, user_id: &str) {
		let mut online = self.online.lock().expect("socket presence registry");
		if let Some(count) = online.get_mut(user_id) {
			*count = count.saturating_sub(1);
			if *count == 0 {
				online.remove(user_id);
			}
		}
	}
}

/// Mount the socket endpoint. It sits at the server **root**, beside the
/// other Audiobookshelf routes, and takes no authentication of its own: the
/// app's socket.io client cannot put a header on the upgrade, so the token
/// arrives afterwards in an `auth` event.
///
/// Both spellings are registered because Engine.IO appends a slash to the
/// configured path before the query string — the app connects to
/// `/socket.io/?EIO=4&transport=websocket` — and axum does not redirect
/// between the two.
pub fn router<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::new()
		.route("/socket.io", get(upgrade))
		.route("/socket.io/", get(upgrade))
}

async fn upgrade(
	upgrade: WebSocketUpgrade,
	Extension(backend): Extension<Arc<dyn AbsBackend>>,
	Extension(events): Extension<AbsEvents>,
) -> Response {
	upgrade.on_upgrade(move |socket| async move {
		Connection::new(socket, backend, events).run().await;
	})
}

/// The user behind an authenticated socket.
struct Session {
	user: AuthUser,
	/// When the token stops being valid; the socket is dropped then, and the
	/// client reconnects with a refreshed one.
	expires_at: Option<Instant>,
}

struct Connection {
	socket: WebSocket,
	backend: Arc<dyn AbsBackend>,
	events: AbsEvents,
	session: Option<Session>,
}

impl Connection {
	fn new(socket: WebSocket, backend: Arc<dyn AbsBackend>, events: AbsEvents) -> Self {
		Self {
			socket,
			backend,
			events,
			session: None,
		}
	}

	async fn run(mut self) {
		let sid = Uuid::new_v4().to_string();
		if self
			.send(protocol::open_frame(
				&sid,
				PING_INTERVAL.as_millis() as u64,
				PING_TIMEOUT.as_millis() as u64,
				MAX_PAYLOAD,
			))
			.await
			.is_err()
		{
			return;
		}

		let mut incoming = self.events.subscribe();
		let mut heartbeat = tokio::time::interval(PING_INTERVAL);
		heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		// `interval` fires immediately; the first ping belongs one interval
		// after the handshake, not on it.
		heartbeat.tick().await;

		// Two deadlines that are usually not armed: `never` parks them
		// without a branch that has to be conditionally compiled out.
		let never = Instant::now() + Duration::from_secs(60 * 60 * 24 * 365);
		let pong_deadline = tokio::time::sleep_until(never);
		let expiry = tokio::time::sleep_until(never);
		tokio::pin!(pong_deadline);
		tokio::pin!(expiry);
		let mut awaiting_pong = false;

		loop {
			tokio::select! {
				frame = self.socket.recv() => {
					let Some(Ok(message)) = frame else { break };
					let text = match message {
						Message::Text(text) => text.to_string(),
						Message::Close(_) => break,
						// Ping/Pong at the *websocket* level are answered by
						// axum; binary frames have no meaning on this lane.
						_ => continue,
					};
					match self.handle(&text).await {
						Flow::Continue => {},
						Flow::Pong => {
							awaiting_pong = false;
							pong_deadline.as_mut().reset(never);
						},
						Flow::Authenticated => {
							if let Some(at) = self.session.as_ref().and_then(|s| s.expires_at) {
								expiry.as_mut().reset(at);
							}
						},
						Flow::Close => break,
					}
				},
				event = incoming.recv() => {
					match event {
						Ok(event) => {
							if self.dispatch(event).await.is_err() {
								break;
							}
						},
						Err(broadcast::error::RecvError::Lagged(skipped)) => {
							tracing::debug!(skipped, "ABS socket subscriber lagged");
						},
						Err(broadcast::error::RecvError::Closed) => break,
					}
				},
				_ = heartbeat.tick() => {
					if self.send(protocol::ping_frame()).await.is_err() {
						break;
					}
					awaiting_pong = true;
					pong_deadline.as_mut().reset(Instant::now() + PING_TIMEOUT);
				},
				_ = &mut pong_deadline, if awaiting_pong => {
					tracing::debug!("ABS socket missed its heartbeat; closing");
					let _ = self.send(protocol::close_frame()).await;
					break;
				},
				_ = &mut expiry => {
					// The credential this socket authenticated with is no
					// longer valid. Closing makes the client reconnect and
					// re-`auth` with a refreshed token, which is exactly
					// what it does after a REST 401.
					tracing::debug!("ABS socket token expired; closing");
					let _ = self.send(protocol::close_frame()).await;
					break;
				},
			}
		}

		if let Some(session) = self.session.as_ref() {
			self.events.leave(&session.user.id);
		}
	}

	async fn send(&mut self, frame: String) -> Result<(), axum::Error> {
		self.socket.send(Message::Text(frame.into())).await
	}

	/// What the caller must do after a client frame was handled.
	async fn handle(&mut self, text: &str) -> Flow {
		match protocol::decode(text) {
			ClientFrame::Ping => {
				if self.send(protocol::pong_frame()).await.is_err() {
					return Flow::Close;
				}
				Flow::Continue
			},
			ClientFrame::Pong => Flow::Pong,
			ClientFrame::Close => Flow::Close,
			ClientFrame::Connect { namespace } => {
				if namespace != DEFAULT_NAMESPACE {
					// abs-ref serves one namespace; a client asking for
					// another is told so rather than left waiting.
					let _ = self
						.send(protocol::event_frame(
							"connect_error",
							Some(serde_json::json!({ "message": "Invalid namespace" })),
						))
						.await;
					return Flow::Continue;
				}
				let socket_id = Uuid::new_v4().to_string();
				if self
					.send(protocol::connect_frame(&socket_id))
					.await
					.is_err()
				{
					return Flow::Close;
				}
				Flow::Continue
			},
			ClientFrame::Event { name, args } if name == "auth" => {
				let token = args.first().and_then(|value| value.as_str()).unwrap_or("");
				self.authenticate(token).await
			},
			// The app emits nothing else; abs-ref's other client events
			// (log listeners, cover search, scan cancellation) are server
			// administration this profile does not expose.
			ClientFrame::Event { .. } | ClientFrame::Ignored => Flow::Continue,
		}
	}

	/// The `auth` round trip: the same HS256 token `POST /login` minted.
	async fn authenticate(&mut self, token: &str) -> Flow {
		let Ok(secret) = self.backend.token_secret().await else {
			return self.auth_failed("Server error").await;
		};
		let Ok(claims) = verify_token(&secret, token) else {
			return self.auth_failed("Invalid token").await;
		};
		if !claims.is_access() {
			// A refresh token buys a new pair at `POST /auth/refresh`; it
			// must not open a live channel.
			return self.auth_failed("Invalid token").await;
		}
		let Ok((user, created_at)) = self.backend.user(&claims.user_id).await else {
			return self.auth_failed("Invalid token").await;
		};
		if user.is_locked {
			return self.auth_failed("Invalid token").await;
		}

		let connections = self.events.join(&user.id);
		let init = SocketInitDto {
			user_id: user.id.clone(),
			username: user.username.clone(),
			users_online: vec![UserOnlineDto {
				id: user.id.clone(),
				username: user.username.clone(),
				user_type: if user.is_server_owner {
					"root".to_owned()
				} else {
					"user".to_owned()
				},
				session: None,
				last_seen: Some(Utc::now().timestamp_millis()),
				created_at: created_at.timestamp_millis(),
				connections: connections as i64,
			}],
		};
		let expires_at = claims.exp.and_then(|exp| {
			let remaining = exp - Utc::now().timestamp();
			(remaining > 0)
				.then(|| Instant::now() + Duration::from_secs(remaining as u64))
		});
		self.session = Some(Session { user, expires_at });

		match serde_json::to_value(init) {
			Ok(payload) => {
				if self
					.send(protocol::event_frame("init", Some(payload)))
					.await
					.is_err()
				{
					return Flow::Close;
				}
				Flow::Authenticated
			},
			Err(_) => Flow::Close,
		}
	}

	async fn auth_failed(&mut self, message: &str) -> Flow {
		// abs-ref keeps the socket open after a failed auth; the client
		// retries on the same connection once it has a fresh token.
		let _ = self
			.send(protocol::event_frame(
				"auth_failed",
				Some(serde_json::json!({ "message": message })),
			))
			.await;
		Flow::Continue
	}

	/// Translate one committed change into the events the app subscribes to.
	///
	/// The frames are built by free functions over the backend rather than
	/// by methods: a `&self` borrow held across an await would make the
	/// connection future require `Sync`, which a websocket is not.
	async fn dispatch(&mut self, event: AbsEvent) -> Result<(), axum::Error> {
		let Some(session) = self.session.as_ref() else {
			// Nothing is pushed before `auth`: an unauthenticated socket has
			// no user to scope visibility by.
			return Ok(());
		};
		let user = session.user.clone();
		let backend = Arc::clone(&self.backend);

		let frames = match event {
			AbsEvent::ProgressChanged { user_id, media_id } if user_id == user.id => {
				let mut frames = Vec::with_capacity(2);
				frames.extend(progress_frame(backend.as_ref(), &user, &media_id).await);
				frames.extend(user_frame(backend.as_ref(), &user).await);
				frames
			},
			AbsEvent::UserChanged { user_id } if user_id == user.id => {
				user_frame(backend.as_ref(), &user)
					.await
					.into_iter()
					.collect()
			},
			AbsEvent::ItemsAdded { media_ids } => {
				items_frame(backend.as_ref(), "items_added", &user, &media_ids)
					.await
					.into_iter()
					.collect()
			},
			// abs-ref's `item_updated` carries the item itself, not an
			// array, so a rewritten series is one event per book.
			AbsEvent::SeriesUpdated { series_id } => {
				series_items(backend.as_ref(), &user, &series_id)
					.await
					.into_iter()
					.filter_map(|item| serde_json::to_value(item).ok())
					.map(|payload| protocol::event_frame("item_updated", Some(payload)))
					.collect()
			},
			AbsEvent::PlaylistAdded { user_id, playlist } if user_id == user.id => {
				playlist_frame("playlist_added", playlist)
					.into_iter()
					.collect()
			},
			AbsEvent::PlaylistUpdated { user_id, playlist } if user_id == user.id => {
				playlist_frame("playlist_updated", playlist)
					.into_iter()
					.collect()
			},
			AbsEvent::PlaylistRemoved { user_id, playlist } if user_id == user.id => {
				playlist_frame("playlist_removed", playlist)
					.into_iter()
					.collect()
			},
			_ => Vec::new(),
		};

		for frame in frames {
			self.send(frame).await?;
		}
		Ok(())
	}
}

/// `user_item_progress_updated`, or `None` when the book is not one this
/// user may see, is not audio, or has no position after the change (an
/// explicit reset).
async fn progress_frame(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	media_id: &str,
) -> Option<String> {
	let row = query::media_for_user(backend, user, media_id).await.ok()?;
	let progress = backend.progress(user, media_id).await.ok()??;
	let book_id = backend
		.book_ids(&[row.id.clone()])
		.await
		.ok()?
		.remove(&row.id)
		.unwrap_or_else(|| row.id.clone());
	let duration_ms = backend
		.audio(media_id)
		.await
		.ok()?
		.map(|audio| audio.duration_ms)
		.unwrap_or(0);
	let data = mapper::progress_dto(&user.id, media_id, &book_id, duration_ms, &progress);
	let payload = serde_json::to_value(UserItemProgressUpdatedDto {
		id: data.id.clone(),
		session_id: None,
		data,
	})
	.ok()?;
	Some(protocol::event_frame(
		"user_item_progress_updated",
		Some(payload),
	))
}

/// `user_updated`: the whole `/api/me` object, which is what abs-ref pushes
/// when a user's progress or bookmarks change and what the app's store
/// replaces wholesale.
async fn user_frame(backend: &dyn AbsBackend, user: &AuthUser) -> Option<String> {
	let payload = serde_json::to_value(me::user_dto(backend, user).await.ok()?).ok()?;
	Some(protocol::event_frame("user_updated", Some(payload)))
}

fn playlist_frame(name: &str, playlist: crate::dto::PlaylistDto) -> Option<String> {
	let payload = serde_json::to_value(playlist).ok()?;
	Some(protocol::event_frame(name, Some(payload)))
}

async fn items_frame(
	backend: &dyn AbsBackend,
	name: &str,
	user: &AuthUser,
	media_ids: &[String],
) -> Option<String> {
	use sea_orm::ColumnTrait;

	let items = items(
		backend,
		user,
		models::entity::media::Column::Id.is_in(media_ids.to_vec()),
	)
	.await;
	if items.is_empty() {
		return None;
	}
	let payload = serde_json::to_value(items).ok()?;
	Some(protocol::event_frame(name, Some(payload)))
}

/// The audible books of one series this user may see.
async fn series_items(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	series_id: &str,
) -> Vec<crate::dto::LibraryItemDto> {
	use sea_orm::ColumnTrait;

	items(
		backend,
		user,
		models::entity::media::Column::SeriesId.eq(series_id),
	)
	.await
}

/// The books matching `condition` that this user may see, as minified items.
/// Visibility is resolved here, on the socket's own user, so an event about
/// a book outside their libraries is silently nothing.
async fn items(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	condition: sea_orm::sea_query::SimpleExpr,
) -> Vec<crate::dto::LibraryItemDto> {
	use sea_orm::QueryFilter;

	let Ok(rows) = query::audio_media(user)
		.filter(condition)
		.all(backend.conn())
		.await
	else {
		return Vec::new();
	};
	if rows.is_empty() {
		return Vec::new();
	}
	let Ok(context) = query::context(backend, user, &rows, false).await else {
		return Vec::new();
	};
	rows.iter()
		.map(|row| context.item(row, ItemShape::Minified, &user.id, None))
		.collect()
}

/// What handling a client frame means for the connection loop.
enum Flow {
	Continue,
	Pong,
	Authenticated,
	Close,
}
