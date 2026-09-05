use std::{collections::VecDeque, fmt::Debug};

use chrono::{DateTime, Utc};
use models::shared::enums::{JobStatus, LogLevel};
use serde::{de, Deserialize, Serialize};

use crate::{JobContext, JobError, JobExecutionContext, JobProgress};

/// A log that will be persisted from a job's execution
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JobExecuteLog {
	pub msg: String,
	pub context: Option<String>,
	pub level: LogLevel,
	pub timestamp: DateTime<Utc>,
}

impl JobExecuteLog {
	/// Construct a [`JobExecuteLog`] with the given msg and level
	pub fn new(msg: String, level: LogLevel) -> Self {
		Self {
			msg,
			context: None,
			level,
			timestamp: Utc::now(),
		}
	}

	/// Construct a [`JobExecuteLog`] with the given msg and [`LogLevel::Error`]
	pub fn error(msg: String) -> Self {
		Self {
			msg,
			context: None,
			level: LogLevel::Error,
			timestamp: Utc::now(),
		}
	}

	/// Construct a [`JobExecuteLog`] with the given msg and [`LogLevel::Warn`]
	pub fn warn(msg: &str) -> Self {
		Self {
			msg: msg.to_string(),
			context: None,
			level: LogLevel::Warn,
			timestamp: Utc::now(),
		}
	}

	/// Construct a new [`JobExecuteLog`] with the given context string
	pub fn with_ctx(self, ctx: String) -> Self {
		Self {
			context: Some(ctx),
			..self
		}
	}
}

/// The working state of a job. This is frequently updated during execution, and is used to track
/// progress internally
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WorkingState<O, T> {
	pub output: Option<O>,
	pub tasks: VecDeque<T>,
	pub logs: Vec<JobExecuteLog>,
}

impl<O, T> Default for WorkingState<O, T> {
	fn default() -> Self {
		Self {
			output: None,
			tasks: VecDeque::new(),
			logs: vec![],
		}
	}
}

/// A trait to extend the output type for a job with a common interface. Job output starts
/// in an 'empty' state (Default) and is frequently updated during execution.
///
/// The state is also serialized and stored in the DB, so it must implement [Serialize] and [`de::DeserializeOwned`].
pub trait JobOutputExt: Serialize + de::DeserializeOwned + Debug {
	/// Update the state with new data. By default, the implementation is a full replacement
	fn update(&mut self, updated: Self) {
		*self = updated;
	}

	/// Serialize the state to JSON. If serialization fails, the error is logged and None is returned.
	fn into_json(self) -> Option<serde_json::Value> {
		serde_json::to_value(&self).map_or_else(
			|error| {
				tracing::error!(?error, job_data = ?self, "Failed to serialize job data!");
				None
			},
			Some,
		)
	}
}

/// A trait that defines the behavior and data types of a job
///
/// The lifecycle is: `init()` → `execute_task()` (loop) → `finalize()`.
#[async_trait::async_trait]
pub trait JobLifecycle: Send + Sync + Sized + 'static {
	const NAME: &'static str;

	/// The host services this job runs against
	type Context: JobExecutionContext;

	/// The output type for the job. This is the data that will be persisted to the DB when the
	/// job completes. All jobs should have a user-friendly representation of their output.
	type Output: Serialize
		+ de::DeserializeOwned
		+ JobOutputExt
		+ Default
		+ Debug
		+ Send
		+ Sync;

	/// The type representing a single task for the job. Each task will be executed
	/// in a loop until all tasks are completed.
	///
	/// If a job should be small enough to not require tasks, this type should be set to `()`.
	/// In that scenario, the job should execute all of its logic in [`JobLifecycle::init`]
	type Task: Serialize + de::DeserializeOwned + Send + Sync;

	/// The description of the job, if any
	fn description(&self) -> Option<String>;

	/// Initialize the job and gather the required tasks
	async fn init(
		&mut self,
		ctx: &JobContext<Self::Context>,
	) -> Result<WorkingState<Self::Output, Self::Task>, JobError>;

	/// Execute a single task. Called repeatedly until all tasks are completed
	async fn execute_task(
		&self,
		ctx: &JobContext<Self::Context>,
		task: Self::Task,
	) -> Result<JobTaskOutput<Self>, JobError>;

	/// Optional finalization after all tasks have completed
	async fn finalize(
		&self,
		_ctx: &JobContext<Self::Context>,
		_output: &Self::Output,
	) -> Result<(), JobError> {
		Ok(())
	}
}

/// The output of a single job task
#[derive(Debug, Serialize)]
pub struct JobTaskOutput<J: JobLifecycle> {
	pub output: J::Output,
	pub subtasks: Vec<J::Task>,
	pub logs: Vec<JobExecuteLog>,
}

/// Run a job through its full lifecycle, persisting the terminal state through the
/// context on every exit path.
pub async fn run_job<J>(ctx: &JobContext<J::Context>, job: &mut J) -> Result<(), JobError>
where
	J: JobLifecycle,
	J::Output: Clone + Into<<J::Context as JobExecutionContext>::Output>,
{
	ctx.report_started();
	ctx.report_progress(JobProgress::status_msg(
		JobStatus::Running,
		"Initializing job",
	));

	let working_state = match job.init(ctx).await {
		Ok(state) => state,
		Err(e) => {
			ctx.fail(JobStatus::Failed, &format!("Init failed: {e}"))
				.await?;
			return Err(e);
		},
	};

	let WorkingState {
		output: initial_output,
		mut tasks,
		mut logs,
	} = working_state;

	let mut output = initial_output.unwrap_or_default();
	let mut completed = 0u64;
	while let Some(task) = tasks.pop_front() {
		if ctx.is_canceled() {
			ctx.cancel().await?;
			return Ok(());
		}

		ctx.report_progress(JobProgress::task_position(
			completed as i32,
			(tasks.len() + 1) as i32,
		));

		match job.execute_task(ctx, task).await {
			Ok(task_output) => {
				output.update(task_output.output);
				logs.extend(task_output.logs);
				for subtask in task_output.subtasks.into_iter().rev() {
					tasks.push_front(subtask);
				}
				completed += 1;
			},
			Err(e) => {
				tracing::error!(error = ?e, job = J::NAME, "Task failed");
				// TODO: Should single task fail entire job? Maybe a fail fast flag?
				ctx.fail(JobStatus::Failed, &format!("Task failed: {e}"))
					.await?;
				return Err(e);
			},
		}
	}

	if let Err(e) = job.finalize(ctx, &output).await {
		tracing::error!(error = ?e, job = J::NAME, "Finalize failed");
		ctx.fail(JobStatus::Failed, &format!("Finalize failed: {e}"))
			.await?;
		return Err(e);
	}

	ctx.report_output(output.clone().into());
	ctx.complete(&output, logs).await
}
