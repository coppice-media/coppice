use std::env;

use serde::{Deserialize, Serialize};
use stump_config_gen::StumpConfigGenerator;

use super::{defaults::*, env_keys::*};

/// Runtime gate and server-to-bridge handoff paths. Flattened into
/// [`super::StumpConfig`] and intentionally omitted from GraphQL.
#[derive(StumpConfigGenerator, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MamAcquisitionConfig {
	/// Enables explicit manager search/grab actions and the grab refresh job.
	#[default_value(DEFAULT_ENABLE_MAM_ACQUISITION)]
	#[env_key(ENABLE_MAM_ACQUISITION_KEY)]
	pub enable_mam_acquisition: bool,

	/// Base URL of the authenticated MAM Bridge sidecar.
	#[default_value(None)]
	#[env_key(MAM_BRIDGE_URL_KEY)]
	pub mam_bridge_url: Option<String>,

	/// File containing the bridge bearer token. Token contents are never exposed
	/// in config output, GraphQL, or logs.
	#[default_value(None)]
	#[env_key(MAM_BRIDGE_TOKEN_FILE_KEY)]
	pub mam_bridge_token_file: Option<String>,

	/// Local path corresponding to the bridge's `/downloads` mount.
	#[default_value(None)]
	#[env_key(MAM_BRIDGE_HANDOFF_ROOT_KEY)]
	pub mam_bridge_handoff_root: Option<String>,

	/// Optional source-worker root label covering `/downloads`; when set, the
	/// verified source-worker path is preferred to the local mapped path.
	#[default_value(None)]
	#[env_key(MAM_BRIDGE_SOURCE_ROOT_KEY)]
	pub mam_bridge_source_root: Option<String>,
}
