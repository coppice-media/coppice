//! Serves the built ingest editor (`editor/build`, a SvelteKit static bundle)
//! under `/editor` when `INGEST_EDITOR_DIR` points at it.
//!
//! This is a runtime switch rather than a Cargo feature: nothing is linked,
//! only files are served, and the same headless binary can run with or
//! without the editor present.

use std::path::{Path, PathBuf};

use axum::{
	http::{header, HeaderValue},
	Router,
};
use tower::ServiceBuilder;
use tower_http::{
	services::{ServeDir, ServeFile},
	set_header::SetResponseHeaderLayer,
};

use crate::config::state::AppState;

/// URL prefix the editor is built for (`paths.base` in `editor/svelte.config.js`).
pub const EDITOR_BASE: &str = "/editor";

/// Resolve the editor directory when it is configured and holds a build.
pub(crate) fn editor_dir(app_state: &AppState) -> Option<PathBuf> {
	let dir = PathBuf::from(app_state.config.ingest_editor_dir.as_deref()?);
	if dir.join("index.html").is_file() {
		Some(dir)
	} else {
		tracing::warn!(
			path = %dir.display(),
			"INGEST_EDITOR_DIR is set but contains no index.html; the ingest editor will not be served"
		);
		None
	}
}

pub(crate) fn mount(dir: &Path) -> Router<AppState> {
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
				// Client-side routes (`/editor/rework?item=…`) fall back to the shell.
				.fallback(ServeFile::new(dir.join("index.html"))),
		);
	Router::new()
		.nest_service(&format!("{EDITOR_BASE}/_app/immutable"), immutable)
		.nest_service(EDITOR_BASE, files)
}
