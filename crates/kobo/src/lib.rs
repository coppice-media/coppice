//! Kobo store-API compatibility for stock Kobo firmware under `/kobo/{api_key}`.
//!
//! This crate owns the route tree, the opaque [`SyncToken`], and the
//! database-backed [`KoboSync`] full/incremental pagination. Authentication,
//! media serving, the `ReadingState` projection (`core/src/kobo`) and KEPUB
//! conversion (`stump_kepub`) are supplied by the host through [`KoboBackend`].
//!
//! See `crates/kobo/README.md` for the Calibre-Web/Komga/kepubify pins, the
//! Liseur `31f8182d` client, decisions, and verification (harness only; no
//! physical Kobo has been tested).

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
	body::Bytes,
	extract::Path,
	http::{HeaderMap, Method},
	response::{IntoResponse, Response},
	routing::{get, post},
	Extension, Router,
};
use stump_auth::AuthContext;
mod sync;
mod sync_token;

pub use sync::{KoboSync, SyncPage};
pub use sync_token::{SyncToken, SyncTokenDeserializeError, SyncTokenSerializeError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderHost {
	url: String,
}

impl ProviderHost {
	pub fn new(url: impl Into<String>) -> Self {
		Self { url: url.into() }
	}

	pub fn url(&self) -> &str {
		&self.url
	}
}

/// Backend operations needed by the Kobo compatibility surface.
///
/// Authentication and permission middleware remain in the server. The adapter owns persistence,
/// media serving, and sync-token handling; this crate owns only protocol extraction and routing.
#[async_trait]
pub trait KoboBackend: Send + Sync + 'static {
	type Error: axum::response::IntoResponse + Send + 'static;

	async fn auth_device(&self, body: Bytes) -> Result<Response, Self::Error>;
	async fn stubbed_route(
		&self,
		method: Method,
		path: String,
		headers: HeaderMap,
		body: Bytes,
	) -> Result<Response, Self::Error>;
	async fn initialization(
		&self,
		host: ProviderHost,
		api_key: String,
	) -> Result<Response, Self::Error>;
	async fn library_sync(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		api_key: String,
		headers: HeaderMap,
	) -> Result<Response, Self::Error>;
	async fn book_metadata(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		api_key: String,
		book_id: String,
	) -> Result<Response, Self::Error>;
	async fn book_thumbnail(
		&self,
		auth: AuthContext,
		book_id: String,
		width: u32,
		height: u32,
	) -> Result<Response, Self::Error>;
	async fn book_download(
		&self,
		auth: AuthContext,
		book_id: String,
		headers: HeaderMap,
	) -> Result<Response, Self::Error>;

	/// Deliver the file requested by Kobo. This seam defaults to the existing
	/// EPUB delivery implementation and may be wrapped by a format converter.
	async fn book_file(
		&self,
		auth: AuthContext,
		book_id: String,
		headers: HeaderMap,
	) -> Result<Response, Self::Error> {
		self.book_download(auth, book_id, headers).await
	}

	async fn book_state(
		&self,
		auth: AuthContext,
		book_id: String,
	) -> Result<Response, Self::Error>;
	async fn update_book_state(
		&self,
		auth: AuthContext,
		book_id: String,
		headers: HeaderMap,
		body: Bytes,
	) -> Result<Response, Self::Error>;
	async fn delete_sync_sessions(
		&self,
		auth: AuthContext,
	) -> Result<Response, Self::Error>;
}

#[derive(Debug, serde::Deserialize)]
struct APIKeyPath {
	api_key: String,
}

#[derive(Debug, serde::Deserialize)]
struct APIKeyAndBookIdPath {
	api_key: String,
	book_id: String,
}

#[derive(Debug, serde::Deserialize)]
struct ThumbnailPath {
	// Consumed by the API-key middleware; kept so the path shape matches the route.
	#[allow(dead_code)]
	api_key: String,
	book_id: String,
	width: u32,
	height: u32,
	#[allow(dead_code)]
	is_greyscale: String,
}

fn kobo_routes<S, B>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	B: KoboBackend,
{
	Router::<S>::new().nest(
		"/kobo",
		Router::new().nest(
			"/{api_key}",
			Router::new()
				.route("/v1/initialization", get(initialization::<B>))
				.route("/v1/library/sync", get(library_sync::<B>))
				.route(
					"/v1/library/{book_id}/state",
					get(book_state::<B>).put(update_book_state::<B>),
				)
				.route("/v1/library/{book_id}/metadata", get(book_metadata::<B>))
				.route(
					"/v1/books/{book_id}/thumbnail/{width}/{height}/{is_greyscale}/image.jpg",
					get(book_thumbnail::<B>),
				)
				.route(
					"/v1/books/{book_id}/thumbnail/{width}/{height}/{quality}/{is_greyscale}/image.jpg",
					get(book_thumbnail_with_quality::<B>),
				)
				.route("/v1/auth/device", post(auth_device::<B>))
				.route("/v1/books/{book_id}/file/epub", get(book_file::<B>))
				.route("/v1/{*path}", axum::routing::any(stubbed_route::<B>)),
		),
	)
}

fn session_routes<S, B>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	B: KoboBackend,
{
	Router::<S>::new().route(
		"/api/v2/kobo/sync-sessions",
		axum::routing::delete(delete_sync_sessions::<B>),
	)
}

/// Build Kobo sync routes with an injected backend.
pub fn kobo_router<S, B>(backend: Arc<B>) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	B: KoboBackend,
{
	kobo_routes::<S, B>().layer(Extension(backend))
}

/// Build the authenticated session-management route with an injected backend.
pub fn session_router<S, B>(backend: Arc<B>) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	B: KoboBackend,
{
	session_routes::<S, B>().layer(Extension(backend))
}

pub fn router<S, B>(backend: B) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	B: KoboBackend,
{
	let backend = Arc::new(backend);
	kobo_router::<S, B>(backend.clone()).merge(session_router::<S, B>(backend))
}

async fn auth_device<B: KoboBackend>(
	Extension(backend): Extension<Arc<B>>,
	body: Bytes,
) -> Result<Response, B::Error> {
	backend.auth_device(body).await
}

async fn stubbed_route<B: KoboBackend>(
	Extension(backend): Extension<Arc<B>>,
	method: Method,
	Path((_, path)): Path<(String, String)>,
	headers: HeaderMap,
	body: Bytes,
) -> Result<Response, B::Error> {
	backend.stubbed_route(method, path, headers, body).await
}

async fn initialization<B: KoboBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
	Path(APIKeyPath { api_key }): Path<APIKeyPath>,
) -> Result<Response, B::Error> {
	backend.initialization(host, api_key).await
}

async fn library_sync<B: KoboBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
	axum::extract::Extension(auth): Extension<AuthContext>,
	Path(APIKeyPath { api_key }): Path<APIKeyPath>,
	headers: HeaderMap,
) -> Result<Response, B::Error> {
	backend.library_sync(auth, host, api_key, headers).await
}

async fn book_state<B: KoboBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(auth): Extension<AuthContext>,
	Path(APIKeyAndBookIdPath { book_id, .. }): Path<APIKeyAndBookIdPath>,
) -> Result<Response, B::Error> {
	backend.book_state(auth, book_id).await
}

async fn update_book_state<B: KoboBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(auth): Extension<AuthContext>,
	Path(APIKeyAndBookIdPath { book_id, .. }): Path<APIKeyAndBookIdPath>,
	headers: HeaderMap,
	body: Bytes,
) -> Result<Response, B::Error> {
	backend
		.update_book_state(auth, book_id, headers, body)
		.await
}

async fn book_metadata<B: KoboBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
	Extension(auth): Extension<AuthContext>,
	Path(APIKeyAndBookIdPath { api_key, book_id }): Path<APIKeyAndBookIdPath>,
) -> Result<Response, B::Error> {
	backend.book_metadata(auth, host, api_key, book_id).await
}

async fn book_thumbnail<B: KoboBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(auth): Extension<AuthContext>,
	Path(ThumbnailPath {
		book_id,
		width,
		height,
		..
	}): Path<ThumbnailPath>,
) -> Result<Response, B::Error> {
	backend.book_thumbnail(auth, book_id, width, height).await
}

async fn book_thumbnail_with_quality<B: KoboBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(auth): Extension<AuthContext>,
	Path((_, book_id, width, height, _quality, _is_greyscale)): Path<(
		String,
		String,
		u32,
		u32,
		String,
		String,
	)>,
) -> Result<Response, B::Error> {
	backend.book_thumbnail(auth, book_id, width, height).await
}

async fn book_file<B: KoboBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(auth): Extension<AuthContext>,
	Path(APIKeyAndBookIdPath { book_id, .. }): Path<APIKeyAndBookIdPath>,
	headers: HeaderMap,
) -> Result<Response, B::Error> {
	backend.book_file(auth, book_id, headers).await
}

async fn delete_sync_sessions<B: KoboBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(auth): Extension<AuthContext>,
) -> Result<Response, B::Error> {
	backend.delete_sync_sessions(auth).await
}

/// Middleware adapters can use this response helper for a successful empty result.
pub fn no_content() -> Response {
	axum::http::StatusCode::NO_CONTENT.into_response()
}

pub mod routes {
	pub use super::{
		kobo_router, no_content, router, session_router, KoboBackend, ProviderHost,
	};
}
