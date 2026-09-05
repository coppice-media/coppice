//! OPDS 1.2 (+ Page Streaming Extension) and OPDS 2.0 route contract.
//!
//! This crate owns [`ProviderHost`], v2 [`BrowseParams`], the [`OpdsBackend`]
//! trait, and the unprefixed [`v1_router`]/[`v2_router`] trees. Wire types stay
//! in `stump_core::opds`; authentication, mounting (`/opds/v1.2`,
//! `/opds/{api_key}/v1.2`, `/opds/v2.0`), and persistence stay in the host.
//!
//! See `crates/opds/README.md` for spec URLs, the Liseur `31f8182d` client
//! (OPDS 1.2 device-verified; OPDS 2.0 unsupported by the app), decisions,
//! and verification.

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
	extract::{Path, Query},
	http::{HeaderMap, StatusCode},
	response::IntoResponse,
	routing::get,
	Extension, Json, Router,
};
use stump_api_types::OffsetPagination;
use stump_auth::AuthContext;
use stump_core::opds::v2_0::progression::OPDSProgressionInput;

/// The externally visible origin used when a provider has to generate absolute links.
///
/// The server creates this value with its proxy-aware host extractor and inserts it into
/// requests before mounting this router. Keeping the type here avoids making the provider
/// crate depend on the server's middleware module.
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

/// OPDS v2 browse filters. These fields intentionally mirror the Readium metadata query names.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseParams {
	#[serde(flatten)]
	pub pagination: OffsetPagination,
	pub author: Option<String>,
	pub penciler: Option<String>,
	pub colorist: Option<String>,
	pub inker: Option<String>,
	pub letterer: Option<String>,
	pub editor: Option<String>,
	pub cover_artist: Option<String>,
	pub subject: Option<String>,
	pub characters: Option<String>,
	pub teams: Option<String>,
}

/// The backend contract needed by the OPDS routes.
///
/// Route handlers deliberately contain only protocol extraction and dispatch. The server adapter
/// owns database/media access and returns the server's existing response and error semantics.
#[async_trait]
pub trait OpdsBackend: Send + Sync + 'static {
	type Error: axum::response::IntoResponse + Send + 'static;

	async fn v1_catalog(
		&self,
		auth: AuthContext,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v1_search_description(
		&self,
		auth: AuthContext,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v1_search_feed(
		&self,
		auth: AuthContext,
		search: Option<String>,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v1_keep_reading(
		&self,
		auth: AuthContext,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v1_get_libraries(
		&self,
		auth: AuthContext,
		search: Option<String>,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v1_get_library_by_id(
		&self,
		auth: AuthContext,
		id: String,
		pagination: OffsetPagination,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v1_get_series(
		&self,
		auth: AuthContext,
		search: Option<String>,
		pagination: OffsetPagination,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v1_get_latest_series(
		&self,
		auth: AuthContext,
		pagination: OffsetPagination,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v1_get_series_by_id(
		&self,
		auth: AuthContext,
		id: String,
		pagination: OffsetPagination,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v1_get_books(
		&self,
		auth: AuthContext,
		search: Option<String>,
		pagination: OffsetPagination,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v1_get_latest_books(
		&self,
		auth: AuthContext,
		pagination: OffsetPagination,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v1_get_book_thumbnail(
		&self,
		auth: AuthContext,
		id: String,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v1_get_book_page(
		&self,
		auth: AuthContext,
		id: String,
		page: i32,
		pagination: OffsetPagination,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v1_download_book(
		&self,
		auth: AuthContext,
		id: String,
		headers: HeaderMap,
	) -> Result<axum::response::Response, Self::Error>;

	async fn v2_auth(
		&self,
		host: ProviderHost,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_catalog(
		&self,
		auth: AuthContext,
		host: ProviderHost,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_search(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		query: Option<String>,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_browse_libraries(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		pagination: OffsetPagination,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_browse_library_by_id(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		id: String,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_browse_library_books(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		id: String,
		pagination: OffsetPagination,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_latest_library_books(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		id: String,
		pagination: OffsetPagination,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_browse_series(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		pagination: OffsetPagination,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_browse_series_by_id(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		id: String,
		pagination: OffsetPagination,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_browse_books(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		params: BrowseParams,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_latest_books(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		pagination: OffsetPagination,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_keep_reading(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		pagination: OffsetPagination,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_get_book_by_id(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		id: String,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_get_book_thumbnail(
		&self,
		auth: AuthContext,
		id: String,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_get_book_page(
		&self,
		auth: AuthContext,
		id: String,
		page: i32,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_get_book_progression(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		id: String,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_update_book_progression(
		&self,
		auth: AuthContext,
		id: String,
		input: OPDSProgressionInput,
	) -> Result<axum::response::Response, Self::Error>;
	async fn v2_download_book(
		&self,
		auth: AuthContext,
		id: String,
		headers: HeaderMap,
	) -> Result<axum::response::Response, Self::Error>;
}

fn v1_routes<S, B>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	B: OpdsBackend,
{
	Router::<S>::new()
		.route("/catalog", get(v1_catalog::<B>))
		.route("/search", get(v1_search_description::<B>))
		.route("/search/feed", get(v1_search_feed::<B>))
		.route("/keep-reading", get(v1_keep_reading::<B>))
		.nest(
			"/libraries",
			Router::new()
				.route("/", get(v1_get_libraries::<B>))
				.route("/{id}", get(v1_get_library_by_id::<B>)),
		)
		.nest(
			"/series",
			Router::new()
				.route("/", get(v1_get_series::<B>))
				.route("/latest", get(v1_get_latest_series::<B>))
				.route("/{id}", get(v1_get_series_by_id::<B>)),
		)
		.nest(
			"/books",
			Router::new()
				.route("/", get(v1_get_books::<B>))
				.route("/latest", get(v1_get_latest_books::<B>))
				.route("/{id}/thumbnail", get(v1_get_book_thumbnail::<B>))
				.route("/{id}/pages/{page}", get(v1_get_book_page::<B>))
				.route("/{id}/file/{filename}", get(v1_download_book::<B>)),
		)
}

/// Build the unprefixed OPDS 1.2 routes with an injected backend.
pub fn v1_router<S, B>(backend: Arc<B>) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	B: OpdsBackend,
{
	v1_routes::<S, B>().layer(Extension(backend))
}

fn v2_routes<S, B>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	B: OpdsBackend,
{
	Router::<S>::new()
		.route("/auth", get(v2_auth::<B>))
		.route("/catalog", get(v2_catalog::<B>))
		.route("/search", get(v2_search::<B>))
		.nest(
			"/libraries",
			Router::new()
				.route("/", get(v2_browse_libraries::<B>))
				.nest(
					"/{id}",
					Router::new()
						.route("/", get(v2_browse_library_by_id::<B>))
						.nest(
							"/books",
							Router::new()
								.route("/", get(v2_browse_library_books::<B>))
								.route("/latest", get(v2_latest_library_books::<B>)),
						),
				),
		)
		.nest(
			"/series",
			Router::new().route("/", get(v2_browse_series::<B>)).nest(
				"/{id}",
				Router::new().route("/", get(v2_browse_series_by_id::<B>)),
			),
		)
		.nest(
			"/books",
			Router::new()
				.route("/browse", get(v2_browse_books::<B>))
				.route("/latest", get(v2_latest_books::<B>))
				.route("/keep-reading", get(v2_keep_reading::<B>))
				.nest(
					"/{id}",
					Router::new()
						.route("/", get(v2_get_book_by_id::<B>))
						.route("/thumbnail", get(v2_get_book_thumbnail::<B>))
						.route("/pages/{page}", get(v2_get_book_page::<B>))
						.route(
							"/progression",
							get(v2_get_book_progression::<B>)
								.put(v2_update_book_progression::<B>),
						)
						.route("/file", get(v2_download_book::<B>)),
				),
		)
}

/// Build the unprefixed OPDS 2.0 routes with an injected backend.
pub fn v2_router<S, B>(backend: Arc<B>) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	B: OpdsBackend,
{
	v2_routes::<S, B>().layer(Extension(backend))
}

/// Build the complete OPDS surface under `/v1.2` and `/v2.0`.
pub fn router<S, B>(backend: B) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	B: OpdsBackend,
{
	let backend = Arc::new(backend);
	Router::<S>::new()
		.nest(
			"/v1.2",
			v1_routes::<S, B>().layer(Extension(backend.clone())),
		)
		.nest("/v2.0", v2_routes::<S, B>().layer(Extension(backend)))
}

async fn v1_catalog<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v1_catalog(auth).await
}

async fn v1_search_description<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v1_search_description(auth).await
}

#[derive(Debug, Default, serde::Deserialize)]
struct V1SearchQuery {
	#[serde(default)]
	search: Option<String>,
}

async fn v1_search_feed<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(auth): Extension<AuthContext>,
	Query(V1SearchQuery { search }): Query<V1SearchQuery>,
) -> Result<axum::response::Response, B::Error> {
	backend.v1_search_feed(auth, search).await
}

async fn v1_keep_reading<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v1_keep_reading(auth).await
}

async fn v1_get_libraries<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Query(V1SearchQuery { search }): Query<V1SearchQuery>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v1_get_libraries(auth, search).await
}

#[derive(Debug, serde::Deserialize)]
struct V1IdParams {
	id: String,
}

#[derive(Debug, serde::Deserialize)]
struct V1PageParams {
	id: String,
	page: i32,
}

#[derive(Debug, serde::Deserialize)]
struct V1FilenameParams {
	id: String,
	#[allow(dead_code)]
	filename: String,
}

async fn v1_get_library_by_id<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Path(V1IdParams { id }): Path<V1IdParams>,
	Query(pagination): Query<OffsetPagination>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v1_get_library_by_id(auth, id, pagination).await
}

async fn v1_get_series<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Query(pagination): Query<OffsetPagination>,
	Query(V1SearchQuery { search }): Query<V1SearchQuery>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v1_get_series(auth, search, pagination).await
}

async fn v1_get_latest_series<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Query(pagination): Query<OffsetPagination>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v1_get_latest_series(auth, pagination).await
}

async fn v1_get_series_by_id<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Path(V1IdParams { id }): Path<V1IdParams>,
	Query(pagination): Query<OffsetPagination>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v1_get_series_by_id(auth, id, pagination).await
}

async fn v1_get_books<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Query(pagination): Query<OffsetPagination>,
	Query(V1SearchQuery { search }): Query<V1SearchQuery>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v1_get_books(auth, search, pagination).await
}

async fn v1_get_latest_books<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Query(pagination): Query<OffsetPagination>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v1_get_latest_books(auth, pagination).await
}

async fn v1_get_book_thumbnail<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Path(V1IdParams { id }): Path<V1IdParams>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v1_get_book_thumbnail(auth, id).await
}

async fn v1_get_book_page<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Path(V1PageParams { id, page }): Path<V1PageParams>,
	Query(pagination): Query<OffsetPagination>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v1_get_book_page(auth, id, page, pagination).await
}

async fn v1_download_book<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Path(V1FilenameParams { id, .. }): Path<V1FilenameParams>,
	Extension(auth): Extension<AuthContext>,
	headers: HeaderMap,
) -> Result<axum::response::Response, B::Error> {
	backend.v1_download_book(auth, id, headers).await
}

async fn v2_auth<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
) -> Result<axum::response::Response, B::Error> {
	backend.v2_auth(host).await
}

async fn v2_catalog<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v2_catalog(auth, host).await
}

#[derive(Debug, Default, serde::Deserialize)]
struct V2SearchQuery {
	#[serde(default)]
	query: Option<String>,
}

async fn v2_search<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
	Query(V2SearchQuery { query }): Query<V2SearchQuery>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v2_search(auth, host, query).await
}

async fn v2_browse_libraries<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
	Query(pagination): Query<OffsetPagination>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v2_browse_libraries(auth, host, pagination).await
}

async fn v2_browse_library_by_id<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
	Path(id): Path<String>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v2_browse_library_by_id(auth, host, id).await
}

async fn v2_browse_library_books<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
	Path(id): Path<String>,
	Query(pagination): Query<OffsetPagination>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend
		.v2_browse_library_books(auth, host, id, pagination)
		.await
}

async fn v2_latest_library_books<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
	Path(id): Path<String>,
	Query(pagination): Query<OffsetPagination>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend
		.v2_latest_library_books(auth, host, id, pagination)
		.await
}

async fn v2_browse_series<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
	Query(pagination): Query<OffsetPagination>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v2_browse_series(auth, host, pagination).await
}

async fn v2_browse_series_by_id<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
	Path(id): Path<String>,
	Query(pagination): Query<OffsetPagination>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend
		.v2_browse_series_by_id(auth, host, id, pagination)
		.await
}

async fn v2_browse_books<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
	Query(params): Query<BrowseParams>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v2_browse_books(auth, host, params).await
}

async fn v2_latest_books<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
	Query(pagination): Query<OffsetPagination>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v2_latest_books(auth, host, pagination).await
}

async fn v2_keep_reading<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
	Query(pagination): Query<OffsetPagination>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v2_keep_reading(auth, host, pagination).await
}

async fn v2_get_book_by_id<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
	Path(id): Path<String>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v2_get_book_by_id(auth, host, id).await
}

async fn v2_get_book_thumbnail<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Path(id): Path<String>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v2_get_book_thumbnail(auth, id).await
}

async fn v2_get_book_page<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Path((id, page)): Path<(String, i32)>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v2_get_book_page(auth, id, page).await
}

async fn v2_get_book_progression<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Extension(host): Extension<ProviderHost>,
	Path(id): Path<String>,
	Extension(auth): Extension<AuthContext>,
) -> Result<axum::response::Response, B::Error> {
	backend.v2_get_book_progression(auth, host, id).await
}

async fn v2_update_book_progression<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Path(id): Path<String>,
	Extension(auth): Extension<AuthContext>,
	Json(input): Json<OPDSProgressionInput>,
) -> Result<axum::response::Response, B::Error> {
	backend.v2_update_book_progression(auth, id, input).await
}

async fn v2_download_book<B: OpdsBackend>(
	Extension(backend): Extension<Arc<B>>,
	Path(id): Path<String>,
	Extension(auth): Extension<AuthContext>,
	headers: HeaderMap,
) -> Result<axum::response::Response, B::Error> {
	backend.v2_download_book(auth, id, headers).await
}

/// A small helper for adapters that need to return a successful empty response.
pub fn no_content() -> axum::response::Response {
	StatusCode::NO_CONTENT.into_response()
}

/// Namespaced exports matching the other compatibility provider crates.
pub mod routes {
	pub use super::{
		router, v1_router, v2_router, BrowseParams, OpdsBackend, ProviderHost,
	};
}
