//! Provider host bootstrap and maintenance.
//!
//! This module owns the lifecycle of the [`stump_provider::ProviderHost`]:
//! whether it runs at all (`STUMP_ENABLE_PROVIDERS` /
//! `providers.enable_providers`), the compiled source factories, and the
//! garbage-collection scheduler that reclaims materialised provider series
//! nobody reads.
//!
//! The host is created once and stored on [`Ctx`]; everything downstream
//! (GraphQL, the Komga/OPDS virtual branches, media resolution) reaches it
//! through [`Ctx::provider_host`].

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::{config::StumpConfig, Ctx};

/// How often the periodic GC sweep runs. `provider_gc_days` is the
/// retention window; this is only how often the window is checked.
pub const GC_SWEEP_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Delay before the first GC sweep after boot, so startup is never delayed.
pub const GC_INITIAL_DELAY: Duration = Duration::from_secs(2 * 60);

/// Whether the provider host should run for this configuration.
pub fn is_enabled(config: &StumpConfig) -> bool {
	config.providers.enable_providers
}

/// Instantiate the provider host from the context's database and config,
/// install it as the `provider://` media resolver, and start the GC
/// scheduler. Returns `None` when providers are disabled
/// (`STUMP_ENABLE_PROVIDERS=false`).
pub async fn init(
	ctx: &Ctx,
) -> crate::error::CoreResult<Option<Arc<stump_provider::ProviderHost>>> {
	if !is_enabled(&ctx.config) {
		tracing::info!("Provider host disabled (providers.enable_providers=false)");
		return Ok(None);
	}

	let factories = default_factories();
	if factories.is_empty() {
		tracing::warn!(
			"Provider host enabled but no source implementations are compiled"
		);
	}

	let host_config = stump_provider::ProviderHostConfig {
		cache_dir: ctx.config.get_cache_dir().join("providers"),
		cache_max_bytes: ctx.config.providers.provider_cache_max_bytes,
		catalog_url: None,
		virtual_series_ttl: Duration::from_secs(ctx.config.providers.virtual_series_ttl),
	};

	let host =
		stump_provider::ProviderHost::open(ctx.conn.clone(), factories, host_config)
			.await
			.map_err(|error| crate::error::CoreError::InternalError(error.to_string()))?;
	host.install();

	if ctx.set_provider_host(host.clone()).is_err() {
		tracing::debug!("Provider host already initialized");
	}

	spawn_gc_scheduler(Arc::new(ctx.clone()));

	tracing::info!("Provider host initialized");
	Ok(Some(host))
}

/// The compiled source factories: every implementation shipped with this
/// build, in stable order.
pub fn default_factories() -> Vec<stump_provider::SourceFactory> {
	vec![stump_provider_mangadex::factory()]
}

/// The GC retention cutoff for a configuration.
pub fn gc_cutoff(config: &StumpConfig) -> DateTime<Utc> {
	Utc::now() - chrono::Duration::days(config.providers.provider_gc_days as i64)
}

/// Spawn the periodic GC sweep that enqueues
/// [`StumpJob::ProviderGc`](crate::job::stump_job::StumpJob::ProviderGc)
/// through the context. The task ends when the context is dropped.
fn spawn_gc_scheduler(ctx: Arc<Ctx>) {
	tokio::spawn(async move {
		tokio::time::sleep(GC_INITIAL_DELAY).await;
		loop {
			if is_enabled(&ctx.config) {
				if let Err(error) = ctx
					.enqueue(crate::job::stump_job::StumpJob::ProviderGc)
					.await
				{
					tracing::warn!(?error, "Failed to enqueue provider GC job");
				}
			} else {
				tracing::debug!("Provider GC skipped: providers disabled");
			}
			tokio::time::sleep(GC_SWEEP_INTERVAL).await;
		}
	});
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn gc_cutoff_respects_provider_gc_days() {
		let mut config = StumpConfig::debug();
		config.providers.provider_gc_days = 7;
		let cutoff = gc_cutoff(&config);
		let expected = Utc::now() - chrono::Duration::days(7);
		assert!(
			(expected - cutoff).num_seconds().abs() < 5,
			"cutoff should be ~7 days ago"
		);
	}

	#[test]
	fn debug_config_has_providers_disabled() {
		let config = StumpConfig::debug();
		assert!(!is_enabled(&config), "debug config has providers off");
	}
}
