//! Audiobookshelf compatibility profile: the `stump_abs` router over a
//! server-side backend adapter, authenticated by Audiobookshelf's credential
//! rules.
//!
//! The profile authenticates a request with one of:
//! - `Authorization: Bearer <jwt>` minted by `POST /login` or
//!   `POST /auth/refresh` — the ordinary Lissen path, since its OkHttp
//!   interceptor puts the stored token on every request including the
//!   ExoPlayer data source (`channel/common/OkHttpClient.kt:47-59`,
//!   `playback/service/LissenDataSourceFactory.kt:37`);
//! - `Authorization: Bearer stump_...`, a Stump API key, so a device
//!   credential drives the profile without a password login;
//! - `?token=<jwt|api key>`, which abs-ref also accepts
//!   (`../komga-compat/abs/capture/negatives.txt`: `query token /api/me: 200`)
//!   and which casting clients need, because a cast receiver cannot set
//!   headers;
//! - `Authorization: Basic`, for scripts and the replay harness.
//!
//! A password login registers (or reuses) a [`DeviceKind::Abs`] device row and
//! stamps its id into the minted tokens, so the device's library scope narrows
//! every later request on that session.

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
	body::Body,
	extract::{Request, State},
	http::{header, HeaderMap, Response},
	middleware::{self, Next},
	response::{IntoResponse, Response as AxumResponse},
	Extension, Router,
};
use base64::Engine;
use chrono::{DateTime, Utc};
use models::{
	domain::edition_pair::PairStatus,
	entity::{
		device, liseur_sync_media_link, media, media_audio, media_audio_chapter,
		media_audio_track, reading_head, reading_list, reading_list_item, user,
		user::AuthUser,
	},
	services::{reading_progress, reading_state as reading_state_service},
	shared::enums::DeviceKind,
};
use prefixed_api_key::PrefixedApiKey;
use sea_orm::{prelude::*, DatabaseConnection, QueryOrder};
use stump_abs::{
	errors::{AbsError, AbsResult},
	model::{
		AbsAudio, AbsAudioChapter, AbsAudioTrack, AbsBookmark, AbsEbookFile, AbsImage,
		AbsPlaylist, AbsPositionUpdate, AbsProgress,
	},
	routes::{AbsBackend, AbsSession},
	AbsEvent, AbsEvents, AbsIds, AbsSessions, IdKind,
};
use stump_api_types::RequestOrigin;
use stump_auth::AuthContext;
use stump_core::CoreEvent;
use stump_devices::{CredentialRef, Protocol};
use stump_media::transform::cache::TransformCache;
use tokio::sync::broadcast;

use crate::{
	config::{jwt::access_token_secret, state::AppState},
	errors::APIError,
	middleware::{
		auth::{bind_device, handle_bearer_auth, inject_avatar_url, validate_api_key},
		host::HostExtractor,
	},
	routers::{api::v2::media as api_media, audio_transform},
	utils::verify_password,
};

pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	let backend: Arc<dyn AbsBackend> =
		Arc::new(AbsBackendAdapter::new(app_state.clone()));
	let events = spawn_event_forwarder(&app_state);
	compose(app_state, backend, events)
}

/// Bridge the process-wide event buses onto the profile's socket lane.
///
/// Two subscriptions, translated into the state changes an Audiobookshelf
/// client can be told: a reading head moved (from any protocol — a Kobo, a
/// KOReader plugin, this profile's own sync), books appeared, or a book's row
/// was rewritten. Playlist mutations are announced by their own route
/// handlers because they are not core-media events. Everything else on the
/// core bus is server business no Audiobookshelf client subscribes to.
///
/// The forwarder lives here, not on the backend adapter, because the adapter
/// is also constructed per request for `Basic` authentication; spawning from
/// its constructor would start a task per login.
fn spawn_event_forwarder(app_state: &AppState) -> AbsEvents {
	let events = AbsEvents::new();

	let mut heads = app_state.reading_state_events();
	let sink = events.clone();
	tokio::spawn(async move {
		loop {
			match heads.recv().await {
				Ok(changed) => sink.send(AbsEvent::ProgressChanged {
					user_id: changed.user_id,
					media_id: changed.media_id,
				}),
				Err(broadcast::error::RecvError::Lagged(skipped)) => {
					tracing::debug!(skipped, "ABS reading-state forwarder lagged")
				},
				Err(broadcast::error::RecvError::Closed) => break,
			}
		}
	});

	let mut core = app_state.get_client_receiver();
	let sink = events.clone();
	tokio::spawn(async move {
		loop {
			match core.recv().await {
				Ok(CoreEvent::CreatedMedia(media)) => sink.send(AbsEvent::ItemsAdded {
					media_ids: vec![media.id],
				}),
				// A materialised provider series rewrote rows that already
				// existed under deterministic ids, so each of its audible
				// books is an update. A filesystem rescan announces a count
				// and a series (`CreatedOrUpdatedManyMedia`) with no ids at
				// all, so it cannot name an item and is not translated.
				Ok(CoreEvent::ProviderSeriesMaterialized(series)) => {
					sink.send(AbsEvent::SeriesUpdated {
						series_id: series.series_id,
					})
				},
				Ok(_) => {},
				Err(broadcast::error::RecvError::Lagged(skipped)) => {
					tracing::debug!(skipped, "ABS core event forwarder lagged")
				},
				Err(broadcast::error::RecvError::Closed) => break,
			}
		}
	});

	events
}

/// The public routes sit at the server root (`/login`, `/logout`,
/// `/auth/refresh`, `/ping`, `/healthcheck`, `/status`, `/socket.io`) and the
/// authenticated ones under `/api`, exactly where an Audiobookshelf client
/// looks for them.
///
/// The socket endpoint carries no auth middleware on purpose: a socket.io
/// client cannot put a header on the upgrade request, so the token arrives
/// in the `auth` event and is verified there.
fn compose(
	app_state: AppState,
	backend: Arc<dyn AbsBackend>,
	events: AbsEvents,
) -> Router<AppState> {
	let protected = Router::new()
		.nest("/api", stump_abs::authenticated_router::<AppState>())
		.layer(middleware::from_fn_with_state(
			app_state,
			abs_auth_middleware,
		));
	stump_abs::public_router::<AppState>()
		.merge(stump_abs::socket_router::<AppState>())
		.merge(protected)
		.layer(Extension(events))
		.layer(Extension(backend))
}

/// The `token` query parameter, if present.
fn token_query(query: Option<&str>) -> Option<String> {
	query?
		.split('&')
		.filter_map(|pair| pair.split_once('='))
		.find(|(key, _)| key.eq_ignore_ascii_case("token"))
		.map(|(_, value)| {
			urlencoding::decode(value)
				.map(|value| value.into_owned())
				.unwrap_or_else(|_| value.to_owned())
		})
		.filter(|value| !value.is_empty())
}

/// `Basic` credentials, decoded.
fn basic_credentials(header_value: &str) -> Option<(String, String)> {
	let encoded = header_value.strip_prefix("Basic ")?.trim();
	let decoded = base64::engine::general_purpose::STANDARD
		.decode(encoded)
		.ok()?;
	let decoded = String::from_utf8(decoded).ok()?;
	let (username, password) = decoded.split_once(':')?;
	Some((username.to_owned(), password.to_owned()))
}

/// Resolve an Audiobookshelf token minted by this profile.
///
/// The `device` claim is the device row the login registered, so the session
/// keeps that device's library scope for its whole lifetime — the same
/// binding the Kobo and KOReader credentials get, without a second
/// round-trip to the credential store.
async fn authenticate_abs_token(
	ctx: &AppState,
	token: &str,
) -> Result<Option<AuthContext>, AxumResponse> {
	if !stump_abs::auth::looks_like_jwt(token) {
		return Ok(None);
	}
	let Ok(secret) = access_token_secret(ctx.conn.as_ref()).await else {
		return Ok(None);
	};
	let Ok(claims) = stump_abs::verify_token(secret.as_bytes(), token) else {
		return Ok(None);
	};

	let user = load_user(ctx.conn.as_ref(), &claims.user_id)
		.await
		.map_err(|error| APIError::from(error).into_response())?;
	let mut auth = AuthContext {
		user,
		api_key: None,
		device_id: None,
	};
	if let Some(device_id) = claims.device.as_deref() {
		apply_device_scope(ctx.conn.as_ref(), &mut auth, device_id)
			.await
			.map_err(|error| error.into_response())?;
	}
	Ok(Some(auth))
}

/// Narrow the request to the library scope of the device its token names.
async fn apply_device_scope(
	conn: &DatabaseConnection,
	auth: &mut AuthContext,
	device_id: &str,
) -> Result<(), APIError> {
	let device = device::Entity::find_by_id(device_id)
		.filter(device::Column::UserId.eq(auth.user.id.clone()))
		.one(conn)
		.await?;
	if let Some(device) = device {
		auth.user.device_library_scope = device.library_scope_ids();
		auth.device_id = Some(device.id);
	}
	Ok(())
}

async fn load_user(
	conn: &DatabaseConnection,
	user_id: &str,
) -> Result<AuthUser, APIError> {
	let user = user::LoginUser::find()
		.filter(user::Column::Id.eq(user_id))
		.filter(user::Column::DeletedAt.is_null())
		.into_model::<user::LoginUser>()
		.one(conn)
		.await?
		.ok_or(APIError::Unauthorized)?;
	if user.is_locked {
		return Err(APIError::Unauthorized);
	}
	Ok(AuthUser::from(user))
}

/// Authenticate an Audiobookshelf request: the profile's own token, then a
/// Stump API key or access token, then Basic. A present credential that fails
/// is a `401` and never falls through to the next one.
async fn abs_auth_middleware(
	State(ctx): State<AppState>,
	HostExtractor(host_details): HostExtractor,
	mut req: Request,
	next: Next,
) -> Result<AxumResponse, AxumResponse> {
	let service =
		RequestOrigin::new(host_details.host.clone(), host_details.scheme.clone());
	let headers = req.headers().clone();
	let authorization = headers
		.get(header::AUTHORIZATION)
		.and_then(|value| value.to_str().ok())
		.map(str::to_owned);
	let bearer = authorization
		.as_deref()
		.and_then(|value| value.strip_prefix("Bearer "))
		.map(str::trim)
		.filter(|value| !value.is_empty())
		.map(str::to_owned)
		.or_else(|| token_query(req.uri().query()));

	let mut auth = if let Some(token) = bearer {
		match authenticate_abs_token(&ctx, &token).await? {
			Some(auth) => auth,
			None => {
				let mut auth = handle_bearer_auth(token, ctx.conn.as_ref())
					.await
					.map_err(|error| error.into_response())?;
				if let Some(api_key) = auth.api_key.clone() {
					bind_device(
						&ctx,
						&mut auth,
						CredentialRef::ApiKey(&api_key),
						Protocol::Api,
					)
					.await
					.map_err(|error| error.into_response())?;
				}
				auth
			},
		}
	} else if let Some((username, password)) =
		authorization.as_deref().and_then(basic_credentials)
	{
		let adapter = AbsBackendAdapter::new(ctx.clone());
		let (user, device_id) = adapter
			.authenticate_password(&username, &password)
			.await
			.map_err(|_| APIError::Unauthorized.into_response())?;
		let mut auth = AuthContext {
			user,
			api_key: None,
			device_id: None,
		};
		if let Some(device_id) = device_id {
			apply_device_scope(ctx.conn.as_ref(), &mut auth, &device_id)
				.await
				.map_err(|error| error.into_response())?;
		}
		auth
	} else {
		return Err(APIError::Unauthorized.into_response());
	};

	auth.user = inject_avatar_url(auth.user, service);
	req.extensions_mut().insert(auth.user.clone());
	req.extensions_mut().insert(auth);
	Ok(next.run(req).await)
}

pub(crate) struct AbsBackendAdapter {
	ctx: AppState,
}

impl AbsBackendAdapter {
	pub(crate) fn new(ctx: AppState) -> Self {
		Self { ctx }
	}

	/// The `media_audio` row plus its tracks and chapters, in one shape.
	fn assemble(
		audio: media_audio::Model,
		tracks: Vec<media_audio_track::Model>,
		chapters: Vec<media_audio_chapter::Model>,
	) -> AbsAudio {
		AbsAudio {
			duration_ms: audio.duration_ms,
			codec: audio.codec,
			sample_rate: audio.sample_rate,
			channels: audio.channels,
			bitrate: audio.bitrate,
			tracks: tracks
				.into_iter()
				.map(|track| AbsAudioTrack {
					index: track.index,
					path: track.path,
					duration_ms: track.duration_ms,
					start_offset_ms: track.start_offset_ms,
					byte_size: track.byte_size,
					mime: track.mime,
				})
				.collect(),
			chapters: chapters
				.into_iter()
				.map(|chapter| AbsAudioChapter {
					index: chapter.index,
					title: chapter.title,
					start_ms: chapter.start_ms,
					end_ms: chapter.end_ms,
				})
				.collect(),
		}
	}

	/// The reading head of a user for one book, which is where every
	/// protocol's position lands.
	async fn head(
		&self,
		user_id: &str,
		media_id: &str,
	) -> AbsResult<Option<reading_head::Model>> {
		Ok(reading_state_service::head(self.conn(), user_id, media_id).await?)
	}

	/// The device row an Audiobookshelf client is registered as: one per
	/// user, reused across logins so the registry does not grow a row per
	/// sign-in. A user who may not hold API keys simply gets no device, and
	/// the session falls back to their own visibility.
	async fn register_device(&self, user: &AuthUser) -> Option<String> {
		let existing = self
			.ctx
			.devices()
			.list(user)
			.await
			.ok()?
			.into_iter()
			.find(|device| device.kind == DeviceKind::Abs);
		if let Some(device) = existing {
			return Some(device.id);
		}
		match self
			.ctx
			.devices()
			.create_device(
				user,
				stump_devices::CredentialIssuance::DelegatedCredential,
				DeviceKind::Abs,
				None,
			)
			.await
		{
			Ok((device, _)) => Some(device.id),
			Err(error) => {
				tracing::debug!(
					?error,
					"Could not register an Audiobookshelf device for this user"
				);
				None
			},
		}
	}

	/// A reading list plus its ordered membership, as the profile's playlist.
	///
	/// `reading_lists` carries no creation timestamp, so `createdAt` and
	/// `lastUpdate` are both `updated_at`. The official app reads neither.
	async fn hydrate_playlist(&self, row: reading_list::Model) -> AbsResult<AbsPlaylist> {
		let media_ids = reading_list_item::Entity::find()
			.filter(reading_list_item::Column::ReadingListId.eq(row.id.clone()))
			.order_by_asc(reading_list_item::Column::DisplayOrder)
			.order_by_asc(reading_list_item::Column::Id)
			.all(self.conn())
			.await?
			.into_iter()
			.map(|item| item.media_id)
			.collect();
		let updated_at = row.updated_at.to_utc();
		Ok(AbsPlaylist {
			id: row.id,
			name: row.name,
			description: row.description,
			media_ids,
			created_at: updated_at,
			updated_at,
		})
	}
}

fn map_server_error(error: APIError) -> AbsError {
	match error {
		APIError::NotFound(message) => AbsError::NotFound(message),
		APIError::BadRequest(message) => AbsError::BadRequest(message),
		APIError::Unauthorized => AbsError::Unauthorized,
		APIError::Forbidden(message) => AbsError::Forbidden(message),
		other => AbsError::InternalServerError(other.to_string()),
	}
}

/// A head's position as this profile reports it. A head with no `position_ms`
/// is a book read by a page- or locator-addressed client; its progression
/// still resumes an audiobook, so it is projected back onto the timeline.
fn progress_from_head(head: &reading_head::Model, duration_ms: i64) -> AbsProgress {
	let position_ms = head.position_ms.unwrap_or_else(|| {
		if duration_ms > 0 {
			(head.progression.clamp(0.0, 1.0) * duration_ms as f64).round() as i64
		} else {
			0
		}
	});
	AbsProgress {
		position_ms,
		track_index: head.track_index,
		is_finished: head.completed,
		started_at: head.created_at.to_utc(),
		last_update: head.updated_at.to_utc(),
		finished_at: head.completed.then(|| head.updated_at.to_utc()),
	}
}

#[async_trait]
impl AbsBackend for AbsBackendAdapter {
	fn conn(&self) -> &DatabaseConnection {
		self.ctx.conn.as_ref()
	}

	async fn token_secret(&self) -> AbsResult<Vec<u8>> {
		access_token_secret(self.conn())
			.await
			.map(String::into_bytes)
			.map_err(map_server_error)
	}

	async fn authenticate_password(
		&self,
		username: &str,
		password: &str,
	) -> AbsResult<(AuthUser, Option<String>)> {
		let candidate = user::LoginUser::find()
			.filter(user::Column::Username.eq(username))
			.filter(user::Column::DeletedAt.is_null())
			.into_model::<user::LoginUser>()
			.one(self.conn())
			.await?
			.ok_or(AbsError::Unauthorized)?;

		// A device API key may stand in for the password, so a client that
		// only has a key can still use the login form.
		let verified = if let Ok(api_key) = PrefixedApiKey::from_string(password) {
			validate_api_key(api_key, self.conn())
				.await
				.map(|user| user.id == candidate.id)
				.unwrap_or(false)
		} else {
			verify_password(&candidate.hashed_password, password)
				.map_err(|error| AbsError::InternalServerError(error.to_string()))?
		};
		if !verified || candidate.is_locked {
			return Err(AbsError::Unauthorized);
		}

		let user = AuthUser::from(candidate);
		let device_id = self.register_device(&user).await;
		Ok((user, device_id))
	}

	fn is_docker(&self) -> bool {
		std::path::Path::new("/.dockerenv").exists()
	}

	async fn user(&self, user_id: &str) -> AbsResult<(AuthUser, DateTime<Utc>)> {
		let row = user::Entity::find_by_id(user_id)
			.filter(user::Column::DeletedAt.is_null())
			.one(self.conn())
			.await?
			.ok_or(AbsError::Unauthorized)?;
		let created_at = row.created_at.to_utc();
		let user = load_user(self.conn(), user_id)
			.await
			.map_err(map_server_error)?;
		Ok((user, created_at))
	}

	async fn audio(&self, media_id: &str) -> AbsResult<Option<AbsAudio>> {
		let Some(audio) = media_audio::Entity::find_by_id(media_id)
			.one(self.conn())
			.await?
		else {
			return Ok(None);
		};
		let tracks = media_audio_track::Entity::find()
			.filter(media_audio_track::Column::MediaId.eq(media_id))
			.order_by_asc(media_audio_track::Column::Index)
			.all(self.conn())
			.await?;
		let chapters = media_audio_chapter::Entity::find()
			.filter(media_audio_chapter::Column::MediaId.eq(media_id))
			.order_by_asc(media_audio_chapter::Column::Index)
			.all(self.conn())
			.await?;
		Ok(Some(Self::assemble(audio, tracks, chapters)))
	}

	async fn audio_batch(
		&self,
		media_ids: &[String],
	) -> AbsResult<std::collections::HashMap<String, AbsAudio>> {
		use std::collections::HashMap;

		if media_ids.is_empty() {
			return Ok(HashMap::new());
		}
		let rows = media_audio::Entity::find()
			.filter(media_audio::Column::MediaId.is_in(media_ids.to_vec()))
			.all(self.conn())
			.await?;
		if rows.is_empty() {
			return Ok(HashMap::new());
		}
		let mut tracks: HashMap<String, Vec<media_audio_track::Model>> = HashMap::new();
		for track in media_audio_track::Entity::find()
			.filter(media_audio_track::Column::MediaId.is_in(media_ids.to_vec()))
			.order_by_asc(media_audio_track::Column::Index)
			.all(self.conn())
			.await?
		{
			tracks
				.entry(track.media_id.clone())
				.or_default()
				.push(track);
		}
		let mut chapters: HashMap<String, Vec<media_audio_chapter::Model>> =
			HashMap::new();
		for chapter in media_audio_chapter::Entity::find()
			.filter(media_audio_chapter::Column::MediaId.is_in(media_ids.to_vec()))
			.order_by_asc(media_audio_chapter::Column::Index)
			.all(self.conn())
			.await?
		{
			chapters
				.entry(chapter.media_id.clone())
				.or_default()
				.push(chapter);
		}

		Ok(rows
			.into_iter()
			.map(|audio| {
				let media_id = audio.media_id.clone();
				let tracks = tracks.remove(&media_id).unwrap_or_default();
				let chapters = chapters.remove(&media_id).unwrap_or_default();
				let audio = Self::assemble(audio, tracks, chapters);
				(media_id, audio)
			})
			.collect())
	}

	async fn progress(
		&self,
		user: &AuthUser,
		media_id: &str,
	) -> AbsResult<Option<AbsProgress>> {
		let Some(head) = self.head(&user.id, media_id).await? else {
			return Ok(None);
		};
		let duration_ms = media_audio::Entity::find_by_id(media_id)
			.one(self.conn())
			.await?
			.map(|audio| audio.duration_ms)
			.unwrap_or(0);
		Ok(Some(progress_from_head(&head, duration_ms)))
	}

	async fn clear_progress(&self, user: &AuthUser, media_id: &str) -> AbsResult<bool> {
		let book = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(media_id))
			.one(self.conn())
			.await?
			.ok_or_else(|| AbsError::NotFound(format!("No library item {media_id}")))?;
		let media_ids = vec![media_id.to_owned()];
		let txn = models::txn::begin_write(self.conn()).await?;
		let removed = reading_state_service::clear(
			&txn,
			&user.id,
			&media_ids,
			stump_core::reading_state::SourceProtocol::Abs,
			None,
		)
		.await?;
		txn.commit().await?;
		if removed > 0 {
			stump_core::reading_state::announce_cleared(
				self.ctx.as_ref(),
				&user.id,
				&book,
				stump_core::reading_state::SourceProtocol::Abs,
			);
		}
		Ok(removed > 0)
	}

	async fn progress_all(
		&self,
		user: &AuthUser,
	) -> AbsResult<Vec<(String, AbsProgress)>> {
		let heads = reading_head::Entity::find()
			.filter(reading_head::Column::UserId.eq(user.id.clone()))
			.order_by_desc(reading_head::Column::UpdatedAt)
			.all(self.conn())
			.await?;
		if heads.is_empty() {
			return Ok(Vec::new());
		}
		let media_ids = heads
			.iter()
			.map(|head| head.media_id.clone())
			.collect::<Vec<_>>();
		let durations = media_audio::Entity::find()
			.filter(media_audio::Column::MediaId.is_in(media_ids))
			.all(self.conn())
			.await?
			.into_iter()
			.map(|audio| (audio.media_id, audio.duration_ms))
			.collect::<std::collections::HashMap<_, _>>();

		Ok(heads
			.into_iter()
			.map(|head| {
				let duration_ms =
					durations.get(&head.media_id).copied().unwrap_or_default();
				let media_id = head.media_id.clone();
				(media_id, progress_from_head(&head, duration_ms))
			})
			.collect())
	}

	async fn apply_position(
		&self,
		user: &AuthUser,
		media_id: &str,
		update: AbsPositionUpdate,
	) -> AbsResult<bool> {
		let book = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(media_id))
			.one(self.conn())
			.await?
			.ok_or_else(|| AbsError::NotFound(format!("No library item {media_id}")))?;

		let progression = (update.duration_ms > 0).then(|| {
			(update.position_ms as f64 / update.duration_ms as f64).clamp(0.0, 1.0)
		});
		let publication = stump_core::reading_state::Publication::from(&book);
		let publication = if update.duration_ms > 0 {
			publication.with_duration_ms(update.duration_ms)
		} else {
			publication
		};

		let txn = models::txn::begin_write(self.conn()).await?;
		let applied = reading_state_service::apply(
			&txn,
			&user.id,
			publication,
			stump_core::reading_state::ProtocolUpdate {
				protocol: stump_core::reading_state::SourceProtocol::Abs,
				device_id: None,
				// The device clock time, for an offline session the app
				// uploaded hours after it was recorded; `None` for a live
				// sync, which stamps the server's own clock.
				updated_at: update.at,
				position: stump_core::reading_state::Position::Time {
					position_ms: update.position_ms,
					track_index: update.track_index,
				},
				progression,
				completed: update.is_finished,
				raw_payload: serde_json::json!({
					"currentTime": update.position_ms as f64 / 1000.0,
					"timeListened": update.elapsed_ms as f64 / 1000.0,
					"duration": update.duration_ms as f64 / 1000.0,
				}),
			},
		)
		.await?;

		// The listening session is what turns accepted positions into time
		// spent listening, which the reading-session history reports.
		if applied.accepted() {
			reading_progress::upsert_reading_session(
				&txn,
				user,
				media_id,
				reading_progress::NormalizedProgression {
					percentage: progression.and_then(|value| {
						sea_orm::prelude::Decimal::from_f64_retain(value)
					}),
					elapsed_seconds_delta: Some(update.elapsed_ms / 1000),
					did_complete: applied.head.completed,
					..Default::default()
				},
			)
			.await?;
		}
		txn.commit().await?;

		stump_core::reading_state::announce(self.ctx.as_ref(), &book, &applied);
		Ok(applied.accepted())
	}
	async fn record_sync(&self, auth: &AuthContext, summary: serde_json::Value) {
		let Some(api_key) = auth.api_key.as_deref() else {
			return;
		};
		if let Err(error) = self
			.ctx
			.devices()
			.touch(CredentialRef::ApiKey(api_key), Protocol::Api, Some(summary))
			.await
		{
			tracing::warn!(?error, "Failed to record the ABS sync on its device");
		}
	}

	async fn bookmarks(
		&self,
		user: &AuthUser,
		media_id: &str,
	) -> AbsResult<Vec<AbsBookmark>> {
		Ok(audio_bookmarks(self.conn(), &user.id, Some(media_id))
			.await?
			.into_iter()
			.map(|(_, bookmark)| bookmark)
			.collect())
	}

	async fn bookmarks_all(
		&self,
		user: &AuthUser,
	) -> AbsResult<Vec<(String, AbsBookmark)>> {
		audio_bookmarks(self.conn(), &user.id, None).await
	}

	async fn upsert_bookmark(
		&self,
		user: &AuthUser,
		media_id: &str,
		position_ms: i64,
		title: &str,
	) -> AbsResult<AbsBookmark> {
		use models::entity::bookmark;

		let existing = bookmark::Entity::find()
			.filter(bookmark::Column::UserId.eq(user.id.clone()))
			.filter(bookmark::Column::MediaId.eq(media_id))
			.filter(bookmark::Column::PositionMs.eq(position_ms))
			.one(self.conn())
			.await?;

		let row = match existing {
			Some(row) => {
				let created_at = row.created_at;
				let mut active: bookmark::ActiveModel = row.into();
				active.preview_content =
					sea_orm::ActiveValue::Set(Some(title.to_owned()));
				let updated = active.update(self.conn()).await?;
				bookmark::Model {
					created_at,
					..updated
				}
			},
			None => {
				bookmark::ActiveModel {
					user_id: sea_orm::ActiveValue::Set(user.id.clone()),
					media_id: sea_orm::ActiveValue::Set(media_id.to_owned()),
					position_ms: sea_orm::ActiveValue::Set(Some(position_ms)),
					preview_content: sea_orm::ActiveValue::Set(Some(title.to_owned())),
					created_at: sea_orm::ActiveValue::Set(Utc::now().into()),
					..Default::default()
				}
				.insert(self.conn())
				.await?
			},
		};

		Ok(AbsBookmark {
			position_ms,
			title: row.preview_content.unwrap_or_default(),
			created_at: row.created_at.to_utc(),
		})
	}

	async fn delete_bookmark(
		&self,
		user: &AuthUser,
		media_id: &str,
		position_ms: i64,
	) -> AbsResult<()> {
		use models::entity::bookmark;

		bookmark::Entity::delete_many()
			.filter(bookmark::Column::UserId.eq(user.id.clone()))
			.filter(bookmark::Column::MediaId.eq(media_id))
			.filter(bookmark::Column::PositionMs.eq(position_ms))
			.exec(self.conn())
			.await?;
		Ok(())
	}

	async fn cover(
		&self,
		user: Option<&AuthUser>,
		media_id: &str,
	) -> AbsResult<AbsImage> {
		// From 2.17.0 the official app sends no credential with an image
		// request; the route has already established the row is an audible,
		// undeleted book, so the anonymous lane reads the row directly.
		let image = match user {
			Some(user) => api_media::get_media_thumbnail_by_id(
				self.ctx.as_ref(),
				user,
				media_id.to_owned(),
			)
			.await
			.map_err(map_server_error)?,
			None => api_media::get_media_thumbnail_unscoped(self.ctx.as_ref(), media_id)
				.await
				.map_err(map_server_error)?,
		};
		Ok(AbsImage {
			content_type: image.content_type.to_string(),
			data: image.data,
		})
	}

	async fn serve_track(
		&self,
		headers: HeaderMap,
		media_id: &str,
		track_index: i32,
		device_id: Option<&str>,
	) -> AbsResult<Response<Body>> {
		let track = media_audio_track::Entity::find()
			.filter(media_audio_track::Column::MediaId.eq(media_id))
			.filter(media_audio_track::Column::Index.eq(track_index))
			.one(self.conn())
			.await?
			.ok_or_else(|| {
				AbsError::NotFound(format!("No file {track_index} for {media_id}"))
			})?;

		// The same delivery lane the native audio route uses, so a device's
		// transform preset applies whichever protocol its player speaks.
		let profile = audio_transform::resolve_audio_profile(&self.ctx, device_id).await;
		audio_transform::serve_track(&self.ctx, headers, &track, &profile)
			.await
			.map(|response| response.map(Body::new))
			.map_err(|error| {
				AbsError::InternalServerError(format!(
					"Failed to serve {}: {error}",
					track.path
				))
			})
	}

	async fn create_session(&self, session: AbsSession) -> AbsResult<()> {
		Ok(AbsSessions::insert(self.conn(), &session).await?)
	}

	async fn session(
		&self,
		user_id: &str,
		session_id: &str,
	) -> AbsResult<Option<AbsSession>> {
		Ok(AbsSessions::get(self.conn(), user_id, session_id).await?)
	}

	async fn sessions(
		&self,
		user_id: &str,
		media_id: Option<&str>,
	) -> AbsResult<Vec<AbsSession>> {
		Ok(AbsSessions::list(self.conn(), user_id, media_id).await?)
	}

	async fn update_session(
		&self,
		session_id: &str,
		current_time_ms: i64,
		time_listening_ms: i64,
	) -> AbsResult<()> {
		Ok(AbsSessions::update(
			self.conn(),
			session_id,
			current_time_ms,
			time_listening_ms,
		)
		.await?)
	}

	async fn close_session(&self, session_id: &str) -> AbsResult<()> {
		Ok(AbsSessions::close(self.conn(), session_id).await?)
	}

	async fn book_ids(
		&self,
		media_ids: &[String],
	) -> AbsResult<std::collections::HashMap<String, String>> {
		Ok(AbsIds::resolve_many(self.conn(), IdKind::Book, media_ids).await?)
	}

	async fn folder_id(&self, library_id: &str) -> AbsResult<String> {
		Ok(AbsIds::resolve(self.conn(), IdKind::Folder, library_id).await?)
	}

	async fn author_ids(
		&self,
		names: &[String],
	) -> AbsResult<std::collections::HashMap<String, String>> {
		Ok(AbsIds::resolve_many(self.conn(), IdKind::Author, names).await?)
	}

	async fn author_name(&self, author_id: &str) -> AbsResult<Option<String>> {
		Ok(AbsIds::lookup(self.conn(), IdKind::Author, author_id).await?)
	}

	async fn session_by_id(&self, session_id: &str) -> AbsResult<Option<AbsSession>> {
		Ok(AbsSessions::get_any(self.conn(), session_id).await?)
	}

	async fn ebook_editions(
		&self,
		user: &AuthUser,
		media_ids: &[String],
	) -> AbsResult<std::collections::HashMap<String, AbsEbookFile>> {
		ebook_editions_for(self.conn(), user, media_ids).await
	}

	async fn paired_ebook_media_ids(&self, user: &AuthUser) -> AbsResult<Vec<String>> {
		let links = confirmed_links(self.conn(), &user.id, None).await?;
		let anchors = links
			.iter()
			.map(|link| link.media_id.clone())
			.collect::<Vec<_>>();
		Ok(ebook_editions_for(self.conn(), user, &anchors)
			.await?
			.into_keys()
			.collect())
	}

	async fn serve_ebook(
		&self,
		headers: HeaderMap,
		user: &AuthUser,
		ebook: &AbsEbookFile,
	) -> AbsResult<Response<Body>> {
		// The pair was resolved for this user, but the ebook row is a
		// separate media row: re-check it is one they may read before its
		// bytes leave the server.
		media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(ebook.media_id.clone()))
			.one(self.conn())
			.await?
			.ok_or_else(|| AbsError::NotFound(format!("No ebook {}", ebook.media_id)))?;
		serve_file(std::path::Path::new(&ebook.path), EPUB_MIME, headers).await
	}

	async fn download_item(
		&self,
		headers: HeaderMap,
		user: &AuthUser,
		media_id: &str,
		ebook: Option<&AbsEbookFile>,
	) -> AbsResult<Response<Body>> {
		let audio = self.audio(media_id).await?;
		let mut files = audio
			.map(|audio| {
				audio
					.tracks
					.iter()
					.map(|track| std::path::PathBuf::from(&track.path))
					.collect::<Vec<_>>()
			})
			.unwrap_or_default();
		if let Some(ebook) = ebook {
			files.push(std::path::PathBuf::from(&ebook.path));
		}
		if files.is_empty() {
			return Err(AbsError::NotFound(format!("No files for {media_id}")));
		}

		let name = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(media_id))
			.one(self.conn())
			.await?
			.map(|row| row.name)
			.unwrap_or_else(|| media_id.to_owned());
		zip_response(&self.ctx, media_id, &name, files, headers).await
	}

	async fn playlists(&self, user: &AuthUser) -> AbsResult<Vec<AbsPlaylist>> {
		let rows = reading_list::Entity::find()
			.filter(reading_list::Column::CreatingUserId.eq(user.id.clone()))
			.order_by_desc(reading_list::Column::UpdatedAt)
			.order_by_asc(reading_list::Column::Id)
			.all(self.conn())
			.await?;
		let mut playlists = Vec::with_capacity(rows.len());
		for row in rows {
			playlists.push(self.hydrate_playlist(row).await?);
		}
		Ok(playlists)
	}

	async fn playlist(
		&self,
		user: &AuthUser,
		playlist_id: &str,
	) -> AbsResult<Option<AbsPlaylist>> {
		let Some(row) = reading_list::Entity::find_by_id(playlist_id.to_owned())
			.filter(reading_list::Column::CreatingUserId.eq(user.id.clone()))
			.one(self.conn())
			.await?
		else {
			return Ok(None);
		};
		Ok(Some(self.hydrate_playlist(row).await?))
	}

	async fn create_playlist(
		&self,
		user: &AuthUser,
		name: &str,
		description: Option<&str>,
		media_ids: &[String],
	) -> AbsResult<AbsPlaylist> {
		// The same mutation path the Komga read-list routes take, so a
		// playlist created in the Audiobookshelf app is the same row a Komga
		// client, a Kobo shelf and the web UI see.
		let row = stump_collections::create_read_list(
			self.ctx.as_ref(),
			user,
			stump_collections::ReadListCreate {
				name: name.to_owned(),
				summary: description.map(str::to_owned),
				// Audiobookshelf playlists are ordered.
				ordered: true,
				book_ids: media_ids.to_vec(),
			},
		)
		.await
		.map_err(|error| map_server_error(APIError::from(error)))?;
		self.hydrate_playlist(row).await
	}

	async fn update_playlist(
		&self,
		user: &AuthUser,
		playlist_id: &str,
		name: Option<&str>,
		description: Option<Option<&str>>,
		media_ids: Option<&[String]>,
	) -> AbsResult<AbsPlaylist> {
		stump_collections::update_read_list(
			self.ctx.as_ref(),
			user,
			playlist_id,
			stump_collections::ReadListUpdate {
				name: name.map(str::to_owned),
				summary: description.map(|value| value.map(str::to_owned)),
				ordered: None,
				book_ids: media_ids.map(<[String]>::to_vec),
			},
		)
		.await
		.map_err(|error| map_server_error(APIError::from(error)))?;
		self.playlist(user, playlist_id)
			.await?
			.ok_or_else(|| AbsError::NotFound(format!("No playlist {playlist_id}")))
	}

	async fn delete_playlist(&self, user: &AuthUser, playlist_id: &str) -> AbsResult<()> {
		stump_collections::delete_read_list(self.ctx.as_ref(), user, playlist_id)
			.await
			.map_err(|error| map_server_error(APIError::from(error)))
	}
}

/// What an EPUB is served as; the app's reader switches on `ebookFormat`,
/// but a browser and ExoPlayer both go by the content type.
const EPUB_MIME: &str = "application/epub+zip";

/// The extensions this profile will report as `media.ebookFile`.
///
/// The official app has readers for epub, mobi/azw3, pdf and cbz/cbr
/// (`components/readers/Reader.vue:300-318`), but only the EPUB one is a
/// paired *edition* in Stump's model, and only EPUB is what the ebook route
/// streams; anything else in a work stays a Stump book and no ABS ebook.
const EBOOK_EXTENSIONS: [&str; 1] = ["epub"];

/// The user's confirmed pair links, optionally narrowed to a set of works.
async fn confirmed_links(
	conn: &DatabaseConnection,
	user_id: &str,
	work_ids: Option<&[String]>,
) -> AbsResult<Vec<liseur_sync_media_link::Model>> {
	let mut select = liseur_sync_media_link::Entity::find()
		.filter(liseur_sync_media_link::Column::UserId.eq(user_id))
		.filter(
			liseur_sync_media_link::Column::PairStatus
				.eq(PairStatus::Confirmed.to_string()),
		);
	if let Some(work_ids) = work_ids {
		select = select
			.filter(liseur_sync_media_link::Column::WorkId.is_in(work_ids.to_vec()));
	}
	Ok(select
		.order_by_asc(liseur_sync_media_link::Column::CreatedAt)
		.order_by_asc(liseur_sync_media_link::Column::Id)
		.all(conn)
		.await?)
}

/// The confirmed EPUB edition of each of `media_ids`, in two queries however
/// long the page is.
///
/// This is the batched form of
/// `models::domain::edition_pair::linked_media(.., Some(PairStatus::Confirmed))`
/// and keeps its rule: only a `confirmed` link is a pair, a `rejected` one is
/// never surfaced, and a `suggested` one must never reach an ABS client.
async fn ebook_editions_for(
	conn: &DatabaseConnection,
	user: &AuthUser,
	media_ids: &[String],
) -> AbsResult<std::collections::HashMap<String, AbsEbookFile>> {
	use std::collections::HashMap;

	if media_ids.is_empty() {
		return Ok(HashMap::new());
	}

	let anchors = liseur_sync_media_link::Entity::find()
		.filter(liseur_sync_media_link::Column::UserId.eq(user.id.clone()))
		.filter(liseur_sync_media_link::Column::MediaId.is_in(media_ids.to_vec()))
		.filter(
			liseur_sync_media_link::Column::PairStatus
				.ne(PairStatus::Rejected.to_string()),
		)
		.all(conn)
		.await?;
	if anchors.is_empty() {
		return Ok(HashMap::new());
	}

	let work_ids = anchors
		.iter()
		.map(|link| link.work_id.clone())
		.collect::<std::collections::HashSet<_>>()
		.into_iter()
		.collect::<Vec<_>>();
	let counterparts = confirmed_links(conn, &user.id, Some(&work_ids)).await?;

	// One `media` read for every counterpart, narrowed to rows the user may
	// see and to the ebook extensions this profile serves.
	let counterpart_ids = counterparts
		.iter()
		.map(|link| link.media_id.clone())
		.collect::<Vec<_>>();
	let rows = media::Entity::find_for_user(user)
		.filter(media::Column::Id.is_in(counterpart_ids))
		.filter(media::Column::DeletedAt.is_null())
		.all(conn)
		.await?
		.into_iter()
		.filter(|row| EBOOK_EXTENSIONS.contains(&row.extension.to_lowercase().as_str()))
		.map(|row| (row.id.clone(), row))
		.collect::<HashMap<_, _>>();

	let mut out = HashMap::new();
	for anchor in &anchors {
		let ebook = counterparts
			.iter()
			.filter(|link| {
				link.work_id == anchor.work_id && link.media_id != anchor.media_id
			})
			.find_map(|link| rows.get(&link.media_id));
		if let Some(row) = ebook {
			out.insert(
				anchor.media_id.clone(),
				AbsEbookFile {
					media_id: row.id.clone(),
					path: row.path.clone(),
					format: row.extension.to_lowercase(),
					byte_size: row.size,
				},
			);
		}
	}
	Ok(out)
}

/// Serve one file, honouring `Range`.
///
/// `ServeFile` answers `206` with a `Content-Range` for a ranged request and
/// sets `Accept-Ranges: bytes` either way, which is what an EPUB reader
/// streaming a spine item needs.
async fn serve_file(
	path: &std::path::Path,
	mime: &str,
	headers: HeaderMap,
) -> AbsResult<Response<Body>> {
	// `ServeFile::try_call` is inherent in tower-http 0.5; no `ServiceExt`.
	use tower_http::services::ServeFile;

	let mut request = axum::extract::Request::new(Body::empty());
	*request.headers_mut() = headers;
	let mut response = ServeFile::new(path)
		.try_call(request)
		.await
		.map_err(|error| {
			tracing::error!(?error, ?path, "Failed to serve file");
			AbsError::NotFound("File is missing".to_owned())
		})?
		.map(Body::new);
	if let Ok(value) = mime.parse::<header::HeaderValue>() {
		response.headers_mut().insert(header::CONTENT_TYPE, value);
	}
	Ok(response)
}

/// Build (or reuse) the item's zip in the transform cache and serve it.
///
/// `zip` 1.1.3 patches each local header after writing its data
/// (`write.rs:1520`), so the archive is written to a seekable file rather
/// than streamed; the transform cache is where the server already keeps
/// expensive, reproducible artifacts of files it owns, so a second download
/// — or a resumed one — is a `ServeFile` over a finished archive with
/// working `Range` support.
///
/// Entries are **stored**, not deflated: audio and EPUB are already
/// compressed. abs-ref's own zip of three 10 450-byte MP3s is 31 738 bytes,
/// which is stored plus headers.
async fn zip_response(
	ctx: &AppState,
	media_id: &str,
	name: &str,
	files: Vec<std::path::PathBuf>,
	headers: HeaderMap,
) -> AbsResult<Response<Body>> {
	let cache = TransformCache::new(
		ctx.config.get_transform_cache_dir(),
		ctx.config.transform.transform_cache_max_bytes,
	);
	// Invalidated by the newest source mtime and the file count, which is
	// what changes when a book is re-scanned, re-tagged or re-paired.
	let stamp = newest_mtime_nanos(&files).await;
	let path = cache
		.dir()
		.join(format!("{media_id}-dl{}-{stamp}.zip", files.len()));

	if !cache.hit(&path) {
		let temp = path.with_extension("zip.part");
		let dir = cache.dir().to_path_buf();
		let build = tokio::task::spawn_blocking({
			let temp = temp.clone();
			let path = path.clone();
			move || -> std::io::Result<()> {
				std::fs::create_dir_all(&dir)?;
				let mut writer = zip::ZipWriter::new(std::fs::File::create(&temp)?);
				let options = zip::write::SimpleFileOptions::default()
					.compression_method(zip::CompressionMethod::Stored)
					.large_file(true);
				for source in &files {
					let entry = source
						.file_name()
						.and_then(|name| name.to_str())
						.unwrap_or("file")
						.to_owned();
					writer.start_file(entry, options)?;
					std::io::copy(&mut std::fs::File::open(source)?, &mut writer)?;
				}
				writer.finish()?.sync_all()?;
				TransformCache::publish(&temp, &path)
			}
		})
		.await;

		match build {
			Ok(Ok(())) => {
				if let Err(error) = cache.sweep() {
					tracing::warn!(%error, "Failed to sweep the transform cache");
				}
			},
			Ok(Err(error)) => {
				let _ = tokio::fs::remove_file(&temp).await;
				return Err(AbsError::InternalServerError(format!(
					"Failed to build the download for {media_id}: {error}"
				)));
			},
			Err(error) => {
				let _ = tokio::fs::remove_file(&temp).await;
				return Err(AbsError::InternalServerError(format!(
					"The download task for {media_id} panicked: {error}"
				)));
			},
		}
	}

	let mut response = serve_file(&path, "application/zip", headers).await?;
	let disposition = format!(
		"attachment; filename=\"{}.zip\"",
		name.replace('\\', r"\\").replace('"', "\\\"")
	);
	if let Ok(value) = disposition.parse::<header::HeaderValue>() {
		response
			.headers_mut()
			.insert(header::CONTENT_DISPOSITION, value);
	}
	Ok(response)
}

/// The newest source mtime in nanoseconds, `0` when none can be read.
async fn newest_mtime_nanos(files: &[std::path::PathBuf]) -> u128 {
	let mut newest = 0u128;
	for path in files {
		if let Ok(meta) = tokio::fs::metadata(path).await {
			if let Ok(modified) = meta.modified() {
				if let Ok(since) = modified.duration_since(std::time::UNIX_EPOCH) {
					newest = newest.max(since.as_nanos());
				}
			}
		}
	}
	newest
}

/// The user's audio bookmarks — the rows that carry a millisecond position —
/// newest position first, optionally for one book.
async fn audio_bookmarks(
	conn: &DatabaseConnection,
	user_id: &str,
	media_id: Option<&str>,
) -> AbsResult<Vec<(String, AbsBookmark)>> {
	use models::entity::bookmark;

	let mut select = bookmark::Entity::find()
		.filter(bookmark::Column::UserId.eq(user_id))
		.filter(bookmark::Column::PositionMs.is_not_null())
		.order_by_asc(bookmark::Column::PositionMs);
	if let Some(media_id) = media_id {
		select = select.filter(bookmark::Column::MediaId.eq(media_id));
	}

	Ok(select
		.all(conn)
		.await?
		.into_iter()
		.filter_map(|row| {
			let position_ms = row.position_ms?;
			Some((
				row.media_id.clone(),
				AbsBookmark {
					position_ms,
					title: row.preview_content.clone().unwrap_or_default(),
					created_at: row.created_at.to_utc(),
				},
			))
		})
		.collect())
}

#[cfg(test)]
mod tests {
	use super::*;
	use sea_orm::{DatabaseBackend, MockDatabase};

	/// Axum panics on overlapping routes when the router is built, and this
	/// profile adds routes at the server root next to `/api`; catch a
	/// collision here rather than at server start.
	#[tokio::test]
	async fn abs_router_composes_without_route_collisions() {
		let ctx = Arc::new(stump_core::Ctx::mock_sea(MockDatabase::new(
			DatabaseBackend::Sqlite,
		)));
		let backend: Arc<dyn AbsBackend> = Arc::new(AbsBackendAdapter::new(ctx.clone()));
		let _router: Router<()> =
			compose(ctx.clone(), backend, AbsEvents::new()).with_state(ctx);
	}

	#[test]
	fn token_query_is_case_insensitive_and_decoded() {
		// abs-ref accepts `?token=`; a cast receiver cannot set headers.
		assert_eq!(
			token_query(Some("raw=1&token=abc%2Bdef")),
			Some("abc+def".to_owned())
		);
		assert_eq!(token_query(Some("TOKEN=x")), Some("x".to_owned()));
		assert_eq!(token_query(Some("token=")), None);
		assert_eq!(token_query(None), None);
	}

	#[test]
	fn basic_credentials_split_on_the_first_colon() {
		let header = format!(
			"Basic {}",
			base64::engine::general_purpose::STANDARD.encode("ada:pa:ss")
		);
		assert_eq!(
			basic_credentials(&header),
			Some(("ada".to_owned(), "pa:ss".to_owned()))
		);
		assert_eq!(basic_credentials("Bearer abc"), None);
		assert_eq!(basic_credentials("Basic !!!"), None);
	}
}
