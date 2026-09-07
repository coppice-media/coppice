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
use stump_provider::event::{ProviderEvent, ProviderEventSink};
use tokio::sync::broadcast::Sender;

use crate::{
	config::StumpConfig,
	event::{
		CoreEvent, ProviderCatalogRefreshed, ProviderSeriesMaterialized,
		ProviderSourceHealthChanged,
	},
	Ctx,
};

/// Forwards provider host activity onto the core event channel, where the
/// GraphQL subscriptions and notification routing pick it up.
///
/// This is the only place that knows both vocabularies: `stump_provider`
/// cannot depend on `stump_core`, so it emits [`ProviderEvent`] and this
/// maps every variant onto a [`CoreEvent`] one-to-one.
pub(crate) struct CoreProviderEventSink {
	events: Sender<CoreEvent>,
}

impl CoreProviderEventSink {
	pub(crate) fn new(events: Sender<CoreEvent>) -> Self {
		Self { events }
	}
}

impl ProviderEventSink for CoreProviderEventSink {
	fn emit(&self, event: ProviderEvent) {
		let event = match event {
			ProviderEvent::SourceHealthChanged {
				source_id,
				name,
				status,
			} => CoreEvent::ProviderSourceHealthChanged(ProviderSourceHealthChanged {
				source_id,
				name,
				status: status.as_str().to_string(),
			}),
			ProviderEvent::SeriesMaterialized {
				series_id,
				source,
				library_id,
			} => CoreEvent::ProviderSeriesMaterialized(ProviderSeriesMaterialized {
				series_id,
				source,
				library_id,
			}),
			ProviderEvent::CatalogRefreshed { count } => {
				CoreEvent::ProviderCatalogRefreshed(ProviderCatalogRefreshed { count })
			},
		};
		let _ = self.events.send(event);
	}
}

/// How often the periodic GC sweep runs. `provider_gc_days` is the
/// retention window; this is only how often the window is checked.
pub const GC_SWEEP_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Delay before the first GC sweep after boot, so startup is never delayed.
pub const GC_INITIAL_DELAY: Duration = Duration::from_secs(2 * 60);

/// Delay before the first health run after boot. Longer than the GC delay so
/// a cold start never fires two provider sweeps at once.
pub const HEALTH_INITIAL_DELAY: Duration = Duration::from_secs(5 * 60);

/// Parallel base URLs probed by one health run.
pub const HEALTH_CONCURRENCY: usize = stump_provider::health::DEFAULT_CONCURRENCY;

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
	let engines = default_engines();
	if factories.is_empty() && engines.is_empty() {
		tracing::warn!(
			"Provider host enabled but no source implementations are compiled"
		);
	}

	let host_config = stump_provider::ProviderHostConfig {
		cache_dir: ctx.config.get_cache_dir().join("providers"),
		cache_max_bytes: ctx.config.providers.provider_cache_max_bytes,
		catalog_url: None,
		definitions_url: Some(ctx.config.providers.source_definitions_url.clone()),
		virtual_series_ttl: Duration::from_secs(ctx.config.providers.virtual_series_ttl),
	};

	let host = stump_provider::ProviderHost::open(
		ctx.conn.clone(),
		factories,
		engines,
		host_config,
	)
	.await
	.map_err(|error| crate::error::CoreError::InternalError(error.to_string()))?;
	host.install();
	host.set_event_sink(Arc::new(CoreProviderEventSink::new(ctx.get_event_tx())));
	// The queue a gated source's `challenge_solve` goes on. Installed here
	// because this is the one place that holds both the host and the context;
	// `Ctx::worker_jobs` is lazy, and `http_server` has already installed the
	// job-kind registry by the time providers start, so forcing it now cannot
	// lose a local implementation.
	host.set_worker_jobs(ctx.worker_jobs());

	if ctx.set_provider_host(host.clone()).is_err() {
		tracing::debug!("Provider host already initialized");
	}

	spawn_gc_scheduler(Arc::new(ctx.clone()));
	spawn_health_scheduler(Arc::new(ctx.clone()));

	tracing::info!("Provider host initialized");
	Ok(Some(host))
}

/// The compiled source factories: every implementation shipped with this
/// build, in stable order.
pub fn default_factories() -> Vec<stump_provider::SourceFactory> {
	vec![stump_provider_mangadex::factory()]
}

/// The compiled theme engines: one per `lib-multisrc` theme Stump can execute
/// from a data-only [`stump_provider::SourceDefinition`], in stable order.
pub fn default_engines() -> Vec<stump_provider::DefinitionEngine> {
	stump_provider_themes::engines()
}

/// The GC retention cutoff for a configuration.
pub fn gc_cutoff(config: &StumpConfig) -> DateTime<Utc> {
	Utc::now() - chrono::Duration::days(config.providers.provider_gc_days as i64)
}

/// How often the health job runs (`provider_health_interval_secs`). A zero
/// or absurdly small value is floored at a minute so a misconfiguration
/// cannot hammer every source in the catalog.
pub fn health_interval(config: &StumpConfig) -> Duration {
	Duration::from_secs(config.providers.provider_health_interval_secs.max(60))
}

/// The consecutive-failure count after which a source is marked dead
/// (`provider_health_dead_after`).
pub fn health_dead_after(config: &StumpConfig) -> i32 {
	config.providers.provider_health_dead_after.max(1) as i32
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

/// Spawn the periodic source-health sweep that enqueues
/// [`StumpJob::ProviderSourceHealth`](crate::job::stump_job::StumpJob::ProviderSourceHealth)
/// every `provider_health_interval_secs`. The task ends when the context is
/// dropped.
fn spawn_health_scheduler(ctx: Arc<Ctx>) {
	let interval = health_interval(&ctx.config);
	tokio::spawn(async move {
		tokio::time::sleep(HEALTH_INITIAL_DELAY).await;
		loop {
			if is_enabled(&ctx.config) {
				if let Err(error) = ctx
					.enqueue(crate::job::stump_job::StumpJob::ProviderSourceHealth)
					.await
				{
					tracing::warn!(
						?error,
						"Failed to enqueue provider source health job"
					);
				}
			} else {
				tracing::debug!("Provider health skipped: providers disabled");
			}
			tokio::time::sleep(interval).await;
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

	#[test]
	fn health_schedule_follows_config_with_sane_floors() {
		let mut config = StumpConfig::debug();
		assert_eq!(
			health_interval(&config),
			Duration::from_secs(6 * 60 * 60),
			"the default sweep is every six hours"
		);
		assert_eq!(health_dead_after(&config), 3);

		config.providers.provider_health_interval_secs = 900;
		config.providers.provider_health_dead_after = 5;
		assert_eq!(health_interval(&config), Duration::from_secs(900));
		assert_eq!(health_dead_after(&config), 5);

		// A zero interval or threshold would probe every source in a tight
		// loop, or mark everything dead before it was ever probed.
		config.providers.provider_health_interval_secs = 0;
		config.providers.provider_health_dead_after = 0;
		assert_eq!(health_interval(&config), Duration::from_secs(60));
		assert_eq!(health_dead_after(&config), 1);
	}

	/// `defaults` cannot reference `stump_provider` (it is compiled without
	/// the `providers` feature), so the literal is pinned here instead.
	#[test]
	fn default_definitions_url_matches_the_host_default() {
		assert_eq!(
			crate::config::defaults::DEFAULT_SOURCE_DEFINITIONS_URL,
			stump_provider::definition::DEFAULT_DEFINITIONS_URL
		);
		assert_eq!(
			StumpConfig::debug().providers.source_definitions_url,
			stump_provider::definition::DEFAULT_DEFINITIONS_URL
		);
	}

	/// Every theme a definition may name must resolve to exactly one engine,
	/// or `enableProviderSource` would silently pick the first match.
	#[test]
	fn theme_engines_are_registered_once_each() {
		let engines = default_engines();
		assert!(!engines.is_empty(), "theme engines are compiled in");
		let mut themes: Vec<&str> = engines.iter().map(|engine| engine.theme).collect();
		let total = themes.len();
		themes.sort_unstable();
		themes.dedup();
		assert_eq!(
			themes.len(),
			total,
			"duplicate theme registration: {themes:?}"
		);
	}
}
