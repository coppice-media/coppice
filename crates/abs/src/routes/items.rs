//! One library item: its detail shape, its cover, its audio bytes, the play
//! request that opens a session, the batch read, and the synthesised authors.

use axum::{
	body::Body,
	extract::{Json, Path, Query},
	http::{header, HeaderMap, Response, StatusCode},
	response::IntoResponse,
	Extension,
};
use chrono::Utc;
use models::entity::{media, media_metadata, series, user::AuthUser};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::Deserialize;
use stump_auth::AuthContext;
use uuid::Uuid;

use crate::{
	dto::*,
	errors::{AbsError, AbsResult},
	mapper,
	model::ItemShape,
	routes::{libraries, query, AbsBackend, AbsSession, Backend, User},
};

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ItemQuery {
	expanded: Option<String>,
	include: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct CoverQuery {
	/// Lissen sends `raw=1` for the full-size cover
	/// (`AudiobookshelfApiClient.kt:176`) and `width` for a thumbnail
	/// (`:182`). Stump serves one rendition, so both are accepted and the
	/// stored cover is returned either way.
	#[allow(dead_code)]
	raw: Option<String>,
	#[allow(dead_code)]
	width: Option<u32>,
}

fn flag(value: Option<&String>) -> bool {
	value
		.map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
		.unwrap_or(false)
}

/// `GET /api/items/{id}`. Lissen calls it with no parameters and decodes the
/// full `media.metadata`/`audioFiles`/`chapters` shape
/// (`library/model/BookResponse.kt`); `?expanded=1&include=progress` is the
/// shape other ABS clients ask for.
pub(crate) async fn detail(
	backend: Backend,
	Extension(user): User,
	Path(item_id): Path<String>,
	Query(params): Query<ItemQuery>,
) -> AbsResult<Json<LibraryItemDto>> {
	let row = query::media_for_user(&**backend, &user, &item_id).await?;
	let include_progress = params
		.include
		.as_deref()
		.map(|include| include.contains("progress"))
		.unwrap_or(false);
	let shape = if flag(params.expanded.as_ref()) || include_progress {
		ItemShape::Expanded
	} else {
		ItemShape::Detail
	};

	let rows = [row];
	let context = query::context(&**backend, &user, &rows, include_progress).await?;
	Ok(Json(context.item(&rows[0], shape, &user.id, None)))
}

/// `POST /api/items/batch/get`. Unknown or invisible ids are dropped rather
/// than failing the batch, which is what abs-ref does; Lissen uses it to
/// refresh a screenful of items it already holds.
pub(crate) async fn batch_get(
	backend: Backend,
	Extension(user): User,
	Json(body): Json<ItemsBatchRequestDto>,
) -> AbsResult<Json<ItemsBatchResponseDto>> {
	if body.library_item_ids.is_empty() {
		return Err(AbsError::BadRequest("No library item ids".to_owned()));
	}

	let rows = query::audio_media(&user)
		.filter(media::Column::Id.is_in(body.library_item_ids.clone()))
		.all(backend.conn())
		.await?;
	let context = query::context(&**backend, &user, &rows, true).await?;

	// Answer in the order the client asked, not in database order.
	let library_items = body
		.library_item_ids
		.iter()
		.filter_map(|id| rows.iter().find(|row| &row.id == id))
		.map(|row| context.item(row, ItemShape::Expanded, &user.id, None))
		.collect();
	Ok(Json(ItemsBatchResponseDto { library_items }))
}

pub(crate) async fn cover(
	backend: Backend,
	Extension(user): User,
	Path(item_id): Path<String>,
	Query(_): Query<CoverQuery>,
) -> AbsResult<Response<Body>> {
	// Resolve through the visibility funnel first: a cover must not leak the
	// existence of a book the request may not see.
	query::media_for_user(&**backend, &user, &item_id).await?;
	let image = backend.cover(&user, &item_id).await?;
	Ok(([(header::CONTENT_TYPE, image.content_type)], image.data).into_response())
}

/// `GET /api/items/{id}/file/{ino}` — the audio bytes of one track.
///
/// `ino` is the 0-based Stump track index (abs-ref uses a filesystem inode,
/// which Stump has no stable equivalent of). Lissen builds this URL itself
/// from the item id and the `ino` it read off `audioFiles[]`
/// (`common/AudiobookshelfChannel.kt:48-64`) and plays it through an OkHttp
/// data source that carries the same bearer token as every other request
/// (`playback/service/LissenDataSourceFactory.kt:37`,
/// `channel/common/OkHttpClient.kt:47-59`), so range support is what makes
/// seeking work.
///
/// The authenticating device is passed through because an audio transform
/// preset is per device: the same book served to a phone with an Opus preset
/// and to a desktop player is the same route answering with two encodings.
pub(crate) async fn file(
	backend: Backend,
	Extension(user): User,
	Extension(auth): Extension<AuthContext>,
	Path((item_id, ino)): Path<(String, String)>,
	headers: HeaderMap,
) -> AbsResult<Response<Body>> {
	query::media_for_user(&**backend, &user, &item_id).await?;
	let index = ino
		.parse::<i32>()
		.map_err(|_| AbsError::NotFound(format!("No file {ino}")))?;
	backend
		.serve_track(headers, &item_id, index, auth.device_id())
		.await
}

/// `POST /api/items/{id}/play`.
///
/// The session is stored (`abs_sessions`) because a client syncs against the
/// *session* id, not the item id, and starts at the position the unified
/// reading state already holds, so a book resumes where any other Stump
/// client left it.
pub(crate) async fn play(
	backend: Backend,
	Extension(user): User,
	Path(item_id): Path<String>,
	body: Option<Json<PlaybackStartRequestDto>>,
) -> AbsResult<Json<PlaybackSessionDto>> {
	let row = query::media_for_user(&**backend, &user, &item_id).await?;
	let audio = backend
		.audio(&item_id)
		.await?
		.ok_or_else(|| AbsError::NotFound(format!("No audio for {item_id}")))?;

	let request = body.map(|Json(body)| body).unwrap_or_default();
	let device = request.device_info.unwrap_or_default();
	let rows = [row];
	let context = query::context(&**backend, &user, &rows, false).await?;
	let item = context.item(&rows[0], ItemShape::Expanded, &user.id, None);

	let current_time_ms = backend
		.progress(&user, &item_id)
		.await?
		.map(|progress| progress.position_ms)
		.unwrap_or(0);

	let now = Utc::now();
	let session = AbsSession {
		id: Uuid::new_v4().to_string(),
		user_id: user.id.clone(),
		media_id: item_id.clone(),
		library_id: item.library_id.clone(),
		device_id: device.device_id.clone(),
		client_name: device.client_name.clone(),
		client_version: device.client_version.clone(),
		media_player: request.media_player.clone(),
		current_time_ms,
		time_listening_ms: 0,
		started_at: now,
		updated_at: now,
	};
	backend.create_session(session.clone()).await?;

	Ok(Json(mapper::session_dto(mapper::SessionInput {
		session_id: &session.id,
		user_id: &user.id,
		item,
		audio: &audio,
		device,
		media_player: session.media_player.clone(),
		current_time_ms,
		time_listening_ms: 0,
		started_at: now,
		updated_at: now,
	})))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct AuthorQuery {
	include: Option<String>,
}

/// `GET /api/authors/{id}?include=items`. Stump has no author row: the id was
/// allocated for a `media_metadata.writers` value, so it is resolved back to
/// that name and the books are the visible audio books crediting it. Lissen
/// decodes the items as minified rows
/// (`common/model/metadata/AuthorItemsResponse.kt` →
/// `library/model/LibraryItemsResponse.kt:16-45`).
pub(crate) async fn author(
	backend: Backend,
	Extension(user): User,
	Path(author_id): Path<String>,
	Query(params): Query<AuthorQuery>,
) -> AbsResult<Json<AuthorDto>> {
	let name = backend
		.author_name(&author_id)
		.await?
		.ok_or_else(|| AbsError::NotFound(format!("No author {author_id}")))?;

	// The author's books, across every library the request may see.
	let media_ids = media_metadata::Entity::find()
		.filter(media_metadata::Column::Writers.contains(name.as_str()))
		.select_only()
		.column(media_metadata::Column::MediaId)
		.into_tuple::<Option<String>>()
		.all(backend.conn())
		.await?
		.into_iter()
		.flatten()
		.collect::<Vec<_>>();

	let rows = if media_ids.is_empty() {
		Vec::new()
	} else {
		query::audio_media(&user)
			.filter(media::Column::Id.is_in(media_ids))
			.order_by_asc(media::Column::Name)
			.all(backend.conn())
			.await?
	};
	// `contains` matches substrings, so a name that is a prefix of another
	// author's would drag their books in; the CSV values decide.
	let rows = {
		let context = query::context(&**backend, &user, &rows, false).await?;
		let rows = rows
			.iter()
			.filter(|row| {
				context
					.metadata
					.get(&row.id)
					.map(|metadata| {
						mapper::csv(metadata.writers.as_deref()).contains(&name)
					})
					.unwrap_or(false)
			})
			.cloned()
			.collect::<Vec<_>>();
		rows
	};

	let context = query::context(&**backend, &user, &rows, false).await?;
	let library_id = rows
		.first()
		.map(|row| {
			context
				.item(row, ItemShape::Minified, &user.id, None)
				.library_id
		})
		.unwrap_or_default();
	let facts = libraries::AuthorFacts {
		name,
		num_books: rows.len() as i64,
		added_at: rows
			.iter()
			.map(|row| row.created_at.timestamp_millis())
			.min()
			.unwrap_or_else(|| Utc::now().timestamp_millis()),
	};

	let mut dto = libraries::author_dto(author_id, &facts, &library_id, true);
	if params
		.include
		.as_deref()
		.map(|include| include.contains("items"))
		.unwrap_or(false)
	{
		dto.library_items = Some(
			rows.iter()
				.map(|row| context.item(row, ItemShape::Minified, &user.id, None))
				.collect(),
		);
	}
	Ok(Json(dto))
}

/// `GET /api/authors/{id}/image`. Stump stores no author images — there is no
/// author row to hang one off — so this is always a 404, which is what Lissen
/// treats as "no image" (`AudiobookshelfApiClient.kt:189`).
pub(crate) async fn author_image(Path(author_id): Path<String>) -> AbsResult<StatusCode> {
	Err(AbsError::NotFound(format!(
		"No image for author {author_id}"
	)))
}

/// The library ids that hold at least one audible book, in `displayOrder`,
/// for `userDefaultLibraryId`.
pub(crate) async fn first_audio_library(
	backend: &dyn AbsBackend,
	user: &AuthUser,
) -> AbsResult<Option<String>> {
	let with_audio = query::audio_library_ids(backend, user).await?;
	Ok(models::entity::library::Entity::find_for_user(user)
		.order_by_asc(models::entity::library::Column::CreatedAt)
		.order_by_asc(models::entity::library::Column::Id)
		.all(backend.conn())
		.await?
		.into_iter()
		.find(|library| with_audio.contains(&library.id))
		.map(|library| library.id))
}

/// Every audible book the user may see, for the whole-account reads
/// (`GET /api/me`).
pub(crate) async fn all_audio_media(
	backend: &dyn AbsBackend,
	user: &AuthUser,
) -> AbsResult<Vec<media::Model>> {
	Ok(query::audio_media(user)
		.filter(series::Column::LibraryId.is_not_null())
		.order_by_asc(media::Column::Name)
		.all(backend.conn())
		.await?)
}
