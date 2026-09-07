//! The per-user surface: `GET /api/me`, the media-progress read/patch, the
//! bookmark writes, and the two screens the official app opens on launch —
//! the "continue" row (`GET /api/me/items-in-progress`) and the listening
//! history (`/listening-sessions`, `/listening-stats`).
//!
//! Lissen reads `GET /api/me` twice for two different projections — once as
//! `UserResponse{id,username,...}` (`AudiobookshelfApiClient.kt:77`) and once
//! as `BookmarksResponse{bookmarks}` (`:62`) — so both have to be on the same
//! object.

use axum::{
	extract::{Json, Path, Query},
	response::Response,
	Extension,
};
use models::entity::{media, user::AuthUser};
use serde::Deserialize;

use crate::{
	dto::*,
	errors::{AbsError, AbsResult},
	mapper::{self, secs_to_ms},
	model::AbsPositionUpdate,
	routes::{items, query, session, AbsBackend, Backend, User},
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

/// The `user` object of `GET /api/me`, and of the socket lane's
/// `user_updated` push — the same object, built one way, so a client that
/// reads it over the socket and over REST cannot see two different users.
pub(crate) async fn user_dto(
	backend: &dyn AbsBackend,
	user: &AuthUser,
) -> AbsResult<UserDto> {
	let (_, created_at) = backend.user(&user.id).await?;
	Ok(mapper::user_dto(mapper::UserInput {
		user,
		created_at,
		legacy_token: legacy_token(backend, user).await?,
		// `GET /api/me` carries `token` alone: `capture/me.json` has neither
		// `accessToken` nor `refreshToken`.
		tokens: mapper::TokenPresentation::LegacyOnly,
		media_progress: media_progress(backend, user).await?,
		bookmarks: bookmarks(backend, user).await?,
		libraries_accessible: user.device_library_scope.clone().unwrap_or_default(),
	}))
}

pub(crate) async fn me(
	backend: Backend,
	Extension(user): User,
) -> AbsResult<Json<UserDto>> {
	Ok(Json(user_dto(&**backend, &user).await?))
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

	// `lastUpdate` is the device clock time of the position, which the
	// official app sends when it pushes a reader position it recorded while
	// offline (`ApiHandler.kt:854-858`). Dating the update with it is what
	// lets the head keep the newer of two positions instead of the last one
	// to arrive.
	let at = body.last_update.and_then(mapper::ms_to_utc);
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
				at,
			},
		)
		.await?;
	Ok(crate::routes::ok_text())
}

/// `GET`/`PATCH /api/me/progress/{itemId}/{episodeId}`.
///
/// The official app addresses a podcast episode's progress with this pair
/// (`ApiHandler.kt:688,696`). The profile serves book libraries only — a
/// Stump audiobook has no episodes — so the episode never resolves and the
/// route answers `404`, which is what abs-ref answers for an episode id that
/// is not in the item and what the app reads as "no progress".
pub(crate) async fn episode_progress(
	Path((item_id, episode_id)): Path<(String, String)>,
) -> AbsResult<Response> {
	Err(AbsError::NotFound(format!(
		"No episode {episode_id} in {item_id}"
	)))
}

/// The socket bus, when the server mounted one. A bookmark write is the one
/// change to the `/api/me` object that no core event announces — nothing
/// outside this profile writes an ABS bookmark — so the routes that make it
/// publish it themselves, and the socket lane pushes `user_updated`.
type Events = Option<Extension<crate::socket::AbsEvents>>;

fn announce_user_changed(events: &Events, user_id: &str) {
	if let Some(Extension(events)) = events {
		events.send(crate::socket::AbsEvent::UserChanged {
			user_id: user_id.to_owned(),
		});
	}
}

/// `POST /api/me/item/{id}/bookmark`.
pub(crate) async fn create_bookmark(
	backend: Backend,
	Extension(user): User,
	events: Events,
	Path(item_id): Path<String>,
	Json(body): Json<BookmarkRequestDto>,
) -> AbsResult<Json<AudioBookmarkDto>> {
	upsert(backend, user, events, item_id, body).await
}

/// `PATCH /api/me/item/{id}/bookmark` — a rename of the bookmark at that
/// second. Lissen only ever creates and deletes
/// (`AudiobookshelfApiClient.kt:65,71`); the official client renames.
pub(crate) async fn update_bookmark(
	backend: Backend,
	Extension(user): User,
	events: Events,
	Path(item_id): Path<String>,
	Json(body): Json<BookmarkRequestDto>,
) -> AbsResult<Json<AudioBookmarkDto>> {
	upsert(backend, user, events, item_id, body).await
}

async fn upsert(
	backend: Backend,
	user: AuthUser,
	events: Events,
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
	announce_user_changed(&events, &user.id);
	Ok(Json(mapper::bookmark_dto(&item_id, &bookmark)))
}

/// `DELETE /api/me/item/{id}/bookmark/{time}`. The time is in the path, in
/// whole seconds (`AudiobookshelfApiClient.kt:71`, `@Path totalTime: Int`).
pub(crate) async fn delete_bookmark(
	backend: Backend,
	Extension(user): User,
	events: Events,
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
	announce_user_changed(&events, &user.id);
	Ok(crate::routes::ok_text())
}

// ---------------------------------------------------------------------------
// The screens the official app opens on launch
// ---------------------------------------------------------------------------

/// abs-ref's default page size for the in-progress row and the listening
/// history; the official app sends no parameters for either
/// (`ApiHandler.kt:612`, `pages/stats.vue:109`).
const DEFAULT_PAGE_SIZE: u64 = 25;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct PageQuery {
	limit: Option<u64>,
	items_per_page: Option<u64>,
	page: Option<u64>,
}

/// `GET /api/me/items-in-progress`: the books the user has started and not
/// finished, newest position first.
///
/// Every row is a **minified** library item plus `progressLastUpdate`, which
/// the app reads with `getLong` and therefore cannot be absent
/// (`data/ItemInProgress.kt`).
pub(crate) async fn items_in_progress(
	backend: Backend,
	Extension(user): User,
	Query(params): Query<PageQuery>,
) -> AbsResult<Json<ItemsInProgressDto>> {
	let limit = params.limit.unwrap_or(DEFAULT_PAGE_SIZE).max(1) as usize;
	let rows = items::all_audio_media(&**backend, &user).await?;
	let context = query::context(&**backend, &user, &rows, true).await?;

	let mut started = rows
		.iter()
		.filter_map(|row| {
			let progress = context.progress.get(&row.id)?;
			(!progress.is_finished && progress.position_ms > 0)
				.then_some((row, progress.last_update))
		})
		.collect::<Vec<_>>();
	started
		.sort_by_key(|(_, last_update)| std::cmp::Reverse(mapper::ts_ms(*last_update)));

	Ok(Json(ItemsInProgressDto {
		library_items: started
			.into_iter()
			.take(limit)
			.map(|(row, last_update)| ItemInProgressDto {
				item: context.item(
					row,
					crate::model::ItemShape::Minified,
					&user.id,
					None,
				),
				progress_last_update: mapper::ts_ms(last_update),
			})
			.collect(),
	}))
}

/// `GET /api/me/listening-sessions`: the play sessions this profile opened
/// and the offline ones the app uploaded, newest first.
pub(crate) async fn listening_sessions(
	backend: Backend,
	Extension(user): User,
	Query(params): Query<PageQuery>,
) -> AbsResult<Json<ListeningSessionsPageDto>> {
	let per_page = params
		.items_per_page
		.or(params.limit)
		.unwrap_or(DEFAULT_PAGE_SIZE)
		.max(1);
	let page = params.page.unwrap_or(0);
	let sessions = session::history(&**backend, &user, None).await?;
	Ok(Json(session::page(sessions, per_page, page)))
}

/// `GET /api/me/item/listening-sessions/{id}`: the same history for one
/// book.
pub(crate) async fn item_listening_sessions(
	backend: Backend,
	Extension(user): User,
	Path(item_id): Path<String>,
	Query(params): Query<PageQuery>,
) -> AbsResult<Json<ListeningSessionsPageDto>> {
	query::media_for_user(&**backend, &user, &item_id).await?;
	let per_page = params
		.items_per_page
		.or(params.limit)
		.unwrap_or(DEFAULT_PAGE_SIZE)
		.max(1);
	let page = params.page.unwrap_or(0);
	let sessions = session::history(&**backend, &user, Some(&item_id)).await?;
	Ok(Json(session::page(sessions, per_page, page)))
}

/// `GET /api/me/listening-stats`, the stats screen.
///
/// Every number is derived from the same session rows the history lists —
/// there is no second store of listening time — so the totals and the list
/// can never disagree.
pub(crate) async fn listening_stats(
	backend: Backend,
	Extension(user): User,
) -> AbsResult<Json<ListeningStatsDto>> {
	let sessions = session::history(&**backend, &user, None).await?;
	Ok(Json(session::stats(sessions)))
}
