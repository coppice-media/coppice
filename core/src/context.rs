use std::{
	sync::{Arc, OnceLock},
	time::Duration,
};

use apalis::{
	layers::WorkerBuilderExt,
	prelude::{MemoryStorage, MessageQueue, Monitor, WorkerBuilder, WorkerFactoryFn},
};
use chrono::Utc;
use models::entity::{job, library, library_config, server_config};
use sea_orm::{prelude::*, DatabaseConnection, MockDatabase, SelectColumns};
use tokio::sync::{
	broadcast::{channel, Receiver, Sender},
	Mutex, Notify,
};

use crate::{
	config::StumpConfig,
	database,
	event::CoreEvent,
	filesystem::scanner::LibraryWatcher,
	ingest::services::IngestServices,
	job::{
		dispatch_job, state::ApalisWorkerState, stump_job::StumpJob, JobScheduler,
		JobStatus,
	},
	CoreError, CoreResult,
};

type EventChannel = (Sender<CoreEvent>, Receiver<CoreEvent>);

/// The lazily-created Apalis queue and worker state for a context.
///
/// Keeping the monitor handle with the runtime makes the first enqueue the
/// lifecycle boundary: a context that never receives background work does not
/// allocate a queue, worker state, or monitor task.
pub struct JobRuntime {
	pub storage: MemoryStorage<StumpJob>,
	pub apalis_state: Arc<ApalisWorkerState>,
	shutdown_notify: Arc<Notify>,
	monitor: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl JobRuntime {
	fn new(
		conn: Arc<DatabaseConnection>,
		config: Arc<StumpConfig>,
		event_tx: Sender<CoreEvent>,
	) -> Self {
		let shutdown_notify = Arc::new(Notify::new());
		let monitor_shutdown_notify = shutdown_notify.clone();
		let storage = MemoryStorage::<StumpJob>::new();
		let apalis_state = Arc::new(ApalisWorkerState::new(
			conn,
			config,
			event_tx,
			storage.clone(),
		));

		let monitor = Monitor::new()
			.register(
				WorkerBuilder::new("stump-worker")
					.enable_tracing()
					.data(apalis_state.clone())
					.concurrency(1)
					.backend(storage.clone())
					.build_fn(dispatch_job),
			)
			.with_terminator(tokio::time::sleep(Duration::from_secs(30)));
		let monitor = tokio::spawn(async move {
			let result = monitor
				.run_with_signal(async move {
					monitor_shutdown_notify.notified().await;
					Ok(())
				})
				.await;
			if let Err(error) = result {
				tracing::error!(?error, "Apalis monitor stopped with an error");
			}
		});

		Self {
			storage,
			apalis_state,
			shutdown_notify,
			monitor: std::sync::Mutex::new(Some(monitor)),
		}
	}

	async fn stop(&self) {
		for entry in self.apalis_state.cancellation_tokens.iter() {
			entry.value().cancel();
		}
		self.shutdown_notify.notify_one();
		let monitor = self
			.monitor
			.lock()
			.expect("job runtime monitor mutex poisoned")
			.take();
		if let Some(monitor) = monitor {
			let _ = monitor.await;
		}
		self.apalis_state.cancellation_tokens.clear();
	}
}

impl Drop for JobRuntime {
	fn drop(&mut self) {
		for entry in self.apalis_state.cancellation_tokens.iter() {
			entry.value().cancel();
		}
		self.shutdown_notify.notify_one();
		if let Some(monitor) = self
			.monitor
			.lock()
			.expect("job runtime monitor mutex poisoned")
			.take()
		{
			monitor.abort();
		}
	}
}

/// Struct that holds the main context for a Stump application. This is passed around
/// to all the different parts of the application, and is used to access the database
/// and manage the event channels.
#[derive(Clone)]
pub struct Ctx {
	pub config: Arc<StumpConfig>,
	pub conn: Arc<DatabaseConnection>,
	pub event_channel: Arc<EventChannel>,
	job_runtime: Arc<OnceLock<Arc<JobRuntime>>>,
	library_watcher: Arc<OnceLock<Arc<LibraryWatcher>>>,
	scheduler: Arc<Mutex<Option<JobScheduler>>>,
	ingest_services: Arc<OnceLock<Arc<IngestServices>>>,
}

impl Ctx {
	/// Creates a new [Ctx] instance. This should only be called once per application.
	/// It takes a sender for the internal event channel, so the core can send events
	/// to the consumer.
	///
	/// ## Example
	/// ```no_run
	/// use stump_core::{Ctx, config::StumpConfig};
	/// use tokio::sync::mpsc::unbounded_channel;
	///
	/// #[tokio::main]
	/// async fn main() {
	///    let config = StumpConfig::debug();
	///    let ctx = Ctx::new(config).await;
	/// }
	/// ```
	pub async fn new(mut config: StumpConfig) -> Ctx {
		config.finalize_media_config();
		let config = Arc::new(config);
		let conn = Arc::new(
			database::connect(&config)
				.await
				.expect("Failed to connect to database"),
		);
		Self::from_parts(config, conn)
	}

	fn from_parts(config: Arc<StumpConfig>, conn: Arc<DatabaseConnection>) -> Ctx {
		Ctx {
			config,
			conn,
			event_channel: Arc::new(channel::<CoreEvent>(1024)),
			job_runtime: Arc::new(OnceLock::new()),
			library_watcher: Arc::new(OnceLock::new()),
			scheduler: Arc::new(Mutex::new(None)),
			ingest_services: Arc::new(OnceLock::new()),
		}
	}

	// TODO(testing): see if i can merge w mock_sea or rm/refactor some bits

	/// Creates a [Ctx] instance for testing **only**
	pub fn for_testing(conn: DatabaseConnection) -> Ctx {
		Self::for_testing_with_config(conn, StumpConfig::debug())
	}

	/// Creates a [Ctx] instance for testing with an explicit configuration.
	pub fn for_testing_with_config(
		conn: DatabaseConnection,
		mut config: StumpConfig,
	) -> Ctx {
		config.finalize_media_config();
		Self::from_parts(Arc::new(config), Arc::new(conn))
	}

	/// Creates a [Ctx] instance for testing **only**
	pub fn mock_sea(mock_db: MockDatabase) -> Ctx {
		Self::for_testing(mock_db.into_connection())
	}

	/// Wraps the [Ctx] in an [Arc], allowing it to be shared across threads. This
	/// is just a simple utility function.
	///
	/// ## Example
	/// ```no_run
	/// use stump_core::{Ctx, config::StumpConfig};
	/// use std::sync::Arc;
	///
	/// #[tokio::main]
	/// async fn main() {
	///     let config = StumpConfig::debug();
	///
	///     let ctx = Ctx::new(config).await;
	///     let arced_ctx = ctx.arced();
	///     let ctx_clone = arced_ctx.clone();
	///
	///     assert_eq!(2, Arc::strong_count(&ctx_clone))
	/// }
	/// ```
	pub fn arced(&self) -> Arc<Ctx> {
		Arc::new(self.clone())
	}

	/// Returns the shared shutdown signal used by lazily-started background work.

	/// Returns whether the Apalis runtime has already been initialized.
	pub fn job_runtime_initialized(&self) -> bool {
		self.job_runtime.get().is_some()
	}

	/// Returns whether the logical library watcher has already been initialized.
	pub fn library_watcher_initialized(&self) -> bool {
		self.library_watcher.get().is_some()
	}

	/// Returns the lazily-created Apalis runtime.
	pub fn job_runtime(&self) -> CoreResult<Arc<JobRuntime>> {
		self.require_background_jobs()?;
		Ok(self
			.job_runtime
			.get_or_init(|| {
				Arc::new(JobRuntime::new(
					self.conn.clone(),
					self.config.clone(),
					self.event_channel.0.clone(),
				))
			})
			.clone())
	}

	/// Reports the current background-job lifecycle state without creating it.
	pub fn jobs_health_status(&self) -> &'static str {
		if !self.background_jobs_enabled() {
			return "disabled";
		}

		let Some(runtime) = self.job_runtime.get() else {
			return "enabled";
		};

		if runtime.apalis_state.cancellation_tokens.is_empty() {
			"idle"
		} else {
			"running"
		}
	}

	/// Reports the current watcher lifecycle state without creating it.
	pub fn watcher_health_status(&self) -> &'static str {
		if !self.background_jobs_enabled() {
			"disabled"
		} else if self.library_watcher.get().is_some() {
			"active"
		} else {
			"inactive"
		}
	}

	/// Cancels jobs left marked as running by a previous server process.
	///
	/// This is deliberately a direct database operation and does not initialize
	/// the lazy Apalis runtime.
	pub async fn cancel_islanded_jobs(&self) -> CoreResult<()> {
		let affected_rows = job::Entity::update_many()
			.filter(job::Column::Status.eq(JobStatus::Running.to_string()))
			.col_expr(
				job::Column::Status,
				Expr::value(JobStatus::Cancelled.to_string()),
			)
			.col_expr(
				job::Column::CompletedAt,
				Expr::value(Some(Utc::now().fixed_offset())),
			)
			.exec(self.conn.as_ref())
			.await?
			.rows_affected;

		tracing::debug!(affected_rows, "Cancelled islanded jobs");
		Ok(())
	}

	/// Cancels a running job without creating the runtime when no job has run.
	pub fn cancel_job(&self, job_id: &str) -> bool {
		self.job_runtime
			.get()
			.is_some_and(|runtime| runtime.apalis_state.cancel_job(job_id))
	}

	fn get_or_init_library_watcher(&self) -> CoreResult<Arc<LibraryWatcher>> {
		self.require_background_jobs()?;
		let runtime = self.job_runtime()?;
		Ok(self
			.library_watcher
			.get_or_init(|| {
				Arc::new(LibraryWatcher::new_with_enabled(
					self.conn.clone(),
					runtime.storage.clone(),
					true,
				))
			})
			.clone())
	}

	/// Initializes the library watcher only when at least one ready library is
	/// configured for watching.
	pub async fn init_library_watcher(&self) -> CoreResult<Option<Arc<LibraryWatcher>>> {
		self.require_background_jobs()?;
		let watched_library_count = library::Entity::find()
			.inner_join(library_config::Entity)
			.filter(library::Column::Status.eq("READY"))
			.filter(library_config::Column::Watch.eq(true))
			.count(self.conn.as_ref())
			.await?;
		if watched_library_count == 0 {
			return Ok(None);
		}

		let watcher = self.get_or_init_library_watcher()?;
		watcher.init().await?;
		Ok(Some(watcher))
	}

	/// Adds a path to the logical library watcher, constructing it on demand.
	pub async fn add_watcher(&self, path: std::path::PathBuf) -> CoreResult<()> {
		self.get_or_init_library_watcher()?.add_watcher(path).await
	}

	/// Removes a path from the logical library watcher when one exists.
	pub async fn remove_watcher(&self, path: std::path::PathBuf) -> CoreResult<()> {
		self.require_background_jobs()?;
		if let Some(watcher) = self.library_watcher.get() {
			watcher.remove_watcher(path).await?;
		}
		Ok(())
	}

	/// Stops the logical library watcher when one exists.
	pub async fn stop_library_watcher(&self) -> CoreResult<()> {
		self.require_background_jobs()?;
		if let Some(watcher) = self.library_watcher.get() {
			watcher.stop().await?;
		}
		Ok(())
	}

	/// Starts or refreshes the scheduler when enabled scheduled-job rows exist.
	pub async fn start_scheduler(&self) -> CoreResult<Option<JobScheduler>> {
		self.require_background_jobs()?;
		let mut scheduler = self.scheduler.lock().await;

		if let Some(existing) = scheduler.clone() {
			if existing.reload(self.arced()).await? {
				return Ok(Some(existing));
			}
			scheduler.take();
			return Ok(None);
		}

		let Some(created) = JobScheduler::init(self.arced()).await? else {
			return Ok(None);
		};
		scheduler.replace(created.clone());
		Ok(Some(created))
	}

	/// Stops the scheduler if it was initialized.
	pub async fn stop_scheduler(&self) {
		if let Some(scheduler) = self.scheduler.lock().await.take() {
			scheduler.stop();
		}
	}

	/// Stops the Apalis monitor if the runtime was initialized.
	pub async fn stop_job_runtime(&self) {
		if let Some(runtime) = self.job_runtime.get() {
			runtime.stop().await;
		}
	}

	/// Returns the staged-ingest services, constructing them lazily on first use.
	pub fn ingest(&self) -> Arc<IngestServices> {
		self.ingest_services
			.get_or_init(|| {
				Arc::new(IngestServices::new(self.config.clone(), self.conn.clone()))
			})
			.clone()
	}
	/// Returns the receiver for the `CoreEvent` channel. See [`emit_event`]
	/// for more information and an example usage.
	pub fn get_client_receiver(&self) -> Receiver<CoreEvent> {
		self.event_channel.0.subscribe()
	}

	pub fn get_event_tx(&self) -> Sender<CoreEvent> {
		self.event_channel.0.clone()
	}

	/// Emits a [`CoreEvent`] to the client event channel.
	pub fn emit_event(&self, event: CoreEvent) {
		let _ = self.event_channel.0.send(event);
	}

	pub fn background_jobs_enabled(&self) -> bool {
		self.config.enable_background_jobs
	}

	pub fn require_background_jobs(&self) -> CoreResult<()> {
		if self.background_jobs_enabled() {
			Ok(())
		} else {
			Err(CoreError::FeatureDisabled("background jobs"))
		}
	}

	/// Enqueue a job into apalis storage, starting the runtime on first use.
	pub async fn enqueue(&self, job: StumpJob) -> CoreResult<()> {
		let runtime = self.job_runtime()?;
		let mut storage = runtime.storage.clone();
		storage
			.enqueue(job)
			.await
			.map_err(|_| CoreError::InternalError("Failed to enqueue job".to_string()))?;
		Ok(())
	}

	/// Send a [`CoreEvent`] through the event channel to any clients listening
	pub fn send_core_event(&self, event: CoreEvent) {
		if let Err(error) = self.event_channel.0.send(event) {
			tracing::error!(error = ?error, "Failed to send core event");
		} else {
			tracing::trace!("Sent core event");
		}
	}

	/// Retrieves the encryption key from the server configuration
	pub async fn get_encryption_key(&self) -> CoreResult<String> {
		let record = server_config::Entity::find()
			.select_column(server_config::Column::EncryptionKey)
			.one(self.conn.as_ref())
			.await?;

		let encryption_key = record
			.and_then(|config| config.encryption_key)
			.ok_or(CoreError::EncryptionKeyNotSet)?;

		Ok(encryption_key)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::filesystem::media::analysis::{AnalysisJobConfig, MediaAnalysisJobScope};
	use migrations::{Migrator, MigratorTrait};
	use models::{entity::job, shared::enums::JobStatus};
	use sea_orm::{Database, EntityTrait, QueryOrder};
	use tokio::time::{sleep, timeout};

	async fn migrated_database() -> DatabaseConnection {
		let db = Database::connect("sqlite::memory:")
			.await
			.expect("failed to connect to sqlite");
		Migrator::up(&db, None)
			.await
			.expect("failed to run migrations");
		db
	}

	#[tokio::test]
	async fn enqueue_initializes_runtime_once_and_monitor_completes_job() {
		let ctx = Ctx::for_testing(migrated_database().await);
		assert!(!ctx.job_runtime_initialized());
		assert_eq!(ctx.jobs_health_status(), "enabled");

		ctx.enqueue(StumpJob::analyze_media(AnalysisJobConfig {
			scope: MediaAnalysisJobScope::Books(Vec::new()),
			force_reanalysis: false,
		}))
		.await
		.expect("failed to enqueue test job");

		assert!(ctx.job_runtime_initialized());
		let runtime = ctx.job_runtime().expect("runtime should be initialized");
		let runtime_again = ctx.job_runtime().expect("runtime should be reusable");
		assert!(Arc::ptr_eq(&runtime, &runtime_again));

		let completed = timeout(Duration::from_secs(10), async {
			loop {
				if let Some(job) = job::Entity::find()
					.order_by_desc(job::Column::CreatedAt)
					.one(ctx.conn.as_ref())
					.await
					.expect("failed to query test job")
				{
					if job.status == JobStatus::Completed {
						break job;
					}
				}
				sleep(Duration::from_millis(10)).await;
			}
		})
		.await
		.expect("test job did not complete");
		assert_eq!(completed.name, "analyze_media");
		assert_eq!(ctx.jobs_health_status(), "idle");
	}

	#[tokio::test]
	async fn stopping_runtime_cancels_running_jobs_and_stops_monitor() {
		let ctx = Ctx::for_testing(migrated_database().await);
		let runtime = ctx.job_runtime().expect("runtime should initialize");
		let token = tokio_util::sync::CancellationToken::new();
		runtime
			.apalis_state
			.cancellation_tokens
			.insert("running-job".to_string(), token.clone());

		ctx.stop_job_runtime().await;

		assert!(token.is_cancelled());
		assert!(runtime
			.monitor
			.lock()
			.expect("job runtime monitor mutex poisoned")
			.is_none());
		assert!(runtime.apalis_state.cancellation_tokens.is_empty());
	}

	#[tokio::test]
	async fn disabled_background_jobs_do_not_create_runtime() {
		let mut config = StumpConfig::debug();
		config.enable_background_jobs = false;
		let ctx = Ctx::for_testing_with_config(migrated_database().await, config);

		assert_eq!(ctx.jobs_health_status(), "disabled");
		assert!(matches!(
			ctx.job_runtime(),
			Err(CoreError::FeatureDisabled("background jobs"))
		));
		assert!(matches!(
			ctx.enqueue(StumpJob::analyze_media(AnalysisJobConfig {
				scope: MediaAnalysisJobScope::Books(Vec::new()),
				force_reanalysis: false,
			}))
			.await,
			Err(CoreError::FeatureDisabled("background jobs"))
		));
		assert!(!ctx.job_runtime_initialized());
	}

	#[tokio::test]
	async fn scheduler_with_no_rows_creates_no_scheduler_or_runtime() {
		let ctx = Ctx::for_testing(migrated_database().await);
		let scheduler = JobScheduler::init(ctx.arced())
			.await
			.expect("scheduler query should succeed");

		assert!(scheduler.is_none());
		assert!(!ctx.job_runtime_initialized());
	}

	#[tokio::test]
	async fn watcher_without_watched_libraries_is_not_constructed() {
		let ctx = Ctx::for_testing(migrated_database().await);
		assert_eq!(ctx.watcher_health_status(), "inactive");

		let watcher = ctx
			.init_library_watcher()
			.await
			.expect("watcher initialization query should succeed");

		assert!(watcher.is_none());
		assert!(!ctx.library_watcher_initialized());
		assert!(!ctx.job_runtime_initialized());
	}
}
