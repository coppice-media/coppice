use std::env;

use itertools::Itertools;
use serde::{Deserialize, Serialize};
use stump_config_gen::StumpConfigGenerator;

use super::env_keys::*;

/// HTTP listener, logging, and proxy settings. Flattened into [`super::StumpConfig`].
#[derive(StumpConfigGenerator, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "StumpServerConfig"))]
pub struct ServerConfig {
	/// The "release" | "debug" profile with which the application is running.
	#[default_value("release".to_string())]
	#[debug_value("debug".to_string())]
	#[env_key(PROFILE_KEY)]
	#[validator(do_validate_profile)]
	pub profile: String,

	/// The IP address on which to listen on (default: "0.0.0.0").
	#[default_value("0.0.0.0".to_string())]
	#[env_key(IP_KEY)]
	pub ip: String,

	/// The port from which to serve the application (default: 10801).
	#[default_value(10801)]
	#[env_key(PORT_KEY)]
	pub port: u16,

	/// The verbosity with which system logs are visible (default: 1).
	#[default_value(1)]
	#[env_key(VERBOSITY_KEY)]
	pub verbosity: u64,

	/// Whether or not to pretty print logs.
	#[default_value(true)]
	#[env_key(PRETTY_LOGS_KEY)]
	pub pretty_logs: bool,

	/// The directory where the applicaiton logs will be stored
	#[default_value(None)]
	#[env_key(LOG_DIR_KEY)]
	pub log_dir: Option<String>,

	/// Whether or not to include ANSI color codes in log files.
	#[default_value(false)]
	#[env_key(COLORFUL_LOGS_KEY)]
	pub colorful_logs: bool,

	/// A list of origins for CORS.
	#[default_value(vec![])]
	#[env_key(ORIGINS_KEY)]
	pub allowed_origins: Vec<String>,

	/// Whether to trust proxy headers for determining client IP and scheme (e.g., X-Forwarded-For)
	#[default_value(false)]
	#[env_key(TRUST_PROXY_HEADERS_KEY)]
	pub trust_proxy_headers: bool,
}

impl ServerConfig {
	/// Returns True if the configuration profile is "debug" and False otherwise.
	pub fn is_debug(&self) -> bool {
		self.profile.as_str() == "debug"
	}
}

fn do_validate_profile(profile: &String) -> bool {
	if profile == "release" || profile == "debug" {
		return true;
	}

	eprintln!("Invalid profile value: {profile}");
	false
}
