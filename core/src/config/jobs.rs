use std::env;

use serde::{Deserialize, Serialize};
use stump_config_gen::StumpConfigGenerator;

use super::{defaults::*, env_keys::*};

/// Background job and scan concurrency settings. Flattened into [`super::StumpConfig`].
#[derive(StumpConfigGenerator, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "StumpJobsConfig"))]
pub struct JobsConfig {
	/// Whether background jobs such as scheduled scans are enabled.
	#[default_value(DEFAULT_ENABLE_BACKGROUND_JOBS)]
	#[env_key(ENABLE_BACKGROUND_JOBS_KEY)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub enable_background_jobs: bool,

	/// A multiplier applied to the number of logical CPUs to derive the default scanner concurrency
	/// limit. Increasing can speed things up but will increase resource usage
	#[default_value(DEFAULT_PARALLELISM_MULTIPLIER)]
	#[env_key(PARALLELISM_MULTIPLIER_KEY)]
	pub parallelism_multiplier: usize,
}

impl JobsConfig {
	/// returns a sensible default concurrency limit based on the number of logical cpus
	/// available to the process, scaled by `parallelism_multiplier`.
	pub fn cpu_concurrency_limit(&self) -> usize {
		let multiplier = std::cmp::max(self.parallelism_multiplier, 1);
		std::thread::available_parallelism()
			.map(|n| n.get() * multiplier)
			.unwrap_or(multiplier)
	}
}
