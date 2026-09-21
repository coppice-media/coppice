use std::path::Path;

use axum::{
	body::Body,
	extract::State,
	http::{header, HeaderMap, HeaderValue, Request, Uri},
	response::IntoResponse,
	response::Response,
	routing::get,
	Router,
};
use tower::ServiceBuilder;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
};

pub const FAVICON: &str = "/favicon.ico";
const SW: &str = "/sw.js";
const INDEX: &str = "/";
const INDEX_HTML: &str = "/index.html";
const MANIFEST: &str = "/assets/manifest.webmanifest";
const ASSETS: &str = "/assets";
const DIST: &str = "/dist";

pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	let dist_path = Path::new(&app_state.config.protocols.client_dir);
	let static_assets = ServiceBuilder::new()
		.layer(SetResponseHeaderLayer::if_not_present(
			header::VARY,
			HeaderValue::from_static("Accept-Encoding"),
		))
		.layer(SetResponseHeaderLayer::overriding(
			header::CACHE_CONTROL,
			HeaderValue::from_static("public, max-age=31536000, immutable, no-transform"),
		))
		.service(
			ServeDir::new(dist_path.join("assets"))
				.precompressed_br()
				.precompressed_gzip(),
		);

	let dist_files = ServiceBuilder::new()
		.layer(SetResponseHeaderLayer::if_not_present(
			header::VARY,
			HeaderValue::from_static("Accept-Encoding"),
		))
		.layer(SetResponseHeaderLayer::if_not_present(
			header::CACHE_CONTROL,
			HeaderValue::from_static("no-cache"),
		))
		.service(
			ServeDir::new(dist_path)
				.precompressed_br()
				.precompressed_gzip(),
		);

	Router::new()
		.route(INDEX, get(index_html))
		.route(INDEX_HTML, get(index_html))
		.route(FAVICON, get(favicon))
		.route(SW, get(serve_sw))
		.route(MANIFEST, get(serve_manifest))
		.nest_service(ASSETS, static_assets)
		.nest_service(DIST, dist_files)
		.fallback(spa_fallback)
}

/// Prefixes the SPA fallback must never answer.
///
/// The web UI's fallback becomes the *server's* fallback once this router is
/// merged, so without this an unknown API path answers `200 text/html` — the
/// SPA shell — instead of a `404`. That is worse than a missing route: a
/// client asked for JSON or a file and got an HTML document with a success
/// status, which is exactly how `GET /api/items/{id}/download` and
/// `/api/items/{id}/ebook` looked like working routes on a `full` build
/// while returning the web UI.
const NON_SPA_PREFIXES: [&str; 4] = ["/api/", "/opds/", "/public/", "/koreader/"];

/// `index.html` for a deep link into the web UI, a JSON `404` for anything
/// under an API prefix.
async fn spa_fallback(
	State(ctx): State<AppState>,
	uri: Uri,
	headers: HeaderMap,
) -> APIResult<Response> {
	let path = uri.path();
	if NON_SPA_PREFIXES
		.iter()
		.any(|prefix| path.starts_with(prefix))
	{
		return Err(APIError::NotFound(format!("No route for {path}")));
	}
	serve_with_no_cache(ctx, headers, "index.html").await
}

pub(crate) fn relative_favicon_path(webui_enabled: bool) -> Option<String> {
	webui_enabled.then(|| format!("{ASSETS}{FAVICON}"))
}

// https://github.com/tokio-rs/axum/discussions/608#discussioncomment-7772294
async fn favicon(
	State(ctx): State<AppState>,
	headers: HeaderMap,
) -> APIResult<impl IntoResponse> {
	let mut response = serve_dist_file(ctx, headers, "favicon.ico").await?;
	response.headers_mut().insert(
		header::CACHE_CONTROL,
		HeaderValue::from_static("public, max-age=86400"),
	);

	Ok(response)
}

async fn index_html(
	State(ctx): State<AppState>,
	headers: HeaderMap,
) -> APIResult<impl IntoResponse> {
	serve_with_no_cache(ctx, headers, "index.html").await
}

async fn serve_sw(
	State(ctx): State<AppState>,
	headers: HeaderMap,
) -> APIResult<impl IntoResponse> {
	serve_with_no_cache(ctx, headers, "sw.js").await
}

async fn serve_manifest(
	State(ctx): State<AppState>,
	headers: HeaderMap,
) -> APIResult<impl IntoResponse> {
	serve_with_no_store(ctx, headers, "assets/manifest.webmanifest").await
}

async fn serve_with_no_cache(
	ctx: AppState,
	headers: HeaderMap,
	path: &str,
) -> APIResult<Response> {
	let mut response = serve_dist_file(ctx, headers, path).await?;
	response.headers_mut().insert(
		header::CACHE_CONTROL,
		HeaderValue::from_static("no-cache, no-store, must-revalidate"),
	);

	Ok(response)
}

async fn serve_with_no_store(
	ctx: AppState,
	headers: HeaderMap,
	path: &str,
) -> APIResult<Response> {
	// Manifest and bootstrap must bypass caches.
	let mut response = serve_dist_file(ctx, headers, path).await?;
	response.headers_mut().insert(
		header::CACHE_CONTROL,
		HeaderValue::from_static("no-cache, no-store, must-revalidate"),
	);

	Ok(response)
}

async fn serve_dist_file(
	ctx: AppState,
	headers: HeaderMap,
	path: &str,
) -> APIResult<Response> {
	let mut req = Request::new(Body::empty());
	*req.headers_mut() = headers;

	match ServeFile::new(Path::new(&ctx.config.protocols.client_dir).join(path))
		.try_call(req)
		.await
	{
		Ok(res) => Ok(res.into_response()),
		Err(e) => {
			tracing::error!(error = ?e, path, "Error serving dist file");
			Err(APIError::InternalServerError(e.to_string()))
		},
	}
}
