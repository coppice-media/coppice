//! Job-type-agnostic background job runtime for Stump.
//!
//! The crate owns the queue, the single executor draining it, queue counters, cancellation,
//! the per-job [`JobContext`], the [`JobLifecycle`] contract concrete jobs implement, and the
//! cron [`JobScheduler`]. It never names a concrete job: the host supplies a
//! [`JobExecutionContext`] that provides database and configuration access, an event sink,
//! job-record persistence, and the dispatch table from queued payloads to work.
//!
//! With the `apalis` feature (default) the executor is a single Apalis worker over an in-memory
//! queue; without it an inline executor runs queued jobs serially on the Tokio blocking pool.
//!
//! See `crates/jobs/README.md` for the crate contract and decisions.

#[cfg(feature = "apalis")]
mod apalis_executor;
mod context;
mod error;
mod inline_executor;
mod lifecycle;
mod progress;
mod queue;
mod runtime;
mod scheduler;
mod worker;

pub use context::{JobEvent, JobExecutionContext, JobOutcome, JobPayload};
pub use error::JobError;
pub use lifecycle::{
	run_job, JobExecuteLog, JobLifecycle, JobOutputExt, JobTaskOutput, WorkingState,
};
pub use progress::{JobProgress, JobUpdate};
pub use queue::JobQueueStatus;
pub use runtime::JobRuntime;
pub use scheduler::{JobScheduler, ScheduledJobDispatcher};
pub use worker::JobContext;

#[cfg(test)]
mod tests;
