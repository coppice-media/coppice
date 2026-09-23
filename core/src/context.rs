use std::sync::{Arc, OnceLock};

use chrono::Utc;
use models::{entity::job, shared::enums::JobStatus};
use sea_orm::{prelude::*, DatabaseConnection, MockDatabase};
use stump_devices::DeviceService;
use stump_jobs::{JobError, JobRuntime, JobScheduler};
#[cfg(feature = "watcher")]
use stump_watcher::{Watcher, DEFAULT_DEBOUNCE};
use stump_worker::{KindRegistry, SourceHub, WorkerJobs};
use tokio::sync::{
	broadcast::{channel, Receiver, Sender},
	Mutex,
};

#[cfg(feature = "watcher")]
use crate::filesystem::scanner::watcher_adapters::{
	watched_libraries_query, EnqueueLibraryScan, WatchedLibraryRoots,
};
#[cfg(feature = "ingest")]
use stump_ingest::IngestServices;

#[cfg(feature = "ingest")]
use crate::component_runtime::COMPONENT_INGEST;
#[cfg(feature = "watcher")]
use crate::component_runtime::COMPONENT_WATCHER;
#[cfg(feature = "ingest")]
use crate::ingest_host::{
	ingest_settings, CoreEventSink, CoreProviderClients, CoreRowFactory,
};
use crate::{
	component_runtime::{
		ComponentDefinition, ComponentRuntime, ProcessMemory, COMPONENT_BACKGROUND_JOBS,
		COMPONENT_SCHEDULER, COMPONENT_WORKER,
	},
	config::StumpConfig,
	database,
	event::CoreEvent,
	filesystem::media::visible_pages::VisiblePagesCache,
	job::{stump_job::StumpJob, JobServices},
	reading_state::ReadingHeadChanged,
	utils::encryption::fetch_encryption_key,
	CoreError, CoreResult,
};

type EventChannel = (Sender<CoreEvent>, Receiver<CoreEvent>);

/// Struct that holds the main context for a Stump application. This is passed around
/// to all the different parts of the application, and is used to access the database
/// and manage the event channels.
///
/// The job runtime, library watcher, and scheduler are created lazily: a context that
/// never receives background work allocates none of them.
#[derive(Clone)]
pub struct Ctx {
	pub config: Arc<StumpConfig>,
	pub conn: Arc<DatabaseConnection>,
	pub event_channel: Arc<EventChannel>,
	component_runtime: Arc<ComponentRuntime>,
	job_runtime: Arc<OnceLock<Arc<JobRuntime<JobServices>>>>,
	#[cfg(feature = "watcher")]
	library_watcher: Arc<OnceLock<Arc<Watcher>>>,
	scheduler: Arc<Mutex<Option<JobScheduler>>>,
	#[cfg(feature = "ingest")]
	ingest_services: Arc<OnceLock<Arc<IngestServices>>>,
	devices: Arc<OnceLock<Arc<DeviceService>>>,
	#[cfg(feature = "crosspoint")]
	crosspoint_delivery:
		Arc<OnceLock<Arc<crate::crosspoint_delivery::CrossPointDeliveryService>>>,
	/// Accepted head changes from every protocol, for adapters that fan them
	/// out to their own clients (Komga SSE). Not part of [`CoreEvent`], which
	/// is streamed to every GraphQL subscriber regardless of user.
	reading_state_events: Arc<Sender<ReadingHeadChanged>>,
	/// Per-media visible-page lists (duplicate-page skipping); shared with jobs.
	visible_pages: Arc<VisiblePagesCache>,
	/// Debounce deadlines for per-user annotation export runs; see
	/// [`crate::annotation_sync::AnnotationSyncDebouncer`].
	pub(crate) annotation_debounce: Arc<crate::annotation_sync::AnnotationSyncDebouncer>,
	/// The remote provider host (`stump_provider`), created by
	/// [`crate::providers::init`] when `STUMP_ENABLE_PROVIDERS` is on.
	#[cfg(feature = "providers")]
	provider_host: Arc<OnceLock<Arc<stump_provider::ProviderHost>>>,
	/// The remote-worker queue and its connected-worker hub
	/// ([`stump_worker`]). Created lazily like the device registry; the job
	/// kinds' local implementations are installed by the host at boot with
	/// [`Ctx::install_worker_registry`], because running one needs `ffmpeg`
	/// and the media rows, neither of which the core owns.
	worker_jobs: Arc<OnceLock<Arc<WorkerJobs>>>,
	worker_registry: Arc<OnceLock<KindRegistry>>,
	/// Connected source-worker control sockets and their bounded read grants.
	source_hub: Arc<OnceLock<Arc<SourceHub>>>,
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
		config.finalize();
		let config = Arc::new(config);
		let conn = Arc::new(
			database::connect(&config)
				.await
				.expect("Failed to connect to database"),
		);
		let ctx = Self::from_parts(config, conn);
		if let Err(error) = ctx.component_runtime.initialize().await {
			tracing::warn!(?error, "Failed to persist runtime component state");
		}
		ctx
	}

	fn from_parts(config: Arc<StumpConfig>, conn: Arc<DatabaseConnection>) -> Ctx {
		let annotation_debounce =
			Arc::new(crate::annotation_sync::AnnotationSyncDebouncer::new(
				config.annotation_sync.annotation_sync_debounce_secs,
			));
		let component_runtime = Arc::new(ComponentRuntime::new(conn.clone(), &config));
		Ctx {
			config,
			conn,
			event_channel: Arc::new(channel::<CoreEvent>(1024)),
			component_runtime,
			job_runtime: Arc::new(OnceLock::new()),
			#[cfg(feature = "watcher")]
			library_watcher: Arc::new(OnceLock::new()),
			scheduler: Arc::new(Mutex::new(None)),
			#[cfg(feature = "ingest")]
			ingest_services: Arc::new(OnceLock::new()),
			devices: Arc::new(OnceLock::new()),
			#[cfg(feature = "crosspoint")]
			crosspoint_delivery: Arc::new(OnceLock::new()),
			annotation_debounce,
			reading_state_events: Arc::new(channel::<ReadingHeadChanged>(256).0),
			visible_pages: Arc::new(VisiblePagesCache::default()),
			#[cfg(feature = "providers")]
			provider_host: Arc::new(OnceLock::new()),
			worker_jobs: Arc::new(OnceLock::new()),
			worker_registry: Arc::new(OnceLock::new()),
			source_hub: Arc::new(OnceLock::new()),
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
		config.finalize();
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
	///     let arced_ctx: Arc<Ctx> = ctx.arced();
	///     let ctx_clone = arced_ctx.clone();
	/// }
	/// ```
	pub fn arced(&self) -> Arc<Ctx> {
		Arc::new(self.clone())
	}

	/// Returns whether the job runtime has already been initialized.
	pub fn job_runtime_initialized(&self) -> bool {
		self.job_runtime.get().is_some()
	}

	/// Accesses the live component registry used by admin controls and HTTP
	/// route guards.
	pub fn component_runtime(&self) -> Arc<ComponentRuntime> {
		self.component_runtime.clone()
	}

	/// Registers descriptors owned by the server host (protocol routes and
	/// Cargo-feature integrations).
	pub async fn register_components(
		&self,
		definitions: impl IntoIterator<Item = ComponentDefinition>,
	) -> CoreResult<()> {
		self.component_runtime.register(definitions).await
	}

	/// Returns whether a mounted route is currently available. This is
	/// intentionally synchronous so request guards never perform database I/O.
	pub fn component_enabled(&self, key: &str) -> bool {
		self.component_runtime.is_effective(key)
	}

	/// Reads process-wide RSS and, on Linux, a PSS/private-dirty breakdown
	/// without assigning any memory to a component.
	pub fn process_memory(&self) -> ProcessMemory {
		ProcessMemory::read()
	}

	/// The shared per-media visible-page cache (duplicate-page skipping),
	/// used by every page-serving route and invalidated by jobs and marks.
	pub fn visible_pages_cache(&self) -> Arc<VisiblePagesCache> {
		self.visible_pages.clone()
	}

	/// Returns the durable CrossPoint delivery coordinator, creating it lazily
	/// with the transform cache as its staging root.
	#[cfg(feature = "crosspoint")]
	pub fn crosspoint_delivery(
		&self,
	) -> CoreResult<Arc<crate::crosspoint_delivery::CrossPointDeliveryService>> {
		if let Some(service) = self.crosspoint_delivery.get() {
			return Ok(service.clone());
		}
		let service = Arc::new(
			crate::crosspoint_delivery::CrossPointDeliveryService::new(
				self.conn.clone(),
				self.config.get_transform_cache_dir(),
				crate::crosspoint_delivery::DeliveryConfig::default(),
			)
			.map_err(|error| CoreError::InitializationError(error.to_string()))?,
		);
		if self.crosspoint_delivery.set(service.clone()).is_err() {
			return Ok(self
				.crosspoint_delivery
				.get()
				.expect("CrossPoint service set concurrently")
				.clone());
		}
		Ok(service)
	}

	/// Returns the lazily-created job runtime, starting its executor on first use.
	pub fn job_runtime(&self) -> CoreResult<Arc<JobRuntime<JobServices>>> {
		self.require_background_jobs()?;
		Ok(self
			.job_runtime
			.get_or_init(|| {
				let services = JobServices::new(
					self.conn.clone(),
					self.config.clone(),
					self.event_channel.0.clone(),
					self.visible_pages.clone(),
				);
				// The same cell `provider_host()` reads, so the health job
				// sees the host as soon as `providers::init` installs it —
				// which is after the runtime may already have been built.
				#[cfg(feature = "providers")]
				let services = services.with_provider_host(self.provider_host.clone());
				Arc::new(JobRuntime::new(Arc::new(services)))
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

		if runtime.is_idle() {
			"idle"
		} else {
			"running"
		}
	}

	/// Cancels jobs left marked as running by a previous server process.
	///
	/// This is deliberately a direct database operation and does not initialize
	/// the lazy job runtime.
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
			.is_some_and(|runtime| runtime.cancel_job(job_id))
	}
}

#[cfg(feature = "watcher")]
impl Ctx {
	/// Returns whether the logical library watcher has already been initialized.
	pub fn library_watcher_initialized(&self) -> bool {
		self.library_watcher.get().is_some()
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

	fn get_or_init_library_watcher(&self) -> CoreResult<Arc<Watcher>> {
		self.require_background_jobs()?;
		let runtime = self.job_runtime()?;
		Ok(self
			.library_watcher
			.get_or_init(|| {
				Arc::new(Watcher::new(
					WatchedLibraryRoots {
						conn: self.conn.clone(),
					},
					EnqueueLibraryScan { runtime },
					DEFAULT_DEBOUNCE,
				))
			})
			.clone())
	}

	/// Initializes the library watcher only when at least one ready library is
	/// configured for watching. Returns whether a watcher was started.
	pub async fn init_library_watcher(&self) -> CoreResult<bool> {
		self.require_background_jobs()?;
		let watched_library_count =
			watched_libraries_query().count(self.conn.as_ref()).await?;
		if watched_library_count == 0 {
			return Ok(false);
		}

		let watcher = self.get_or_init_library_watcher()?;
		watcher.init().await?;
		self.component_runtime.record_activity(COMPONENT_WATCHER);
		Ok(true)
	}

	/// Adds a path to the logical library watcher, constructing it on demand.
	pub async fn add_watcher(&self, path: std::path::PathBuf) -> CoreResult<()> {
		self.get_or_init_library_watcher()?
			.add_watcher(path)
			.await?;
		Ok(())
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
}

/// Without the `watcher` feature no filesystem watcher is linked: library `watch`
/// settings are persisted but never acted on, and health reports `unavailable`.
#[cfg(not(feature = "watcher"))]
impl Ctx {
	pub fn library_watcher_initialized(&self) -> bool {
		false
	}

	pub fn watcher_health_status(&self) -> &'static str {
		"unavailable"
	}

	pub async fn init_library_watcher(&self) -> CoreResult<bool> {
		self.require_background_jobs()?;
		Ok(false)
	}

	pub async fn add_watcher(&self, path: std::path::PathBuf) -> CoreResult<()> {
		self.require_background_jobs()?;
		tracing::debug!(path = %path.display(), "Compiled without the watcher feature; not watching path");
		Ok(())
	}

	pub async fn remove_watcher(&self, _path: std::path::PathBuf) -> CoreResult<()> {
		self.require_background_jobs()
	}

	pub async fn stop_library_watcher(&self) -> CoreResult<()> {
		self.require_background_jobs()
	}
}

impl Ctx {
	/// Starts or refreshes the scheduler when enabled scheduled-job rows exist.
	///
	/// The job runtime is only created once a valid scheduled row needs it.
	pub async fn start_scheduler(&self) -> CoreResult<Option<JobScheduler>> {
		self.require_background_jobs()?;
		if !self.component_enabled(crate::component_runtime::COMPONENT_SCHEDULER) {
			return Ok(None);
		}
		let mut scheduler = self.scheduler.lock().await;
		let runtime = || self.job_runtime().map_err(JobError::from);

		if let Some(existing) = scheduler.clone() {
			if existing.reload(self.conn.as_ref(), runtime).await? {
				self.component_runtime.record_activity(COMPONENT_SCHEDULER);
				return Ok(Some(existing));
			}
			scheduler.take();
			return Ok(None);
		}

		let Some(created) = JobScheduler::init(self.conn.as_ref(), runtime).await? else {
			return Ok(None);
		};
		scheduler.replace(created.clone());
		self.component_runtime.record_activity(COMPONENT_SCHEDULER);
		Ok(Some(created))
	}

	/// Stops the scheduler if it was initialized.
	pub async fn stop_scheduler(&self) {
		if let Some(scheduler) = self.scheduler.lock().await.take() {
			scheduler.stop();
		}
	}

	/// Stops the job executor if the runtime was initialized.
	pub async fn stop_job_runtime(&self) {
		if let Some(runtime) = self.job_runtime.get() {
			runtime.stop().await;
		}
	}

	/// Returns the staged-ingest services, constructing them lazily on first
	#[cfg(feature = "ingest")]
	pub fn ingest(&self) -> Arc<IngestServices> {
		let services = self
			.ingest_services
			.get_or_init(|| {
				Arc::new(
					IngestServices::new(
						Arc::new(ingest_settings(&self.config)),
						self.conn.clone(),
						Arc::new(CoreRowFactory::new(self.config.clone())),
						Arc::new(CoreProviderClients::new(self.conn.clone())),
					)
					.with_event_sink(Arc::new(CoreEventSink::new(self.get_event_tx()))),
				)
			})
			.clone();
		self.component_runtime.record_activity(COMPONENT_INGEST);
		services
	}

	/// Returns the device registry, constructing it lazily on first use. Device
	/// sightings recorded through it are forwarded as [`CoreEvent::DeviceSeen`].
	pub fn devices(&self) -> Arc<DeviceService> {
		self.devices
			.get_or_init(|| {
				let event_tx = self.event_channel.0.clone();
				Arc::new(DeviceService::new(self.conn.clone()).with_seen_listener(
					move |seen| {
						let _ = event_tx.send(CoreEvent::DeviceSeen(seen));
					},
				))
			})
			.clone()
	}

	/// Install the job kinds' local implementations. Called once by the host
	/// during startup, before any route that can enqueue is mounted.
	///
	/// A registry installed after the service was first used is refused rather
	/// than silently ignored: a `transcode` that silently lost its `ffmpeg`
	/// fallback would answer `needs_worker` on a server that can do the work.
	pub fn install_worker_registry(&self, registry: KindRegistry) {
		if self.worker_registry.get().is_some() {
			return;
		}
		if self.worker_jobs.get().is_some() {
			tracing::error!(
				"The worker job registry was installed after the queue was first used; the local implementations it carries will not be reachable"
			);
			return;
		}
		let _ = self.worker_registry.set(registry);
	}

	/// The remote-worker queue, constructed lazily on first use. Transitions
	/// recorded through it are forwarded as [`CoreEvent::WorkerJobChanged`].
	pub fn worker_jobs(&self) -> Arc<WorkerJobs> {
		let service = self
			.worker_jobs
			.get_or_init(|| {
				let event_tx = self.event_channel.0.clone();
				let service = WorkerJobs::new(
					self.conn.clone(),
					// A subdirectory of the transform cache: the outputs are
					// the same kind of artifact and share its disk, but the
					// cache's LRU sweep enumerates only its own directory, so
					// an in-flight upload can be neither served nor evicted.
					self.config.get_transform_cache_dir().join("worker-output"),
				)
				.with_registry(self.worker_registry.get().cloned().unwrap_or_default())
				.with_change_listener(Arc::new(move |changed| {
					let _ = event_tx.send(CoreEvent::WorkerJobChanged(changed));
				}));
				let service = Arc::new(service);
				service.start_claim_sweep();
				service
			})
			.clone();
		self.component_runtime.record_activity(COMPONENT_WORKER);
		service
	}

	/// Connected source-worker control sockets and bounded transfer grants.
	/// This registry is independent from the compute-worker queue and is
	/// constructed only when a source route or remote location is used.
	pub fn source_hub(&self) -> Arc<SourceHub> {
		self.source_hub
			.get_or_init(|| Arc::new(SourceHub::new()))
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
		self.config.jobs.enable_background_jobs
	}

	pub fn require_background_jobs(&self) -> CoreResult<()> {
		if self.background_jobs_enabled() {
			Ok(())
		} else {
			Err(CoreError::FeatureDisabled("background jobs"))
		}
	}

	/// Enqueue a job, starting the runtime on first use.
	pub async fn enqueue(&self, job: StumpJob) -> CoreResult<()> {
		let runtime = self.job_runtime()?;
		runtime
			.enqueue(job)
			.await
			.map_err(|error| CoreError::InternalError(error.to_string()))?;
		self.component_runtime
			.record_activity(COMPONENT_BACKGROUND_JOBS);
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

	/// Subscribe to accepted reading-head changes from every protocol.
	pub fn reading_state_events(&self) -> Receiver<ReadingHeadChanged> {
		self.reading_state_events.subscribe()
	}

	/// Publish an accepted reading-head change to protocol adapters.
	pub fn emit_reading_head_changed(&self, event: ReadingHeadChanged) {
		// A send only fails when nobody is subscribed, which is not an error.
		let _ = self.reading_state_events.send(event);
	}

	/// Records annotation/reading-head activity for `user_id`, (re)arming the
	/// debounced annotation export. Callers: annotation mutations, bookmark
	/// mutations, and the reading-head announce paths.
	pub fn note_annotation_activity(&self, user_id: &str) {
		self.annotation_debounce.note(user_id);
	}

	/// Whether a debounced annotation export is pending for `user_id`.
	pub fn annotation_sync_pending(&self, user_id: &str) -> bool {
		self.annotation_debounce.is_pending(user_id)
	}

	/// The provider host, when it has been initialized
	/// ([`crate::providers::init`] with providers enabled).
	#[cfg(feature = "providers")]
	pub fn provider_host(&self) -> Option<Arc<stump_provider::ProviderHost>> {
		self.provider_host.get().cloned()
	}

	/// Store the provider host; fails if one is already installed.
	#[cfg(feature = "providers")]
	pub(crate) fn set_provider_host(
		&self,
		host: Arc<stump_provider::ProviderHost>,
	) -> Result<(), Arc<stump_provider::ProviderHost>> {
		self.provider_host.set(host)
	}

	/// Retrieves the encryption key from the server configuration
	pub async fn get_encryption_key(&self) -> CoreResult<String> {
		fetch_encryption_key(self.conn.as_ref()).await
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::filesystem::media::analysis::{AnalysisJobConfig, MediaAnalysisJobScope};
	use migrations::{Migrator, MigratorTrait};
	use models::{entity::job, shared::enums::JobStatus};
	use sea_orm::{Database, EntityTrait, QueryOrder};
	use std::time::Duration;
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
	async fn enqueue_initializes_runtime_once_and_executor_completes_job() {
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
		assert!(completed.output_data.is_some());
		assert_eq!(ctx.jobs_health_status(), "idle");
		assert_eq!(runtime.queue_depth().count, 0);
	}

	#[tokio::test]
	async fn stopping_runtime_cancels_running_jobs_and_stops_executor() {
		let ctx = Ctx::for_testing(migrated_database().await);
		let runtime = ctx.job_runtime().expect("runtime should initialize");
		let job = StumpJob::analyze_media(AnalysisJobConfig {
			scope: MediaAnalysisJobScope::Books(Vec::new()),
			force_reanalysis: false,
		});
		let job_ctx = runtime
			.open_job("running-job".to_string(), &job)
			.await
			.expect("job context");
		assert_eq!(ctx.jobs_health_status(), "running");

		ctx.stop_job_runtime().await;

		assert!(job_ctx.is_canceled());
		assert_eq!(ctx.jobs_health_status(), "idle");
		assert!(
			matches!(
				runtime
					.enqueue(StumpJob::analyze_media(AnalysisJobConfig {
						scope: MediaAnalysisJobScope::Books(Vec::new()),
						force_reanalysis: false,
					}))
					.await,
				Err(JobError::Unknown(_))
			) || runtime.backend() == "apalis"
		);
	}

	#[tokio::test]
	async fn disabled_background_jobs_do_not_create_runtime() {
		let mut config = StumpConfig::debug();
		config.jobs.enable_background_jobs = false;
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
		let scheduler = ctx
			.start_scheduler()
			.await
			.expect("scheduler query should succeed");

		assert!(scheduler.is_none());
		assert!(!ctx.job_runtime_initialized());
	}

	#[tokio::test]
	async fn watcher_without_watched_libraries_is_not_constructed() {
		let ctx = Ctx::for_testing(migrated_database().await);
		let expected = if cfg!(feature = "watcher") {
			"inactive"
		} else {
			"unavailable"
		};
		assert_eq!(ctx.watcher_health_status(), expected);

		let started = ctx
			.init_library_watcher()
			.await
			.expect("watcher initialization query should succeed");

		assert!(!started);
		assert!(!ctx.library_watcher_initialized());
		assert!(!ctx.job_runtime_initialized());
	}

	#[tokio::test]
	async fn disabled_background_jobs_reject_watcher_operations() {
		let mut config = StumpConfig::debug();
		config.jobs.enable_background_jobs = false;
		let ctx = Ctx::for_testing_with_config(migrated_database().await, config);
		let path = std::path::PathBuf::from("/tmp/stump");

		assert!(matches!(
			ctx.init_library_watcher().await,
			Err(CoreError::FeatureDisabled("background jobs"))
		));
		assert!(matches!(
			ctx.add_watcher(path.clone()).await,
			Err(CoreError::FeatureDisabled("background jobs"))
		));
		assert!(matches!(
			ctx.remove_watcher(path).await,
			Err(CoreError::FeatureDisabled("background jobs"))
		));
		assert!(matches!(
			ctx.stop_library_watcher().await,
			Err(CoreError::FeatureDisabled("background jobs"))
		));
		assert!(!ctx.library_watcher_initialized());
		assert!(!ctx.job_runtime_initialized());
	}

	#[cfg(feature = "watcher")]
	#[tokio::test]
	async fn watched_library_starts_watcher_from_database_roots() {
		use models::{entity::library_config, shared::enums::FileStatus};
		use sea_orm::ActiveModelTrait;

		let db = migrated_database().await;
		let root = tempfile::tempdir().expect("tempdir");
		let library = ::tests::fake_data::Library {
			path: Some(root.path().to_string_lossy().to_string()),
			..Default::default()
		}
		.insert(&db)
		.await;
		assert_eq!(library.status, FileStatus::Ready);
		library_config::ActiveModel {
			id: sea_orm::Set(library.config_id),
			watch: sea_orm::Set(true),
			..Default::default()
		}
		.update(&db)
		.await
		.expect("failed to enable watching");

		let ctx = Ctx::for_testing(db);
		let started = ctx
			.init_library_watcher()
			.await
			.expect("watcher should start for a watched, existing root");

		assert!(started);
		assert!(ctx.library_watcher_initialized());
		assert_eq!(ctx.watcher_health_status(), "active");
		ctx.stop_library_watcher()
			.await
			.expect("watcher should stop");
	}
}
