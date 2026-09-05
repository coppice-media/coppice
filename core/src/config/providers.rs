use std::env;

use serde::{Deserialize, Serialize};
use stump_config_gen::StumpConfigGenerator;

use super::{defaults::*, env_keys::*};

/// Metadata/content provider host settings. Flattened into [`super::StumpConfig`].
/// None of these keys are exposed through GraphQL, so the whole group is skipped there.
#[derive(StumpConfigGenerator, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProvidersConfig {
	/// Whether the provider host is enabled.
	#[default_value(DEFAULT_ENABLE_PROVIDERS)]
	#[env_key(ENABLE_PROVIDERS_KEY)]
	pub enable_providers: bool,

	/// Upper bound, in bytes, of the on-disk provider cache.
	#[default_value(DEFAULT_PROVIDER_CACHE_MAX_BYTES)]
	#[env_key(PROVIDER_CACHE_MAX_BYTES_KEY)]
	pub provider_cache_max_bytes: u64,
}
