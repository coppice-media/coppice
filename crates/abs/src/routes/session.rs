//! Play sessions: the read, the position sync, the close, the listening
//! history and the offline merge.
//!
//! A client syncs against the session id it was handed by
//! `POST /api/items/{id}/play`, not against the item
//! (Lissen `common/api/library/AudioBookshelfLibrarySyncService.kt:17-28`
//! passes the session id into `POST /api/session/{id}/sync`), so the session
//! row is what maps a sync back onto a book. `timeListened` is a delta in
//! seconds since the previous sync, which accumulates on the session, while
//! `currentTime` is an absolute position that lands on the unified reading
//! state.
//!
//! The official app also plays **offline**, from a downloaded copy, and
//! uploads whole sessions afterwards (`POST /api/session/local` per session,
//! `/local-all` for the backlog; `server/ApiHandler.kt:644,742`). Those
//! carry the device clock time of the position, which is what decides
//! whether the upload still moves the head: see [`merge_local`].

use std::collections::{BTreeMap, HashMap};

use axum::{
	extract::{Json, Path},
	response::Response,
	Extension,
};
use chrono::Utc;
use models::entity::{media, user::AuthUser};
use sea_orm::{ColumnTrait, QueryFilter};
use stump_auth::AuthContext;

use crate::{
	dto::*,
	errors::{AbsError, AbsResult},
	mapper::{self, SessionShape},
	model::{AbsPositionUpdate, ItemShape},
	routes::{query, AbsBackend, AbsSession, Backend, User, PLAY_METHOD_LOCAL},
};

async fn load(
	backend: &dyn AbsBackend,
	user_id: &str,
	session_id: &str,
) -> AbsResult<AbsSession> {
	backend
		.session(user_id, session_id)
		.await?
		.ok_or_else(|| AbsError::NotFound(format!("No session {session_id}")))
}

/// `GET /api/session/{id}`.
pub(crate) async fn detail(
	backend: Backend,
	Extension(user): User,
	Path(session_id): Path<String>,
) -> AbsResult<Json<PlaybackSessionDto>> {
	let session = load(&**backend, &user.id, &session_id).await?;
	let audio = backend
		.audio(&session.media_id)
		.await?
		.ok_or_else(|| AbsError::NotFound(format!("No audio for {session_id}")))?;

	let row = query::media_for_user(&**backend, &user, &session.media_id).await?;
	let rows = [row];
	let context = query::context(&**backend, &user, &rows, false).await?;
	let item = context.item(&rows[0], ItemShape::Expanded, &user.id, None);

	Ok(Json(mapper::session_dto(mapper::SessionInput {
		session_id: &session.id,
		user_id: &user.id,
		item,
		audio: &audio,
		device: DeviceInfoRequestDto {
			device_id: session.device_id.clone(),
			client_name: session.client_name.clone(),
			client_version: session.client_version.clone(),
			..Default::default()
		},
		media_player: session.media_player.clone(),
		current_time_ms: session.current_time_ms,
		time_listening_ms: session.time_listening_ms,
		started_at: session.started_at,
		updated_at: session.updated_at,
		play_method: session.play_method,
		shape: SessionShape::Full,
	})))
}

/// Apply one `{currentTime, timeListened, duration}` report to both the
/// session row and the unified reading state.
async fn apply(
	backend: &dyn AbsBackend,
	user: &models::entity::user::AuthUser,
	session: &AbsSession,
	body: ProgressSyncRequestDto,
) -> AbsResult<(bool, i64)> {
	let audio = backend.audio(&session.media_id).await?;
	// The client's own `duration` wins when it sends one: a transcoding
	// client can report a length the server's probe never saw.
	let duration_ms = body
		.duration
		.map(mapper::secs_to_ms)
		.or_else(|| audio.as_ref().map(|audio| audio.duration_ms))
		.unwrap_or(0);
	let position_ms = body
		.current_time
		.map(mapper::secs_to_ms)
		.unwrap_or(session.current_time_ms);
	// `timeListened` is a per-sync delta; a client that omits it (or sends a
	// negative one after a clock change) must not rewind the total.
	let elapsed_ms = body
		.time_listened
		.map(mapper::secs_to_ms)
		.unwrap_or(0)
		.max(0);
	let track_index = audio
		.as_ref()
		.and_then(|audio| audio.track_at(position_ms).map(|track| track.index));

	let applied = backend
		.apply_position(
			user,
			&session.media_id,
			AbsPositionUpdate {
				position_ms,
				track_index,
				duration_ms,
				elapsed_ms,
				is_finished: None,
				// A live sync is happening now: the server's clock is the
				// honest source time, unlike an uploaded offline session.
				at: None,
			},
		)
		.await?;
	if applied {
		backend
			.update_session(
				&session.id,
				position_ms,
				session.time_listening_ms.saturating_add(elapsed_ms),
			)
			.await?;
	}
	Ok((applied, position_ms))
}
async fn record_sync(
	backend: &dyn AbsBackend,
	auth: &AuthContext,
	session: &AbsSession,
	position_ms: i64,
) {
	backend
		.record_sync(
			auth,
			serde_json::json!({
				"protocol": "abs",
				"profile": "abs",
				"session_id": session.id.clone(),
				"media_id": session.media_id.clone(),
				"position_ms": position_ms,
			}),
		)
		.await;
}

pub(crate) async fn sync(
	backend: Backend,
	Extension(user): User,
	Extension(auth): Extension<AuthContext>,
	Path(session_id): Path<String>,
	body: Option<Json<ProgressSyncRequestDto>>,
) -> AbsResult<Response> {
	let session = load(&**backend, &user.id, &session_id).await?;
	let (applied, position_ms) = apply(
		&**backend,
		&user,
		&session,
		body.map(|Json(body)| body).unwrap_or_default(),
	)
	.await?;
	if applied {
		record_sync(&**backend, &auth, &session, position_ms).await;
	}
	Ok(crate::routes::ok_text())
}

/// `POST /api/session/{id}/close`: the same update, then the session is
/// closed. abs-ref accepts a body here and Lissen sends none.
pub(crate) async fn close(
	backend: Backend,
	Extension(user): User,
	Extension(auth): Extension<AuthContext>,
	Path(session_id): Path<String>,
	body: Option<Json<ProgressSyncRequestDto>>,
) -> AbsResult<Response> {
	let session = load(&**backend, &user.id, &session_id).await?;
	let body = body.map(|Json(body)| body).unwrap_or_default();
	// A close with no position must not reset the session to zero.
	let sync = if body != ProgressSyncRequestDto::default() {
		Some(apply(&**backend, &user, &session, body).await?)
	} else {
		None
	};
	backend.close_session(&session.id).await?;
	if let Some((applied, position_ms)) = sync {
		if applied {
			record_sync(&**backend, &auth, &session, position_ms).await;
		}
	}
	Ok(crate::routes::ok_text())
}
// ---------------------------------------------------------------------------
// Offline merge — POST /api/session/local, /local-all
// ---------------------------------------------------------------------------
///
/// An offline session becomes listening history only when its position is
/// accepted by the unified head; stale uploads remain provenance only.
async fn merge_local(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	body: LocalSessionDto,
) -> AbsResult<(String, bool)> {
	let session_id = body
		.id
		.clone()
		.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
	let item_id = body
		.library_item_id
		.clone()
		.ok_or_else(|| AbsError::BadRequest("No libraryItemId".to_owned()))?;
	// The same visibility funnel every other route resolves through: an
	// upload naming a book this user cannot see is a 404, not a write.
	let row = query::media_for_user(backend, user, &item_id).await?;
	let audio = backend.audio(&item_id).await?;

	let duration_ms = body
		.duration
		.map(mapper::secs_to_ms)
		.filter(|duration| *duration > 0)
		.or_else(|| audio.as_ref().map(|audio| audio.duration_ms))
		.unwrap_or(0);
	let position_ms = body.current_time.map(mapper::secs_to_ms).unwrap_or(0);
	let elapsed_ms = body
		.time_listening
		.map(mapper::secs_to_ms)
		.unwrap_or(0)
		.max(0);
	let track_index = audio
		.as_ref()
		.and_then(|audio| audio.track_at(position_ms).map(|track| track.index));
	let updated_at = body
		.updated_at
		.and_then(mapper::ms_to_utc)
		.unwrap_or_else(Utc::now);
	let started_at = body
		.started_at
		.and_then(mapper::ms_to_utc)
		.unwrap_or(updated_at);

	let synced = backend
		.apply_position(
			user,
			&item_id,
			AbsPositionUpdate {
				position_ms,
				track_index,
				duration_ms,
				elapsed_ms,
				is_finished: None,
				at: Some(updated_at),
			},
		)
		.await?;

	if synced {
		let device = body.device_info.unwrap_or_default();
		backend
			.create_session(AbsSession {
				id: session_id.clone(),
				user_id: user.id.clone(),
				media_id: row.id,
				library_id: library_of(backend, user, &item_id).await?,
				device_id: device.device_id,
				client_name: device.client_name,
				client_version: device.client_version,
				media_player: body.media_player,
				current_time_ms: position_ms,
				time_listening_ms: elapsed_ms,
				started_at,
				updated_at,
				play_method: PLAY_METHOD_LOCAL,
			})
			.await?;
	}
	Ok((session_id, synced))
}

/// The library a book lives in, as the session row records it.
async fn library_of(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	item_id: &str,
) -> AbsResult<String> {
	let row = query::media_for_user(backend, user, item_id).await?;
	let rows = [row];
	let context = query::context(backend, user, &rows, false).await?;
	Ok(context
		.item(&rows[0], ItemShape::Minified, &user.id, None)
		.library_id)
}

/// `POST /api/session/local` → `200 OK`.
///
/// The app sends one session per call while it plays a downloaded book
/// (`MediaProgressSyncer.kt:276`) and considers the upload successful when
/// the response has no `error` (`ApiHandler.kt:642-650`).
pub(crate) async fn local(
	backend: Backend,
	Extension(user): User,
	Json(body): Json<LocalSessionDto>,
) -> AbsResult<Response> {
	merge_local(&**backend, &user, body).await?;
	Ok(crate::routes::ok_text())
}

/// `POST /api/session/local-all`: the backlog, in one request.
///
/// One bad session never fails the batch. The official app correlates each
/// result to a stored session by id and logs moved progress or per-session
/// errors (`ApiHandler.kt:747-760`), so a missing book fails only its result,
/// not the HTTP request.
pub(crate) async fn local_all(
	backend: Backend,
	Extension(user): User,
	Json(body): Json<LocalSessionsRequestDto>,
) -> AbsResult<Json<LocalSessionsResponseDto>> {
	let mut results = Vec::with_capacity(body.sessions.len());
	for session in body.sessions {
		let id = session.id.clone().unwrap_or_default();
		match merge_local(&**backend, &user, session).await {
			Ok((id, synced)) => results.push(LocalSessionResultDto {
				id,
				success: true,
				progress_synced: Some(synced),
				error: None,
			}),
			Err(error) => results.push(LocalSessionResultDto {
				id,
				success: false,
				progress_synced: None,
				error: Some(error.to_string()),
			}),
		}
	}
	Ok(Json(LocalSessionsResponseDto { results }))
}

// ---------------------------------------------------------------------------
// Listening history — GET /api/me/listening-sessions, /listening-stats
// ---------------------------------------------------------------------------

/// Every session of the user (or of one book) as a listening-history row.
///
/// The rows are mapped in one pass: the books they name are loaded together
/// and share a single [`query::context`], so a history of fifty sessions
/// over five books is five books' worth of queries, not fifty.
pub(crate) async fn history(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	media_id: Option<&str>,
) -> AbsResult<Vec<PlaybackSessionDto>> {
	let sessions = backend.sessions(&user.id, media_id).await?;
	if sessions.is_empty() {
		return Ok(Vec::new());
	}

	let mut media_ids = sessions
		.iter()
		.map(|session| session.media_id.clone())
		.collect::<Vec<_>>();
	media_ids.sort();
	media_ids.dedup();

	let rows = query::audio_media(user)
		.filter(media::Column::Id.is_in(media_ids.clone()))
		.all(backend.conn())
		.await?;
	let context = query::context(backend, user, &rows, false).await?;
	let audio = backend.audio_batch(&media_ids).await?;
	let items = rows
		.iter()
		.map(|row| {
			(
				row.id.clone(),
				context.item(row, ItemShape::Minified, &user.id, None),
			)
		})
		.collect::<HashMap<_, _>>();

	Ok(sessions
		.into_iter()
		.filter_map(|session| {
			// A session whose book was deleted, or which belongs to a book
			// this user may no longer see, drops out of the history rather
			// than being reported without its metadata.
			let item = items.get(&session.media_id)?.clone();
			let audio = audio.get(&session.media_id)?;
			Some(mapper::session_dto(mapper::SessionInput {
				session_id: &session.id,
				user_id: &user.id,
				item,
				audio,
				device: DeviceInfoRequestDto {
					device_id: session.device_id.clone(),
					client_name: session.client_name.clone(),
					client_version: session.client_version.clone(),
					..Default::default()
				},
				media_player: session.media_player.clone(),
				current_time_ms: session.current_time_ms,
				time_listening_ms: session.time_listening_ms,
				started_at: session.started_at,
				updated_at: session.updated_at,
				play_method: session.play_method,
				shape: SessionShape::Listed,
			}))
		})
		.collect())
}

/// One page of the history, in abs-ref's envelope.
pub(crate) fn page(
	sessions: Vec<PlaybackSessionDto>,
	items_per_page: u64,
	page_number: u64,
) -> ListeningSessionsPageDto {
	let total = sessions.len() as i64;
	let per_page = items_per_page.max(1);
	let num_pages = total.div_euclid(per_page as i64)
		+ i64::from(total.rem_euclid(per_page as i64) > 0);
	ListeningSessionsPageDto {
		total,
		num_pages,
		page: page_number as i64,
		items_per_page: per_page as i64,
		sessions: sessions
			.into_iter()
			.skip((page_number.saturating_mul(per_page)) as usize)
			.take(per_page as usize)
			.collect(),
	}
}

/// How many sessions the stats screen shows beside the totals; abs-ref sends
/// the ten newest.
const RECENT_SESSIONS: usize = 10;

/// Derive the stats screen from the history rows.
///
/// A session is counted on the day it *started*, in the server's timezone,
/// which is the day abs-ref stamps on the session itself — so the per-day
/// bars and the session list agree about when the listening happened.
pub(crate) fn stats(sessions: Vec<PlaybackSessionDto>) -> ListeningStatsDto {
	let today = Utc::now().format("%Y-%m-%d").to_string();
	let mut items: BTreeMap<String, ListeningStatsItemDto> = BTreeMap::new();
	let mut days: BTreeMap<String, f64> = BTreeMap::new();
	let mut day_of_week: BTreeMap<String, f64> = BTreeMap::new();
	let mut total = 0.0;

	for session in &sessions {
		let listened = session.time_listening as f64;
		total += listened;
		*days.entry(session.date.clone()).or_default() += listened;
		*day_of_week.entry(session.day_of_week.clone()).or_default() += listened;
		items
			.entry(session.library_item_id.clone())
			.and_modify(|item| item.time_listening += listened)
			.or_insert_with(|| ListeningStatsItemDto {
				id: session.library_item_id.clone(),
				time_listening: listened,
				media_metadata: session.media_metadata.clone(),
			});
	}

	ListeningStatsDto {
		total_time: total,
		today: days.get(&today).copied().unwrap_or(0.0),
		items,
		days,
		day_of_week,
		recent_sessions: sessions.into_iter().take(RECENT_SESSIONS).collect(),
	}
}
