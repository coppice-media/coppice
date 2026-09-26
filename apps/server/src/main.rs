// The merged GraphQL schema nests deeply enough that layout computation
// overflows rustc's default query depth.
#![recursion_limit = "256"]
use cli::{handle_command, Cli, Parser};
use errors::EntryError;
use stump_core::{
	config::bootstrap_config_dir, config::logging::init_tracing, StumpCore,
};

mod config;
mod errors;
mod http_server;
mod middleware;
mod routers;
mod utils;

#[cfg(debug_assertions)]
fn debug_setup() {
	std::env::set_var("STUMP_PROFILE", "debug");
	std::env::set_var("STUMP_COLORFUL_LOGS", "true");
}

fn main() -> Result<(), EntryError> {
	#[cfg(debug_assertions)]
	debug_setup();

	let runtime = config::runtime::build_runtime().map_err(EntryError::InvalidConfig)?;
	runtime.block_on(run())
}

async fn run() -> Result<(), EntryError> {
	let config_dir = bootstrap_config_dir();

	let config = StumpCore::init_config(config_dir)
		.map_err(|e| EntryError::InvalidConfig(e.to_string()))?;

	let cli = Cli::parse();

	if let Some(command) = cli.command {
		let resolved_config = cli.config.merge_stump_config(config);
		stump_core::database::validate_pool_config(&resolved_config)
			.map_err(|e| EntryError::InvalidConfig(e.to_string()))?;
		Ok(handle_command(command, &resolved_config).await?)
	} else {
		let resolved_config = cli.config.merge_stump_config(config);
		stump_core::database::validate_pool_config(&resolved_config)
			.map_err(|e| EntryError::InvalidConfig(e.to_string()))?;
		// Note: init_tracing after loading the environment so the correct verbosity
		// level is used for logging.
		init_tracing(&resolved_config);

		if let Some(oidc) = &resolved_config.oidc {
			tracing::info!(enabled = oidc.enabled, "OIDC configuration loaded");
		}

		if resolved_config.server.verbosity >= 3 {
			tracing::trace!(?resolved_config, "App config");
		}

		Ok(http_server::run_http_server(resolved_config).await?)
	}
}
