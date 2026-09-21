//! Router composition and the server-facing backend contract.

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
	body::Body,
	http::{HeaderMap, Response},
	routing::{any, MethodRouter},
	Extension, Router,
};
use chrono::{DateTime, Utc};
use models::entity::user::AuthUser;
use sea_orm::DatabaseConnection;
use stump_auth::AuthContext;

use crate::errors::APIResult;

mod account;
mod annotation;
mod book;
mod collection;
mod download;
mod filter;
mod image;
mod library;
mod metadata;
mod query;
mod reader;
mod reading_list;
mod reading_profile;
mod search;
mod series;
mod series_filter;
mod server;
mod stats;
mod tachiyomi;
mod users;
mod want_to_read;
pub use account::LoginOutcome;

/// Media whose `path` is a provider URI rather than a file: the provider host
/// owns the scheme (`crates/media/src/virtual_media.rs`,
/// `stump_media::virtual_media::PROVIDER_SCHEME`), and the protocol crates
/// stay free of `stump_media`, so the prefix is checked here.
const PROVIDER_SCHEME: &str = "provider://";

/// Whether this media item is provider-backed and therefore has no file on
/// disk: its pages are fetched live, so it has neither bytes to stream
/// ([`download`]) nor a page analysis to report ([`reader`]).
pub(crate) fn is_provider_media(media: &models::entity::media::Model) -> bool {
	media.path.starts_with(PROVIDER_SCHEME)
}

/// Image bytes with their MIME type, as served by cover and page routes.
#[derive(Debug, Clone)]
pub struct KavitaImage {
	pub content_type: String,
	pub data: Vec<u8>,
}

impl KavitaImage {
	pub fn new(content_type: impl Into<String>, data: Vec<u8>) -> Self {
		Self {
			content_type: content_type.into(),
			data,
		}
	}
}

/// Facts about this installation for `GET /api/Server/server-info-slim`.
#[derive(Debug, Clone)]
pub struct ServerFacts {
	pub is_docker: bool,
	pub first_install_date: Option<DateTime<Utc>>,
}

/// Bytes plus MIME type for an EPUB page or an in-book resource, as served
/// by `GET /api/Book/{chapterId}/book-page` and `book-resources`.
#[derive(Debug, Clone)]
pub struct KavitaBookResource {
	pub content_type: String,
	pub data: Vec<u8>,
}

/// One spine item of an EPUB and the number of Stump pages it covers.
///
/// Stump's EPUB page count is the Readium synthetic count — a spine item
/// contributes `ceil(compressed_size / 1KiB)` pages, at least one — so the
/// per-item counts partition the very page space the rest of the Kavita
/// profile reports for that chapter. Non-linear spine items contribute `0`
/// pages and are unreachable, exactly as they are for Readium.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KavitaSpineItem {
	pub pages: i32,
}

/// One EPUB navigation entry: `spine_index` is the spine item it points at
/// and `fragment` the anchor inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KavitaNavPoint {
	pub title: String,
	pub fragment: String,
	pub spine_index: usize,
	pub children: Vec<KavitaNavPoint>,
}

/// What `BookController` needs about an EPUB: its page-bearing spine and its
/// navigation tree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KavitaBookStructure {
	pub spine: Vec<KavitaSpineItem>,
	pub navigation: Vec<KavitaNavPoint>,
}

impl KavitaBookStructure {
	/// The first Stump page covered by each spine item, and the total.
	pub fn page_offsets(&self) -> (Vec<i32>, i32) {
		let mut offsets = Vec::with_capacity(self.spine.len());
		let mut total = 0;
		for item in &self.spine {
			offsets.push(total);
			total += item.pages.max(0);
		}
		(offsets, total)
	}

	/// The spine item holding Stump page `page`, or `None` past the end.
	pub fn spine_index_for_page(&self, page: i32) -> Option<usize> {
		let (offsets, total) = self.page_offsets();
		if page < 0 || page >= total {
			return None;
		}
		offsets
			.iter()
			.enumerate()
			.filter(|(index, offset)| **offset <= page && self.spine[*index].pages > 0)
			.map(|(index, _)| index)
			.next_back()
	}

	/// The first Stump page of spine item `index`.
	pub fn page_for_spine_index(&self, index: usize) -> i32 {
		self.page_offsets().0.get(index).copied().unwrap_or(0)
	}
}

/// The persistence and platform operations the Kavita HTTP surface needs from
/// the server. Database access happens through [`KavitaBackend::conn`] with
/// the shared SeaORM visibility helpers; everything that touches files, images
/// or Stump's credential stores stays behind the async methods.
#[async_trait]
pub trait KavitaBackend: Send + Sync {
	fn conn(&self) -> &DatabaseConnection;

	/// The secret Kavita tokens are signed with (Stump's JWT access secret).
	async fn token_secret(&self) -> APIResult<Vec<u8>>;

	/// Resolve a Stump API key presented as a Kavita `apiKey`.
	async fn authenticate_api_key(&self, api_key: &str) -> APIResult<AuthUser>;

	/// Resolve username/password credentials (`POST /api/Account/login`).
	async fn authenticate_password(
		&self,
		username: &str,
		password: &str,
	) -> APIResult<AuthUser>;
	/// Record a successful progress write against the API-key-bound device.
	/// Registry failures are best effort and never fail the request.
	async fn record_sync(&self, _auth: &AuthContext, _summary: serde_json::Value) {}

	fn server_facts(&self) -> ServerFacts;

	/// Render page `page` (1-based, Stump numbering) of a media item.
	async fn media_page(
		&self,
		user: &AuthUser,
		media_id: &str,
		page: i32,
	) -> APIResult<KavitaImage>;

	async fn media_thumbnail(
		&self,
		user: &AuthUser,
		media_id: &str,
	) -> APIResult<KavitaImage>;

	async fn series_thumbnail(
		&self,
		user: &AuthUser,
		series_id: &str,
	) -> APIResult<KavitaImage>;

	/// Stream the media file itself (`GET /api/Reader/pdf`), honouring range
	/// requests from `headers`.
	async fn serve_media_file(
		&self,
		auth: AuthContext,
		headers: HeaderMap,
		media_id: &str,
	) -> APIResult<Response<Body>>;

	/// The whole download payload of one media item, in memory: the file on
	/// disk for a stored row, and a CBZ packed from its pages for a
	/// provider-backed (`provider://`) row, which has no file to stream.
	///
	/// `GET /api/Download/chapter` streams a stored file with
	/// [`KavitaBackend::serve_media_file`] instead, so this is only reached
	/// where the payload has to be buffered: a provider-backed chapter, and
	/// every member of the zip a multi-file `GET /api/Download/series`
	/// returns.
	async fn media_bytes(&self, user: &AuthUser, media_id: &str) -> APIResult<Vec<u8>>;

	/// The spine page budget and navigation tree of an EPUB, for
	/// `GET /api/Book/{chapterId}/book-info` and `chapters`.
	async fn book_structure(
		&self,
		user: &AuthUser,
		media_id: &str,
	) -> APIResult<KavitaBookStructure>;

	/// One EPUB spine document. Relative resource links come back as
	/// `epub://<package-relative-path>` URIs, which the route rewrites onto
	/// `book-resources`.
	async fn book_page(
		&self,
		user: &AuthUser,
		media_id: &str,
		spine_index: usize,
	) -> APIResult<KavitaBookResource>;

	/// One resource inside an EPUB, by package-relative path.
	async fn book_resource(
		&self,
		user: &AuthUser,
		media_id: &str,
		file: &str,
	) -> APIResult<KavitaBookResource>;

	/// Create a reading list owned by `user` (`POST /api/ReadingList/create`).
	async fn create_read_list(
		&self,
		user: &AuthUser,
		name: String,
	) -> APIResult<models::entity::reading_list::Model>;

	/// Replace a reading list's ordered membership; the Kavita
	/// `update-by-*` routes append to what they read back first.
	async fn set_read_list_items(
		&self,
		user: &AuthUser,
		id: &str,
		book_ids: Vec<String>,
	) -> APIResult<()>;

	/// Delete a reading list owned by `user`
	/// (`DELETE /api/ReadingList?readingListId=`).
	async fn delete_read_list(&self, user: &AuthUser, id: &str) -> APIResult<()>;

	/// Create a collection owned by `user` with the given members
	/// (`POST /api/Collection/update-for-series` with `collectionTagId: 0`).
	async fn create_collection(
		&self,
		user: &AuthUser,
		name: String,
		series_ids: Vec<String>,
	) -> APIResult<models::entity::collection::Model>;

	/// Replace a collection's membership; the Kavita `update-for-series` and
	/// `update-series` routes add to and remove from what they read back
	/// first.
	async fn set_collection_series(
		&self,
		user: &AuthUser,
		id: &str,
		series_ids: Vec<String>,
	) -> APIResult<()>;

	/// Enqueue a library scan (`POST /api/Library/scan?libraryId&force`); the
	/// same job Komga's `/api/v1/libraries/{id}/scan` enqueues. `force`
	/// rebuilds every book instead of only the changed ones.
	async fn enqueue_library_scan(
		&self,
		library_id: String,
		path: String,
		force: bool,
	) -> APIResult<()>;

	/// Enqueue a scan of one series' folder (`POST /api/Series/scan` and
	/// `POST /api/Series/refresh-metadata`).
	async fn enqueue_series_scan(
		&self,
		series_id: String,
		path: String,
		force: bool,
	) -> APIResult<()>;

	/// Enqueue media analysis for one series (`POST /api/Series/analyze`);
	/// the same job Komga's `/api/v1/series/{id}/analyze` enqueues.
	async fn enqueue_series_analysis(&self, series_id: String) -> APIResult<()>;
}

/// Kavita's ASP.NET routing is case-insensitive and the inventoried clients
/// mix `/api/Series/...` with `/api/series/...`; every route is registered
/// under its canonical casing and fully lower-cased.
pub(crate) fn route_ci<S>(
	router: Router<S>,
	path: &str,
	handler: MethodRouter<S>,
) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	// Only literal segments get a lowercase twin; a `{param}` segment is a
	// wildcard already, and re-registering it under another name is a
	// matchit conflict.
	let lower = path
		.split('/')
		.map(|segment| {
			if segment.starts_with('{') {
				segment.to_owned()
			} else {
				segment.to_ascii_lowercase()
			}
		})
		.collect::<Vec<_>>()
		.join("/");
	if lower == path {
		router.route(path, handler)
	} else {
		router.route(path, handler.clone()).route(&lower, handler)
	}
}

/// The routes that must be reachable without an authenticated context:
/// `Plugin/authenticate`, `Plugin/version`, `Account/login` and `Health`.
pub fn public_router<S>(backend: Arc<dyn KavitaBackend>) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::<S>::new()
		.merge(account::public_routes::<S>())
		.merge(server::public_routes::<S>())
		.layer(Extension(backend))
}

/// Build the authenticated Kavita router. The server applies its Kavita
/// authentication middleware (API key header/query, Kavita JWT, Stump bearer)
/// on top of this router; handlers read the resulting `AuthContext`.
pub fn router<S>(backend: Arc<dyn KavitaBackend>) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new()
		.merge(account::routes::<S>())
		.merge(server::routes::<S>())
		.merge(library::routes::<S>())
		.merge(series::routes::<S>())
		.merge(image::routes::<S>())
		.merge(reader::routes::<S>())
		.merge(book::routes::<S>())
		.merge(download::routes::<S>())
		.merge(reading_list::routes::<S>())
		.merge(reading_profile::routes::<S>())
		.merge(want_to_read::routes::<S>())
		.merge(tachiyomi::routes::<S>())
		.merge(metadata::routes::<S>())
		.merge(filter::routes::<S>())
		.merge(users::routes::<S>())
		.merge(search::routes::<S>())
		.merge(collection::routes::<S>())
		.merge(stats::routes::<S>())
		.merge(annotation::routes::<S>());
	// Unimplemented Kavita controllers answer 404 here rather than falling
	// through to the web UI; each miss is logged so the next wave sees it.
	// Controllers that already own a `/{param}` route at this depth (Series,
	// Library, Volume, Book, reading-profile) are excluded: `matchit` rejects a
	// path parameter and a catch-all in the same position.
	let router = [
		"/api/Reader",
		"/api/Image",
		"/api/Chapter",
		"/api/Metadata",
		"/api/Filter",
		"/api/Tachiyomi",
		"/api/Plugin",
		"/api/Account",
		"/api/Server",
		"/api/Collection",
		"/api/ReadingList",
		"/api/Stream",
		"/api/Search",
		"/api/Download",
		"/api/Person",
		"/api/Rating",
		"/api/Stats",
		"/api/Annotation",
		"/api/Settings",
		"/api/Font",
		"/api/Users",
		"/api/Panel",
		"/api/want-to-read",
		"/api/Device",
		"/api/Scrobbling",
		"/api/Recommended",
		"/api/Review",
		"/api/Theme",
		"/api/Upload",
		"/api/Cbl",
		"/api/Locale",
		"/api/Manage",
		"/api/Media",
	]
	.into_iter()
	.fold(router, |router, prefix| {
		route_ci(router, &format!("{prefix}/{{*path}}"), any(log_unmatched))
	});
	router.layer(Extension(backend))
}

/// Return whether a path belongs to the Kavita compatibility surface.
///
/// Every Kavita controller lives directly below `/api/`; Stump's own API and
/// the Komga profile use `/api/v1`, `/api/v2`, `/api/graphql` and `/api/logout`,
/// which are excluded here.
pub fn is_kavita_path(path: &str) -> bool {
	let Some(rest) = path.strip_prefix("/api/") else {
		return false;
	};
	let controller = rest.split('/').next().unwrap_or_default();
	let controller = controller.split('?').next().unwrap_or_default();
	!controller.is_empty()
		&& !matches!(
			controller.to_ascii_lowercase().as_str(),
			"v1" | "v2" | "graphql" | "logout" | "opds"
		)
}

async fn log_unmatched(
	method: axum::http::Method,
	uri: axum::http::Uri,
) -> axum::http::StatusCode {
	tracing::warn!(%method, %uri, "Unmatched Kavita-profile route");
	axum::http::StatusCode::NOT_FOUND
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn kavita_paths_exclude_stump_and_komga_namespaces() {
		assert!(is_kavita_path("/api/Series/1"));
		assert!(is_kavita_path("/api/series/all-v2"));
		assert!(is_kavita_path("/api/Reader/image?chapterId=1"));
		assert!(is_kavita_path("/api/Plugin/authenticate"));
		assert!(!is_kavita_path("/api/v1/series"));
		assert!(!is_kavita_path("/api/v2/users/me"));
		assert!(!is_kavita_path("/api/graphql"));
		assert!(!is_kavita_path("/api/logout"));
		assert!(!is_kavita_path("/api/"));
		assert!(!is_kavita_path("/opds/v1.2/catalog"));
		assert!(!is_kavita_path("/komga/api/v1/series"));
	}
}
