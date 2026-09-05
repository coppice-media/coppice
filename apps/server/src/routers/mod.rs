use axum::Router;

use crate::config::state::AppState;

mod api;
mod ingest_editor;
#[cfg(feature = "kobo")]
mod kobo_backend;
#[cfg(feature = "kavita")]
mod kavita;
#[cfg(feature = "komga")]
mod komga;
#[cfg(feature = "komga")]
mod komga_backend;
#[cfg(feature = "koreader")]
mod koreader_backend;
#[cfg(feature = "liseur-sync")]
mod liseur_sync;
#[cfg(feature = "opds")]
mod opds_backend;
pub(crate) use api::v2::auth::enforce_max_sessions;

#[cfg(feature = "webui")]
mod spa;

#[cfg(feature = "webui")]
pub(crate) use spa::relative_favicon_path;

#[cfg(not(feature = "webui"))]
pub(crate) fn relative_favicon_path(_webui_enabled: bool) -> Option<String> {
	None
}

pub async fn mount(app_state: AppState) -> Router<AppState> {
	let mut app_router = Router::new();

	#[cfg(feature = "koreader")]
	if app_state.config.protocols.enable_koreader_sync {
		app_router = app_router.merge(koreader_backend::mount(app_state.clone()));
	}

	#[cfg(feature = "kobo")]
	if app_state.config.protocols.enable_kobo_sync {
		app_router = app_router.merge(kobo_backend::mount(app_state.clone()));
	}

	#[cfg(feature = "komga")]
	if app_state.config.protocols.enable_komga {
		app_router = app_router.merge(komga::mount(app_state.clone()));
	}

	#[cfg(feature = "liseur-sync")]
	{
		app_router = app_router.merge(liseur_sync::mount(app_state.clone()));
	}

	#[cfg(feature = "kavita")]
	if app_state.config.protocols.enable_kavita {
		app_router = app_router.merge(kavita::mount(app_state.clone()));
	}

	#[cfg(not(feature = "kavita"))]
	if app_state.config.protocols.enable_kavita {
		tracing::warn!(
			"STUMP_ENABLE_KAVITA is enabled, but this server was compiled without the `kavita` feature; serving native API routes without Kavita compatibility"
		);
	}

	// Mounted before the web UI so `/editor/*` is not swallowed by the SPA
	// fallback; both can coexist in a full build.
	if let Some(dir) = ingest_editor::editor_dir(&app_state) {
		app_router = app_router.merge(ingest_editor::mount(&dir));
	}

	#[cfg(feature = "webui")]
	if app_state.config.protocols.enable_webui {
		app_router = app_router.merge(spa::mount(app_state.clone()));
	}

	#[cfg(all(not(feature = "webui"), feature = "opds"))]
	if app_state.config.protocols.enable_webui {
		tracing::warn!(
			"STUMP_ENABLE_WEBUI is enabled, but this server was compiled without the `webui` feature; serving API and OPDS routes without the web UI"
		);
	}

	#[cfg(all(not(feature = "webui"), not(feature = "opds")))]
	if app_state.config.protocols.enable_webui {
		tracing::warn!(
			"STUMP_ENABLE_WEBUI is enabled, but this server was compiled without the `webui` feature; serving API routes without the web UI"
		);
	}

	#[cfg(not(feature = "komga"))]
	if app_state.config.protocols.enable_komga {
		tracing::warn!(
			"STUMP_ENABLE_KOMGA is enabled, but this server was compiled without the `komga` feature; serving native API routes without Komga compatibility"
		);
	}

	app_router = app_router.merge(api::mount(app_state.clone()).await);

	#[cfg(feature = "opds")]
	{
		app_router = app_router.merge(opds_backend::mount(app_state));
	}

	app_router
}
