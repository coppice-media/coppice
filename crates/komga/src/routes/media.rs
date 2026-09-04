use std::{convert::TryFrom, path::PathBuf, sync::Arc};
use tokio::fs;

use crate::{
	DirectoryListing, DirectoryRequest, KomgaBookId, KomgaBookPage, KomgaBookThumbnail,
	KomgaSeriesThumbnail, KomgaThumbnailId, Path as KomgaPath,
};
use axum::{
	body::Body,
	extract::{Path, Query},
	http::{header, HeaderMap, HeaderValue, StatusCode},
	response::Response,
	routing::{get, post},
	Extension, Router,
};
use models::{
	entity::{library, media},
	shared::enums::UserPermission,
};
use sea_orm::{prelude::*, QuerySelect};
use serde::Deserialize;
use stump_auth::AuthContext;

use super::{KomgaBackend, KomgaImage};
use crate::errors::{APIError, APIResult};

use super::response::{cached_bytes, cached_json};

const BOOK_THUMBNAIL_PREFIX: &str = "stump-book-";

/// Komelia's page numbering is one-based. The compatibility layer deliberately does not expose
/// Komga's optional zero-based mode or content negotiation because Stump cannot honor either mode
/// without changing the meaning of a requested page.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PageQuery {
	#[serde(default)]
	convert: Option<String>,
	#[serde(default)]
	zero_based: Option<bool>,
	#[serde(default, rename = "contentNegotiation")]
	content_negotiation: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScanQuery {
	#[serde(default)]
	deep: Option<bool>,
}

#[derive(Debug)]
struct ThumbnailDetails {
	media_type: String,
	file_size: i64,
	width: i32,
	height: i32,
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::<S>::new()
		.route("/api/v1/books/{book_id}/pages", get(get_book_pages))
		.route("/api/v1/books/{book_id}/pages/{page}", get(get_book_page))
		.route(
			"/api/v1/books/{book_id}/pages/{page}/thumbnail",
			get(get_book_page_thumbnail),
		)
		.route("/api/v1/books/{book_id}/thumbnail", get(get_book_thumbnail))
		.route(
			"/api/v1/books/{book_id}/thumbnails",
			get(get_book_thumbnails),
		)
		.route(
			"/api/v1/books/{book_id}/thumbnails/{thumbnail_id}",
			get(get_book_thumbnail_by_id),
		)
		.route(
			"/api/v1/series/{series_id}/thumbnail",
			get(get_series_thumbnail),
		)
		.route(
			"/api/v1/series/{series_id}/thumbnails",
			get(get_series_thumbnails),
		)
		.route(
			"/api/v1/series/{series_id}/thumbnails/{thumbnail_id}",
			get(get_series_thumbnail_by_id),
		)
		.route("/api/v1/books/{book_id}/file", get(get_book_file))
		.route("/api/v1/libraries/{library_id}/scan", post(scan_library))
		.route("/api/v1/books/{book_id}/analyze", post(analyze_book))
		.route("/api/v1/filesystem", post(get_filesystem_listing))
}
async fn find_book(
	conn: &DatabaseConnection,
	user: &models::entity::user::AuthUser,
	id: &str,
) -> APIResult<media::Model> {
	media::Entity::find_for_user(user)
		.filter(media::Column::Id.eq(id.to_owned()))
		.one(conn)
		.await?
		.ok_or_else(|| APIError::NotFound("Book not found".to_string()))
}

fn thumbnail_id(prefix: &str, id: &str) -> KomgaThumbnailId {
	KomgaThumbnailId::new(format!("{prefix}{id}"))
}

fn book_thumbnail_id(id: &str) -> KomgaThumbnailId {
	thumbnail_id(BOOK_THUMBNAIL_PREFIX, id)
}

pub(crate) fn cache_image(
	headers: &HeaderMap,
	image: KomgaImage,
) -> APIResult<Response<Body>> {
	if !image.content_type.starts_with("image/") {
		return Err(APIError::BadRequest(
			"The requested asset is not an image".to_string(),
		));
	}

	cached_bytes(headers, &image.content_type, image.data)
}

fn validate_page_number(page: u32) -> APIResult<()> {
	if page == 0 || page > i32::MAX as u32 {
		return Err(APIError::BadRequest(
			"Page numbers are one-based and must fit in a signed 32-bit integer"
				.to_string(),
		));
	}

	Ok(())
}

fn validate_page_query(query: &PageQuery) -> APIResult<()> {
	if query.zero_based.unwrap_or(false) {
		return Err(APIError::BadRequest(
			"Komga zero-based page numbering is not supported".to_string(),
		));
	}
	if query.content_negotiation == Some(false) {
		return Err(APIError::BadRequest(
			"Disabling content negotiation is not supported".to_string(),
		));
	}
	if query.convert.is_some() {
		return Err(APIError::BadRequest(
			"Page format conversion is not supported".to_string(),
		));
	}

	Ok(())
}

async fn build_book_pages(
	backend: &dyn KomgaBackend,
	book: &media::Model,
) -> APIResult<Vec<KomgaBookPage>> {
	backend.book_pages(book).await
}
async fn load_book_page(
	backend: &dyn KomgaBackend,
	user: &models::entity::user::AuthUser,
	book_id: String,
	page: u32,
) -> APIResult<KomgaImage> {
	backend.book_page(user, book_id, page).await
}
async fn get_book_pages(
	Path(book_id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = req.user();
	let book = find_book(ctx.conn(), &user, &book_id).await?;
	let pages = build_book_pages(ctx.as_ref(), &book).await?;
	cached_json(&headers, &pages)
}

async fn get_book_page(
	Path((book_id, page)): Path<(String, u32)>,
	Query(query): Query<PageQuery>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	validate_page_number(page)?;
	validate_page_query(&query)?;
	let image = load_book_page(ctx.as_ref(), &req.user(), book_id, page).await?;
	cache_image(&headers, image)
}

async fn get_book_page_thumbnail(
	Path((book_id, page)): Path<(String, u32)>,
	Query(query): Query<PageQuery>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	validate_page_number(page)?;
	validate_page_query(&query)?;
	let image = ctx.book_page_thumbnail(&req.user(), book_id, page).await?;
	if !image.content_type.starts_with("image/") {
		return Err(APIError::BadRequest(
			"Only image pages have thumbnails".to_string(),
		));
	}

	let mut image = image;
	if image.width.is_none() || image.height.is_none() {
		return Err(APIError::BadRequest(
			"Page thumbnail metadata is unavailable".to_string(),
		));
	}
	image.content_type = "image/jpeg".to_string();
	cache_image(&headers, image)
}

pub(crate) async fn load_book_thumbnail(
	backend: &dyn KomgaBackend,
	user: &models::entity::user::AuthUser,
	book_id: String,
) -> APIResult<KomgaImage> {
	backend.book_thumbnail(user, book_id).await
}

pub(crate) async fn load_series_thumbnail(
	backend: &dyn KomgaBackend,
	user: &models::entity::user::AuthUser,
	series_id: &str,
) -> APIResult<KomgaImage> {
	backend.series_thumbnail(user, series_id).await
}

async fn thumbnail_details(image: KomgaImage) -> APIResult<ThumbnailDetails> {
	if !image.content_type.starts_with("image/") {
		return Err(APIError::BadRequest(
			"The requested asset is not an image".to_string(),
		));
	}
	let file_size = i64::try_from(image.data.len())?;
	let (Some(width), Some(height)) = (image.width, image.height) else {
		return Err(APIError::BadRequest(
			"Thumbnail dimensions are unavailable".to_string(),
		));
	};
	Ok(ThumbnailDetails {
		media_type: image.content_type,
		file_size,
		width,
		height,
	})
}

async fn get_book_thumbnail(
	Path(book_id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let image = load_book_thumbnail(ctx.as_ref(), &req.user(), book_id).await?;
	cache_image(&headers, image)
}

async fn get_series_thumbnail(
	Path(series_id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let image = load_series_thumbnail(ctx.as_ref(), &req.user(), &series_id).await?;
	cache_image(&headers, image)
}

async fn get_book_thumbnails(
	Path(book_id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = req.user();
	let details = thumbnail_details(
		load_book_thumbnail(ctx.as_ref(), &user, book_id.clone()).await?,
	)
	.await?;
	let thumbnails = vec![KomgaBookThumbnail {
		id: book_thumbnail_id(&book_id),
		book_id: KomgaBookId::new(book_id),
		r#type: "GENERATED".to_string(),
		selected: true,
		media_type: details.media_type,
		file_size: details.file_size,
		width: details.width,
		height: details.height,
	}];

	cached_json(&headers, &thumbnails)
}

async fn get_book_thumbnail_by_id(
	Path((book_id, thumbnail)): Path<(String, String)>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	if thumbnail != book_thumbnail_id(&book_id).to_string() {
		return Err(APIError::NotFound("Thumbnail not found".to_string()));
	}
	let image = load_book_thumbnail(ctx.as_ref(), &req.user(), book_id).await?;
	cache_image(&headers, image)
}

/// Komga's `ThumbnailSeries.Type` is only `SIDECAR | USER_UPLOADED`: a series
/// cover generated from its first book is served by `/series/{id}/thumbnail`
/// but never appears in the thumbnail list. Komelia's offline import calls an
/// unguarded `valueOf` on this field, so listing a `GENERATED` entry here
/// crashed every download at the series import step. Stump has no persisted
/// sidecar or uploaded series artwork, so the list is empty by contract.
async fn get_series_thumbnails(
	Path(series_id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	// Visibility check keeps the 404-for-hidden-series behavior of the
	// other series routes.
	load_series_thumbnail(ctx.as_ref(), &req.user(), &series_id).await?;
	cached_json(&headers, &Vec::<KomgaSeriesThumbnail>::new())
}

async fn get_series_thumbnail_by_id(
	Path((_series_id, _thumbnail)): Path<(String, String)>,
) -> APIResult<Response<Body>> {
	Err(APIError::NotFound("Thumbnail not found".to_string()))
}

fn download_mime_type(extension: &str) -> Option<String> {
	let mime = match extension.to_ascii_lowercase().as_str() {
		"cbz" => "application/vnd.comicbook+zip",
		"cbr" => "application/vnd.comicbook-rar",
		"cb7" => "application/x-7z-compressed",
		"pdf" => "application/pdf",
		"epub" => "application/epub+zip",
		"zip" => "application/zip",
		"rar" => "application/vnd.rar",
		_ => return None,
	};
	Some(mime.to_string())
}

async fn get_book_file(
	Path(book_id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = req.user();
	let mut response = ctx.serve_book_file(req, headers, book_id.clone()).await?;
	let extension = media::Entity::find_for_user(&user)
		.filter(media::Column::Id.eq(book_id))
		.select_only()
		.column(media::Column::Extension)
		.into_tuple::<String>()
		.one(ctx.conn())
		.await?
		.ok_or_else(|| {
			APIError::InternalServerError(
				"Book disappeared while serving file".to_owned(),
			)
		})?;

	if let Some(mime_type) = download_mime_type(&extension) {
		let value = HeaderValue::from_str(&mime_type)
			.map_err(|error| APIError::InternalServerError(error.to_string()))?;
		response.headers_mut().insert(header::CONTENT_TYPE, value);
	}
	Ok(response)
}

async fn scan_library(
	Path(library_id): Path<String>,
	Query(query): Query<ScanQuery>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<StatusCode> {
	req.enforce_permissions(&[UserPermission::ScanLibrary])
		.map_err(|_| APIError::forbidden_discreet())?;

	let user = req.user();
	let library = library::Entity::find_for_user(&user)
		.filter(library::Column::Id.eq(library_id))
		.into_model::<library::LibraryIdentSelect>()
		.one(ctx.conn())
		.await?
		.ok_or_else(|| APIError::NotFound("Library not found".to_string()))?;

	ctx.enqueue_library_scan(library.id, library.path, query.deep.unwrap_or(false))
		.await?;

	Ok(StatusCode::ACCEPTED)
}

async fn analyze_book(
	Path(book_id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<StatusCode> {
	req.enforce_permissions(&[UserPermission::ManageLibrary])
		.map_err(|_| APIError::forbidden_discreet())?;

	let user = req.user();
	let book = media::Entity::find_for_user(&user)
		.select_only()
		.columns(media::MediaIdentSelect::columns())
		.filter(media::Column::Id.eq(book_id))
		.into_model::<media::MediaIdentSelect>()
		.one(ctx.conn())
		.await?
		.ok_or_else(|| APIError::NotFound("Book not found".to_string()))?;

	ctx.enqueue_book_analysis(book.id).await?;
	Ok(StatusCode::ACCEPTED)
}

async fn get_filesystem_listing(
	Extension(_ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
	axum::Json(request): axum::Json<DirectoryRequest>,
) -> APIResult<Response<Body>> {
	req.enforce_server_owner()
		.map_err(|_| APIError::forbidden_discreet())?;

	let requested = PathBuf::from(request.path);
	if !requested.is_absolute() {
		return Err(APIError::BadRequest("Path must be absolute".to_string()));
	}

	let canonical = fs::canonicalize(&requested)
		.await
		.map_err(|_| APIError::BadRequest("Path does not exist".to_string()))?;
	let canonical_metadata = fs::metadata(&canonical)
		.await
		.map_err(|_| APIError::BadRequest("Path does not exist".to_string()))?;
	if !canonical_metadata.is_dir() {
		return Err(APIError::BadRequest(
			"Path must refer to a directory".to_string(),
		));
	}

	let mut entries = Vec::new();
	let mut directory = fs::read_dir(&canonical)
		.await
		.map_err(|_| APIError::BadRequest("Path could not be listed".to_string()))?;
	while let Some(entry) = directory
		.next_entry()
		.await
		.map_err(|_| APIError::BadRequest("Path could not be listed".to_string()))?
	{
		let name = entry.file_name().to_string_lossy().into_owned();
		if name.starts_with('.') {
			continue;
		}

		let file_type = match entry.file_type().await {
			Ok(file_type) => file_type,
			Err(_) => continue,
		};
		if !file_type.is_dir() && !file_type.is_symlink() {
			continue;
		}

		let child = match fs::canonicalize(entry.path()).await {
			Ok(child) => child,
			Err(_) => continue,
		};
		if child == canonical || !child.starts_with(&canonical) {
			continue;
		}
		match fs::metadata(&child).await {
			Ok(metadata) if metadata.is_dir() => {},
			_ => continue,
		}

		entries.push(KomgaPath {
			r#type: "directory".to_string(),
			name,
			path: child.to_string_lossy().into_owned(),
		});
	}

	entries.sort_by(|left, right| {
		left.name
			.to_ascii_lowercase()
			.cmp(&right.name.to_ascii_lowercase())
			.then_with(|| left.name.cmp(&right.name))
			.then_with(|| left.path.cmp(&right.path))
	});

	let parent = canonical
		.parent()
		.unwrap_or(canonical.as_path())
		.to_string_lossy()
		.into_owned();
	cached_json(
		&headers,
		&DirectoryListing {
			parent: Some(parent),
			directories: entries,
		},
	)
}

#[cfg(test)]
mod tests {
	use super::download_mime_type;

	#[test]
	fn cbz_download_uses_the_comic_zip_mime_type() {
		assert_eq!(
			download_mime_type("cbz").as_deref(),
			Some("application/vnd.comicbook+zip")
		);
		assert_ne!(
			download_mime_type("cbz").as_deref(),
			Some("application/x-cbr")
		);
	}
}
