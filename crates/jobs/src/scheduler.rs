use std::{
	str::FromStr,
	sync::{Arc, Mutex},
	time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
use croner::Cron;
use models::entity::scheduled_job;
use sea_orm::{prelude::*, DatabaseConnection, EntityTrait, QueryFilter};

use crate::{JobError, JobExecutionContext, JobRuntime};

/// Turns a persisted scheduled-job row into queued work when its cron fires.
#[async_trait]
pub trait ScheduledJobDispatcher: JobExecutionContext {
	async fn dispatch_scheduled(
		&self,
		job: &scheduled_job::Model,
		runtime: &JobRuntime<Self>,
	) -> Result<(), JobError>;

	/// Performs one host-owned durable maintenance scan. The default is a
	/// no-op so existing scheduler hosts need not opt into a scan.
	async fn dispatch_due(&self, _runtime: &JobRuntime<Self>) -> Result<(), JobError> {
		Ok(())
	}
}

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
	maintenance: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl JobScheduler {
	/// Loads enabled scheduled jobs. No scheduler, task, or runtime is created when no
	/// valid enabled row exists; `runtime` is only called once a loop needs it.
	pub async fn init<X, F>(
		conn: &DatabaseConnection,
		runtime: F,
	) -> Result<Option<Self>, JobError>
	where
		X: ScheduledJobDispatcher,
		F: FnOnce() -> Result<Arc<JobRuntime<X>>, JobError>,
	{
		Self::init_with_maintenance(conn, runtime, false).await
	}

	/// Loads cron rows and, when requested, starts one durable maintenance scan
	/// alongside them. The scan is host-defined through [`dispatch_due`] and is
	/// useful for persisted work whose next attempt is time-based rather than a
	/// user-authored cron expression.
	pub async fn init_with_maintenance<X, F>(
		conn: &DatabaseConnection,
		runtime: F,
		with_maintenance: bool,
	) -> Result<Option<Self>, JobError>
	where
		X: ScheduledJobDispatcher,
		F: FnOnce() -> Result<Arc<JobRuntime<X>>, JobError>,
	{
		let (handles, maintenance) =
			load_handles(conn, runtime, with_maintenance).await?;
		if handles.is_empty() && maintenance.is_none() {
			tracing::info!("No enabled scheduled jobs; scheduler remains idle");
			return Ok(None);
		}

		let scheduler = Self {
			inner: Arc::new(JobSchedulerInner {
				handles: Mutex::new(handles),
				maintenance: Mutex::new(maintenance),
			}),
		};
		tracing::info!(job_count = scheduler.job_count(), "Scheduler initialized");
		Ok(Some(scheduler))
	}

	/// Replaces the active loops with the currently enabled scheduled-job rows.
	pub async fn reload<X, F>(
		&self,
		conn: &DatabaseConnection,
		runtime: F,
	) -> Result<bool, JobError>
	where
		X: ScheduledJobDispatcher,
		F: FnOnce() -> Result<Arc<JobRuntime<X>>, JobError>,
	{
		let with_maintenance = self
			.inner
			.maintenance
			.lock()
			.expect("scheduler mutex poisoned")
			.is_some();
		self.reload_with_maintenance(conn, runtime, with_maintenance)
			.await
	}

	/// Replaces cron loops and explicitly enables or disables the host-owned
	/// maintenance scan.
	pub async fn reload_with_maintenance<X, F>(
		&self,
		conn: &DatabaseConnection,
		runtime: F,
		with_maintenance: bool,
	) -> Result<bool, JobError>
	where
		X: ScheduledJobDispatcher,
		F: FnOnce() -> Result<Arc<JobRuntime<X>>, JobError>,
	{
		let (handles, maintenance) =
			load_handles(conn, runtime, with_maintenance).await?;
		let has_jobs = !handles.is_empty() || maintenance.is_some();
		let mut current = self.inner.handles.lock().expect("scheduler mutex poisoned");
		for handle in current.drain(..) {
			handle.abort();
		}
		*current = handles;
		let mut current_maintenance = self
			.inner
			.maintenance
			.lock()
			.expect("scheduler mutex poisoned");
		if let Some(handle) = current_maintenance.take() {
			handle.abort();
		}
		*current_maintenance = maintenance;
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

	pub fn maintenance_enabled(&self) -> bool {
		self.inner
			.maintenance
			.lock()
			.expect("scheduler mutex poisoned")
			.is_some()
	}

	/// Aborts all scheduled loops immediately.
	pub fn stop(&self) {
		let mut handles = self.inner.handles.lock().expect("scheduler mutex poisoned");
		for handle in handles.drain(..) {
			handle.abort();
		}
		if let Some(handle) = self
			.inner
			.maintenance
			.lock()
			.expect("scheduler mutex poisoned")
			.take()
		{
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
		if let Some(handle) = self
			.maintenance
			.get_mut()
			.expect("scheduler mutex poisoned")
			.take()
		{
			handle.abort();
		}
	}
}
async fn load_handles<X, F>(
	conn: &DatabaseConnection,
	runtime: F,
	with_maintenance: bool,
) -> Result<
	(
		Vec<tokio::task::JoinHandle<()>>,
		Option<tokio::task::JoinHandle<()>>,
	),
	JobError,
>
where
	X: ScheduledJobDispatcher,
	F: FnOnce() -> Result<Arc<JobRuntime<X>>, JobError>,
{
	let jobs = scheduled_job::Entity::find()
		.filter(scheduled_job::Column::Enabled.eq(true))
		.all(conn)
		.await?;

	let mut schedules = Vec::with_capacity(jobs.len());
	for job in jobs {
		match Cron::from_str(&job.schedule) {
			Ok(cron) => schedules.push((job, cron)),
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
	if schedules.is_empty() && !with_maintenance {
		return Ok((Vec::new(), None));
	}

	let runtime = runtime()?;
	let handles = schedules
		.into_iter()
		.map(|(job, cron)| {
			tracing::info!(
				id = job.id,
				name = %job.name,
				kind = ?job.kind,
				schedule = %job.schedule,
				"Starting scheduled job"
			);
			tokio::spawn(cron_loop(job, cron, Arc::clone(&runtime)))
		})
		.collect();
	let maintenance =
		with_maintenance.then(|| tokio::spawn(maintenance_loop(Arc::clone(&runtime))));

	Ok((handles, maintenance))
}

const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(15);

/// Periodically asks the host to enqueue durable work whose persisted
/// next-attempt timestamp has arrived. This loop never performs the work
/// itself, so it cannot consume an executor permit while waiting.
async fn maintenance_loop<X: ScheduledJobDispatcher>(runtime: Arc<JobRuntime<X>>) {
	let mut interval = tokio::time::interval(MAINTENANCE_INTERVAL);
	loop {
		interval.tick().await;
		if let Err(error) = runtime.services().dispatch_due(&runtime).await {
			tracing::error!(?error, "Scheduled maintenance dispatch failed");
		}
	}
}

/// The main loop for a single scheduled job based on its cron expression
#[tracing::instrument(fields(job_id = %job.id, job_name = %job.name), skip(runtime))]
async fn cron_loop<X: ScheduledJobDispatcher>(
	job: scheduled_job::Model,
	cron: Cron,
	runtime: Arc<JobRuntime<X>>,
) {
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

		tracing::info!("Firing scheduled job");

		if let Err(error) = runtime.services().dispatch_scheduled(&job, &runtime).await {
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
			.exec(runtime.services().conn())
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
