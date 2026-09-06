//! The per-user surface: `GET /api/me`, the media-progress read/patch and
//! the bookmark writes.
//!
//! Lissen reads `GET /api/me` twice for two different projections — once as
//! `UserResponse{id,username,...}` (`AudiobookshelfApiClient.kt:77`) and once
//! as `BookmarksResponse{bookmarks}` (`:62`) — so both have to be on the same
//! object.

use axum::{
	extract::{Json, Path},
	response::Response,
	Extension,
};
use models::entity::{media, user::AuthUser};

use crate::{
	dto::*,
	errors::{AbsError, AbsResult},
	mapper::{self, secs_to_ms},
	model::AbsPositionUpdate,
	routes::{items, query, AbsBackend, Backend, User},
};

/// Audiobookshelf identifies a bookmark by `(item, whole second)`, so every
/// position on that surface is rounded to a second before it is stored or
/// compared.
fn bookmark_position_ms(time: f64) -> i64 {
	secs_to_ms(time.trunc())
}

/// The user's `mediaProgress[]`: one entry per book they have a position for
/// and can still see.
pub(crate) async fn media_progress(
	backend: &dyn AbsBackend,
	user: &AuthUser,
) -> AbsResult<Vec<MediaProgressDto>> {
	let rows = items::all_audio_media(backend, user).await?;
	let context = query::context(backend, user, &rows, true).await?;
	Ok(context.progress_dtos(&user.id, &rows))
}

/// The user's `bookmarks[]`, across every book they can still see.
pub(crate) async fn bookmarks(
	backend: &dyn AbsBackend,
	user: &AuthUser,
) -> AbsResult<Vec<AudioBookmarkDto>> {
	let visible = items::all_audio_media(backend, user)
		.await?
		.into_iter()
		.map(|row| row.id)
		.collect::<Vec<_>>();
	Ok(backend
		.bookmarks_all(user)
		.await?
		.into_iter()
		.filter(|(media_id, _)| visible.contains(media_id))
		.map(|(media_id, bookmark)| mapper::bookmark_dto(&media_id, &bookmark))
		.collect())
}

/// `userDefaultLibraryId`: the first library with audio in it, which is the
/// one a client opens on first launch.
pub(crate) async fn default_library_id(
	backend: &dyn AbsBackend,
	user: &AuthUser,
) -> AbsResult<Option<String>> {
	items::first_audio_library(backend, user).await
}

pub(crate) async fn me(
	backend: Backend,
	Extension(user): User,
) -> AbsResult<Json<UserDto>> {
	let (_, created_at) = backend.user(&user.id).await?;
	Ok(Json(mapper::user_dto(mapper::UserInput {
		user: &user,
		created_at,
		legacy_token: legacy_token(&**backend, &user).await?,
		// `GET /api/me` carries `token` alone: `capture/me.json` has neither
		// `accessToken` nor `refreshToken`.
		tokens: mapper::TokenPresentation::LegacyOnly,
		media_progress: media_progress(&**backend, &user).await?,
		bookmarks: bookmarks(&**backend, &user).await?,
		libraries_accessible: user.device_library_scope.clone().unwrap_or_default(),
	})))
}

/// `user.token`, the never-expiring legacy token. abs-ref hands the same
/// value back on every read of the user object, and clients that predate
/// `accessToken` use it as their only credential.
pub(crate) async fn legacy_token(
	backend: &dyn AbsBackend,
	user: &AuthUser,
) -> AbsResult<String> {
	let secret = backend.token_secret().await?;
	Ok(crate::auth::mint_tokens(&secret, &user.id, &user.username, None)?.token)
}

/// The book, its ABS `media.id` and its duration — everything a progress
/// answer needs beyond the position itself.
async fn progress_target(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	item_id: &str,
) -> AbsResult<(media::Model, String, i64)> {
	let row = query::media_for_user(backend, user, item_id).await?;
	let book_id = backend
		.book_ids(&[row.id.clone()])
		.await?
		.remove(&row.id)
		.unwrap_or_else(|| row.id.clone());
	let duration_ms = backend
		.audio(item_id)
		.await?
		.map(|audio| audio.duration_ms)
		.unwrap_or(0);
	Ok((row, book_id, duration_ms))
}

/// `GET /api/me/progress/{itemId}`. abs-ref answers `404` for a book with no
/// progress (`capture/negatives.txt`), which Lissen reads as "not started".
pub(crate) async fn progress(
	backend: Backend,
	Extension(user): User,
	Path(item_id): Path<String>,
) -> AbsResult<Json<MediaProgressDto>> {
	let (_, book_id, duration_ms) = progress_target(&**backend, &user, &item_id).await?;
	let progress = backend
		.progress(&user, &item_id)
		.await?
		.ok_or_else(|| AbsError::NotFound(format!("No progress for {item_id}")))?;
	Ok(Json(mapper::progress_dto(
		&user.id,
		&item_id,
		&book_id,
		duration_ms,
		&progress,
	)))
}

/// `PATCH /api/me/progress/{itemId}`: a partial update. The two fields that
/// matter are `currentTime` and `isFinished`; a patch that carries neither
/// still succeeds, as it does on abs-ref.
pub(crate) async fn patch_progress(
	backend: Backend,
	Extension(user): User,
	Path(item_id): Path<String>,
	Json(body): Json<ProgressPatchDto>,
) -> AbsResult<Response> {
	let (_, _, duration_ms) = progress_target(&**backend, &user, &item_id).await?;
	let existing = backend.progress(&user, &item_id).await?;

	// `isFinished: true` without a position means "the end", which is what a
	// client showing a "mark as finished" button sends.
	let position_ms = match (body.current_time, body.progress, body.is_finished) {
		(Some(current_time), _, _) => secs_to_ms(current_time),
		(None, Some(fraction), _) if duration_ms > 0 => {
			(fraction.clamp(0.0, 1.0) * duration_ms as f64).round() as i64
		},
		(None, None, Some(true)) => duration_ms,
		_ => existing.as_ref().map(|p| p.position_ms).unwrap_or(0),
	};

	backend
		.apply_position(
			&user,
			&item_id,
			AbsPositionUpdate {
				position_ms,
				track_index: existing.as_ref().and_then(|p| p.track_index),
				duration_ms,
				elapsed_ms: 0,
				is_finished: body.is_finished,
			},
		)
		.await?;
	Ok(crate::routes::ok_text())
}

/// `POST /api/me/item/{id}/bookmark`.
pub(crate) async fn create_bookmark(
	backend: Backend,
	Extension(user): User,
	Path(item_id): Path<String>,
	Json(body): Json<BookmarkRequestDto>,
) -> AbsResult<Json<AudioBookmarkDto>> {
	upsert(backend, user, item_id, body).await
}

/// `PATCH /api/me/item/{id}/bookmark` — a rename of the bookmark at that
/// second. Lissen only ever creates and deletes
/// (`AudiobookshelfApiClient.kt:65,71`); the official client renames.
pub(crate) async fn update_bookmark(
	backend: Backend,
	Extension(user): User,
	Path(item_id): Path<String>,
	Json(body): Json<BookmarkRequestDto>,
) -> AbsResult<Json<AudioBookmarkDto>> {
	upsert(backend, user, item_id, body).await
}

async fn upsert(
	backend: Backend,
	user: AuthUser,
	item_id: String,
	body: BookmarkRequestDto,
) -> AbsResult<Json<AudioBookmarkDto>> {
	query::media_for_user(&**backend, &user, &item_id).await?;
	let time = body
		.time
		.ok_or_else(|| AbsError::BadRequest("No bookmark time".to_owned()))?;
	let title = body.title.unwrap_or_default();
	let bookmark = backend
		.upsert_bookmark(&user, &item_id, bookmark_position_ms(time), &title)
		.await?;
	Ok(Json(mapper::bookmark_dto(&item_id, &bookmark)))
}

/// `DELETE /api/me/item/{id}/bookmark/{time}`. The time is in the path, in
/// whole seconds (`AudiobookshelfApiClient.kt:71`, `@Path totalTime: Int`).
pub(crate) async fn delete_bookmark(
	backend: Backend,
	Extension(user): User,
	Path((item_id, time)): Path<(String, String)>,
) -> AbsResult<Response> {
	query::media_for_user(&**backend, &user, &item_id).await?;
	let seconds = time
		.parse::<f64>()
		.map_err(|_| AbsError::BadRequest(format!("Bad bookmark time {time}")))?;
	let position_ms = bookmark_position_ms(seconds);
	let existing = backend.bookmarks(&user, &item_id).await?;
	if !existing
		.iter()
		.any(|bookmark| bookmark.position_ms == position_ms)
	{
		return Err(AbsError::NotFound(format!(
			"No bookmark at {time} for {item_id}"
		)));
	}
	backend
		.delete_bookmark(&user, &item_id, position_ms)
		.await?;
	Ok(crate::routes::ok_text())
}
