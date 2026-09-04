use std::str::FromStr;
use std::sync::{Arc, Mutex, Weak};

use chrono::Utc;
use croner::Cron;
use models::entity::{library, metadata_fetch_record, scheduled_job};
use models::shared::enums::{MetadataFetchStatus, ScheduledJobKind};
use sea_orm::{prelude::*, EntityTrait, QueryFilter};

use crate::filesystem::metadata::MetadataFetchJobParams;
use crate::job::stump_job::StumpJob;
use crate::{CoreError, CoreResult, Ctx};

/// A scheduler that loads cron-based jobs and spawns them accordingly.
///
/// The inner handle collection is shared so a server-owned handle and the
/// context's refresh hook always refer to the same loops.
#[derive(Clone)]
#[must_use = "dropping the JobScheduler aborts all scheduled job loops"]
pub struct JobScheduler {
	inner: Arc<JobSchedulerInner>,
}

struct JobSchedulerInner {
	handles: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl JobScheduler {
	/// Loads enabled scheduled jobs. No scheduler or task is created when no
	/// valid enabled row exists.
	pub async fn init(ctx: Arc<Ctx>) -> CoreResult<Option<Self>> {
		let handles = Self::load_handles(ctx).await?;
		if handles.is_empty() {
			tracing::info!("No enabled scheduled jobs; scheduler remains idle");
			return Ok(None);
		}

		let scheduler = Self {
			inner: Arc::new(JobSchedulerInner {
				handles: Mutex::new(handles),
			}),
		};
		tracing::info!(job_count = scheduler.job_count(), "Scheduler initialized");
		Ok(Some(scheduler))
	}

	async fn load_handles(ctx: Arc<Ctx>) -> CoreResult<Vec<tokio::task::JoinHandle<()>>> {
		let jobs = scheduled_job::Entity::find()
			.filter(scheduled_job::Column::Enabled.eq(true))
			.all(ctx.conn.as_ref())
			.await?;

		let mut handles = Vec::with_capacity(jobs.len());
		for job in jobs {
			match Cron::from_str(&job.schedule) {
				Ok(cron) => {
					tracing::info!(
						id = job.id,
						name = %job.name,
						kind = ?job.kind,
						schedule = %job.schedule,
						"Starting scheduled job"
					);
					let ctx = Arc::downgrade(&ctx);
					handles.push(tokio::spawn(cron_loop(job, cron, ctx)));
				},
				Err(error) => {
					// TODO: Persisted log for UI to see
					tracing::error!(
						id = job.id,
						name = %job.name,
						schedule = %job.schedule,
						?error,
						"Invalid cron expression, skipping scheduled job"
					);
				},
			}
		}

		Ok(handles)
	}

	/// Replaces the active loops with the currently enabled scheduled-job rows.
	pub async fn reload(&self, ctx: Arc<Ctx>) -> CoreResult<bool> {
		let handles = Self::load_handles(ctx).await?;
		let has_jobs = !handles.is_empty();
		let mut current = self.inner.handles.lock().expect("scheduler mutex poisoned");
		for handle in current.drain(..) {
			handle.abort();
		}
		*current = handles;
		tracing::info!(job_count = current.len(), "Scheduler reloaded");
		Ok(has_jobs)
	}

	pub fn job_count(&self) -> usize {
		self.inner
			.handles
			.lock()
			.expect("scheduler mutex poisoned")
			.len()
	}

	/// Aborts all scheduled loops immediately.
	pub fn stop(&self) {
		let mut handles = self.inner.handles.lock().expect("scheduler mutex poisoned");
		for handle in handles.drain(..) {
			handle.abort();
		}
	}
}

impl Drop for JobSchedulerInner {
	fn drop(&mut self) {
		for handle in self
			.handles
			.get_mut()
			.expect("scheduler mutex poisoned")
			.drain(..)
		{
			handle.abort();
		}
	}
}

/// The main loop for a single scheduled job based on its cron expression
#[tracing::instrument(fields(job_id = %job.id, job_name = %job.name), skip(ctx))]
async fn cron_loop(job: scheduled_job::Model, cron: Cron, ctx: Weak<Ctx>) {
	loop {
		let now = Utc::now();
		let next = match cron.find_next_occurrence(&now, false) {
			Ok(t) => t,
			Err(e) => {
				tracing::warn!(?e, "No upcoming fire time for cron schedule, stopping");
				return;
			},
		};

		let duration = (next - now).to_std().unwrap_or_default();
		tracing::debug!(
			next = %next,
			secs_until = duration.as_secs(),
			"Sleeping until next fire"
		);

		tokio::time::sleep(duration).await;

		let Some(ctx) = ctx.upgrade() else {
			return;
		};
		tracing::info!("Firing scheduled job");

		if let Err(error) = dispatch(&job, &ctx).await {
			tracing::error!(
				id = job.id,
				name = %job.name,
				?error,
				"Scheduled job dispatch failed"
			);
		}

		if let Err(error) = scheduled_job::Entity::update_many()
			.col_expr(
				scheduled_job::Column::LastRunAt,
				sea_orm::sea_query::Expr::value(Utc::now()),
			)
			.filter(scheduled_job::Column::Id.eq(job.id))
			.exec(ctx.conn.as_ref())
			.await
		{
			tracing::error!(
				id = job.id,
				name = %job.name,
				?error,
				"Failed to update last_run_at"
			);
		}
	}
}

/// Dispatch a scheduled job based on its kind
async fn dispatch(job: &scheduled_job::Model, ctx: &Ctx) -> CoreResult<()> {
	match job.kind {
		ScheduledJobKind::LibraryScan => dispatch_library_scan(job, ctx).await,
		ScheduledJobKind::MetadataRetry => dispatch_metadata_retry(job, ctx).await,
	}
}

async fn dispatch_library_scan(job: &scheduled_job::Model, ctx: &Ctx) -> CoreResult<()> {
	let config = job.library_scan_config().ok_or(CoreError::InternalError(
		"Invalid scheduled scan config".to_string(),
	))?;

	let libraries = if config.library_ids.is_empty() {
		library::Entity::find().all(ctx.conn.as_ref()).await?
	} else {
		library::Entity::find()
			.filter(library::Column::Id.is_in(config.library_ids.clone()))
			.all(ctx.conn.as_ref())
			.await?
	};

	if libraries.is_empty() {
		tracing::warn!("No libraries found for scheduled scan");
		return Ok(());
	}

	for lib in libraries {
		tracing::info!(
			library_name = %lib.name,
			"Enqueuing library scan from scheduler"
		);
		ctx.enqueue(StumpJob::library_scan(
			lib.id.clone(),
			lib.path.clone(),
			None,
		))
		.await
		.map_err(|e| CoreError::InternalError(e.to_string()))?;
	}

	Ok(())
}

async fn dispatch_metadata_retry(
	job: &scheduled_job::Model,
	ctx: &Ctx,
) -> CoreResult<()> {
	let config = job.metadata_retry_config();

	let statuses = config
		.as_ref()
		.map(|c| c.statuses.clone())
		.unwrap_or_else(|| vec![MetadataFetchStatus::RateLimited]);

	let records = metadata_fetch_record::Entity::find()
		.filter(metadata_fetch_record::Column::Status.is_in(statuses))
		.all(ctx.conn.as_ref())
		.await?;

	if records.is_empty() {
		tracing::debug!(
			id = job.id,
			name = %job.name,
			"No records to retry"
		);
		return Ok(());
	}

	let series_ids: Vec<String> =
		records.iter().filter_map(|r| r.series_id.clone()).collect();
	let media_ids: Vec<String> =
		records.iter().filter_map(|r| r.media_id.clone()).collect();

	if !series_ids.is_empty() {
		tracing::info!(
			count = series_ids.len(),
			"Enqueuing metadata retry for series"
		);
		let params = MetadataFetchJobParams::series(series_ids);
		ctx.enqueue(StumpJob::metadata_fetch(params))
			.await
			.map_err(|e| CoreError::InternalError(e.to_string()))?;
	}

	if !media_ids.is_empty() {
		tracing::info!(
			count = media_ids.len(),
			"Enqueuing metadata retry for media"
		);
		let params = MetadataFetchJobParams::media(media_ids);
		ctx.enqueue(StumpJob::metadata_fetch(params))
			.await
			.map_err(|e| CoreError::InternalError(e.to_string()))?;
	}

	Ok(())
}
