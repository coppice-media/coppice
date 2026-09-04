use crate::{
	job::{state::JobQueueState, stump_job::StumpJob},
	CoreError, CoreResult,
};
use apalis::prelude::{MemoryStorage, MessageQueue};
use async_trait::async_trait;
use models::entity::{library, library_config};
use notify::{Event, RecommendedWatcher, Watcher};
use sea_orm::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::{oneshot, Mutex};

#[derive(Debug)]
enum LibraryWatcherCommand {
	AddWatcher {
		path: PathBuf,
		result_tx: oneshot::Sender<notify::Result<()>>,
	},
	RemoveWatcher(PathBuf),
	ChangedFiles(Vec<PathBuf>),
	Flush,
	StopWatchers,
}

fn create_watcher(
	sender: UnboundedSender<LibraryWatcherCommand>,
) -> notify::Result<RecommendedWatcher> {
	notify::recommended_watcher(move |result: Result<Event, _>| match result {
		Ok(event) => match event.kind {
			notify::EventKind::Create(_) | notify::EventKind::Modify(_) => {
				let _ = sender
					.send(LibraryWatcherCommand::ChangedFiles(event.paths))
					.map_err(|e| {
						tracing::error!(error = ?e, "Error sending file paths");
					});
			},
			_ => {},
		},
		Err(e) => {
			tracing::error!(?e, "Error processing file");
		},
	})
}

struct LibraryWatcherInternal {
	wait_interval: Duration,
	sender: UnboundedSender<LibraryWatcherCommand>,
	watcher: Option<RecommendedWatcher>,
	last_update_time: Arc<Mutex<std::time::SystemTime>>,
	accumulated_paths: HashSet<PathBuf>,
	wait_thread: Option<tokio::task::JoinHandle<()>>,
}

impl LibraryWatcherInternal {
	fn new(
		watcher: Option<RecommendedWatcher>,
		sender: UnboundedSender<LibraryWatcherCommand>,
		wait_duration: Duration,
	) -> LibraryWatcherInternal {
		LibraryWatcherInternal {
			wait_interval: wait_duration,
			sender: sender.clone(),
			watcher,
			last_update_time: Arc::new(Mutex::new(std::time::SystemTime::now())),
			accumulated_paths: HashSet::new(),
			wait_thread: None,
		}
	}
	fn ensure_watcher(&mut self) -> notify::Result<()> {
		if self.watcher.is_none() {
			self.watcher = Some(create_watcher(self.sender.clone())?);
		}

		Ok(())
	}

	fn stop(&mut self) {
		if let Some(wait_thread) = self.wait_thread.take() {
			wait_thread.abort();
		}
	}

	async fn handle_changed_files(&mut self, paths: Vec<PathBuf>) {
		{
			let mut last_update_time = self.last_update_time.lock().await;
			*last_update_time = std::time::SystemTime::now();
		}

		self.accumulated_paths.extend(paths);

		if self.wait_thread.is_none() {
			let sender = self.sender.clone();
			let interval = self.wait_interval;
			let last_update_time = self.last_update_time.clone();

			// send Flush command 5 seconds after the last update
			// the reason is avoid scanning a library that is still being updated. For example, if
			// a user is copying a large file we will get a inotify when the file is first create
			// and progressively as the file is being copied. We only want to trigger the scan
			// after the file has been fully copied.
			self.wait_thread = Some(tokio::spawn(async move {
				loop {
					tokio::time::sleep(interval).await;

					let last_update_time = last_update_time.lock().await;
					if let Ok(elapsed) = last_update_time.elapsed() {
						if elapsed > interval {
							let _ = sender.send(LibraryWatcherCommand::Flush);
							break;
						}
					}
				}
			}));
		}
	}

	async fn flush(&mut self) -> HashSet<PathBuf> {
		self.wait_thread = None;
		std::mem::take(&mut self.accumulated_paths)
	}
}

#[async_trait]
trait LibrariesProvider {
	async fn get_libraries(&self) -> CoreResult<Vec<library::LibraryIdentSelect>>;
}

#[derive(Debug, Clone)]
struct LibraryProvider {
	conn: Arc<DatabaseConnection>,
}

#[async_trait]
impl LibrariesProvider for LibraryProvider {
	#[tracing::instrument(skip(self), err)]
	async fn get_libraries(&self) -> CoreResult<Vec<library::LibraryIdentSelect>> {
		// get list of all libraries
		// for each library, if watching is enabled, watch their directory
		let conn = self.conn.as_ref();

		let libraries: Vec<library::LibraryIdentSelect> = library::Entity::find()
			.inner_join(library_config::Entity)
			.filter(library::Column::Status.eq("READY"))
			.filter(library_config::Column::Watch.eq(true))
			.into_partial_model::<library::LibraryIdentSelect>()
			.all(conn)
			.await?;

		Ok(libraries)
	}
}

#[async_trait]
trait SubmitScanJob {
	async fn submit(&self, id: String, path: String) -> Result<(), ()>;
}

#[derive(Clone)]
struct ApalisJobSubmitter {
	storage: MemoryStorage<StumpJob>,
	queue_state: Option<Arc<JobQueueState>>,
}

#[async_trait]
impl SubmitScanJob for ApalisJobSubmitter {
	async fn submit(&self, id: String, path: String) -> Result<(), ()> {
		let job = StumpJob::library_scan(id, path, None);
		if let Some(queue_state) = &self.queue_state {
			queue_state.enqueued(&job);
		}
		let mut storage = self.storage.clone();
		let queued_job = job.clone();
		match storage.enqueue(job).await {
			Ok(_) => Ok(()),
			Err(error) => {
				if let Some(queue_state) = &self.queue_state {
					queue_state.enqueue_failed(&queued_job);
				}
				tracing::error!(error = ?error, "Error enqueuing library scan job");
				Err(())
			},
		}
	}
}

pub struct LibraryWatcher {
	enabled: bool,
	sender: UnboundedSender<LibraryWatcherCommand>,
	library_provider: Arc<dyn LibrariesProvider + Send + Sync>,
	job_submitter: Arc<dyn SubmitScanJob + Send + Sync>,
}

impl LibraryWatcher {
	pub fn new(
		conn: Arc<DatabaseConnection>,
		storage: MemoryStorage<StumpJob>,
	) -> LibraryWatcher {
		Self::new_with_enabled(conn, storage, true)
	}

	pub(crate) fn new_with_enabled(
		conn: Arc<DatabaseConnection>,
		storage: MemoryStorage<StumpJob>,
		enabled: bool,
	) -> LibraryWatcher {
		let library_provider = LibraryProvider { conn };
		let job_submitter = ApalisJobSubmitter {
			storage,
			queue_state: None,
		};
		let (tx, rx) = unbounded_channel();
		Self::new_internal_with_enabled(
			tx,
			rx,
			None,
			library_provider,
			job_submitter,
			Duration::from_millis(5000),
			enabled,
		)
	}

	pub(crate) fn new_with_enabled_and_queue(
		conn: Arc<DatabaseConnection>,
		storage: MemoryStorage<StumpJob>,
		queue_state: Arc<JobQueueState>,
		enabled: bool,
	) -> LibraryWatcher {
		let library_provider = LibraryProvider { conn };
		let job_submitter = ApalisJobSubmitter {
			storage,
			queue_state: Some(queue_state),
		};
		let (tx, rx) = unbounded_channel();
		Self::new_internal_with_enabled(
			tx,
			rx,
			None,
			library_provider,
			job_submitter,
			Duration::from_millis(5000),
			enabled,
		)
	}

	fn new_internal(
		tx: UnboundedSender<LibraryWatcherCommand>,
		rx: UnboundedReceiver<LibraryWatcherCommand>,
		watcher: Option<RecommendedWatcher>,
		library_provider: impl LibrariesProvider + Send + Sync + 'static,
		job_submitter: impl SubmitScanJob + Send + Sync + 'static,
		wait_duration: Duration,
	) -> LibraryWatcher {
		Self::new_internal_with_enabled(
			tx,
			rx,
			watcher,
			library_provider,
			job_submitter,
			wait_duration,
			true,
		)
	}

	fn new_internal_with_enabled(
		tx: UnboundedSender<LibraryWatcherCommand>,
		rx: UnboundedReceiver<LibraryWatcherCommand>,
		watcher: Option<RecommendedWatcher>,
		library_provider: impl LibrariesProvider + Send + Sync + 'static,
		job_submitter: impl SubmitScanJob + Send + Sync + 'static,
		wait_duration: Duration,
		enabled: bool,
	) -> LibraryWatcher {
		let this = LibraryWatcher {
			enabled,
			sender: tx,
			library_provider: Arc::new(library_provider),
			job_submitter: Arc::new(job_submitter),
		};

		if enabled {
			LibraryWatcher::listen(
				watcher,
				this.sender.clone(),
				rx,
				this.library_provider.clone(),
				this.job_submitter.clone(),
				wait_duration,
			);
		}
		this
	}

	fn listen(
		watcher: Option<RecommendedWatcher>,
		sender: UnboundedSender<LibraryWatcherCommand>,
		mut receiver: UnboundedReceiver<LibraryWatcherCommand>,
		library_provider: Arc<dyn LibrariesProvider + Send + Sync>,
		job_submitter: Arc<dyn SubmitScanJob + Send + Sync>,
		wait_duration: Duration,
	) {
		tokio::spawn(async move {
			let mut lib_watcher =
				LibraryWatcherInternal::new(watcher, sender, wait_duration);
			while let Some(command) = receiver.recv().await {
				match command {
					LibraryWatcherCommand::AddWatcher { path, result_tx } => {
						tracing::debug!("Adding watcher for path: {:?}", path);
						let result = lib_watcher.ensure_watcher().and_then(|()| {
							lib_watcher
								.watcher
								.as_mut()
								.ok_or_else(|| {
									notify::Error::generic(
										"File watcher was not initialized",
									)
								})?
								.watch(path.as_path(), notify::RecursiveMode::Recursive)
						});

						if let Err(error) = &result {
							tracing::error!(%error, "Error adding file watcher");
						}
						let _ = result_tx.send(result);
					},
					LibraryWatcherCommand::RemoveWatcher(path) => {
						tracing::debug!("Removing watcher for path: {:?}", path);
						if let Some(watcher) = lib_watcher.watcher.as_mut() {
							if let Err(e) = watcher.unwatch(path.as_path()) {
								tracing::error!(error = ?e, "Error removing file watcher");
								break;
							}
						}
					},
					LibraryWatcherCommand::ChangedFiles(paths) => {
						lib_watcher.handle_changed_files(paths).await;
					},
					LibraryWatcherCommand::Flush => {
						let _ = Self::start_jobs(
							&library_provider,
							&job_submitter,
							lib_watcher.flush().await,
						)
						.await;
					},
					LibraryWatcherCommand::StopWatchers => {
						lib_watcher.stop();
						break;
					},
				};
			}
		});
	}
	async fn start_jobs(
		library_provider: &Arc<dyn LibrariesProvider + Send + Sync + 'static>,
		job_submitter: &Arc<dyn SubmitScanJob + Send + Sync + 'static>,
		paths: HashSet<PathBuf>,
	) -> Result<(), CoreError> {
		let libraries = library_provider.as_ref().get_libraries().await?;

		let mut libraries_to_scan = HashMap::new();
		for library in libraries {
			for path in &paths {
				if path.starts_with(&library.path) {
					libraries_to_scan.insert(library.id.clone(), library.path.clone());
				}
			}
		}

		let results = libraries_to_scan
			.into_iter()
			.map(|(id, path_str)| job_submitter.submit(id, path_str));

		for result in results {
			result.await.map_err(|e| {
				CoreError::InitializationError(format!("Failed to submit job: {:?}", e))
			})?;
		}

		Ok(())
	}

	pub async fn remove_watcher(&self, path: PathBuf) -> CoreResult<()> {
		if !self.enabled {
			return Err(CoreError::FeatureDisabled("background jobs"));
		}

		self.sender
			.send(LibraryWatcherCommand::RemoveWatcher(path))
			.map_err(|e| {
				tracing::error!(error = ?e, "Error sending remove watcher command");
				CoreError::InitializationError(format!(
					"Failed to send remove watcher command: {e:?}"
				))
			})
	}

	pub async fn stop(&self) -> CoreResult<()> {
		if !self.enabled {
			return Err(CoreError::FeatureDisabled("background jobs"));
		}

		self.sender
			.send(LibraryWatcherCommand::StopWatchers)
			.map_err(|e| {
				CoreError::InitializationError(format!(
					"Failed to send stop watcher command: {e:?}"
				))
			})
	}

	async fn add_watcher_inner(&self, path: PathBuf) -> notify::Result<()> {
		let (result_tx, result_rx) = oneshot::channel();
		self.sender
			.send(LibraryWatcherCommand::AddWatcher { path, result_tx })
			.map_err(|e| {
				notify::Error::generic(&format!(
					"Failed to send add watcher command: {e:?}"
				))
			})?;

		result_rx.await.map_err(|e| {
			notify::Error::generic(&format!(
				"File watcher stopped before adding the path: {e}"
			))
		})?
	}

	pub async fn add_watcher(&self, path: PathBuf) -> CoreResult<()> {
		if !self.enabled {
			return Err(CoreError::FeatureDisabled("background jobs"));
		}

		self.add_watcher_inner(path)
			.await
			.map_err(|e| CoreError::InitializationError(e.to_string()))
	}

	pub async fn init(&self) -> CoreResult<()> {
		if !self.enabled {
			return Err(CoreError::FeatureDisabled("background jobs"));
		}

		let libraries = self.library_provider.get_libraries().await?;
		for library in libraries {
			let path = PathBuf::from(&library.path);
			match self.add_watcher_inner(path.clone()).await {
				Ok(()) => {},
				Err(error) if matches!(&error.kind, notify::ErrorKind::PathNotFound) => {
					tracing::warn!(
						library_id = %library.id,
						path = %path.display(),
						"Library path is missing; skipping watcher initialization"
					);
				},
				Err(error) => {
					return Err(CoreError::InitializationError(format!(
						"Failed to initialize watcher for {}: {error}",
						path.display()
					)));
				},
			}
		}
		Ok(())
	}
}

mod tests {
	use super::*;

	#[allow(dead_code)]
	struct MockLibraryProvider {
		libraries: Vec<library::LibraryIdentSelect>,
	}

	#[async_trait]
	impl LibrariesProvider for MockLibraryProvider {
		async fn get_libraries(&self) -> CoreResult<Vec<library::LibraryIdentSelect>> {
			Ok(self.libraries.clone())
		}
	}

	#[allow(dead_code)]
	struct MockJobControllerSubmitter {
		tx: UnboundedSender<(String, String)>,
	}

	#[async_trait]
	impl SubmitScanJob for MockJobControllerSubmitter {
		async fn submit(&self, id: String, path: String) -> Result<(), ()> {
			let _ = self.tx.send((id, path)).map_err(|e| {
				eprintln!("Error sending job: {:?}", e);
			});
			Ok(())
		}
	}

	#[allow(dead_code)]
	struct MockObjs {
		library_watcher: LibraryWatcher,
		sender: UnboundedSender<LibraryWatcherCommand>,
		jobs_receiver: UnboundedReceiver<(String, String)>,
	}

	#[allow(dead_code)]
	async fn create_mock_library_internal(
		libraries: Vec<library::LibraryIdentSelect>,
		initialize: bool,
	) -> Result<MockObjs, CoreError> {
		let (tx_jobs, rx_jobs) = tokio::sync::mpsc::unbounded_channel();
		let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

		let library_provider = MockLibraryProvider { libraries };
		let job_submitter = MockJobControllerSubmitter {
			tx: tx_jobs.clone(),
		};

		let watcher = initialize.then(|| create_watcher(tx.clone()).unwrap());
		let library_watcher = LibraryWatcher::new_internal(
			tx.clone(),
			rx,
			watcher,
			library_provider,
			job_submitter,
			Duration::from_millis(10),
		);

		if initialize {
			library_watcher.init().await.unwrap();
		}

		Ok(MockObjs {
			library_watcher,
			sender: tx,
			jobs_receiver: rx_jobs,
		})
	}

	#[allow(dead_code)]
	async fn create_mock_library(
		libraries: Vec<library::LibraryIdentSelect>,
	) -> Result<MockObjs, CoreError> {
		create_mock_library_internal(libraries, true).await
	}

	#[allow(dead_code)]
	async fn create_lazy_mock_library(
		libraries: Vec<library::LibraryIdentSelect>,
	) -> Result<MockObjs, CoreError> {
		create_mock_library_internal(libraries, false).await
	}

	#[tokio::test]
	async fn test_disabled_watcher_does_not_spawn_listener_or_accept_commands() {
		let (tx_jobs, _rx_jobs) = tokio::sync::mpsc::unbounded_channel();
		let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
		let library_watcher = LibraryWatcher::new_internal_with_enabled(
			tx.clone(),
			rx,
			None,
			MockLibraryProvider {
				libraries: Vec::new(),
			},
			MockJobControllerSubmitter { tx: tx_jobs },
			Duration::from_millis(10),
			false,
		);

		assert!(!library_watcher.enabled);
		assert!(tx.send(LibraryWatcherCommand::Flush).is_err());
		assert!(matches!(
			library_watcher.init().await.unwrap_err(),
			CoreError::FeatureDisabled("background jobs")
		));
		assert!(matches!(
			library_watcher
				.add_watcher(PathBuf::from("/tmp/stump"))
				.await
				.unwrap_err(),
			CoreError::FeatureDisabled("background jobs")
		));
		assert!(matches!(
			library_watcher
				.remove_watcher(PathBuf::from("/tmp/stump"))
				.await
				.unwrap_err(),
			CoreError::FeatureDisabled("background jobs")
		));
		assert!(matches!(
			library_watcher.stop().await.unwrap_err(),
			CoreError::FeatureDisabled("background jobs")
		));
	}

	#[allow(dead_code)]
	fn create_test_libraries(base_dir: String) -> Vec<library::LibraryIdentSelect> {
		vec![library::LibraryIdentSelect {
			id: "42".to_string(),
			name: "Test Library".to_string(),
			path: base_dir,
		}]
	}

	#[test]
	fn test_watcher_is_lazy_and_reused() {
		let (tx, _rx) = unbounded_channel();
		let mut internal =
			LibraryWatcherInternal::new(None, tx, Duration::from_millis(10));

		assert!(internal.watcher.is_none());
		internal.ensure_watcher().unwrap();
		let watcher = internal
			.watcher
			.as_ref()
			.map(|watcher| watcher as *const RecommendedWatcher);
		assert!(watcher.is_some());

		internal.ensure_watcher().unwrap();
		assert_eq!(
			watcher,
			internal
				.watcher
				.as_ref()
				.map(|watcher| watcher as *const RecommendedWatcher)
		);
	}

	#[tokio::test]
	async fn test_library_watcher_init_without_libraries_is_lazy() {
		let mock_objs = create_lazy_mock_library(Vec::new()).await.unwrap();

		assert!(mock_objs.library_watcher.init().await.is_ok());
		assert!(mock_objs.library_watcher.stop().await.is_ok());
	}

	#[tokio::test]
	async fn test_library_watcher_init_skips_missing_library_path() {
		let tmp_dir = tempfile::tempdir().unwrap();
		let missing_path = tmp_dir.path().join("missing");
		let libraries = create_test_libraries(missing_path.to_string_lossy().to_string());
		let mock_objs = create_lazy_mock_library(libraries).await.unwrap();

		assert!(mock_objs.library_watcher.init().await.is_ok());
		assert!(mock_objs.library_watcher.stop().await.is_ok());
	}

	#[tokio::test]
	async fn test_library_watcher_init_propagates_stopped_listener() {
		let tmp_dir = tempfile::tempdir().unwrap();
		let libraries =
			create_test_libraries(tmp_dir.path().to_string_lossy().to_string());
		let mock_objs = create_lazy_mock_library(libraries).await.unwrap();

		mock_objs.library_watcher.stop().await.unwrap();
		tokio::time::timeout(Duration::from_secs(1), async {
			while !mock_objs.sender.is_closed() {
				tokio::task::yield_now().await;
			}
		})
		.await
		.expect("watcher listener should stop");

		assert!(matches!(
			mock_objs.library_watcher.init().await.unwrap_err(),
			CoreError::InitializationError(message)
				if message.contains("Failed to send add watcher command")
		));
	}

	#[tokio::test]
	async fn test_first_add_lazily_creates_watcher() {
		let tmp_dir = std::env::temp_dir().join("stump_test");
		std::fs::create_dir_all(&tmp_dir).unwrap();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());
		let mock_objs = create_lazy_mock_library(libraries).await.unwrap();

		assert!(mock_objs
			.library_watcher
			.add_watcher(tmp_dir.clone())
			.await
			.is_ok());
		assert!(mock_objs.library_watcher.stop().await.is_ok());
	}

	#[tokio::test]
	async fn test_remove_before_add_is_a_noop() {
		let tmp_dir = std::env::temp_dir().join("stump_test");
		std::fs::create_dir_all(&tmp_dir).unwrap();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());
		let mock_objs = create_lazy_mock_library(libraries).await.unwrap();

		assert!(mock_objs
			.library_watcher
			.remove_watcher(tmp_dir.clone())
			.await
			.is_ok());
		assert!(mock_objs.library_watcher.stop().await.is_ok());
	}

	#[tokio::test]
	async fn test_lazy_add_watcher_twice_reuses_watcher() {
		let tmp_dir = std::env::temp_dir().join("stump_test");
		std::fs::create_dir_all(&tmp_dir).unwrap();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());
		let mock_objs = create_lazy_mock_library(libraries).await.unwrap();

		assert!(mock_objs
			.library_watcher
			.add_watcher(tmp_dir.clone())
			.await
			.is_ok());
		assert!(mock_objs.library_watcher.add_watcher(tmp_dir).await.is_ok());
		assert!(mock_objs.library_watcher.stop().await.is_ok());
	}

	#[tokio::test]
	async fn test_library_watcher_stop_without_add() {
		let mock_objs = create_lazy_mock_library(Vec::new()).await.unwrap();

		assert!(mock_objs.library_watcher.stop().await.is_ok());
	}

	#[tokio::test]
	async fn test_library_watcher_init() {
		let tmp_dir = std::env::temp_dir().join("stump_test");
		std::fs::create_dir_all(&tmp_dir).unwrap();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());

		let mut mock_objs = create_mock_library(libraries).await.unwrap();

		assert!(mock_objs
			.library_watcher
			.add_watcher(tmp_dir.clone())
			.await
			.is_ok());
		let new_file = tmp_dir.join("new_file");
		assert!(mock_objs
			.sender
			.send(LibraryWatcherCommand::ChangedFiles(vec![new_file.clone()]))
			.is_ok());

		// Wait for the background thread to trigger the flush
		tokio::time::sleep(Duration::from_millis(100)).await;
		let (id, path) = mock_objs.jobs_receiver.try_recv().expect("Expected a job");
		assert_eq!(id, "42");
		assert_eq!(path, tmp_dir.to_string_lossy().to_string());
	}

	#[tokio::test]
	async fn test_remove() {
		let tmp_dir = std::env::temp_dir().join("stump_test");
		std::fs::create_dir_all(&tmp_dir).unwrap();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());

		let mock_objs = create_mock_library(libraries).await.unwrap();

		assert!(mock_objs
			.library_watcher
			.add_watcher(tmp_dir.clone())
			.await
			.is_ok());
		assert!(mock_objs
			.library_watcher
			.remove_watcher(tmp_dir.clone())
			.await
			.is_ok());

		// try removing again
		assert!(mock_objs
			.library_watcher
			.remove_watcher(tmp_dir.clone())
			.await
			.is_ok());
	}

	#[tokio::test]
	async fn test_add_watcher_twice() {
		let tmp_dir = std::env::temp_dir().join("stump_test");
		std::fs::create_dir_all(&tmp_dir).unwrap();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());

		let mock_objs = create_mock_library(libraries).await.unwrap();
		assert!(mock_objs
			.library_watcher
			.add_watcher(tmp_dir.clone())
			.await
			.is_ok());
		assert!(mock_objs
			.library_watcher
			.add_watcher(tmp_dir.clone())
			.await
			.is_ok());
	}

	#[tokio::test]
	async fn test_library_watcher_stop() {
		let tmp_dir = std::env::temp_dir().join("stump_test");
		std::fs::create_dir_all(&tmp_dir).unwrap();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());

		let mock_objs = create_mock_library(libraries).await.unwrap();

		assert!(mock_objs
			.library_watcher
			.add_watcher(tmp_dir.clone())
			.await
			.is_ok());
		let new_file = tmp_dir.join("new_file");
		assert!(mock_objs
			.sender
			.send(LibraryWatcherCommand::ChangedFiles(vec![new_file.clone()]))
			.is_ok());
		assert!(mock_objs.library_watcher.stop().await.is_ok());
	}

	#[tokio::test]
	async fn test_start_jobs() {
		let tmp_dir = std::env::temp_dir().join("stump_test");
		std::fs::create_dir_all(&tmp_dir).unwrap();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());

		let paths = HashSet::from_iter(vec![tmp_dir.clone().join("new_file")]);

		let mut mock_objs = create_mock_library(libraries).await.unwrap();

		assert!(LibraryWatcher::start_jobs(
			&mock_objs.library_watcher.library_provider,
			&mock_objs.library_watcher.job_submitter,
			paths,
		)
		.await
		.is_ok());

		let (id, path) = mock_objs.jobs_receiver.try_recv().expect("Expected a job");
		assert_eq!(id, "42");
		assert_eq!(path, tmp_dir.to_string_lossy().to_string());
	}

	#[tokio::test]
	async fn test_start_jobs_on_bad_library() {
		let tmp_dir = std::env::temp_dir().join("stump_test");
		std::fs::create_dir_all(&tmp_dir).unwrap();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());

		let bad_paths =
			HashSet::from_iter(vec![PathBuf::from("/home/user/test/new_file")]);

		let mut mock_objs = create_mock_library(libraries).await.unwrap();

		assert!(LibraryWatcher::start_jobs(
			&mock_objs.library_watcher.library_provider,
			&mock_objs.library_watcher.job_submitter,
			bad_paths,
		)
		.await
		.is_ok());

		assert_eq!(
			mock_objs.jobs_receiver.try_recv().unwrap_err(),
			tokio::sync::mpsc::error::TryRecvError::Empty
		);
	}
}
