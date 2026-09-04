use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use axum::{
	body::Body,
	http::{HeaderMap, Response},
	routing::any,
	Extension, Router,
};
use models::entity::{media as media_entity, user::AuthUser};
use sea_orm::DatabaseConnection;
use stump_auth::AuthContext;
use tokio::sync::broadcast::{self, Receiver, Sender};

use crate::{
	errors::APIResult, KomgaBookPage, KomgaBookThumbnail, KomgaEvent,
	KomgaSeriesThumbnail,
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
