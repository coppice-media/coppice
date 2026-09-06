use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use axum::{
	body::Body,
	http::{HeaderMap, Response},
	routing::any,
	Extension, Router,
};
use models::entity::{
	collection as collection_entity, library as library_entity, media as media_entity,
	reading_list as reading_list_entity, user::AuthUser,
};
use sea_orm::DatabaseConnection;
use stump_auth::AuthContext;
use tokio::sync::broadcast::{self, Receiver, Sender};

use crate::{
	errors::APIResult, KomgaBookPage, KomgaBookThumbnail, KomgaEvent,
	KomgaLibraryCreateRequest, KomgaLibraryUpdateRequest, KomgaSeriesThumbnail,
};
mod book;
mod catalog;
mod grimmory;
mod library;
mod lists;
mod mapper;
mod media;
mod progress;
pub use progress::{
	KomgaReadProgressDto, KomgaReadProgressUpdateDto, KomgaSeriesReadProgressDto,
	KomgaSeriesReadProgressUpdateDto,
};
mod readium;
pub mod response;
mod series;
mod sse;
#[derive(Debug, Clone)]
pub struct KomgaImage {
	pub content_type: String,
	pub data: Vec<u8>,
	pub width: Option<i32>,
	pub height: Option<i32>,
}

impl KomgaImage {
	pub fn new(content_type: impl Into<String>, data: Vec<u8>) -> Self {
		Self {
			content_type: content_type.into(),
			data,
			width: None,
			height: None,
		}
	}
}

/// Core events are deliberately reduced to the one event with an honest Komga
/// equivalent.  The server adapter maps its internal event enum to this
/// protocol-neutral representation before publishing it to the SSE route.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "__typename")]
pub enum KomgaCoreEvent {
	CreatedMedia {
		id: String,
		#[serde(rename = "seriesId")]
		series_id: String,
		#[serde(rename = "libraryId")]
		library_id: String,
	},
	MediaDeleted {
		id: String,
		#[serde(rename = "seriesId")]
		series_id: String,
		#[serde(rename = "libraryId")]
		library_id: String,
	},
	SeriesDeleted {
		id: String,
		#[serde(rename = "libraryId")]
		library_id: String,
	},
	JobQueueStatus {
		count: i32,
		#[serde(rename = "countByType")]
		count_by_type: BTreeMap<String, i32>,
	},
	/// A reading head moved (or was cleared) through a non-Komga protocol.
	ReadProgressChanged {
		#[serde(rename = "bookId")]
		book_id: String,
		#[serde(rename = "seriesId")]
		series_id: String,
		#[serde(rename = "userId")]
		user_id: String,
		#[serde(default)]
		deleted: bool,
	},
	/// A library was created (any protocol; see `stump_core::library`).
	LibraryCreated { id: String },
	/// A library's name, root, or configuration changed.
	LibraryUpdated { id: String },
	/// A library was deleted.
	LibraryDeleted { id: String },
	/// A collection was created (any protocol, incl. Kobo shelf write-back).
	CollectionAdded {
		#[serde(rename = "collectionId")]
		collection_id: String,
		#[serde(rename = "seriesIds", default)]
		series_ids: Vec<String>,
	},
	/// A collection's name, ordering, or membership changed.
	CollectionChanged {
		#[serde(rename = "collectionId")]
		collection_id: String,
		#[serde(rename = "seriesIds", default)]
		series_ids: Vec<String>,
	},
	/// A collection was deleted.
	CollectionDeleted {
		#[serde(rename = "collectionId")]
		collection_id: String,
		#[serde(rename = "seriesIds", default)]
		series_ids: Vec<String>,
	},
	/// A reading list was created (any protocol, incl. Kobo shelf write-back).
	ReadListAdded {
		#[serde(rename = "readListId")]
		read_list_id: String,
		#[serde(rename = "bookIds", default)]
		book_ids: Vec<String>,
	},
	/// A reading list's name, summary, ordering, or membership changed.
	ReadListChanged {
		#[serde(rename = "readListId")]
		read_list_id: String,
		#[serde(rename = "bookIds", default)]
		book_ids: Vec<String>,
	},
	/// A reading list was deleted.
	ReadListDeleted {
		#[serde(rename = "readListId")]
		read_list_id: String,
		#[serde(rename = "bookIds", default)]
		book_ids: Vec<String>,
	},
	#[serde(other)]
	Other,
}

/// The persistence and platform operations needed by the Komga HTTP surface.
///
/// The compatibility crate owns request routing and DTO mapping while the
/// server supplies this adapter over its Ctx.  Keeping the database accessor
/// narrow lets the existing SeaORM visibility/query expressions remain in one
/// place; platform-specific file, image, EPUB and job operations stay behind
/// the async methods below.
#[async_trait]
pub trait KomgaBackend: Send + Sync {
	fn conn(&self) -> &DatabaseConnection;
	fn conn_arc(&self) -> Arc<DatabaseConnection>;

	/// Return a fresh subscription to protocol-neutral core events.
	fn core_events(&self) -> Receiver<KomgaCoreEvent>;

	async fn enqueue_library_scan(
		&self,
		library_id: String,
		path: String,
		deep: bool,
	) -> APIResult<()>;
	async fn enqueue_library_analysis(&self, library_id: String) -> APIResult<()>;
	async fn enqueue_book_analysis(&self, book_id: String) -> APIResult<()>;
	async fn book_pages(
		&self,
		book: &media_entity::Model,
	) -> APIResult<Vec<KomgaBookPage>>;
	/// Physical page numbers (1-based) that remain visible for `book` under
	/// duplicate-page skipping, or `None` when nothing is skipped. Page
	/// routes renumber their output onto this list.
	async fn visible_pages(
		&self,
		book: &media_entity::Model,
	) -> APIResult<Option<Vec<i32>>>;
	async fn book_page(
		&self,
		user: &AuthUser,
		book_id: String,
		page: u32,
	) -> APIResult<KomgaImage>;
	async fn book_page_thumbnail(
		&self,
		user: &AuthUser,
		book_id: String,
		page: u32,
	) -> APIResult<KomgaImage>;
	async fn book_thumbnail(
		&self,
		user: &AuthUser,
		book_id: String,
	) -> APIResult<KomgaImage>;
	async fn book_thumbnails(
		&self,
		user: &AuthUser,
		book_id: String,
	) -> APIResult<Vec<KomgaBookThumbnail>>;
	async fn book_thumbnail_by_id(
		&self,
		user: &AuthUser,
		book_id: String,
		thumbnail_id: String,
	) -> APIResult<KomgaImage>;
	async fn upload_book_thumbnail(
		&self,
		user: &AuthUser,
		book_id: String,
		bytes: Vec<u8>,
		selected: bool,
	) -> APIResult<KomgaBookThumbnail>;
	async fn delete_book_thumbnail(
		&self,
		user: &AuthUser,
		book_id: String,
		thumbnail_id: String,
	) -> APIResult<()>;
	async fn series_thumbnail(
		&self,
		user: &AuthUser,
		series_id: &str,
	) -> APIResult<KomgaImage>;
	async fn series_thumbnails(
		&self,
		user: &AuthUser,
		series_id: String,
	) -> APIResult<Vec<KomgaSeriesThumbnail>>;
	async fn series_thumbnail_by_id(
		&self,
		user: &AuthUser,
		series_id: String,
		thumbnail_id: String,
	) -> APIResult<KomgaImage>;
	async fn upload_series_thumbnail(
		&self,
		user: &AuthUser,
		series_id: String,
		bytes: Vec<u8>,
		selected: bool,
	) -> APIResult<KomgaSeriesThumbnail>;
	async fn delete_series_thumbnail(
		&self,
		user: &AuthUser,
		series_id: String,
		thumbnail_id: String,
	) -> APIResult<()>;
	async fn serve_book_file(
		&self,
		auth: AuthContext,
		headers: HeaderMap,
		book_id: String,
	) -> APIResult<Response<Body>>;

	async fn readium_manifest(
		&self,
		path: String,
		base_url: String,
	) -> APIResult<serde_json::Value>;
	async fn readium_positions(
		&self,
		path: String,
		base_url: String,
	) -> APIResult<serde_json::Value>;
	async fn readium_resource(
		&self,
		path: String,
		resource_path: PathBuf,
	) -> APIResult<KomgaImage>;

	/// Record a completed read-progress sync on the device the request's
	/// credential belongs to. `summary` describes what was synced; the call
	/// is best-effort and never fails the request.
	async fn record_sync(&self, auth: &AuthContext, summary: serde_json::Value);

	/// The provider source instance backing a virtual (Mode B) library, when
	/// the backend can browse it live. Default: no provider support.
	async fn virtual_library_source(&self, _library_id: &str) -> Option<String> {
		None
	}

	/// One live page of series for a virtual library. Returns `None` when
	/// this backend does not implement provider browsing; `Some(Err(...))`
	/// surfaces source failures to the client. Default: `None`.
	async fn virtual_series_list(
		&self,
		_library_id: String,
		_search: &crate::KomgaSeriesSearch,
		_sorts: &[String],
		_page: i32,
		_size: i32,
		_unpaged: bool,
	) -> Option<APIResult<crate::Page<crate::KomgaSeries>>> {
		None
	}

	/// Live details for a series id that is not materialised. Returns
	/// `None` when the id belongs to a stored (or unknown) series and the
	/// ordinary database path should serve it. Default: `None`.
	async fn virtual_series_by_id(
		&self,
		_user: &AuthUser,
		_series_id: &str,
	) -> Option<APIResult<crate::KomgaSeries>> {
		None
	}

	/// Materialise a virtual series before its books are listed. Returns
	/// `Ok(false)` when the id is not a provider series. Default: no-op.
	async fn virtual_materialise_series(
		&self,
		_user: &AuthUser,
		_series_id: &str,
	) -> APIResult<bool> {
		Ok(false)
	}

	/// Create a library from a Komga `LibraryCreationDto`. Returns the
	/// created library row for DTO rendering by the route. Validation failures
	/// (missing root, nesting conflicts, duplicate name) surface as `400`.
	async fn create_library(
		&self,
		request: KomgaLibraryCreateRequest,
	) -> APIResult<library_entity::Model>;

	/// Apply a Komga `LibraryUpdateDto` patch to an existing library owned
	/// by `user`. The adapter merges provided fields onto the stored row.
	async fn update_library(
		&self,
		user: &AuthUser,
		id: &str,
		request: KomgaLibraryUpdateRequest,
	) -> APIResult<()>;

	/// Delete a library owned by `user` together with its series/media.
	async fn delete_library(&self, user: &AuthUser, id: &str) -> APIResult<()>;

	/// Enqueue the analysis job for every book of a series (Komf's
	/// `POST /api/v1/series/{id}/analyze`).
	async fn enqueue_series_analysis(&self, series_id: String) -> APIResult<()>;

	/// Configured filesystem roots constraining library creation and the
	/// filesystem browser (`STUMP_LIBRARY_ROOTS`). Empty means unconstrained.
	fn library_roots(&self) -> Vec<String>;

	/// Create a collection. The adapter validates member visibility against
	/// `user` and emits the change event; the route renders the DTO.
	async fn create_collection(
		&self,
		user: &AuthUser,
		name: String,
		ordered: bool,
		series_ids: Vec<String>,
	) -> APIResult<collection_entity::Model>;

	/// Rename/reorder a collection. Passing `series_ids` replaces the full
	/// ordered membership (Komga's reorder idiom). Only the creating user
	/// may update a collection.
	async fn update_collection(
		&self,
		user: &AuthUser,
		id: &str,
		name: Option<String>,
		ordered: Option<bool>,
		series_ids: Option<Vec<String>>,
	) -> APIResult<()>;

	/// Delete a collection owned by `user`.
	async fn delete_collection(&self, user: &AuthUser, id: &str) -> APIResult<()>;

	/// Create a reading list. `book_ids` must be visible to `user`.
	async fn create_read_list(
		&self,
		user: &AuthUser,
		name: String,
		summary: Option<String>,
		ordered: bool,
		book_ids: Vec<String>,
	) -> APIResult<reading_list_entity::Model>;

	/// Rename/resummarize/reorder a reading list. Passing `book_ids`
	/// replaces the full ordered membership.
	async fn update_read_list(
		&self,
		user: &AuthUser,
		id: &str,
		name: Option<String>,
		summary: Option<Option<String>>,
		ordered: Option<bool>,
		book_ids: Option<Vec<String>>,
	) -> APIResult<()>;

	/// Delete a reading list owned by `user`.
	async fn delete_read_list(&self, user: &AuthUser, id: &str) -> APIResult<()>;
}

#[derive(Clone)]
pub struct KomgaEvents {
	sender: Arc<Sender<KomgaEvent>>,
}

impl KomgaEvents {
	pub fn new() -> Self {
		Self {
			sender: Arc::new(broadcast::channel(256).0),
		}
	}

	pub fn subscribe(&self) -> Receiver<KomgaEvent> {
		self.sender.subscribe()
	}

	pub fn send(&self, event: KomgaEvent) {
		let _ = self.sender.send(event);
	}
}

impl Default for KomgaEvents {
	fn default() -> Self {
		Self::new()
	}
}

/// Build the app-state-agnostic Komga router.
///
/// Backend operations are injected as an extension.  `S` remains generic so
/// the server can apply its own stateful authentication middleware after this
/// router is composed into the application router. The backend is shared by
/// reference so the server can mount the same adapter under several prefixes.
pub fn router<S>(backend: Arc<dyn KomgaBackend>) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let events = KomgaEvents::new();
	Router::<S>::new()
		.merge(catalog::routes::<S>())
		.merge(lists::routes::<S>())
		.merge(library::routes::<S>())
		.merge(book::routes::<S>())
		.merge(series::routes::<S>())
		.merge(media::routes::<S>())
		.merge(progress::routes::<S>())
		.merge(readium::routes::<S>())
		.merge(sse::routes::<S>())
		// The Komga profile owns the /api/v1 namespace; log anything this
		// adapter does not implement without leaking a fallback to the app.
		.route("/api/v1/{*path}", any(log_unmatched))
		.layer(Extension(backend))
		.layer(Extension(events))
}

/// Build the root-only legacy book listing route.
///
/// Grimmory uses the same path below its `/komga` prefix with a different
/// response profile, so this route is mounted by the server alongside the
/// ordinary provider router rather than being part of it.
pub fn legacy_books_router<S>(backend: Arc<dyn KomgaBackend>) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	catalog::legacy_books::<S>().layer(Extension(backend))
}

/// Grimmory's Komga shim differs from Komga at two routes (`GET /api/v1/books`
/// listing and its own identity shape at `GET /api/v2/users/me`). The server
/// mounts these under the `/komga` prefix only, so Stump's Komga identity at
/// the root is untouched. The same backend must be supplied so the shim's
/// handlers see the `Extension<Arc<dyn KomgaBackend>>` the base router injects.
pub fn grimmory_routes<S>(backend: Arc<dyn KomgaBackend>) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	grimmory::routes::<S>().layer(Extension(backend))
}

/// Return whether a path belongs to the Komga compatibility surface.
///
/// The server auth middleware uses this single predicate for basic-auth and
/// remember-me handling, keeping path ownership in the provider crate.
pub fn is_komga_path(path: &str) -> bool {
	path.starts_with("/api/v1/")
		|| path == "/api/v1"
		|| path.starts_with("/api/v2/")
		|| path == "/api/v2"
		|| path.starts_with("/sse/v1/")
		|| path == "/sse/v1"
}

async fn log_unmatched(
	method: axum::http::Method,
	uri: axum::http::Uri,
) -> axum::http::StatusCode {
	tracing::warn!(%method, %uri, "Unmatched Komga-profile route");
	axum::http::StatusCode::NOT_FOUND
}
