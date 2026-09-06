//! Play sessions: the read, the position sync and the close.
//!
//! A client syncs against the session id it was handed by
//! `POST /api/items/{id}/play`, not against the item
//! (Lissen `common/api/library/AudioBookshelfLibrarySyncService.kt:17-28`
//! passes the session id into `POST /api/session/{id}/sync`), so the session
//! row is what maps a sync back onto a book. `timeListened` is a delta in
//! seconds since the previous sync, which accumulates on the session, while
//! `currentTime` is an absolute position that lands on the unified reading
//! state.

use axum::{
	extract::{Json, Path},
	response::Response,
	Extension,
};

use crate::{
	dto::{DeviceInfoRequestDto, PlaybackSessionDto, ProgressSyncRequestDto},
	errors::{AbsError, AbsResult},
	mapper,
	model::{AbsPositionUpdate, ItemShape},
	routes::{query, AbsBackend, AbsSession, Backend, User},
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
	})))
}

/// Apply one `{currentTime, timeListened, duration}` report to both the
/// session row and the unified reading state.
async fn apply(
	backend: &dyn AbsBackend,
	user: &models::entity::user::AuthUser,
	session: &AbsSession,
	body: ProgressSyncRequestDto,
) -> AbsResult<()> {
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

	backend
		.apply_position(
			user,
			&session.media_id,
			AbsPositionUpdate {
				position_ms,
				track_index,
				duration_ms,
				elapsed_ms,
				is_finished: None,
			},
		)
		.await?;
	backend
		.update_session(
			&session.id,
			position_ms,
			session.time_listening_ms.saturating_add(elapsed_ms),
		)
		.await
}

/// `POST /api/session/{id}/sync` → `200 OK`
/// (`capture/session_sync.txt`).
pub(crate) async fn sync(
	backend: Backend,
	Extension(user): User,
	Path(session_id): Path<String>,
	body: Option<Json<ProgressSyncRequestDto>>,
) -> AbsResult<Response> {
	let session = load(&**backend, &user.id, &session_id).await?;
	apply(
		&**backend,
		&user,
		&session,
		body.map(|Json(body)| body).unwrap_or_default(),
	)
	.await?;
	Ok(crate::routes::ok_text())
}

/// `POST /api/session/{id}/close`: the same update, then the session is
/// closed. abs-ref accepts a body here and Lissen sends none.
pub(crate) async fn close(
	backend: Backend,
	Extension(user): User,
	Path(session_id): Path<String>,
	body: Option<Json<ProgressSyncRequestDto>>,
) -> AbsResult<Response> {
	let session = load(&**backend, &user.id, &session_id).await?;
	let body = body.map(|Json(body)| body).unwrap_or_default();
	// A close with no position must not reset the session to zero.
	if body != ProgressSyncRequestDto::default() {
		apply(&**backend, &user, &session, body).await?;
	}
	backend.close_session(&session.id).await?;
	Ok(crate::routes::ok_text())
}
