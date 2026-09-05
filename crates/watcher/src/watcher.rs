use std::{
	collections::{HashMap, HashSet},
	path::PathBuf,
	sync::Arc,
	time::{Duration, Instant},
};

use notify::{Event, RecommendedWatcher, Watcher as _};
use tokio::sync::{
	mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
	oneshot, Mutex,
};

use crate::{LibraryRoot, ScanRequest, ScanSubmitter, WatchedLibraries, WatcherError};

/// How long a library root must stay quiet before its accumulated changes trigger a scan.
///
/// Copying a large file produces a create event followed by a stream of modify events; the scan
/// must wait until the copy has finished.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_secs(5);

#[derive(Debug)]
enum Command {
	AddWatcher {
		path: PathBuf,
		result_tx: oneshot::Sender<notify::Result<()>>,
	},
	RemoveWatcher(PathBuf),
	ChangedFiles(Vec<PathBuf>),
	Flush,
	Stop,
}

fn create_backend(sender: UnboundedSender<Command>) -> notify::Result<RecommendedWatcher> {
	notify::recommended_watcher(move |result: Result<Event, _>| match result {
		Ok(event) => match event.kind {
			notify::EventKind::Create(_) | notify::EventKind::Modify(_) => {
				let _ = sender.send(Command::ChangedFiles(event.paths)).map_err(|e| {
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

/// The listener-owned state: the lazily created `notify` backend plus the debounce window.
struct Listener {
	debounce: Duration,
	sender: UnboundedSender<Command>,
	backend: Option<RecommendedWatcher>,
	last_change: Arc<Mutex<Instant>>,
	accumulated_paths: HashSet<PathBuf>,
	flush_task: Option<tokio::task::JoinHandle<()>>,
}

impl Listener {
	fn new(
		backend: Option<RecommendedWatcher>,
		sender: UnboundedSender<Command>,
		debounce: Duration,
	) -> Self {
		Listener {
			debounce,
			sender,
			backend,
			last_change: Arc::new(Mutex::new(Instant::now())),
			accumulated_paths: HashSet::new(),
			flush_task: None,
		}
	}

	/// Creates the OS watcher on first use; a process that never watches a path never
	/// allocates one.
	fn ensure_backend(&mut self) -> notify::Result<&mut RecommendedWatcher> {
		if self.backend.is_none() {
			self.backend = Some(create_backend(self.sender.clone())?);
		}
		Ok(self
			.backend
			.as_mut()
			.expect("backend was created immediately above"))
	}

	fn stop(&mut self) {
		if let Some(flush_task) = self.flush_task.take() {
			flush_task.abort();
		}
	}

	async fn handle_changed_files(&mut self, paths: Vec<PathBuf>) {
		*self.last_change.lock().await = Instant::now();
		self.accumulated_paths.extend(paths);

		if self.flush_task.is_none() {
			let sender = self.sender.clone();
			let debounce = self.debounce;
			let last_change = self.last_change.clone();

			self.flush_task = Some(tokio::spawn(async move {
				loop {
					tokio::time::sleep(debounce).await;
					if last_change.lock().await.elapsed() > debounce {
						let _ = sender.send(Command::Flush);
						break;
					}
				}
			}));
		}
	}

	fn flush(&mut self) -> HashSet<PathBuf> {
		self.flush_task = None;
		std::mem::take(&mut self.accumulated_paths)
	}
}

/// Watches library roots and submits one debounced scan per changed library.
///
/// Constructing a `Watcher` spawns its command listener; the `notify` backend itself is created
/// lazily by the first successful [`Watcher::add_watcher`].
pub struct Watcher {
	sender: UnboundedSender<Command>,
	libraries: Arc<dyn WatchedLibraries>,
	submitter: Arc<dyn ScanSubmitter>,
}

impl Watcher {
	pub fn new(
		libraries: impl WatchedLibraries + 'static,
		submitter: impl ScanSubmitter + 'static,
		debounce: Duration,
	) -> Self {
		let (tx, rx) = unbounded_channel();
		Self::from_channel(tx, rx, None, libraries, submitter, debounce)
	}

	fn from_channel(
		tx: UnboundedSender<Command>,
		rx: UnboundedReceiver<Command>,
		backend: Option<RecommendedWatcher>,
		libraries: impl WatchedLibraries + 'static,
		submitter: impl ScanSubmitter + 'static,
		debounce: Duration,
	) -> Self {
		let this = Watcher {
			sender: tx,
			libraries: Arc::new(libraries),
			submitter: Arc::new(submitter),
		};
		Self::listen(
			backend,
			this.sender.clone(),
			rx,
			this.libraries.clone(),
			this.submitter.clone(),
			debounce,
		);
		this
	}

	fn listen(
		backend: Option<RecommendedWatcher>,
		sender: UnboundedSender<Command>,
		mut receiver: UnboundedReceiver<Command>,
		libraries: Arc<dyn WatchedLibraries>,
		submitter: Arc<dyn ScanSubmitter>,
		debounce: Duration,
	) {
		tokio::spawn(async move {
			let mut listener = Listener::new(backend, sender, debounce);
			while let Some(command) = receiver.recv().await {
				match command {
					Command::AddWatcher { path, result_tx } => {
						tracing::debug!(path = %path.display(), "Adding watcher for path");
						let result = listener.ensure_backend().and_then(|backend| {
							backend.watch(&path, notify::RecursiveMode::Recursive)
						});
						if let Err(error) = &result {
							tracing::error!(%error, "Error adding file watcher");
						}
						let _ = result_tx.send(result);
					},
					Command::RemoveWatcher(path) => {
						tracing::debug!(path = %path.display(), "Removing watcher for path");
						if let Some(backend) = listener.backend.as_mut() {
							if let Err(error) = backend.unwatch(&path) {
								tracing::error!(%error, "Error removing file watcher");
							}
						}
					},
					Command::ChangedFiles(paths) => {
						listener.handle_changed_files(paths).await;
					},
					Command::Flush => {
						if let Err(error) =
							Self::submit_scans(&libraries, &submitter, listener.flush())
								.await
						{
							tracing::error!(%error, "Error submitting scans for changed files");
						}
					},
					Command::Stop => {
						listener.stop();
						break;
					},
				}
			}
		});
	}

	/// Submits one scan for every watched library that contains at least one changed path.
	async fn submit_scans(
		libraries: &Arc<dyn WatchedLibraries>,
		submitter: &Arc<dyn ScanSubmitter>,
		paths: HashSet<PathBuf>,
	) -> Result<(), WatcherError> {
		let roots = libraries
			.enabled_libraries()
			.await
			.map_err(WatcherError::Libraries)?;

		let mut libraries_to_scan: HashMap<String, String> = HashMap::new();
		for LibraryRoot { id, path } in roots {
			if paths.iter().any(|changed| changed.starts_with(&path)) {
				libraries_to_scan.insert(id, path);
			}
		}

		for (library_id, path) in libraries_to_scan {
			submitter
				.submit(ScanRequest {
					library_id: library_id.clone(),
					path,
				})
				.await
				.map_err(|source| WatcherError::Submit { library_id, source })?;
		}

		Ok(())
	}

	fn send(&self, command: Command, what: &str) -> Result<(), WatcherError> {
		self.sender.send(command).map_err(|e| {
			WatcherError::Stopped(format!("Failed to send {what} command: {e:?}"))
		})
	}

	/// Stops watching `path`. Removing a path that was never watched is a no-op.
	pub async fn remove_watcher(&self, path: PathBuf) -> Result<(), WatcherError> {
		self.send(Command::RemoveWatcher(path), "remove watcher")
	}

	/// Stops the listener; every later command fails with [`WatcherError::Stopped`].
	pub async fn stop(&self) -> Result<(), WatcherError> {
		self.send(Command::Stop, "stop watcher")
	}

	/// Watches `path` recursively, creating the `notify` backend on first use.
	pub async fn add_watcher(&self, path: PathBuf) -> Result<(), WatcherError> {
		let (result_tx, result_rx) = oneshot::channel();
		self.send(Command::AddWatcher { path, result_tx }, "add watcher")?;

		result_rx
			.await
			.map_err(|e| {
				WatcherError::Stopped(format!(
					"File watcher stopped before adding the path: {e}"
				))
			})?
			.map_err(WatcherError::Notify)
	}

	/// Watches every currently enabled library root, skipping roots that do not exist on disk.
	pub async fn init(&self) -> Result<(), WatcherError> {
		let roots = self
			.libraries
			.enabled_libraries()
			.await
			.map_err(WatcherError::Libraries)?;

		for LibraryRoot { id, path } in roots {
			let path = PathBuf::from(path);
			match self.add_watcher(path.clone()).await {
				Ok(()) => {},
				Err(error) if error.is_path_not_found() => {
					tracing::warn!(
						library_id = %id,
						path = %path.display(),
						"Library path is missing; skipping watcher initialization"
					);
				},
				Err(error) => return Err(error),
			}
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use async_trait::async_trait;

	struct MockLibraries {
		libraries: Vec<LibraryRoot>,
	}

	#[async_trait]
	impl WatchedLibraries for MockLibraries {
		async fn enabled_libraries(&self) -> Result<Vec<LibraryRoot>, crate::BoxError> {
			Ok(self.libraries.clone())
		}
	}

	struct MockSubmitter {
		tx: UnboundedSender<ScanRequest>,
	}

	#[async_trait]
	impl ScanSubmitter for MockSubmitter {
		async fn submit(&self, request: ScanRequest) -> Result<(), crate::BoxError> {
			self.tx.send(request).map_err(|e| e.into())
		}
	}

	struct MockObjs {
		watcher: Watcher,
		sender: UnboundedSender<Command>,
		scans: UnboundedReceiver<ScanRequest>,
	}

	async fn create_mock_watcher_internal(
		libraries: Vec<LibraryRoot>,
		initialize: bool,
	) -> MockObjs {
		let (tx_scans, rx_scans) = unbounded_channel();
		let (tx, rx) = unbounded_channel();

		let backend = initialize.then(|| create_backend(tx.clone()).unwrap());
		let watcher = Watcher::from_channel(
			tx.clone(),
			rx,
			backend,
			MockLibraries { libraries },
			MockSubmitter { tx: tx_scans },
			Duration::from_millis(10),
		);

		if initialize {
			watcher.init().await.unwrap();
		}

		MockObjs {
			watcher,
			sender: tx,
			scans: rx_scans,
		}
	}

	async fn create_mock_watcher(libraries: Vec<LibraryRoot>) -> MockObjs {
		create_mock_watcher_internal(libraries, true).await
	}

	async fn create_lazy_mock_watcher(libraries: Vec<LibraryRoot>) -> MockObjs {
		create_mock_watcher_internal(libraries, false).await
	}

	fn create_test_libraries(base_dir: String) -> Vec<LibraryRoot> {
		vec![LibraryRoot {
			id: "42".to_string(),
			path: base_dir,
		}]
	}

	fn test_dir() -> PathBuf {
		let tmp_dir = std::env::temp_dir().join("stump_test");
		std::fs::create_dir_all(&tmp_dir).unwrap();
		tmp_dir
	}

	#[test]
	fn test_backend_is_lazy_and_reused() {
		let (tx, _rx) = unbounded_channel();
		let mut listener = Listener::new(None, tx, Duration::from_millis(10));

		assert!(listener.backend.is_none());
		let first = listener.ensure_backend().unwrap() as *const RecommendedWatcher;
		assert!(listener.backend.is_some());

		let second = listener.ensure_backend().unwrap() as *const RecommendedWatcher;
		assert_eq!(first, second);
	}

	#[tokio::test]
	async fn test_init_without_libraries_is_lazy() {
		let mock_objs = create_lazy_mock_watcher(Vec::new()).await;

		assert!(mock_objs.watcher.init().await.is_ok());
		assert!(mock_objs.watcher.stop().await.is_ok());
	}

	#[tokio::test]
	async fn test_init_skips_missing_library_path() {
		let tmp_dir = tempfile::tempdir().unwrap();
		let missing_path = tmp_dir.path().join("missing");
		let libraries = create_test_libraries(missing_path.to_string_lossy().to_string());
		let mock_objs = create_lazy_mock_watcher(libraries).await;

		assert!(mock_objs.watcher.init().await.is_ok());
		assert!(mock_objs.watcher.stop().await.is_ok());
	}

	#[tokio::test]
	async fn test_init_propagates_stopped_listener() {
		let tmp_dir = tempfile::tempdir().unwrap();
		let libraries =
			create_test_libraries(tmp_dir.path().to_string_lossy().to_string());
		let mock_objs = create_lazy_mock_watcher(libraries).await;

		mock_objs.watcher.stop().await.unwrap();
		tokio::time::timeout(Duration::from_secs(1), async {
			while !mock_objs.sender.is_closed() {
				tokio::task::yield_now().await;
			}
		})
		.await
		.expect("watcher listener should stop");

		assert!(matches!(
			mock_objs.watcher.init().await.unwrap_err(),
			WatcherError::Stopped(message)
				if message.contains("Failed to send add watcher command")
		));
	}

	#[tokio::test]
	async fn test_first_add_lazily_creates_backend() {
		let tmp_dir = test_dir();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());
		let mock_objs = create_lazy_mock_watcher(libraries).await;

		assert!(mock_objs.watcher.add_watcher(tmp_dir.clone()).await.is_ok());
		assert!(mock_objs.watcher.stop().await.is_ok());
	}

	#[tokio::test]
	async fn test_add_missing_path_reports_path_not_found() {
		let tmp_dir = tempfile::tempdir().unwrap();
		let mock_objs = create_lazy_mock_watcher(Vec::new()).await;

		let error = mock_objs
			.watcher
			.add_watcher(tmp_dir.path().join("missing"))
			.await
			.unwrap_err();
		assert!(error.is_path_not_found(), "{error}");
		assert!(mock_objs.watcher.stop().await.is_ok());
	}

	#[tokio::test]
	async fn test_remove_before_add_is_a_noop() {
		let tmp_dir = test_dir();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());
		let mock_objs = create_lazy_mock_watcher(libraries).await;

		assert!(mock_objs
			.watcher
			.remove_watcher(tmp_dir.clone())
			.await
			.is_ok());
		assert!(mock_objs.watcher.stop().await.is_ok());
	}

	#[tokio::test]
	async fn test_lazy_add_watcher_twice_reuses_backend() {
		let tmp_dir = test_dir();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());
		let mock_objs = create_lazy_mock_watcher(libraries).await;

		assert!(mock_objs.watcher.add_watcher(tmp_dir.clone()).await.is_ok());
		assert!(mock_objs.watcher.add_watcher(tmp_dir).await.is_ok());
		assert!(mock_objs.watcher.stop().await.is_ok());
	}

	#[tokio::test]
	async fn test_stop_without_add() {
		let mock_objs = create_lazy_mock_watcher(Vec::new()).await;

		assert!(mock_objs.watcher.stop().await.is_ok());
	}

	#[tokio::test]
	async fn test_changed_files_are_debounced_into_one_scan() {
		let tmp_dir = test_dir();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());

		let mut mock_objs = create_mock_watcher(libraries).await;

		assert!(mock_objs.watcher.add_watcher(tmp_dir.clone()).await.is_ok());
		let new_file = tmp_dir.join("new_file");
		assert!(mock_objs
			.sender
			.send(Command::ChangedFiles(vec![new_file.clone()]))
			.is_ok());

		// Wait for the background task to trigger the flush
		tokio::time::sleep(Duration::from_millis(100)).await;
		let request = mock_objs.scans.try_recv().expect("Expected a scan request");
		assert_eq!(request.library_id, "42");
		assert_eq!(request.path, tmp_dir.to_string_lossy().to_string());
		assert_eq!(
			mock_objs.scans.try_recv().unwrap_err(),
			tokio::sync::mpsc::error::TryRecvError::Empty
		);
	}

	#[tokio::test]
	async fn test_remove_twice_keeps_listener_alive() {
		let tmp_dir = test_dir();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());

		let mock_objs = create_mock_watcher(libraries).await;

		assert!(mock_objs.watcher.add_watcher(tmp_dir.clone()).await.is_ok());
		assert!(mock_objs
			.watcher
			.remove_watcher(tmp_dir.clone())
			.await
			.is_ok());

		// try removing again
		assert!(mock_objs
			.watcher
			.remove_watcher(tmp_dir.clone())
			.await
			.is_ok());

		// the listener must survive the failed unwatch and keep serving commands
		assert!(mock_objs.watcher.add_watcher(tmp_dir.clone()).await.is_ok());
		assert!(mock_objs.watcher.stop().await.is_ok());
	}

	#[tokio::test]
	async fn test_add_watcher_twice() {
		let tmp_dir = test_dir();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());

		let mock_objs = create_mock_watcher(libraries).await;
		assert!(mock_objs.watcher.add_watcher(tmp_dir.clone()).await.is_ok());
		assert!(mock_objs.watcher.add_watcher(tmp_dir.clone()).await.is_ok());
	}

	#[tokio::test]
	async fn test_stop_after_changes() {
		let tmp_dir = test_dir();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());

		let mock_objs = create_mock_watcher(libraries).await;

		assert!(mock_objs.watcher.add_watcher(tmp_dir.clone()).await.is_ok());
		let new_file = tmp_dir.join("new_file");
		assert!(mock_objs
			.sender
			.send(Command::ChangedFiles(vec![new_file.clone()]))
			.is_ok());
		assert!(mock_objs.watcher.stop().await.is_ok());
	}

	#[tokio::test]
	async fn test_submit_scans() {
		let tmp_dir = test_dir();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());

		let paths = HashSet::from_iter(vec![tmp_dir.clone().join("new_file")]);

		let mut mock_objs = create_mock_watcher(libraries).await;

		assert!(Watcher::submit_scans(
			&mock_objs.watcher.libraries,
			&mock_objs.watcher.submitter,
			paths,
		)
		.await
		.is_ok());

		let request = mock_objs.scans.try_recv().expect("Expected a scan request");
		assert_eq!(request.library_id, "42");
		assert_eq!(request.path, tmp_dir.to_string_lossy().to_string());
	}

	#[tokio::test]
	async fn test_submit_scans_ignores_paths_outside_libraries() {
		let tmp_dir = test_dir();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());

		let bad_paths =
			HashSet::from_iter(vec![PathBuf::from("/home/user/test/new_file")]);

		let mut mock_objs = create_mock_watcher(libraries).await;

		assert!(Watcher::submit_scans(
			&mock_objs.watcher.libraries,
			&mock_objs.watcher.submitter,
			bad_paths,
		)
		.await
		.is_ok());

		assert_eq!(
			mock_objs.scans.try_recv().unwrap_err(),
			tokio::sync::mpsc::error::TryRecvError::Empty
		);
	}

	#[tokio::test]
	async fn test_submit_scans_surfaces_submitter_error() {
		let tmp_dir = test_dir();
		let libraries = create_test_libraries(tmp_dir.to_string_lossy().to_string());
		let paths = HashSet::from_iter(vec![tmp_dir.clone().join("new_file")]);

		let mock_objs = create_lazy_mock_watcher(libraries).await;
		// Dropping the receiver makes every submit fail.
		drop(mock_objs.scans);

		let error = Watcher::submit_scans(
			&mock_objs.watcher.libraries,
			&mock_objs.watcher.submitter,
			paths,
		)
		.await
		.unwrap_err();
		assert!(matches!(error, WatcherError::Submit { ref library_id, .. } if library_id == "42"));
	}
}
