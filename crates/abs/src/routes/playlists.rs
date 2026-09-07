//! Playlists, mapped onto Stump reading lists.
//!
//! An Audiobookshelf playlist and a Stump reading list are the same idea —
//! an ordered set of books belonging to one user — so the profile projects
//! one onto the other instead of growing a second table. Every mutation goes
//! through `stump_collections` on the server side, exactly as the Komga
//! read-list routes do, so a playlist created in the official app is the same
//! row a Komga client, a Kobo shelf or the web UI sees.
//!
//! Two shape differences are unavoidable and are documented in
//! `abs-compat.mdx`:
//!
//! - A reading list is not scoped to a library. `libraryId` is reported as
//!   the library of the playlist's first audible member, and a member-less
//!   playlist is listed under every library.
//! - A reading list may hold books this profile does not serve (a comic, an
//!   ebook, a book in a library the request cannot reach). Those entries are
//!   dropped from `items[]` rather than emitted as ids the app would
//!   dereference — `pages/playlist/_id.vue:52` reads
//!   `playlist.items[0].libraryItem.mediaType` with no null check.
//!
//! Wire shapes were captured live from abs-ref 2.36.0; the request bodies are
//! the official app's own literals
//! (`components/modals/playlists/AddCreateModal.vue:137,155,180-185`,
//! `components/modals/ItemMoreMenuModal.vue:491-495`).

use axum::{
	body::Body,
	extract::{Json, Path},
	response::Response,
	Extension,
};
use models::entity::{media, user::AuthUser};
use sea_orm::{ColumnTrait, QueryFilter};

use crate::{
	dto::*,
	errors::{AbsError, AbsResult},
	mapper,
	model::{AbsPlaylist, ItemShape},
	routes::{ok_text, query, AbsBackend, Backend, User},
};

/// Render one playlist, resolving its members into expanded library items.
pub(crate) async fn render(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	playlist: &AbsPlaylist,
	fallback_library_id: Option<&str>,
) -> AbsResult<PlaylistDto> {
	// One query for the whole membership, narrowed by the profile's own
	// funnel: audible, undeleted, visible to this user.
	let rows = if playlist.media_ids.is_empty() {
		Vec::new()
	} else {
		query::audio_media(user)
			.filter(media::Column::Id.is_in(playlist.media_ids.clone()))
			.all(backend.conn())
			.await?
	};
	// The reading list's own order, not the database's.
	let rows = playlist
		.media_ids
		.iter()
		.filter_map(|id| rows.iter().find(|row| &row.id == id).cloned())
		.collect::<Vec<_>>();

	let context = query::context(backend, user, &rows, false).await?;
	let items = rows
		.iter()
		.map(|row| context.item(row, ItemShape::Expanded, &user.id, None))
		.collect::<Vec<_>>();
	let library_id = items
		.first()
		.map(|item| item.library_id.clone())
		.or_else(|| fallback_library_id.map(str::to_owned))
		.unwrap_or_default();

	Ok(mapper::playlist_dto(playlist, &user.id, &library_id, items))
}

/// The ids the profile will accept from a playlist request body: audible
/// books the user may see, in the order the client sent them.
///
/// An unknown or invisible id is dropped rather than failing the whole
/// request, which is how `POST /api/items/batch/get` already behaves.
async fn resolve_items(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	refs: &[PlaylistItemRefDto],
) -> AbsResult<Vec<String>> {
	let requested = refs
		.iter()
		.map(|item| item.library_item_id.clone())
		.collect::<Vec<_>>();
	if requested.is_empty() {
		return Ok(Vec::new());
	}
	let rows = query::audio_media(user)
		.filter(media::Column::Id.is_in(requested.clone()))
		.all(backend.conn())
		.await?;
	let mut seen = Vec::with_capacity(requested.len());
	for id in requested {
		if rows.iter().any(|row| row.id == id) && !seen.contains(&id) {
			seen.push(id);
		}
	}
	Ok(seen)
}

async fn load(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	playlist_id: &str,
) -> AbsResult<AbsPlaylist> {
	backend
		.playlist(user, playlist_id)
		.await?
		.ok_or_else(|| AbsError::NotFound(format!("No playlist {playlist_id}")))
}

/// The socket bus, when the server mounted one. Playlist mutation routes
/// publish the same expanded DTO they return over REST, matching
/// `PlaylistController.js:117-119,249-255,265-271`.
type Events = Option<Extension<crate::socket::AbsEvents>>;

fn announce_playlist(events: &Events, event: crate::socket::AbsEvent) {
	if let Some(Extension(events)) = events.as_ref() {
		events.send(event);
	}
}

/// `POST /api/playlists` — `{items, libraryId, name}`.
pub(crate) async fn create(
	backend: Backend,
	Extension(user): User,
	events: Events,
	Json(body): Json<PlaylistCreateRequestDto>,
) -> AbsResult<Json<PlaylistDto>> {
	let name = body
		.name
		.as_deref()
		.map(str::trim)
		.filter(|name| !name.is_empty())
		.ok_or_else(|| AbsError::BadRequest("A playlist needs a name".to_owned()))?;
	if let Some(library_id) = body.library_id.as_deref() {
		query::library(&**backend, &user, library_id).await?;
	}
	let media_ids = resolve_items(&**backend, &user, &body.items).await?;
	let playlist = backend
		.create_playlist(&user, name, body.description.as_deref(), &media_ids)
		.await?;
	let dto = render(&**backend, &user, &playlist, body.library_id.as_deref()).await?;
	announce_playlist(
		&events,
		crate::socket::AbsEvent::PlaylistAdded {
			user_id: user.id.clone(),
			playlist: dto.clone(),
		},
	);
	Ok(Json(dto))
}

/// `GET /api/playlists` — every playlist of the user. abs-ref answers the
/// same `{results,total,limit,page}` envelope here as per library.
pub(crate) async fn list_all(
	backend: Backend,
	Extension(user): User,
) -> AbsResult<Json<PlaylistsPageDto>> {
	let playlists = backend.playlists(&user).await?;
	let mut results = Vec::with_capacity(playlists.len());
	for playlist in &playlists {
		results.push(render(&**backend, &user, playlist, None).await?);
	}
	let total = results.len() as i64;
	Ok(Json(PlaylistsPageDto {
		results,
		total,
		limit: 0,
		page: 0,
	}))
}

/// `GET /api/playlists/{id}` (`pages/playlist/_id.vue:41`).
pub(crate) async fn detail(
	backend: Backend,
	Extension(user): User,
	Path(playlist_id): Path<String>,
) -> AbsResult<Json<PlaylistDto>> {
	let playlist = load(&**backend, &user, &playlist_id).await?;
	Ok(Json(render(&**backend, &user, &playlist, None).await?))
}

/// `PATCH /api/playlists/{id}`.
pub(crate) async fn update(
	backend: Backend,
	Extension(user): User,
	events: Events,
	Path(playlist_id): Path<String>,
	Json(body): Json<PlaylistUpdateRequestDto>,
) -> AbsResult<Json<PlaylistDto>> {
	load(&**backend, &user, &playlist_id).await?;
	let media_ids = match body.items.as_deref() {
		Some(items) => Some(resolve_items(&**backend, &user, items).await?),
		None => None,
	};
	let playlist = backend
		.update_playlist(
			&user,
			&playlist_id,
			body.name
				.as_deref()
				.map(str::trim)
				.filter(|n| !n.is_empty()),
			body.description.as_ref().map(|value| value.as_deref()),
			media_ids.as_deref(),
		)
		.await?;
	let dto = render(&**backend, &user, &playlist, None).await?;
	announce_playlist(
		&events,
		crate::socket::AbsEvent::PlaylistUpdated {
			user_id: user.id.clone(),
			playlist: dto.clone(),
		},
	);
	Ok(Json(dto))
}

/// `DELETE /api/playlists/{id}`. abs-ref answers `200 text/plain` here.
pub(crate) async fn remove(
	backend: Backend,
	Extension(user): User,
	events: Events,
	Path(playlist_id): Path<String>,
) -> AbsResult<Response<Body>> {
	let playlist = load(&**backend, &user, &playlist_id).await?;
	let dto = render(&**backend, &user, &playlist, None).await?;
	backend.delete_playlist(&user, &playlist_id).await?;
	announce_playlist(
		&events,
		crate::socket::AbsEvent::PlaylistRemoved {
			user_id: user.id.clone(),
			playlist: dto,
		},
	);
	Ok(ok_text())
}

/// `POST /api/playlists/{id}/item` — `{libraryItemId}`.
pub(crate) async fn add_item(
	backend: Backend,
	Extension(user): User,
	events: Events,
	Path(playlist_id): Path<String>,
	Json(body): Json<PlaylistItemRefDto>,
) -> AbsResult<Json<PlaylistDto>> {
	mutate_items(backend, user, events, playlist_id, vec![body], true).await
}

/// `DELETE /api/playlists/{id}/item/{itemId}[/{episodeId}]`
/// (`components/modals/ItemMoreMenuModal.vue:491-495`).
pub(crate) async fn remove_item(
	backend: Backend,
	Extension(user): User,
	events: Events,
	Path((playlist_id, item_id)): Path<(String, String)>,
) -> AbsResult<Json<PlaylistDto>> {
	let item = PlaylistItemRefDto {
		library_item_id: item_id,
		episode_id: None,
	};
	mutate_items(backend, user, events, playlist_id, vec![item], false).await
}

/// `POST /api/playlists/{id}/batch/add`.
pub(crate) async fn batch_add(
	backend: Backend,
	Extension(user): User,
	events: Events,
	Path(playlist_id): Path<String>,
	Json(body): Json<PlaylistItemsRequestDto>,
) -> AbsResult<Json<PlaylistDto>> {
	mutate_items(backend, user, events, playlist_id, body.items, true).await
}

/// `POST /api/playlists/{id}/batch/remove`.
pub(crate) async fn batch_remove(
	backend: Backend,
	Extension(user): User,
	events: Events,
	Path(playlist_id): Path<String>,
	Json(body): Json<PlaylistItemsRequestDto>,
) -> AbsResult<Json<PlaylistDto>> {
	mutate_items(backend, user, events, playlist_id, body.items, false).await
}

/// Add or remove members and answer the updated playlist, which is what
/// every one of the four membership routes does
/// (`AddCreateModal.vue:140,158` read the response as the updated playlist).
async fn mutate_items(
	backend: Backend,
	user: AuthUser,
	events: Events,
	playlist_id: String,
	refs: Vec<PlaylistItemRefDto>,
	add: bool,
) -> AbsResult<Json<PlaylistDto>> {
	let playlist = load(&**backend, &user, &playlist_id).await?;
	let mut media_ids = playlist.media_ids.clone();
	if add {
		for id in resolve_items(&**backend, &user, &refs).await? {
			if !media_ids.contains(&id) {
				media_ids.push(id);
			}
		}
	} else {
		// Removal is not filtered through the visibility funnel: a member
		// the user can no longer see must still be removable.
		let dropped = refs
			.iter()
			.map(|item| item.library_item_id.as_str())
			.collect::<Vec<_>>();
		media_ids.retain(|id| !dropped.contains(&id.as_str()));
	}

	let playlist = backend
		.update_playlist(&user, &playlist_id, None, None, Some(&media_ids))
		.await?;
	let dto = render(&**backend, &user, &playlist, None).await?;
	announce_playlist(
		&events,
		crate::socket::AbsEvent::PlaylistUpdated {
			user_id: user.id.clone(),
			playlist: dto.clone(),
		},
	);
	Ok(Json(dto))
}

/// `GET /api/libraries/{id}/playlists`. The app reads `data.results`
/// (`components/modals/playlists/AddCreateModal.vue:113-115`).
///
/// A playlist belongs to the library of its first audible member; one with
/// no members belongs to whichever library is being browsed.
pub(crate) async fn list_for_library(
	backend: Backend,
	Extension(user): User,
	Path(library_id): Path<String>,
) -> AbsResult<Json<PlaylistsPageDto>> {
	query::library(&**backend, &user, &library_id).await?;
	let playlists = backend.playlists(&user).await?;
	let mut results = Vec::new();
	for playlist in &playlists {
		let dto = render(&**backend, &user, playlist, Some(&library_id)).await?;
		if dto.library_id == library_id {
			results.push(dto);
		}
	}
	let total = results.len() as i64;
	Ok(Json(PlaylistsPageDto {
		results,
		total,
		limit: 0,
		page: 0,
	}))
}
