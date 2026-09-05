use std::{sync::Arc, time::Duration};

use tokio::{
	sync::{mpsc, Notify},
	task::JoinHandle,
};

use crate::{
	inline_executor,
	worker::{JobContext, WorkerState},
	JobError, JobExecutionContext, JobQueueStatus,
};

/// How long a stopping runtime waits for the in-flight job before abandoning it.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(30);

/// The executor-specific half of the queue: where `enqueue` pushes a payload.
#[derive(Clone)]
pub(crate) enum QueueSender<J> {
	Inline(mpsc::UnboundedSender<J>),
	#[cfg(feature = "apalis")]
	Apalis(apalis::prelude::MemoryStorage<J>),
}

impl<J: Send + Sync + 'static> QueueSender<J> {
	pub(crate) async fn send(&self, job: J) -> Result<(), JobError> {
		match self {
			Self::Inline(sender) => sender.send(job).map_err(|_| {
				JobError::Unknown("The job executor has stopped".to_string())
			}),
			#[cfg(feature = "apalis")]
			Self::Apalis(storage) => {
				use apalis::prelude::MessageQueue;
				storage.clone().enqueue(job).await.map_err(|()| {
					JobError::Unknown("Failed to enqueue job".to_string())
				})
			},
		}
	}
}

/// Asks an executor to finish its in-flight job and exit.
enum StopSignal {
	Inline(Arc<Notify>),
	#[cfg(feature = "apalis")]
	Apalis(apalis::prelude::Worker<apalis::prelude::Context>),
}

impl StopSignal {
	fn signal(&self) {
		match self {
			Self::Inline(shutdown) => shutdown.notify_one(),
			#[cfg(feature = "apalis")]
			Self::Apalis(worker) => worker.stop(),
		}
	}
}

/// A queue plus the single executor draining it.
///
/// The runtime is the lifecycle boundary for background work: a host that never enqueues
/// never creates one. `new` picks the Apalis executor when the `apalis` feature is enabled
/// and the inline executor otherwise; both share the same queue counters, cancellation
/// registry, and [`JobContext`] hooks.
pub struct JobRuntime<X: JobExecutionContext> {
	worker: Arc<WorkerState<X>>,
	stop: StopSignal,
	executor: std::sync::Mutex<Option<JoinHandle<()>>>,
	backend: &'static str,
}

impl<X: JobExecutionContext> JobRuntime<X> {
	/// Start the default executor for this build.
	pub fn new(services: Arc<X>) -> Self {
		#[cfg(feature = "apalis")]
		{
			Self::apalis(services)
		}
		#[cfg(not(feature = "apalis"))]
		{
			Self::inline(services)
		}
	}

	/// Start an executor that runs queued jobs one at a time on the Tokio blocking pool.
	pub fn inline(services: Arc<X>) -> Self {
		let (sender, receiver) = mpsc::unbounded_channel();
		let worker = Arc::new(WorkerState::new(services, QueueSender::Inline(sender)));
		let shutdown = Arc::new(Notify::new());
		let executor =
			inline_executor::spawn(Arc::clone(&worker), receiver, Arc::clone(&shutdown));
		Self {
			worker,
			stop: StopSignal::Inline(shutdown),
			executor: std::sync::Mutex::new(Some(executor)),
			backend: "inline",
		}
	}

	/// Start a single Apalis worker over an in-memory queue.
	#[cfg(feature = "apalis")]
	pub fn apalis(services: Arc<X>) -> Self {
		let storage = apalis::prelude::MemoryStorage::new();
		let worker = Arc::new(WorkerState::new(
			services,
			QueueSender::Apalis(storage.clone()),
		));
		let (executor, handle) = crate::apalis_executor::spawn(Arc::clone(&worker), storage);
		Self {
			worker,
			stop: StopSignal::Apalis(handle),
			executor: std::sync::Mutex::new(Some(executor)),
			backend: "apalis",
		}
	}

	/// The executor draining this runtime's queue: `apalis` or `inline`.
	pub fn backend(&self) -> &'static str {
		self.backend
	}

	pub fn services(&self) -> &Arc<X> {
		&self.worker.services
	}

	/// Enqueue a job; the queue counters account for it before the executor can dequeue it.
	pub async fn enqueue(&self, job: X::Job) -> Result<(), JobError> {
		self.worker.enqueue(job).await
	}

	/// A lock-free snapshot of queued and running jobs.
	pub fn queue_depth(&self) -> JobQueueStatus {
		self.worker.queue.snapshot()
	}

	/// Cancel a running job by ID, returning true if a cancellation token was found and cancelled
	pub fn cancel_job(&self, job_id: &str) -> bool {
		self.worker.cancel_job(job_id)
	}

	/// Whether no job is currently running
	pub fn is_idle(&self) -> bool {
		self.worker.cancellation_tokens.is_empty()
	}

	/// Persist and register a job without queueing it, returning the context to drive its
	/// lifecycle directly (benchmarks and tests).
	pub async fn open_job(
		&self,
		job_id: String,
		job: &X::Job,
	) -> Result<JobContext<X>, JobError> {
		JobContext::open(Arc::clone(&self.worker), job_id, job).await
	}

	/// Cancel running jobs and stop the executor, waiting up to [`SHUTDOWN_GRACE`] for the
	/// in-flight job before abandoning it.
	pub async fn stop(&self) {
		self.worker.cancel_all();
		self.stop.signal();
		let executor = self
			.executor
			.lock()
			.expect("job runtime executor mutex poisoned")
			.take();
		if let Some(executor) = executor {
			let abort = executor.abort_handle();
			if tokio::time::timeout(SHUTDOWN_GRACE, executor).await.is_err() {
				tracing::warn!(
					backend = self.backend,
					"Job executor did not stop in time; abandoning it"
				);
				abort.abort();
			}
		}
		self.worker.cancellation_tokens.clear();
	}
}

impl<X: JobExecutionContext> Drop for JobRuntime<X> {
	fn drop(&mut self) {
		self.worker.cancel_all();
		self.stop.signal();
		if let Some(executor) = self
			.executor
			.lock()
			.expect("job runtime executor mutex poisoned")
			.take()
		{
			executor.abort();
		}
	}
}
