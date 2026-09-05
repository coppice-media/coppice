use std::{
	sync::{
		atomic::{AtomicBool, Ordering},
		Arc,
	},
	time::Instant,
};

use dashmap::DashMap;
use models::shared::enums::JobStatus;
use sea_orm::DatabaseConnection;
use serde::Serialize;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
	queue::JobQueueState, runtime::QueueSender, JobError, JobEvent, JobExecuteLog,
	JobExecutionContext, JobOutcome, JobOutputExt, JobPayload, JobProgress,
	JobQueueStatus, JobUpdate,
};

/// State shared by the queue handle, the executor, and every running job: the host
/// services, the queue sender, cancellation tokens, and the queue counters.
pub(crate) struct WorkerState<X: JobExecutionContext> {
	pub(crate) services: Arc<X>,
	pub(crate) sender: QueueSender<X::Job>,
	pub(crate) cancellation_tokens: DashMap<String, CancellationToken>,
	pub(crate) queue: JobQueueState,
}

impl<X: JobExecutionContext> WorkerState<X> {
	pub(crate) fn new(services: Arc<X>, sender: QueueSender<X::Job>) -> Self {
		Self {
			services,
			sender,
			cancellation_tokens: DashMap::new(),
			queue: JobQueueState::default(),
		}
	}

	fn publish(&self, status: JobQueueStatus) {
		self.services.emit(JobEvent::QueueStatus(status).into());
	}

	/// Enqueue a job and account for it before the executor can dequeue it.
	pub(crate) async fn enqueue(&self, job: X::Job) -> Result<(), JobError> {
		let kind = job.kind();
		self.publish(self.queue.enqueued(kind));
		if let Err(error) = self.sender.send(job).await {
			self.publish(self.queue.enqueue_failed(kind));
			return Err(error);
		}
		Ok(())
	}

	/// Cancel a running job by ID, returning true if a cancellation token was found and cancelled
	pub(crate) fn cancel_job(&self, job_id: &str) -> bool {
		match self.cancellation_tokens.get(job_id) {
			Some(entry) => {
				entry.value().cancel();
				true
			},
			None => false,
		}
	}

	pub(crate) fn cancel_all(&self) {
		for entry in self.cancellation_tokens.iter() {
			entry.value().cancel();
		}
	}
}

/// Runs one dequeued job through the host's dispatch table. Shared by every executor.
pub(crate) async fn dispatch<X: JobExecutionContext>(
	worker: Arc<WorkerState<X>>,
	job: X::Job,
) -> Result<(), JobError> {
	let job_id = Uuid::new_v4().to_string();
	let job_name = job.name();
	tracing::info!(%job_id, job_name, "Starting job");

	let ctx = match JobContext::open(Arc::clone(&worker), job_id, &job).await {
		Ok(ctx) => ctx,
		Err(error) => {
			worker.publish(worker.queue.enqueue_failed(job.kind()));
			tracing::error!(?error, job_name, "Failed to start job");
			return Err(error);
		},
	};

	let result = worker.services.run(job, &ctx).await;
	if let Err(error) = &result {
		tracing::error!(?error, job_name, "Job failed");
	}
	result
}

/// Per-execution context for a specific running job
pub struct JobContext<X: JobExecutionContext> {
	job_id: String,
	kind: &'static str,
	worker: Arc<WorkerState<X>>,
	cancel_token: CancellationToken,
	queue_finished: AtomicBool,
	start: Instant,
}

impl<X: JobExecutionContext> JobContext<X> {
	pub(crate) async fn open(
		worker: Arc<WorkerState<X>>,
		job_id: String,
		job: &X::Job,
	) -> Result<Self, JobError> {
		worker.services.persist_started(&job_id, job).await?;

		let cancel_token = CancellationToken::new();
		worker
			.cancellation_tokens
			.insert(job_id.clone(), cancel_token.clone());
		worker.publish(worker.queue.started(job.kind()));

		Ok(Self {
			job_id,
			kind: job.kind(),
			worker,
			cancel_token,
			queue_finished: AtomicBool::new(false),
			start: Instant::now(),
		})
	}

	pub fn job_id(&self) -> &str {
		&self.job_id
	}

	/// The host services this job runs against
	pub fn services(&self) -> &X {
		&self.worker.services
	}

	pub fn conn(&self) -> &DatabaseConnection {
		self.worker.services.conn()
	}

	pub fn config(&self) -> &X::Config {
		self.worker.services.config()
	}

	pub fn cancel_token(&self) -> &CancellationToken {
		&self.cancel_token
	}

	/// Check if this job has been canceled by looking up its cancellation token
	pub fn is_canceled(&self) -> bool {
		self.cancel_token.is_cancelled()
	}

	/// Sends an event to the host event stream
	pub fn emit_event(&self, event: X::Event) {
		self.worker.services.emit(event);
	}

	pub(crate) fn report_started(&self) {
		self.emit_event(
			JobEvent::Started {
				id: self.job_id.clone(),
			}
			.into(),
		);
	}

	/// Sends a [`JobProgress`] update event to the host event stream
	pub fn report_progress(&self, progress: JobProgress) {
		self.emit_event(
			JobEvent::Progress(JobUpdate {
				id: self.job_id.clone(),
				payload: progress,
			})
			.into(),
		);
	}

	/// Sends the final output of this job to the host event stream
	pub fn report_output(&self, output: X::Output) {
		self.emit_event(
			JobEvent::Output {
				id: self.job_id.clone(),
				output,
			}
			.into(),
		);
	}

	/// Enqueue a follow-up job from this job's execution
	pub async fn enqueue(&self, job: X::Job) -> Result<(), JobError> {
		self.worker.enqueue(job).await
	}

	/// Persist the outputs and logs of a completed job
	pub async fn complete<O: Serialize + JobOutputExt>(
		&self,
		output: &O,
		logs: Vec<JobExecuteLog>,
	) -> Result<(), JobError> {
		let elapsed = self.start.elapsed();
		self.report_progress(JobProgress::finished());

		let outcome = JobOutcome {
			status: JobStatus::Completed,
			elapsed,
			output: serde_json::to_vec(output).ok(),
			logs,
		};
		self.worker
			.services
			.persist_finished(&self.job_id, outcome)
			.await?;
		self.finish();

		Ok(())
	}

	/// Mark a job as failed with a given status and message
	pub async fn fail(&self, status: JobStatus, message: &str) -> Result<(), JobError> {
		let elapsed = self.start.elapsed();
		self.report_progress(JobProgress::status_msg(status, message));

		let outcome = JobOutcome {
			status,
			elapsed,
			output: None,
			logs: Vec::new(),
		};
		self.worker
			.services
			.persist_finished(&self.job_id, outcome)
			.await?;
		self.finish();

		Ok(())
	}

	/// Mark a job as cancelled
	pub async fn cancel(&self) -> Result<(), JobError> {
		self.fail(JobStatus::Cancelled, "Job was cancelled").await
	}

	/// Unregister the cancellation token and release the running counter exactly once
	fn finish(&self) {
		self.worker.cancellation_tokens.remove(&self.job_id);
		if !self.queue_finished.swap(true, Ordering::AcqRel) {
			self.worker.publish(self.worker.queue.finished(self.kind));
		}
	}
}

impl<X: JobExecutionContext> Drop for JobContext<X> {
	fn drop(&mut self) {
		self.finish();
	}
}
