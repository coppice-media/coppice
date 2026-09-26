//! Kobo store-API compatibility for stock Kobo firmware under `/kobo/{api_key}`.
//!
//! This crate owns the route tree, the opaque [`SyncToken`], and the
//! database-backed [`KoboSync`] full/incremental pagination. Authentication,
//! media serving, the `ReadingState` projection (`core/src/kobo`) and KEPUB
//! conversion (`stump_kepub`) are supplied by the host through [`KoboBackend`].
//!
//! See `crates/kobo/README.md` for pinned references and clients, decisions,
//! and verification status.

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
	body::Bytes,
	extract::{Json, Path},
	http::{HeaderMap, Method, StatusCode},
	response::{IntoResponse, Response},
	routing::{get, post, put},
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
	/// Create a shelf for the device user. The shelf id is returned so the
	/// device can address it later; creation is idempotent.
	async fn create_tag(
		&self,
		auth: AuthContext,
		request: TagCreateRequest,
	) -> Result<String, Self::Error>;
	/// Rename a shelf; last writer wins.
	async fn rename_tag(
		&self,
		auth: AuthContext,
		tag_id: String,
		name: String,
	) -> Result<(), Self::Error>;
	/// Delete a shelf.
	async fn delete_tag(
		&self,
		auth: AuthContext,
		tag_id: String,
	) -> Result<(), Self::Error>;
	/// Add book revision ids to a shelf (unknown books silently ignored).
	async fn add_tag_items(
		&self,
		auth: AuthContext,
		tag_id: String,
		revision_ids: Vec<String>,
	) -> Result<(), Self::Error>;
	/// Remove book revision ids from a shelf (unknown books silently ignored).
	async fn remove_tag_items(
		&self,
		auth: AuthContext,
		tag_id: String,
		revision_ids: Vec<String>,
	) -> Result<(), Self::Error>;
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
				.route("/v1/library/tags", post(create_tag::<B>))
				.route(
					"/v1/library/tags/{tag_id}",
					put(rename_tag::<B>).delete(delete_tag::<B>),
				)
				.route("/v1/library/tags/{tag_id}/items", post(add_tag_items::<B>))
				// The device constructs this variant from the `tag_items`
				// resource template served by `initialization`.
				.route("/v1/library/tags/{tag_id}/Items", post(add_tag_items::<B>))
				.route(
					"/v1/library/tags/{tag_id}/items/delete",
					post(remove_tag_items::<B>),
				)
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

#[derive(Debug, serde::Deserialize)]
struct TagPath {
	api_key: String,
	tag_id: String,
}

/// Wire body for `POST /v1/library/tags`:
/// `{"Name": ..., "Items": [...]}` (Calibre-Web
/// `HandleTagCreate@a97826402f1b39c45b7ea8d906efddc9f1750934`). An optional
/// `Id` is honored when present so devices that pre-generate shelf ids keep
/// them stable.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TagCreateRequest {
	pub id: Option<String>,
	pub name: String,
	#[serde(default)]
	pub items: Vec<TagItemPayload>,
}

impl TagCreateRequest {
	/// Book revision ids of the initial shelf items; see [`known_revision_ids`].
	pub fn revision_ids(&self) -> Vec<String> {
		known_revision_ids(&self.items)
	}
}

/// Wire body for the add/remove item routes:
/// `{"Items": [{"RevisionId": ..., "Type": "ProductRevisionTagItem"}]}`.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TagItemsRequest {
	#[serde(default)]
	pub items: Vec<TagItemPayload>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TagItemPayload {
	#[serde(rename = "Type")]
	pub r#type: Option<String>,
	pub revision_id: Option<String>,
}

/// Shelf items are keyed by book revision with an explicit item type;
/// anything else is silently ignored, matching Calibre-Web
/// (`add_items_to_shelf@a97826402f1b39c45b7ea8d906efddc9f1750934`).
pub(crate) fn known_revision_ids(items: &[TagItemPayload]) -> Vec<String> {
	items
		.iter()
		.filter_map(|item| {
			let known_type = item
				.r#type
				.as_deref()
				.is_some_and(|kind| kind == "ProductRevisionTagItem");
			known_type
				.then_some(())
				.and_then(|_| item.revision_id.clone())
		})
		.collect()
}

async fn create_tag<B: KoboBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(auth): Extension<AuthContext>,
	Json(request): Json<TagCreateRequest>,
) -> Result<(StatusCode, Json<String>), B::Error> {
	let shelf_id = backend.create_tag(auth, request).await?;
	Ok((StatusCode::CREATED, Json(shelf_id)))
}

/// Wire body for `PUT /v1/library/tags/{tag_id}`:
/// `{"Name": "..."}` (Calibre-Web
/// `HandleTagUpdate@a97826402f1b39c45b7ea8d906efddc9f1750934`).
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TagRenameRequest {
	pub name: String,
}

async fn rename_tag<B: KoboBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(auth): Extension<AuthContext>,
	Path(TagPath { api_key: _, tag_id }): Path<TagPath>,
	Json(request): Json<TagRenameRequest>,
) -> Result<StatusCode, B::Error> {
	backend.rename_tag(auth, tag_id, request.name).await?;
	Ok(StatusCode::OK)
}

async fn delete_tag<B: KoboBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(auth): Extension<AuthContext>,
	Path(TagPath { api_key: _, tag_id }): Path<TagPath>,
) -> Result<StatusCode, B::Error> {
	backend.delete_tag(auth, tag_id).await?;
	Ok(StatusCode::OK)
}

async fn add_tag_items<B: KoboBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(auth): Extension<AuthContext>,
	Path(TagPath { api_key: _, tag_id }): Path<TagPath>,
	Json(request): Json<TagItemsRequest>,
) -> Result<StatusCode, B::Error> {
	let revision_ids = known_revision_ids(&request.items);
	backend.add_tag_items(auth, tag_id, revision_ids).await?;
	Ok(StatusCode::CREATED)
}
async fn remove_tag_items<B: KoboBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(auth): Extension<AuthContext>,
	Path(TagPath { api_key: _, tag_id }): Path<TagPath>,
	Json(request): Json<TagItemsRequest>,
) -> Result<StatusCode, B::Error> {
	let revision_ids = known_revision_ids(&request.items);
	backend.remove_tag_items(auth, tag_id, revision_ids).await?;
	Ok(StatusCode::OK)
}

/// Middleware adapters can use this response helper for a successful empty result.
pub fn no_content() -> Response {
	axum::http::StatusCode::NO_CONTENT.into_response()
}

pub mod routes {
	pub use super::{
		kobo_router, no_content, router, session_router, KoboBackend, ProviderHost,
		TagCreateRequest, TagItemsRequest,
	};
}
