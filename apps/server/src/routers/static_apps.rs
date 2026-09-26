//! Serves built static single-page apps from configured directories: the
//! ingest editor under `/editor` (`INGEST_EDITOR_DIR`) and the Home app under
//! `/app` (`STUMP_HOME_APP_DIR`).
//!
//! This is a runtime switch rather than a Cargo feature: nothing is linked,
//! only files are served, and the same headless binary can run with or
//! without any app present.

use std::path::{Path, PathBuf};

use axum::{
	http::{header, HeaderValue},
	response::Redirect,
	routing::get,
	Router,
};
use tower::ServiceBuilder;
use tower_http::{
	services::{ServeDir, ServeFile},
	set_header::SetResponseHeaderLayer,
};

use stump_core::config::env_keys::{HOME_APP_DIR_KEY, INGEST_EDITOR_DIR_KEY};

use crate::config::state::AppState;

/// URL prefix the ingest editor is built for (`paths.base` in `editor/svelte.config.js`).
pub const EDITOR_BASE: &str = "/editor";

/// URL prefix the Home app is built for (`paths.base` in `home/svelte.config.js`).
pub const APP_BASE: &str = "/app";

/// A built single-page app: its URL prefix and build directory.
struct StaticApp {
	base: &'static str,
	dir: PathBuf,
}

/// Resolve every configured app whose directory actually holds a build.
fn configured_apps(app_state: &AppState) -> Vec<StaticApp> {
	let mut apps = Vec::new();

	if let Some(dir) = validated_dir(
		app_state.config.ingest.ingest_editor_dir.as_deref(),
		INGEST_EDITOR_DIR_KEY,
		"the ingest editor",
	) {
		apps.push(StaticApp {
			base: EDITOR_BASE,
			dir,
		});
	}

	if let Some(dir) = validated_dir(
		app_state.config.server.home_app_dir.as_deref(),
		HOME_APP_DIR_KEY,
		"the Home app",
	) {
		apps.push(StaticApp {
			base: APP_BASE,
			dir,
		});
	}

	apps
}

/// Accept `dir` when it holds a built SPA (`index.html`); warn otherwise so a
/// stale or empty directory is visible in the logs instead of a silent 404.
fn validated_dir(dir: Option<&str>, env_key: &str, label: &str) -> Option<PathBuf> {
	let dir = PathBuf::from(dir?);
	if dir.join("index.html").is_file() {
		Some(dir)
	} else {
		tracing::warn!(
			path = %dir.display(),
			"{env_key} is set but contains no index.html; {label} will not be served"
		);
		None
	}
}

/// Mount configured Home and Editor apps before the web UI browser redirect
/// fallback so their base paths and assets retain precedence.
pub(crate) fn mount(app_state: &AppState) -> Router<AppState> {
	let mut router = Router::new();
	for app in configured_apps(app_state) {
		tracing::info!(base = app.base, dir = %app.dir.display(), "Serving static app");
		router = router.merge(mount_app(&app));
	}
	router
}

/// Make the Home app the landing page when the web UI redirect router is
/// disabled or not compiled. The web UI-enabled route owner mounts the root
/// redirect itself to avoid duplicate `/` routes.
pub(crate) fn home_landing(app_state: &AppState) -> Router<AppState> {
	let served = configured_apps(app_state)
		.iter()
		.any(|app| app.base == APP_BASE);
	if !served {
		return Router::new();
	}
	tracing::info!(base = APP_BASE, "Home app is the landing page for /");
	Router::new().route("/", get(|| async { Redirect::to(APP_BASE) }))
}

fn mount_app(app: &StaticApp) -> Router<AppState> {
	let dir: &Path = &app.dir;
	// SvelteKit emits hashed, immutable assets under `_app/immutable`; everything
	// else (index.html, favicon, manifest) must revalidate.
	let immutable = ServiceBuilder::new()
		.layer(SetResponseHeaderLayer::if_not_present(
			header::VARY,
			HeaderValue::from_static("Accept-Encoding"),
		))
		.layer(SetResponseHeaderLayer::overriding(
			header::CACHE_CONTROL,
			HeaderValue::from_static("public, max-age=31536000, immutable, no-transform"),
		))
		.service(
			ServeDir::new(dir.join("_app/immutable"))
				.precompressed_br()
				.precompressed_gzip(),
		);
	let files = ServiceBuilder::new()
		.layer(SetResponseHeaderLayer::if_not_present(
			header::CACHE_CONTROL,
			HeaderValue::from_static("no-cache"),
		))
		.service(
			ServeDir::new(dir)
				.precompressed_br()
				.precompressed_gzip()
				// ServeDir's directory redirect drops the outer Axum nest
				// prefix (`/app/requests` became `/requests/`). Let the SPA
				// fallback answer extensionless client-side routes instead.
				.append_index_html_on_directories(false)
				.fallback(ServeFile::new(dir.join("index.html"))),
		);
	Router::new()
		.nest_service(&format!("{}/_app/immutable", app.base), immutable)
		.nest_service(app.base, files)
}
