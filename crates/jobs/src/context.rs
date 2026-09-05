use std::time::Duration;

use async_trait::async_trait;
use models::shared::enums::JobStatus;
use sea_orm::DatabaseConnection;

use crate::{JobContext, JobError, JobExecuteLog, JobQueueStatus, JobUpdate};

/// A serialized job description accepted by the queue.
///
/// The runtime only reads this metadata; the host turns a payload into concrete work in
/// [`JobExecutionContext::run`].
pub trait JobPayload: Send + Sync + Unpin + 'static {
	/// The persisted job name, e.g. `library_scan`
	fn name(&self) -> &'static str;

	/// A human-readable description persisted with the job record
	fn description(&self) -> Option<String>;

	/// The bucket this job is counted under in [`JobQueueStatus`], e.g. `SCAN`
	fn kind(&self) -> &'static str;
}

/// Runtime notifications the host forwards to its own event stream.
#[derive(Debug, Clone)]
pub enum JobEvent<O> {
	/// A job has been dequeued and its record persisted
	Started { id: String },
	/// A progress patch for a running job
	Progress(JobUpdate),
	/// The final output of a completed job
	Output { id: String, output: O },
	/// Queued and running counts changed
	QueueStatus(JobQueueStatus),
}

/// The terminal state of a job as handed to [`JobExecutionContext::persist_finished`]
#[derive(Debug)]
pub struct JobOutcome {
	pub status: JobStatus,
	pub elapsed: Duration,
	/// The JSON-serialized output when the job completed successfully
	pub output: Option<Vec<u8>>,
	/// Logs recorded during execution; empty when the job did not complete
	pub logs: Vec<JobExecuteLog>,
}

/// The host services a job runtime executes against: database and configuration access,
/// the event sink, job-record persistence, and the dispatch table from queued payloads to
/// concrete work. The runtime owns queueing, counters, and cancellation.
#[async_trait]
pub trait JobExecutionContext: Sized + Send + Sync + 'static {
	/// The queued payload type
	type Job: JobPayload;
	/// The configuration view exposed to jobs through [`JobContext::config`]
	type Config: Send + Sync;
	/// The output union reported when a job completes
	type Output: Send;
	/// The host event type every [`JobEvent`] is forwarded as
	type Event: From<JobEvent<Self::Output>> + Send;

	fn conn(&self) -> &DatabaseConnection;

	fn config(&self) -> &Self::Config;

	/// Publish an event to the host's event stream
	fn emit(&self, event: Self::Event);

	/// Persist the running record for a dequeued job before its lifecycle starts
	async fn persist_started(&self, id: &str, job: &Self::Job) -> Result<(), JobError>;

	/// Persist the terminal status, output, and logs of a job
	async fn persist_finished(&self, id: &str, outcome: JobOutcome)
		-> Result<(), JobError>;

	/// Run a dequeued payload to completion under its per-execution context
	async fn run(&self, job: Self::Job, ctx: &JobContext<Self>) -> Result<(), JobError>;
}
