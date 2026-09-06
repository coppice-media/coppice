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

	/// How long a virtual library's live browse page stays cached before the
	/// source is hit again, in seconds.
	#[default_value(DEFAULT_VIRTUAL_SERIES_TTL_SECS)]
	#[env_key(VIRTUAL_SERIES_TTL_KEY)]
	pub virtual_series_ttl: u64,

	/// Materialised provider series with no reading head are reclaimed by
	/// the GC job once they are older than this many days.
	#[default_value(DEFAULT_PROVIDER_GC_DAYS)]
	#[env_key(PROVIDER_GC_DAYS_KEY)]
	pub provider_gc_days: u64,

	/// How often the source-health job probes every catalog source, in
	/// seconds.
	#[default_value(DEFAULT_PROVIDER_HEALTH_INTERVAL_SECS)]
	#[env_key(PROVIDER_HEALTH_INTERVAL_SECS_KEY)]
	pub provider_health_interval_secs: u64,

	/// Consecutive failed health runs after which a source is marked dead
	/// and hidden from the catalog. One reachable run resets the count.
	#[default_value(DEFAULT_PROVIDER_HEALTH_DEAD_AFTER)]
	#[env_key(PROVIDER_HEALTH_DEAD_AFTER_KEY)]
	pub provider_health_dead_after: u32,
}
