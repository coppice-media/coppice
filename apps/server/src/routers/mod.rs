use axum::Router;

use stump_core::component_runtime::{
	ComponentDefinition, TransitionMode, COMPONENT_ABS, COMPONENT_API,
	COMPONENT_CROSSPOINT, COMPONENT_KAVITA, COMPONENT_KOBO, COMPONENT_KOMGA,
	COMPONENT_KOREADER, COMPONENT_LISEUR_SYNC, COMPONENT_OPDS, COMPONENT_WEBUI,
};

use crate::{config::state::AppState, middleware::component::gate};

#[cfg(feature = "abs")]
mod abs_backend;
pub(crate) mod api;
mod audio_transform;
#[cfg(feature = "kavita")]
mod kavita;
#[cfg(feature = "kobo")]
mod kobo_backend;
#[cfg(feature = "komf")]
mod komf_backend;
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
#[cfg(feature = "providers")]
pub(crate) mod provider_virtual;
mod static_apps;
pub(crate) use api::v2::auth::enforce_max_sessions;
use api::v2::transcode_job::registry as worker_job_registry;

#[cfg(feature = "webui")]
mod spa;

fn server_component_definitions(state: &AppState) -> Vec<ComponentDefinition> {
	vec![
		ComponentDefinition::new(
			COMPONENT_KOREADER,
			"KOReader sync",
			"integration",
			cfg!(feature = "koreader"),
			state.config.protocols.enable_koreader_sync,
			TransitionMode::Hot,
			Vec::<&str>::new(),
		)
		.with_activity_source("KOReader route requests"),
		ComponentDefinition::new(
			COMPONENT_KOBO,
			"Kobo sync",
			"integration",
			cfg!(feature = "kobo"),
			state.config.protocols.enable_kobo_sync,
			TransitionMode::Hot,
			Vec::<&str>::new(),
		)
		.with_activity_source("Kobo route requests"),
		ComponentDefinition::new(
			COMPONENT_CROSSPOINT,
			"CrossPoint delivery",
			"integration",
			cfg!(all(feature = "crosspoint", feature = "koreader")),
			true,
			TransitionMode::Hot,
			[COMPONENT_KOREADER],
		)
		.with_activity_source("CrossPoint route requests"),
		ComponentDefinition::new(
			COMPONENT_KOMGA,
			"Komga compatibility",
			"integration",
			cfg!(feature = "komga"),
			state.config.protocols.enable_komga,
			TransitionMode::Hot,
			Vec::<&str>::new(),
		)
		.with_activity_source("Komga route requests"),
		ComponentDefinition::new(
			COMPONENT_KAVITA,
			"Kavita compatibility",
			"integration",
			cfg!(feature = "kavita"),
			state.config.protocols.enable_kavita,
			TransitionMode::Hot,
			Vec::<&str>::new(),
		)
		.with_activity_source("Kavita route requests"),
		ComponentDefinition::new(
			COMPONENT_ABS,
			"Audiobookshelf compatibility",
			"integration",
			cfg!(feature = "abs"),
			state.config.protocols.enable_abs,
			TransitionMode::Hot,
			Vec::<&str>::new(),
		)
		.with_activity_source("Audiobookshelf route requests"),
		ComponentDefinition::new(
			COMPONENT_LISEUR_SYNC,
			"Liseur sync",
			"integration",
			cfg!(feature = "liseur-sync"),
			true,
			TransitionMode::Hot,
			Vec::<&str>::new(),
		)
		.with_activity_source("Liseur route requests"),
		ComponentDefinition::new(
			COMPONENT_OPDS,
			"OPDS",
			"integration",
			cfg!(feature = "opds"),
			true,
			TransitionMode::Hot,
			Vec::<&str>::new(),
		)
		.with_activity_source("OPDS route requests"),
		ComponentDefinition::new(
			COMPONENT_WEBUI,
			"Web UI",
			"presentation",
			cfg!(feature = "webui"),
			state.config.protocols.enable_webui,
			TransitionMode::Restart,
			Vec::<&str>::new(),
		),
		ComponentDefinition::new(
			COMPONENT_API,
			"Native API",
			"api",
			true,
			true,
			TransitionMode::Hot,
			Vec::<&str>::new(),
		)
		.with_activity_source("native API route requests"),
	]
}

pub(crate) fn install_worker_job_registry(app_state: &AppState) {
	app_state.install_worker_registry(worker_job_registry(app_state));
	#[cfg(feature = "readium")]
	{
		let jobs = app_state.worker_jobs();
		if !jobs.has_result_validator() {
			let validator = stump_library::sync_maps::alignment_result_validator(
				app_state.conn.clone(),
				app_state.config.get_transform_cache_dir(),
			);
			if !jobs.set_result_validator(validator) && !jobs.has_result_validator() {
				panic!("read-aloud ALIGN result validator could not be installed");
			}
		}
		if !jobs.has_result_validator() {
			panic!("read-aloud ALIGN result validator is not installed");
		}
	}
}

pub async fn mount(app_state: AppState) -> Router<AppState> {
	if let Err(error) = app_state
		.register_components(server_component_definitions(&app_state))
		.await
	{
		tracing::error!(?error, "Failed to register server runtime components");
	}

	// The job kinds' local implementations, installed before any route that
	// can enqueue is mounted: without them a `transcode` on a server that has
	// `ffmpeg` would answer `needs_worker`. Installed here rather than by the
	// binary so the test harness and any other router owner get it too.
	install_worker_job_registry(&app_state);
	let mut app_router = Router::new();

	#[cfg(feature = "koreader")]
	{
		app_router = app_router.merge(gate(
			koreader_backend::mount(app_state.clone()),
			app_state.clone(),
			COMPONENT_KOREADER,
		));
	}

	#[cfg(feature = "kobo")]
	{
		app_router = app_router.merge(gate(
			kobo_backend::mount(app_state.clone()),
			app_state.clone(),
			COMPONENT_KOBO,
		));
	}

	#[cfg(feature = "komga")]
	{
		app_router = app_router.merge(gate(
			komga::mount(app_state.clone()),
			app_state.clone(),
			COMPONENT_KOMGA,
		));
	}
	#[cfg(feature = "komf")]
	if app_state.config.protocols.enable_komf {
		app_router = app_router.merge(komf_backend::mount(app_state.clone()));
	}

	#[cfg(feature = "liseur-sync")]
	{
		app_router = app_router.merge(gate(
			liseur_sync::mount(app_state.clone()),
			app_state.clone(),
			COMPONENT_LISEUR_SYNC,
		));
	}

	#[cfg(feature = "kavita")]
	{
		app_router = app_router.merge(gate(
			kavita::mount(app_state.clone()),
			app_state.clone(),
			COMPONENT_KAVITA,
		));
	}

	#[cfg(feature = "abs")]
	{
		app_router = app_router.merge(gate(
			abs_backend::mount(app_state.clone()),
			app_state.clone(),
			COMPONENT_ABS,
		));
	}

	#[cfg(not(feature = "abs"))]
	if app_state.config.protocols.enable_abs {
		tracing::warn!(
			"STUMP_ENABLE_ABS is enabled, but this server was compiled without the `abs` feature; serving native API routes without Audiobookshelf compatibility"
		);
	}

	#[cfg(not(feature = "kavita"))]
	if app_state.config.protocols.enable_kavita {
		tracing::warn!(
			"STUMP_ENABLE_KAVITA is enabled, but this server was compiled without the `kavita` feature; serving native API routes without Kavita compatibility"
		);
	}

	#[cfg(not(feature = "koreader"))]
	if app_state.config.protocols.enable_koreader_sync {
		tracing::warn!(
			"ENABLE_KOREADER_SYNC is enabled, but this server was compiled without the `koreader` feature; serving native API routes without KOReader compatibility"
		);
	}

	#[cfg(not(feature = "kobo"))]
	if app_state.config.protocols.enable_kobo_sync {
		tracing::warn!(
			"ENABLE_KOBO_SYNC is enabled, but this server was compiled without the `kobo` feature; serving native API routes without Kobo compatibility"
		);
	}

	// Mount static Home and Editor apps before the web UI redirect fallback.
	app_router = app_router.merge(static_apps::mount(&app_state));

	#[cfg(feature = "webui")]
	let web_ui_owns_root = app_state.component_enabled(COMPONENT_WEBUI);
	#[cfg(not(feature = "webui"))]
	let web_ui_owns_root = false;
	if web_ui_owns_root {
		#[cfg(feature = "webui")]
		{
			app_router = app_router.merge(spa::mount());
		}
	} else {
		app_router = app_router.merge(static_apps::home_landing(&app_state));
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
	#[cfg(not(feature = "komf"))]
	if app_state.config.protocols.enable_komf {
		tracing::warn!(
			"STUMP_ENABLE_KOMF is enabled, but this server was compiled without the `komf` feature; Komf compatibility routes are unavailable"
		);
	}

	app_router = app_router.merge(gate(
		api::mount(app_state.clone()).await,
		app_state.clone(),
		COMPONENT_API,
	));

	#[cfg(feature = "opds")]
	{
		app_router = app_router.merge(gate(
			opds_backend::mount(app_state.clone()),
			app_state.clone(),
			COMPONENT_OPDS,
		));
	}

	app_router
}
