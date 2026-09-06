use std::sync::Arc;

use crate::KomgaBookPage;
use axum::{
	body::Body,
	extract::{Multipart, Path, Query},
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

/// Komelia's page numbering is one-based. The compatibility layer deliberately does not expose
/// Komga's optional zero-based mode or content negotiation because Stump cannot honor either mode
/// without changing the meaning of a requested page.
#[derive(Debug, Clone, Deserialize)]
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
			get(get_book_thumbnails).post(upload_book_thumbnail),
		)
		.route(
			"/api/v1/books/{book_id}/thumbnails/{thumbnail_id}",
			get(get_book_thumbnail_by_id).delete(delete_book_thumbnail),
		)
		.route(
			"/api/v1/series/{series_id}/thumbnail",
			get(get_series_thumbnail),
		)
		.route(
			"/api/v1/series/{series_id}/thumbnails",
			get(get_series_thumbnails).post(upload_series_thumbnail),
		)
		.route(
			"/api/v1/series/{series_id}/thumbnails/{thumbnail_id}",
			get(get_series_thumbnail_by_id).delete(delete_series_thumbnail),
		)
		.route("/api/v1/books/{book_id}/file", get(get_book_file))
		.route("/api/v1/libraries/{library_id}/scan", post(scan_library))
		.route("/api/v1/books/{book_id}/analyze", post(analyze_book))
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

fn enforce_manage_library(auth: &AuthContext) -> APIResult<()> {
	auth.enforce_permissions(&[UserPermission::ManageLibrary])
		.map_err(|_| APIError::forbidden_discreet())
}

async fn parse_thumbnail_upload(mut multipart: Multipart) -> APIResult<(Vec<u8>, bool)> {
	let mut file = None;
	let mut selected = None;
	while let Some(field) = multipart
		.next_field()
		.await
		.map_err(|error| APIError::BadRequest(error.to_string()))?
	{
		match field.name() {
			Some("file") => {
				file = Some(
					field
						.bytes()
						.await
						.map_err(|error| APIError::BadRequest(error.to_string()))?
						.to_vec(),
				);
			},
			Some("selected") => {
				let value = field
					.text()
					.await
					.map_err(|error| APIError::BadRequest(error.to_string()))?;
				selected = Some(value.parse::<bool>().map_err(|_| {
					APIError::BadRequest(
						"Thumbnail selected must be a boolean".to_string(),
					)
				})?);
			},
			_ => {},
		}
	}
	let file = file
		.ok_or_else(|| APIError::BadRequest("Thumbnail file is required".to_string()))?;
	Ok((file, selected.unwrap_or(true)))
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
	let pages = backend.book_pages(book).await?;
	// Duplicate-page skipping renumbers the visible pages onto the physical
	// file: the list stays contiguous 1..=N while skipped physical pages
	// disappear. Nothing changes when nothing is skipped.
	let Some(visible) = backend.visible_pages(book).await? else {
		return Ok(pages);
	};
	Ok(pages
		.into_iter()
		.filter(|page| visible.contains(&page.number))
		.enumerate()
		.map(|(index, mut page)| {
			page.number = index as i32 + 1;
			page.file_name = format!("page-{}", page.number);
			page
		})
		.collect())
}

/// Map a visible 1-based page number onto its physical page. Returns the
/// number unchanged when nothing is skipped; a visible number past the end
/// of the renumbered list is not found.
async fn visible_to_physical(
	backend: &dyn KomgaBackend,
	book: &media::Model,
	page: u32,
) -> APIResult<u32> {
	let Some(visible) = backend.visible_pages(book).await? else {
		return Ok(page);
	};
	let page = usize::try_from(page)
		.map_err(|_| APIError::BadRequest("page index out of range".to_string()))?;
	visible
		.get(
			page.checked_sub(1).ok_or_else(|| {
				APIError::BadRequest("page index out of range".to_string())
			})?,
		)
		.copied()
		.and_then(|physical| u32::try_from(physical).ok())
		.ok_or(APIError::NotFound("Page not found".to_string()))
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
	let user = req.user();
	let book = find_book(ctx.conn(), &user, &book_id).await?;
	let physical = visible_to_physical(ctx.as_ref(), &book, page).await?;
	let image = load_book_page(ctx.as_ref(), &user, book_id, physical).await?;
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
	let user = req.user();
	let book = find_book(ctx.conn(), &user, &book_id).await?;
	let physical = visible_to_physical(ctx.as_ref(), &book, page).await?;
	let image = ctx.book_page_thumbnail(&user, book_id, physical).await?;
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
	let thumbnails = ctx.book_thumbnails(&req.user(), book_id).await?;
	cached_json(&headers, &thumbnails)
}

async fn get_book_thumbnail_by_id(
	Path((book_id, thumbnail_id)): Path<(String, String)>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let image = ctx
		.book_thumbnail_by_id(&req.user(), book_id, thumbnail_id)
		.await?;
	cache_image(&headers, image)
}

async fn upload_book_thumbnail(
	Path(book_id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
	multipart: Multipart,
) -> APIResult<Response<Body>> {
	enforce_manage_library(&req)?;
	let (bytes, selected) = parse_thumbnail_upload(multipart).await?;
	let thumbnail = ctx
		.upload_book_thumbnail(&req.user(), book_id, bytes, selected)
		.await?;
	cached_json(&headers, &thumbnail)
}

async fn delete_book_thumbnail(
	Path((book_id, thumbnail_id)): Path<(String, String)>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<StatusCode> {
	enforce_manage_library(&req)?;
	ctx.delete_book_thumbnail(&req.user(), book_id, thumbnail_id)
		.await?;
	Ok(StatusCode::NO_CONTENT)
}

/// Komga's `ThumbnailSeries.Type` is only `SIDECAR | USER_UPLOADED`: a series
/// cover generated from its first book is served by `/series/{id}/thumbnail`
/// but never appears in the thumbnail list. Komelia's offline import calls an
/// unguarded `valueOf` on this field, so listing a `GENERATED` entry here would
/// crash every download at the series import step.
async fn get_series_thumbnails(
	Path(series_id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let thumbnails = ctx.series_thumbnails(&req.user(), series_id).await?;
	cached_json(&headers, &thumbnails)
}

async fn get_series_thumbnail_by_id(
	Path((series_id, thumbnail_id)): Path<(String, String)>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let image = ctx
		.series_thumbnail_by_id(&req.user(), series_id, thumbnail_id)
		.await?;
	cache_image(&headers, image)
}

async fn upload_series_thumbnail(
	Path(series_id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
	multipart: Multipart,
) -> APIResult<Response<Body>> {
	enforce_manage_library(&req)?;
	let (bytes, selected) = parse_thumbnail_upload(multipart).await?;
	let thumbnail = ctx
		.upload_series_thumbnail(&req.user(), series_id, bytes, selected)
		.await?;
	cached_json(&headers, &thumbnail)
}

async fn delete_series_thumbnail(
	Path((series_id, thumbnail_id)): Path<(String, String)>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<StatusCode> {
	enforce_manage_library(&req)?;
	ctx.delete_series_thumbnail(&req.user(), series_id, thumbnail_id)
		.await?;
	Ok(StatusCode::NO_CONTENT)
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

#[cfg(test)]
mod renumbering_tests {
	use super::{get_book_page, get_book_pages, PageQuery};
	use crate::{
		errors::APIError,
		routes::{KomgaBackend, KomgaCoreEvent, KomgaImage},
		KomgaBookPage, KomgaBookThumbnail, KomgaSeriesThumbnail,
	};
	use axum::{
		extract::{Path, Query},
		Extension,
	};
	use models::{
		entity::{media, series, user::AuthUser},
		shared::enums::FileStatus,
	};
	use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
	use std::sync::Arc;
	use stump_auth::AuthContext;

	struct RenumberBackend {
		conn: Arc<DatabaseConnection>,
	}

	fn pages(count: i32) -> Vec<KomgaBookPage> {
		(1..=count)
			.map(|number| KomgaBookPage {
				number,
				file_name: format!("page-{number}"),
				media_type: "image/jpeg".to_string(),
				width: None,
				height: None,
				size_bytes: None,
				size: "0 B".to_owned(),
			})
			.collect()
	}

	#[async_trait::async_trait]
	impl KomgaBackend for RenumberBackend {
		fn conn(&self) -> &DatabaseConnection {
			&self.conn
		}

		fn conn_arc(&self) -> Arc<DatabaseConnection> {
			self.conn.clone()
		}

		fn core_events(&self) -> tokio::sync::broadcast::Receiver<KomgaCoreEvent> {
			tokio::sync::broadcast::channel(1).1
		}

		async fn book_pages(
			&self,
			_book: &media::Model,
		) -> crate::errors::APIResult<Vec<KomgaBookPage>> {
			Ok(pages(5))
		}

		async fn visible_pages(
			&self,
			book: &media::Model,
		) -> crate::errors::APIResult<Option<Vec<i32>>> {
			// Physical page 2 was marked SKIP in the seeded library; page 5
			// is also skipped to prove non-contiguous renumbering.
			if book.name == "skipped" {
				Ok(Some(vec![1, 3, 4]))
			} else {
				Ok(None)
			}
		}

		async fn book_page(
			&self,
			_user: &AuthUser,
			_book_id: String,
			page: u32,
		) -> crate::errors::APIResult<KomgaImage> {
			// The served bytes carry the physical page that was read.
			Ok(KomgaImage::new("image/jpeg", vec![page as u8]))
		}

		async fn book_page_thumbnail(
			&self,
			user: &AuthUser,
			book_id: String,
			page: u32,
		) -> crate::errors::APIResult<KomgaImage> {
			let mut image = self.book_page(user, book_id, page).await?;
			image.width = Some(1);
			image.height = Some(1);
			Ok(image)
		}

		async fn enqueue_library_scan(
			&self,
			_library_id: String,
			_path: String,
			_deep: bool,
		) -> crate::errors::APIResult<()> {
			unimplemented!("not used by the renumbering route test")
		}

		async fn enqueue_library_analysis(
			&self,
			_library_id: String,
		) -> crate::errors::APIResult<()> {
			unimplemented!()
		}

		async fn create_library(
			&self,
			_request: crate::KomgaLibraryCreateRequest,
		) -> crate::errors::APIResult<models::entity::library::Model> {
			unimplemented!()
		}

		async fn update_library(
			&self,
			_user: &models::entity::user::AuthUser,
			_id: &str,
			_request: crate::KomgaLibraryUpdateRequest,
		) -> crate::errors::APIResult<()> {
			unimplemented!()
		}

		async fn delete_library(
			&self,
			_user: &models::entity::user::AuthUser,
			_id: &str,
		) -> crate::errors::APIResult<()> {
			unimplemented!()
		}

		async fn enqueue_series_analysis(
			&self,
			_series_id: String,
		) -> crate::errors::APIResult<()> {
			unimplemented!()
		}

		fn library_roots(&self) -> Vec<String> {
			Vec::new()
		}

		async fn create_collection(
			&self,
			_user: &models::entity::user::AuthUser,
			_name: String,
			_ordered: bool,
			_series_ids: Vec<String>,
		) -> crate::errors::APIResult<models::entity::collection::Model> {
			unimplemented!()
		}

		async fn update_collection(
			&self,
			_user: &models::entity::user::AuthUser,
			_id: &str,
			_name: Option<String>,
			_ordered: Option<bool>,
			_series_ids: Option<Vec<String>>,
		) -> crate::errors::APIResult<()> {
			unimplemented!()
		}

		async fn delete_collection(
			&self,
			_user: &models::entity::user::AuthUser,
			_id: &str,
		) -> crate::errors::APIResult<()> {
			unimplemented!()
		}

		async fn create_read_list(
			&self,
			_user: &models::entity::user::AuthUser,
			_name: String,
			_summary: Option<String>,
			_ordered: bool,
			_book_ids: Vec<String>,
		) -> crate::errors::APIResult<models::entity::reading_list::Model> {
			unimplemented!()
		}

		async fn update_read_list(
			&self,
			_user: &models::entity::user::AuthUser,
			_id: &str,
			_name: Option<String>,
			_summary: Option<Option<String>>,
			_ordered: Option<bool>,
			_book_ids: Option<Vec<String>>,
		) -> crate::errors::APIResult<()> {
			unimplemented!()
		}

		async fn delete_read_list(
			&self,
			_user: &models::entity::user::AuthUser,
			_id: &str,
		) -> crate::errors::APIResult<()> {
			unimplemented!()
		}

		async fn enqueue_book_analysis(
			&self,
			_book_id: String,
		) -> crate::errors::APIResult<()> {
			unimplemented!()
		}

		async fn book_thumbnail(
			&self,
			_user: &AuthUser,
			_book_id: String,
		) -> crate::errors::APIResult<KomgaImage> {
			unimplemented!()
		}

		async fn book_thumbnails(
			&self,
			_user: &AuthUser,
			_book_id: String,
		) -> crate::errors::APIResult<Vec<KomgaBookThumbnail>> {
			unimplemented!()
		}

		async fn book_thumbnail_by_id(
			&self,
			_user: &AuthUser,
			_book_id: String,
			_thumbnail_id: String,
		) -> crate::errors::APIResult<KomgaImage> {
			unimplemented!()
		}

		async fn upload_book_thumbnail(
			&self,
			_user: &AuthUser,
			_book_id: String,
			_bytes: Vec<u8>,
			_selected: bool,
		) -> crate::errors::APIResult<KomgaBookThumbnail> {
			unimplemented!()
		}

		async fn delete_book_thumbnail(
			&self,
			_user: &AuthUser,
			_book_id: String,
			_thumbnail_id: String,
		) -> crate::errors::APIResult<()> {
			unimplemented!()
		}

		async fn series_thumbnail(
			&self,
			_user: &AuthUser,
			_series_id: &str,
		) -> crate::errors::APIResult<KomgaImage> {
			unimplemented!()
		}

		async fn series_thumbnails(
			&self,
			_user: &AuthUser,
			_series_id: String,
		) -> crate::errors::APIResult<Vec<KomgaSeriesThumbnail>> {
			unimplemented!()
		}

		async fn series_thumbnail_by_id(
			&self,
			_user: &AuthUser,
			_series_id: String,
			_thumbnail_id: String,
		) -> crate::errors::APIResult<KomgaImage> {
			unimplemented!()
		}

		async fn upload_series_thumbnail(
			&self,
			_user: &AuthUser,
			_series_id: String,
			_bytes: Vec<u8>,
			_selected: bool,
		) -> crate::errors::APIResult<KomgaSeriesThumbnail> {
			unimplemented!()
		}

		async fn delete_series_thumbnail(
			&self,
			_user: &AuthUser,
			_series_id: String,
			_thumbnail_id: String,
		) -> crate::errors::APIResult<()> {
			unimplemented!()
		}

		async fn serve_book_file(
			&self,
			_auth: AuthContext,
			_headers: axum::http::HeaderMap,
			_book_id: String,
		) -> crate::errors::APIResult<axum::response::Response<axum::body::Body>> {
			unimplemented!()
		}

		async fn readium_manifest(
			&self,
			_path: String,
			_base_url: String,
		) -> crate::errors::APIResult<serde_json::Value> {
			unimplemented!()
		}

		async fn readium_positions(
			&self,
			_path: String,
			_base_url: String,
		) -> crate::errors::APIResult<serde_json::Value> {
			unimplemented!()
		}

		async fn readium_resource(
			&self,
			_path: String,
			_resource_path: std::path::PathBuf,
		) -> crate::errors::APIResult<KomgaImage> {
			unimplemented!()
		}

		async fn record_sync(&self, _auth: &AuthContext, _summary: serde_json::Value) {
			unimplemented!()
		}
	}

	async fn seed_book(conn: &DatabaseConnection, name: &str) {
		series::ActiveModel {
			id: Set(format!("series-{name}")),
			name: Set("Series".to_string()),
			path: Set("/lib/series".to_string()),
			status: Set(FileStatus::Ready),
			library_id: Set(Some("lib".to_string())),
			..Default::default()
		}
		.insert(conn)
		.await
		.unwrap();
		media::ActiveModel {
			id: Set(format!("book-{name}")),
			name: Set(name.to_string()),
			path: Set(format!("/lib/series/{name}.cbz")),
			extension: Set("cbz".to_string()),
			series_id: Set(Some(format!("series-{name}"))),
			pages: Set(5),
			size: Set(1),
			status: Set(FileStatus::Ready),
			..Default::default()
		}
		.insert(conn)
		.await
		.unwrap();
	}

	async fn body(response: axum::response::Response<axum::body::Body>) -> Vec<u8> {
		axum::body::to_bytes(response.into_body(), usize::MAX)
			.await
			.unwrap()
			.to_vec()
	}

	fn auth() -> Extension<AuthContext> {
		Extension(AuthContext {
			user: AuthUser::default(),
			api_key: None,
		})
	}

	#[tokio::test]
	async fn page_routes_renumber_visible_pages() {
		let conn = ::tests::db::test_database().await;
		::tests::fake_data::Library {
			id: Some("lib".to_string()),
			..Default::default()
		}
		.insert(&conn)
		.await;
		seed_book(&conn, "skipped").await;
		seed_book(&conn, "plain").await;
		let backend: Arc<dyn KomgaBackend> = Arc::new(RenumberBackend {
			conn: Arc::new(conn),
		});
		let query = Query(PageQuery {
			convert: None,
			zero_based: None,
			content_negotiation: None,
		});

		// The page list is renumbered 1..=3 while nothing else changes.
		let response = get_book_pages(
			Path("book-skipped".to_string()),
			Extension(backend.clone()),
			auth(),
			Default::default(),
		)
		.await
		.unwrap();
		let listed =
			serde_json::from_slice::<Vec<KomgaBookPage>>(&body(response).await).unwrap();
		assert_eq!(
			listed.iter().map(|page| page.number).collect::<Vec<_>>(),
			vec![1, 2, 3]
		);

		// Visible page 2 reads physical page 3; page 3 reads physical 4.
		for (visible, physical) in [(1u32, 1u8), (2, 3), (3, 4)] {
			let response = get_book_page(
				Path(("book-skipped".to_string(), visible)),
				query.clone(),
				Extension(backend.clone()),
				auth(),
				Default::default(),
			)
			.await
			.unwrap();
			assert_eq!(body(response).await, vec![physical]);
		}

		// Past the end of the renumbered list there is no page.
		let error = get_book_page(
			Path(("book-skipped".to_string(), 4)),
			query.clone(),
			Extension(backend.clone()),
			auth(),
			Default::default(),
		)
		.await
		.unwrap_err();
		assert!(matches!(error, APIError::NotFound(_)));

		// Without any skip mark, numbering is untouched.
		let response = get_book_pages(
			Path("book-plain".to_string()),
			Extension(backend.clone()),
			auth(),
			Default::default(),
		)
		.await
		.unwrap();
		let listed =
			serde_json::from_slice::<Vec<KomgaBookPage>>(&body(response).await).unwrap();
		assert_eq!(
			listed.iter().map(|page| page.number).collect::<Vec<_>>(),
			vec![1, 2, 3, 4, 5]
		);
		let response = get_book_page(
			Path(("book-plain".to_string(), 2)),
			query,
			Extension(backend),
			auth(),
			Default::default(),
		)
		.await
		.unwrap();
		assert_eq!(body(response).await, vec![2]);
	}
}
