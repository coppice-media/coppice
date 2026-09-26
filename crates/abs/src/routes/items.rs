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
use models::entity::{media, series, user::AuthUser};
use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};
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
	/// (`AudiobookshelfApiClient.kt:192`) and `width` for a thumbnail
	/// (`:198`). Stump serves one rendition, so both are accepted and the
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

/// `GET /api/items/{id}/cover`.
///
/// Unauthenticated on purpose: from Audiobookshelf 2.17.0 the official app
/// stops putting a token on image requests
/// (`store/index.js:89-102` — `getDoesServerImagesRequireToken` is *false*
/// for any server >= 2.17 — used by `store/globals.js:54-56,68-70`; the
/// native notification art loader does the same at
/// The profile reports 2.36.1, so every cover request from this app arrives
/// anonymous, and answering `401` is what left the app's shelves blank.
///
/// A credential is still honoured when one is present, and the row must
/// still be an audible, undeleted book: the route is a cover lane, not a
/// generic file oracle.
pub(crate) async fn cover(
	backend: Backend,
	user: Option<Extension<AuthUser>>,
	Path(item_id): Path<String>,
	Query(_): Query<CoverQuery>,
) -> AbsResult<Response<Body>> {
	let user = match user {
		Some(Extension(user)) => {
			query::media_for_user(&**backend, &user, &item_id).await?;
			Some(user)
		},
		None => {
			query::audio_media_row(&**backend, &item_id).await?;
			None
		},
	};
	let image = backend.cover(user.as_ref(), &item_id).await?;
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

/// `GET /api/items/{id}/file/{ino}/download` — the same bytes as
/// [`file`], with `Content-Disposition: attachment`.
///
/// This is the route the official app's downloader actually uses, once per
/// audio track and once for the paired ebook
/// (`android/.../plugins/AbsDownloader.kt:184,201,242`); `ino` there is
/// `audioFiles[].ino` for a track and `media.ebookFile.ino` for the ebook,
/// so both kinds of id land here.
pub(crate) async fn file_download(
	backend: Backend,
	Extension(user): User,
	Extension(auth): Extension<AuthContext>,
	Path((item_id, ino)): Path<(String, String)>,
	headers: HeaderMap,
) -> AbsResult<Response<Body>> {
	query::media_for_user(&**backend, &user, &item_id).await?;

	// The ebook part of a download names the paired row's media id, not a
	// track index, so that lane is resolved first.
	if let Some(ebook) = backend
		.ebook_editions(&user, std::slice::from_ref(&item_id))
		.await?
		.remove(&item_id)
		.filter(|ebook| ebook.media_id == ino)
	{
		let mut response = backend.serve_ebook(headers, &user, &ebook).await?;
		attach(&mut response, &ebook.path);
		return Ok(response);
	}

	let index = ino
		.parse::<i32>()
		.map_err(|_| AbsError::NotFound(format!("No file {ino}")))?;
	let track_path = backend
		.audio(&item_id)
		.await?
		.and_then(|audio| audio.track(index).map(|track| track.path.clone()))
		.ok_or_else(|| AbsError::NotFound(format!("No file {ino}")))?;
	let mut response = backend
		.serve_track(headers, &item_id, index, auth.device_id())
		.await?;
	attach(&mut response, &track_path);
	Ok(response)
}

/// `GET /api/items/{id}/download` — every file of the item as one zip.
///
/// abs-ref answers `application/zip` with
/// `Content-Disposition: attachment; filename="<title>.zip"`, and accepts
/// `?token=` as well as a bearer header, which the profile's auth middleware
/// already covers.
pub(crate) async fn download(
	backend: Backend,
	Extension(user): User,
	Path(item_id): Path<String>,
	headers: HeaderMap,
) -> AbsResult<Response<Body>> {
	query::media_for_user(&**backend, &user, &item_id).await?;
	let ebook = backend
		.ebook_editions(&user, std::slice::from_ref(&item_id))
		.await?
		.remove(&item_id);
	backend
		.download_item(headers, &user, &item_id, ebook.as_ref())
		.await
}

/// `GET /api/items/{id}/ebook` and `/ebook/{fileId}` — the paired EPUB.
///
/// The app's reader fetches the second form when it knows a file id and the
/// first otherwise (`components/readers/Reader.vue:325-335`), so both are
/// the same bytes; a `fileId` that is not this item's ebook is a `404`
/// rather than a silent fallback.
pub(crate) async fn ebook(
	backend: Backend,
	Extension(user): User,
	Path(item_id): Path<String>,
	headers: HeaderMap,
) -> AbsResult<Response<Body>> {
	serve_paired_ebook(backend, user, item_id, None, headers).await
}

pub(crate) async fn ebook_file(
	backend: Backend,
	Extension(user): User,
	Path((item_id, file_id)): Path<(String, String)>,
	headers: HeaderMap,
) -> AbsResult<Response<Body>> {
	serve_paired_ebook(backend, user, item_id, Some(file_id), headers).await
}

async fn serve_paired_ebook(
	backend: Backend,
	user: AuthUser,
	item_id: String,
	file_id: Option<String>,
	headers: HeaderMap,
) -> AbsResult<Response<Body>> {
	query::media_for_user(&**backend, &user, &item_id).await?;
	let ebook = backend
		.ebook_editions(&user, std::slice::from_ref(&item_id))
		.await?
		.remove(&item_id)
		.filter(|ebook| file_id.as_ref().is_none_or(|id| id == &ebook.media_id))
		.ok_or_else(|| AbsError::NotFound(format!("No ebook for {item_id}")))?;
	backend.serve_ebook(headers, &user, &ebook).await
}

/// `PATCH /api/items/{id}/ebook/{ino}/status` — abs-ref's toggle between a
/// primary and a supplementary ebook file
/// (`components/tables/ebook/EbookFilesTable.vue:84`).
///
/// Stump's pairing has no supplementary tier: an edition is an edition. The
/// route exists so the app's ebook table does not get a `404` on a tap, and
/// answers the item unchanged.
pub(crate) async fn ebook_status(
	backend: Backend,
	Extension(user): User,
	Path((item_id, _file_id)): Path<(String, String)>,
) -> AbsResult<Json<LibraryItemDto>> {
	let row = query::media_for_user(&**backend, &user, &item_id).await?;
	let rows = [row];
	let context = query::context(&**backend, &user, &rows, false).await?;
	Ok(Json(context.item(
		&rows[0],
		ItemShape::Expanded,
		&user.id,
		None,
	)))
}

/// `GET /public/session/{sessionId}/track/{index}` — the direct-play track
/// lane, unauthenticated.
///
/// **This is the route whose absence made the official app unusable.** From
/// Audiobookshelf 2.22.0 the app stops streaming `audioTracks[].contentUrl`
/// and streams this instead (`android/.../data/PlaybackSession.kt:196-204`,
/// `plugins/capacitor/AbsAudioPlayer.js:254-261`). Against a server that
/// does not serve it, ExoPlayer fails to load, which lands in
/// `PlayerNotificationService.handlePlayerPlaybackError`
/// (`android/.../player/PlayerNotificationService.kt:617-641`): a direct-play
/// session there is retried by re-POSTing `/api/items/{id}/play`, whose new
/// session is direct play again, which fails again — an unthrottled loop
/// that only stops at the inbound rate limiter.
///
/// `index` is `audioTracks[].index`, which is **1-based**, while the
/// profile's own track index and the `ino` of
/// `GET /api/items/{id}/file/{ino}` are 0-based (`mapper::audio_files`).
/// abs-ref answers `404` for index `0` and for an unknown session, honours
/// `Range` with a `206`, and sets `Accept-Ranges: bytes`.
pub(crate) async fn public_track(
	backend: Backend,
	Path((session_id, track)): Path<(String, String)>,
	headers: HeaderMap,
) -> AbsResult<Response<Body>> {
	let session = backend
		.session_by_id(&session_id)
		.await?
		.ok_or_else(|| AbsError::NotFound(format!("No session {session_id}")))?;
	let wire_index = track
		.parse::<i32>()
		.ok()
		.filter(|index| *index >= 1)
		.ok_or_else(|| AbsError::NotFound(format!("No track {track}")))?;
	backend
		.serve_track(
			headers,
			&session.media_id,
			wire_index - 1,
			session.device_id.as_deref(),
		)
		.await
}

/// Turn a byte response into a download by naming the file it came from.
fn attach(response: &mut Response<Body>, path: &str) {
	let filename = path.rsplit('/').next().unwrap_or(path);
	// The header is a quoted-string; a quote or a backslash in a file name
	// would otherwise end it early.
	let escaped = filename.replace('\\', r"\\").replace('"', "\\\"");
	if let Ok(value) =
		header::HeaderValue::from_str(&format!("attachment; filename=\"{escaped}\""))
	{
		response
			.headers_mut()
			.insert(header::CONTENT_DISPOSITION, value);
	}
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
		play_method: crate::routes::PLAY_METHOD_DIRECT,
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
		play_method: crate::routes::PLAY_METHOD_DIRECT,
		shape: crate::mapper::SessionShape::Full,
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
	let media_ids = query::media_ids_crediting(&**backend, &user, &name).await?;
	let rows = if media_ids.is_empty() {
		Vec::new()
	} else {
		query::audio_media(&user)
			.filter(media::Column::Id.is_in(media_ids))
			.order_by_asc(media::Column::Name)
			.all(backend.conn())
			.await?
	};
	// ABS 2.36.1 author endpoints return 404 when the caller cannot access the
	// author's library. Stump's author ids are global names, so require at least
	// one visible audiobook before emitting an author object.
	if rows.is_empty() {
		return Err(AbsError::NotFound(format!(
			"No visible books for author {author_id}"
		)));
	}

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
		updated_at: rows
			.iter()
			.map(|row| {
				row.updated_at
					.unwrap_or_else(|| row.created_at.clone())
					.timestamp_millis()
			})
			.max()
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
/// treats as "no image" (`AudiobookshelfApiClient.kt:205`).
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
