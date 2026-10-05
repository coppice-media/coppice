//! Live change notifications for guest reader sessions.
//!
//! A stream carries only *which kinds* of session state changed, never progress,
//! annotation, or message content: the client refetches through the authorized
//! GETs. Every notification and a periodic timer re-run the full capability
//! check, so a revoked, rotated, expired, or closed capability (or a lost
//! issuer/publisher) ends its stream with `revoked`.

use std::{
	collections::HashMap,
	convert::Infallible,
	ops::{BitOr, BitOrAssign},
	sync::{
		atomic::{AtomicU64, Ordering},
		Arc,
	},
	time::Duration,
};

use axum::{
	extract::{Path as AxumPath, State},
	http::HeaderMap,
	response::sse::{Event, KeepAlive, Sse},
	Extension,
};
use futures_util::Stream;
use parking_lot::Mutex;
use serde_json::json;
use tokio::{
	sync::broadcast::{
		self,
		error::{RecvError, TryRecvError},
	},
	time::{interval_at, Instant, MissedTickBehavior},
};

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
};

use super::{media as reader_media, reader, security};

const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);
/// How often an idle stream re-runs the capability check, which is also how a
/// change made outside these routes (queue reorder, role or account loss) is
/// noticed.
const RECHECK_INTERVAL: Duration = Duration::from_secs(30);
const MAX_STREAMS_PER_PARTICIPANT: usize = 3;
/// Notifications are tiny and merged on receipt; a lagging stream reports
/// every kind as changed.
const CHANNEL_CAPACITY: usize = 16;

/// A set of changed state kinds; notifications merge by union.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Kinds(u8);

impl Kinds {
	/// Changes nothing a viewer sees, but still makes every stream of the
	/// session re-run its capability check (credential rotation).
	pub(super) const NONE: Self = Self(0);
	pub(super) const PROGRESS: Self = Self(1);
	pub(super) const ANNOTATIONS: Self = Self(1 << 1);
	pub(super) const PARTICIPANTS: Self = Self(1 << 2);
	pub(super) const PUBLICATION: Self = Self(1 << 3);
	pub(super) const MESSAGES: Self = Self(1 << 4);
	const ALL: Self = Self(0b1_1111);
	const NAMES: [(Self, &'static str); 5] = [
		(Self::PROGRESS, "progress"),
		(Self::ANNOTATIONS, "annotations"),
		(Self::PARTICIPANTS, "participants"),
		(Self::PUBLICATION, "publication"),
		(Self::MESSAGES, "messages"),
	];

	pub(super) fn is_empty(self) -> bool {
		self.0 == 0
	}

	fn names(self) -> Vec<&'static str> {
		Self::NAMES
			.iter()
			.filter(|(kind, _)| self.0 & kind.0 != 0)
			.map(|(_, name)| *name)
			.collect()
	}
}

impl BitOr for Kinds {
	type Output = Self;

	fn bitor(self, other: Self) -> Self {
		Self(self.0 | other.0)
	}
}

impl BitOrAssign for Kinds {
	fn bitor_assign(&mut self, other: Self) {
		self.0 |= other.0;
	}
}

/// The in-process fan-out of session change notifications, shared by the guest
/// and management routers as a router extension.
#[derive(Default)]
pub(super) struct ReaderEvents {
	/// One channel per session that has at least one open stream.
	sessions: Mutex<HashMap<String, broadcast::Sender<Kinds>>>,
	/// Open streams per participant, capped at [`MAX_STREAMS_PER_PARTICIPANT`].
	streams: Mutex<HashMap<String, usize>>,
	/// The last SSE event id handed out in this process.
	last_event_id: AtomicU64,
}

impl ReaderEvents {
	/// Notify the session's open streams. Call only after the change committed.
	pub(super) fn publish(&self, session_id: &str, kinds: Kinds) {
		let mut sessions = self.sessions.lock();
		if let Some(sender) = sessions.get(session_id) {
			if sender.send(kinds).is_err() {
				sessions.remove(session_id);
			}
		}
	}

	/// Take one of the participant's stream slots and subscribe to the session;
	/// `None` when the participant already has the maximum open.
	fn subscribe(
		self: &Arc<Self>,
		session_id: &str,
		participant_id: &str,
	) -> Option<Subscription> {
		{
			let mut streams = self.streams.lock();
			let open = streams.entry(participant_id.to_owned()).or_default();
			if *open >= MAX_STREAMS_PER_PARTICIPANT {
				return None;
			}
			*open += 1;
		}
		let receiver = self
			.sessions
			.lock()
			.entry(session_id.to_owned())
			.or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0)
			.subscribe();
		Some(Subscription {
			events: Arc::clone(self),
			session_id: session_id.to_owned(),
			participant_id: participant_id.to_owned(),
			receiver: Some(receiver),
		})
	}

	fn next_event_id(&self) -> String {
		(self.last_event_id.fetch_add(1, Ordering::Relaxed) + 1).to_string()
	}
}

/// One open stream's receiver and slot; dropping it releases both and drops
/// the session channel once nobody listens.
struct Subscription {
	events: Arc<ReaderEvents>,
	session_id: String,
	participant_id: String,
	/// Always `Some` until [`Drop`], which must release it before pruning.
	receiver: Option<broadcast::Receiver<Kinds>>,
}

impl Subscription {
	/// The next notification merged with any already queued behind it; `None`
	/// once the channel is gone.
	async fn next(&mut self) -> Option<Kinds> {
		let receiver = self.receiver.as_mut()?;
		let mut kinds = match receiver.recv().await {
			Ok(kinds) => kinds,
			Err(RecvError::Lagged(_)) => Kinds::ALL,
			Err(RecvError::Closed) => return None,
		};
		loop {
			match receiver.try_recv() {
				Ok(more) => kinds |= more,
				Err(TryRecvError::Lagged(_)) => kinds |= Kinds::ALL,
				Err(TryRecvError::Empty | TryRecvError::Closed) => return Some(kinds),
			}
		}
	}
}

impl Drop for Subscription {
	fn drop(&mut self) {
		drop(self.receiver.take());
		{
			let mut sessions = self.events.sessions.lock();
			if sessions
				.get(&self.session_id)
				.is_some_and(|sender| sender.receiver_count() == 0)
			{
				sessions.remove(&self.session_id);
			}
		}
		let mut streams = self.events.streams.lock();
		if let Some(open) = streams.get_mut(&self.participant_id) {
			*open = open.saturating_sub(1);
			if *open == 0 {
				streams.remove(&self.participant_id);
			}
		}
	}
}

/// `GET /sessions/{sessionId}/events`: `changed` (`{"kinds": [...]}`) whenever
/// the session's visible state changes, then `revoked` and the end of the stream
/// once the capability stops authorizing. Pausing (the published book is no
/// longer the queue head) is a `publication` change, not a revocation.
pub(super) async fn stream(
	AxumPath(session_id): AxumPath<String>,
	State(ctx): State<AppState>,
	Extension(events): Extension<Arc<ReaderEvents>>,
	headers: HeaderMap,
) -> APIResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
	let secret = security::cookie_token(&headers)?;
	let access = reader::load_access_from_secret(&ctx, &session_id, &secret).await?;
	let mut subscription = events
		.subscribe(&access.session.id, &access.participant.id)
		.ok_or(APIError::TooManyRequests)?;
	let mut live_book = reader_media::live_book_id(
		ctx.conn.as_ref(),
		&access.session.book_club_id,
		access.session.published_book_id.as_deref(),
	)
	.await?;

	let stream = async_stream::stream! {
		let mut recheck =
			interval_at(Instant::now() + RECHECK_INTERVAL, RECHECK_INTERVAL);
		recheck.set_missed_tick_behavior(MissedTickBehavior::Delay);
		loop {
			let mut kinds = tokio::select! {
				_ = recheck.tick() => Kinds::NONE,
				next = subscription.next() => match next {
					Some(kinds) => kinds,
					None => break,
				},
			};
			let access =
				match reader::load_access_from_secret(&ctx, &session_id, &secret).await {
					Ok(access) => access,
					Err(APIError::Unauthorized) => {
						yield Ok::<Event, Infallible>(
							Event::default()
								.id(events.next_event_id())
								.event("revoked")
								.data("{}"),
						);
						break;
					},
					Err(error) => {
						tracing::warn!(?error, "Club reader event stream recheck failed");
						break;
					},
				};
			match reader_media::live_book_id(
				ctx.conn.as_ref(),
				&access.session.book_club_id,
				access.session.published_book_id.as_deref(),
			)
			.await
			{
				Ok(current) if current != live_book => {
					kinds |= Kinds::PUBLICATION;
					live_book = current;
				},
				Ok(_) => {},
				Err(error) => {
					tracing::warn!(?error, "Club reader event stream recheck failed");
					break;
				},
			}
			if !kinds.is_empty() {
				yield Ok(
					Event::default()
						.id(events.next_event_id())
						.event("changed")
						.data(json!({ "kinds": kinds.names() }).to_string()),
				);
			}
		}
	};
	Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(KEEP_ALIVE_INTERVAL)))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn kinds_serialize_in_contract_order() {
		let kinds = Kinds::MESSAGES | Kinds::PROGRESS | Kinds::PUBLICATION;
		assert_eq!(kinds.names(), ["progress", "publication", "messages"]);
		assert!(Kinds::NONE.is_empty());
		assert_eq!(Kinds::ALL.names().len(), Kinds::NAMES.len());
	}

	#[tokio::test]
	async fn streams_are_capped_per_participant_and_channels_pruned() {
		let events = Arc::new(ReaderEvents::default());
		let held = (0..MAX_STREAMS_PER_PARTICIPANT)
			.map(|_| events.subscribe("session", "participant").expect("slot"))
			.collect::<Vec<_>>();
		assert!(events.subscribe("session", "participant").is_none());
		let mut other = events.subscribe("session", "other").expect("own slots");

		events.publish("session", Kinds::PROGRESS);
		events.publish("session", Kinds::MESSAGES);
		assert_eq!(other.next().await, Some(Kinds::PROGRESS | Kinds::MESSAGES));

		drop(held);
		assert!(events.subscribe("session", "participant").is_some());
		drop(other);
		assert!(events.sessions.lock().is_empty());
		assert!(events.streams.lock().is_empty());
	}
}
