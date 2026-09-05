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
mod filter;
mod image;
mod library;
mod metadata;
mod query;
mod reader;
mod series;
mod series_filter;
mod server;
mod tachiyomi;
pub use account::LoginOutcome;

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
	let lower = path.to_ascii_lowercase();
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
		.merge(tachiyomi::routes::<S>())
		.merge(metadata::routes::<S>())
		.merge(filter::routes::<S>());
	// Unimplemented Kavita controllers answer 404 here rather than falling
	// through to the web UI; each miss is logged so the next wave sees it.
	// Controllers that already own a `/{param}` route at this depth
	// (Series, Library, Volume) are excluded to keep the route tree conflict-free.
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
		"/api/Book",
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
		"/api/reading-profile",
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
