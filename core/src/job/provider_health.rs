//! The provider source-health job: probe every catalog source's base URL and
//! its theme's latest-updates path, and persist the observation.
//!
//! One task is one base URL (several catalog sources can share a host, and a
//! host is probed once per run). A source that fails
//! `provider_health_dead_after` runs in a row is marked `DEAD` and hidden
//! from the catalog until one reachable run resets its counter; the probe
//! itself lives in [`stump_provider::health`].
//!
//! The job runs on the scheduler every `provider_health_interval_secs` (see
//! `crate::providers`) and on demand through the `runProviderHealth`
//! mutation. It builds its own catalog reader and HTTP client rather than
//! borrowing the [`stump_provider::ProviderHost`]: jobs only ever get the
//! database and the configuration, and the catalog index is a file both read.

use models::entity::provider_source;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use stump_jobs::{
	JobContext, JobError, JobExecuteLog, JobLifecycle, JobProgress, JobTaskOutput,
	WorkingState,
};
use stump_provider::{
	catalog::SourceCatalog,
	health::{self, HealthChecker, HealthTarget},
};

use crate::job::{output::ProviderSourceHealthOutput, JobServices};

/// Probes the catalog and records `source_health` rows.
pub struct ProviderSourceHealthJob {
	checker: Option<HealthChecker>,
}

impl Default for ProviderSourceHealthJob {
	fn default() -> Self {
		Self::new()
	}
}

impl ProviderSourceHealthJob {
	pub fn new() -> Self {
		Self { checker: None }
	}
}

#[async_trait::async_trait]
impl JobLifecycle for ProviderSourceHealthJob {
	const NAME: &'static str = "provider_source_health";

	type Context = JobServices;
	type Output = ProviderSourceHealthOutput;
	type Task = HealthTarget;

	fn description(&self) -> Option<String> {
		Some("Probe remote provider sources".to_string())
	}

	async fn init(
		&mut self,
		ctx: &JobContext<Self::Context>,
	) -> Result<WorkingState<Self::Output, Self::Task>, JobError> {
		ctx.report_progress(JobProgress::msg("Reading the source catalog"));
		self.checker = Some(
			HealthChecker::new()
				.map_err(|error| JobError::InitFailed(error.to_string()))?,
		);
		let targets = plan_run(ctx.conn(), ctx.config()).await?;
		Ok(WorkingState {
			output: Some(Self::Output::default()),
			tasks: targets.into(),
			logs: vec![],
		})
	}

	async fn execute_task(
		&self,
		ctx: &JobContext<Self::Context>,
		task: Self::Task,
	) -> Result<JobTaskOutput<Self>, JobError> {
		let checker = self
			.checker
			.as_ref()
			.ok_or_else(|| JobError::Unknown("health checker missing".to_string()))?;
		let dead_after = ctx.config().providers.provider_health_dead_after as i32;
		ctx.report_progress(JobProgress::msg(&format!("Probing {}", task.base_url)));

		let summary =
			health::probe_target(ctx.conn(), checker, &task, dead_after).await?;
		let mut logs = Vec::new();
		if summary.dead > 0 {
			logs.push(
				JobExecuteLog::warn(&format!(
					"{} is dead after {dead_after} consecutive failures",
					task.base_url
				))
				.with_ctx(format!("{} source(s)", summary.dead)),
			);
		}

		Ok(JobTaskOutput {
			output: ProviderSourceHealthOutput {
				probed_urls: summary.probed_urls as u32,
				updated_sources: summary.updated_sources as u32,
				ok: summary.ok as u32,
				degraded: summary.degraded as u32,
				dead: summary.dead as u32,
			},
			subtasks: vec![],
			logs,
		})
	}
}

/// The run plan: one [`HealthTarget`] per catalog base URL, enabled source
/// instances first. Shared by [`ProviderSourceHealthJob::init`] and its test;
/// the catalog index is read from `<config>/cache/providers` without touching
/// the network when a snapshot is on disk.
pub async fn plan_run<C: sea_orm::ConnectionTrait>(
	conn: &C,
	config: &crate::config::StumpConfig,
) -> Result<Vec<HealthTarget>, JobError> {
	let client =
		stump_provider::http::build_client(None, stump_provider::http::DEFAULT_TIMEOUT)
			.map_err(|error| JobError::InitFailed(error.to_string()))?;
	let catalog =
		SourceCatalog::new(client, &config.get_cache_dir().join("providers"), None);
	let snapshot = catalog
		.snapshot()
		.await
		.map_err(|error| JobError::InitFailed(error.to_string()))?;

	// Sources an operator actually enabled are probed first, so their health
	// is fresh even if the run is interrupted.
	let priority: Vec<String> = provider_source::Entity::find()
		.select_only()
		.column(provider_source::Column::BaseUrl)
		.filter(provider_source::Column::Enabled.eq(true))
		.into_tuple()
		.all(conn)
		.await?;

	Ok(health::plan(conn, &snapshot, &priority).await?)
}

#[cfg(test)]
mod tests {
	use models::entity::source_health;
	use sea_orm::{ActiveModelTrait, ActiveValue::Set};
	use stump_provider::SourceTheme;

	use super::*;
	use crate::config::StumpConfig;

	/// The plan reads the on-disk catalog snapshot, groups sources by host,
	/// reuses a known theme, and probes enabled instances first.
	#[tokio::test]
	async fn plan_run_groups_hosts_and_prioritises_enabled_sources() {
		let dir = tempfile::tempdir().expect("temp dir");
		let mut config = StumpConfig::debug();
		config.config_dir = dir.path().to_string_lossy().to_string();
		let cache = dir.path().join("cache/providers");
		std::fs::create_dir_all(&cache).expect("cache dir");
		std::fs::write(
			cache.join("keiyoushi-index.json"),
			serde_json::json!([
				{
					"name": "Zeta",
					"pkg": "eu.kanade.tachiyomi.extension.en.zeta",
					"lang": "en",
					"sources": [
						{"id": "1", "name": "Zeta", "lang": "en", "baseUrl": "http://zeta.invalid"},
						{"id": "2", "name": "ZetaMirror", "lang": "en", "baseUrl": "http://zeta.invalid/"}
					]
				},
				{
					"name": "Alpha",
					"pkg": "eu.kanade.tachiyomi.extension.en.alpha",
					"lang": "en",
					"sources": [
						{"id": "3", "name": "Alpha", "lang": "en", "baseUrl": "http://alpha.invalid"}
					]
				}
			])
			.to_string(),
		)
		.expect("write index");

		let db = ::tests::db::test_database().await;
		// An enabled instance on the second host, and a theme an earlier run
		// detected for one of the sources sharing the first host.
		provider_source::ActiveModel {
			id: Set("zeta-en".to_string()),
			implementation: Set("zeta".to_string()),
			name: Set("Zeta".to_string()),
			lang: Set("en".to_string()),
			base_url: Set("http://zeta.invalid".to_string()),
			enabled: Set(true),
			..Default::default()
		}
		.insert(&db)
		.await
		.expect("source insert");
		source_health::ActiveModel {
			source_id: Set("2".to_string()),
			name: Set("ZetaMirror".to_string()),
			lang: Set("en".to_string()),
			base_url: Set("http://zeta.invalid".to_string()),
			theme: Set(Some(SourceTheme::Madara.as_str().to_string())),
			status: Set("OK".to_string()),
			consecutive_failures: Set(0),
			..Default::default()
		}
		.insert(&db)
		.await
		.expect("health insert");

		let targets = plan_run(&db, &config).await.expect("plan");
		assert_eq!(
			targets
				.iter()
				.map(|target| target.base_url.as_str())
				.collect::<Vec<_>>(),
			vec!["http://zeta.invalid", "http://alpha.invalid"],
			"the enabled instance's host is probed first"
		);
		assert_eq!(
			targets[0]
				.sources
				.iter()
				.map(|source| source.id.as_str())
				.collect::<Vec<_>>(),
			vec!["1", "2"],
			"sources sharing a host are probed once, together"
		);
		assert_eq!(targets[0].known_theme, Some(SourceTheme::Madara));
		assert!(targets[1].known_theme.is_none());
	}
}
