use std::{
	collections::BTreeMap,
	sync::{
		atomic::{AtomicBool, Ordering},
		Arc, Mutex,
	},
	time::Instant,
};

use crate::{
	config::StumpConfig,
	event::{CoreEvent, JobOutput, JobQueueStatus},
	CoreError,
};
use dashmap::DashMap;
use models::entity::{job, log, server_config};
use sea_orm::{
	prelude::*, sea_query::OnConflict, sqlx::types::chrono::Utc, ActiveValue::Set,
	DatabaseConnection, SelectColumns,
};
use serde::Serialize;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use apalis::prelude::{MemoryStorage, MessageQueue};

use super::{
	error::JobError, stump_job::StumpJob, CoreJobOutput, JobExecuteLog, JobOutputExt,
	JobProgress, JobStatus, JobUpdate,
};

#[derive(Debug, Default)]
struct JobQueueCounts {
	queued: BTreeMap<&'static str, i32>,
	running: BTreeMap<&'static str, i32>,
}

/// Tracks queued and running jobs without querying persistence. A snapshot is
/// emitted on every transition so protocol adapters can publish queue status
/// events without waking the worker or touching the database.
#[derive(Clone)]
pub(crate) struct JobQueueState {
	counts: Arc<Mutex<JobQueueCounts>>,
	event_tx: broadcast::Sender<CoreEvent>,
}

impl JobQueueState {
	pub(crate) fn new(event_tx: broadcast::Sender<CoreEvent>) -> Self {
		Self {
			counts: Arc::new(Mutex::new(JobQueueCounts::default())),
			event_tx,
		}
	}

	pub(crate) fn enqueued(&self, job: &StumpJob) {
		self.change(job.name(), |counts, kind| {
			increment(&mut counts.queued, kind);
		});
	}

	pub(crate) fn enqueue_failed(&self, job: &StumpJob) {
		self.change(job.name(), |counts, kind| {
			decrement(&mut counts.queued, kind);
		});
	}

	pub(crate) fn started(&self, job_name: &'static str) {
		self.change(job_name, |counts, kind| {
			decrement(&mut counts.queued, kind);
			increment(&mut counts.running, kind);
		});
	}

	pub(crate) fn finished(&self, job_name: &'static str) {
		self.change(job_name, |counts, kind| {
			decrement(&mut counts.running, kind);
		});
	}

	pub(crate) fn snapshot(&self) -> JobQueueStatus {
		let counts = self.counts.lock().expect("job queue state mutex poisoned");
		let mut count_by_type = BTreeMap::new();
		for map in [&counts.queued, &counts.running] {
			for (&kind, &count) in map {
				if count > 0 {
					*count_by_type.entry(kind.to_owned()).or_insert(0) += count;
				}
			}
		}
		let count = count_by_type.values().copied().sum();
		JobQueueStatus {
			count,
			count_by_type,
		}
	}

	fn change(
		&self,
		job_name: &'static str,
		mut transition: impl FnMut(&mut JobQueueCounts, &'static str),
	) {
		let kind = komga_job_type(job_name);
		let status = {
			let mut counts = self.counts.lock().expect("job queue state mutex poisoned");
			transition(&mut counts, kind);
			let mut count_by_type = BTreeMap::new();
			for map in [&counts.queued, &counts.running] {
				for (&kind, &count) in map {
					if count > 0 {
						*count_by_type.entry(kind.to_owned()).or_insert(0) += count;
					}
				}
			}
			JobQueueStatus {
				count: count_by_type.values().copied().sum(),
				count_by_type,
			}
		};
		let _ = self.event_tx.send(CoreEvent::JobQueueStatus(status));
	}
}

fn increment(map: &mut BTreeMap<&'static str, i32>, kind: &'static str) {
	let count = map.entry(kind).or_insert(0);
	*count = count.saturating_add(1);
}

fn decrement(map: &mut BTreeMap<&'static str, i32>, kind: &'static str) {
	let Some(count) = map.get_mut(kind) else {
		return;
	};
	*count = count.saturating_sub(1);
	if *count == 0 {
		map.remove(kind);
	}
}

fn komga_job_type(job_name: &'static str) -> &'static str {
	match job_name {
		"library_scan" | "series_scan" => "SCAN",
		"analyze_media" => "ANALYZE",
		"metadata_fetch" => "METADATA",
		"thumbnail_generation" | "placeholder_generation" => "THUMBNAIL",
		_ => "OTHER",
	}
}

#[derive(Clone)]
pub struct ApalisWorkerState {
	pub conn: Arc<DatabaseConnection>,
	pub config: Arc<StumpConfig>,
	pub core_event_tx: broadcast::Sender<CoreEvent>,
	pub cancellation_tokens: Arc<DashMap<String, CancellationToken>>,
	pub job_storage: MemoryStorage<StumpJob>,
	pub(crate) queue_state: Arc<JobQueueState>,
}

impl ApalisWorkerState {
	pub fn new(
		conn: Arc<DatabaseConnection>,
		config: Arc<StumpConfig>,
		core_event_tx: broadcast::Sender<CoreEvent>,
		job_storage: MemoryStorage<StumpJob>,
	) -> Self {
		let queue_state = Arc::new(JobQueueState::new(core_event_tx.clone()));
		Self {
			conn,
			config,
			core_event_tx,
			cancellation_tokens: Arc::new(DashMap::new()),
			job_storage,
			queue_state,
		}
	}

	/// Enqueue a job and account for it before the worker can dequeue it.
	pub(crate) async fn enqueue_job(&self, job: StumpJob) -> Result<(), ()> {
		self.queue_state.enqueued(&job);
		let mut storage = self.job_storage.clone();
		if let Err(error) = storage.enqueue(job.clone()).await {
			self.queue_state.enqueue_failed(&job);
			return Err(error);
		}
		Ok(())
	}

	/// Cancel a running job by ID, returning true if a cancellation token was found and cancelled
	pub fn cancel_job(&self, job_id: &str) -> bool {
		if let Some(entry) = self.cancellation_tokens.get(job_id) {
			entry.value().cancel();
			true
		} else {
			false
		}
	}

	/// Cancel all jobs still marked as Running in the DB
	pub async fn cancel_islanded_jobs(&self) -> Result<(), JobError> {
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
}

/// Per-execution context for a specific running job
pub struct JobContext {
	pub job_id: String,
	pub apalis_state: Arc<ApalisWorkerState>,
	pub cancel_token: CancellationToken,
	job_name: &'static str,
	queue_finished: AtomicBool,
	start: Instant,
}

impl JobContext {
	pub async fn new(
		apalis_state: Arc<ApalisWorkerState>,
		job_id: String,
		job: &StumpJob,
	) -> Result<JobContext, JobError> {
		let active_model = job::ActiveModel {
			id: Set(job_id.clone()),
			name: Set(job.name().to_string()),
			description: Set(job.description()),
			status: Set(JobStatus::Running),
			created_at: Set(Utc::now().into()),
			ms_elapsed: Set(0),
			..Default::default()
		};

		job::Entity::insert(active_model)
			.on_conflict(
				OnConflict::column(job::Column::Id)
					.update_column(job::Column::Status)
					.to_owned(),
			)
			.exec_without_returning(apalis_state.conn.as_ref())
			.await?;

		let cancel_token = CancellationToken::new();
		apalis_state
			.cancellation_tokens
			.insert(job_id.clone(), cancel_token.clone());
		apalis_state.queue_state.started(job.name());

		Ok(JobContext {
			job_id,
			apalis_state,
			cancel_token,
			job_name: job.name(),
			queue_finished: AtomicBool::new(false),
			start: Instant::now(),
		})
	}

	fn finish_queue(&self) {
		if !self.queue_finished.swap(true, Ordering::AcqRel) {
			self.apalis_state.queue_state.finished(self.job_name);
		}
	}

	/// Check if this job has been canceled by looking up its cancellation token
	pub fn is_canceled(&self) -> bool {
		self.cancel_token.is_cancelled()
	}

	/// Sends an event to the core event channel
	pub fn emit_event(&self, event: CoreEvent) {
		if let Err(e) = self.apalis_state.core_event_tx.send(event) {
			tracing::error!(error = ?e, "Failed to emit core event");
		}
	}

	/// Sends a [`JobProgress`] update event to the core event channel
	pub fn report_progress(&self, progress: JobProgress) {
		self.emit_event(CoreEvent::JobUpdate(JobUpdate {
			id: self.job_id.clone(),
			payload: progress,
		}));
	}

	/// Get a reference to the database connection from the worker state
	pub fn conn(&self) -> &DatabaseConnection {
		self.apalis_state.conn.as_ref()
	}

	/// Get a reference to the config from the worker state
	pub fn config(&self) -> &StumpConfig {
		self.apalis_state.config.as_ref()
	}

	/// A convenience method to fetch the encryption key from the server config
	pub async fn get_encryption_key(&self) -> Result<String, CoreError> {
		let record = server_config::Entity::find()
			.select_column(server_config::Column::EncryptionKey)
			.one(self.apalis_state.conn.as_ref())
			.await?;

		let encryption_key = record
			.and_then(|config| config.encryption_key)
			.ok_or(CoreError::EncryptionKeyNotSet)?;

		Ok(encryption_key)
	}

	/// Send a [`JobOutput`] event to the core event channel with the given output data
	pub fn report_output(&self, output: CoreJobOutput) {
		let event = CoreEvent::JobOutput(JobOutput {
			id: self.job_id.clone(),
			output,
		});
		self.emit_event(event);
	}

	/// A convenience method to take the outputs and logs of a job and persist them into the database
	pub async fn complete<O: Serialize + JobOutputExt + std::fmt::Debug>(
		&self,
		output: &O,
		logs: Vec<JobExecuteLog>,
	) -> Result<(), JobError> {
		let elapsed = self.start.elapsed();
		self.report_progress(JobProgress::finished());

		if !logs.is_empty() {
			let models = logs.into_iter().map(|l| log::ActiveModel {
				job_id: Set(Some(self.job_id.clone())),
				message: Set(l.msg),
				level: Set(l.level),
				timestamp: Set(l.timestamp.into()),
				context: Set(l.context),
				..Default::default()
			});
			log::Entity::insert_many(models).exec(self.conn()).await?;
		}

		let output_data = serde_json::to_vec(output).ok();

		job::Entity::update_many()
			.filter(job::Column::Id.eq(&self.job_id))
			.col_expr(job::Column::OutputData, Expr::value(output_data))
			.col_expr(
				job::Column::Status,
				Expr::value(JobStatus::Completed.to_string()),
			)
			.col_expr(
				job::Column::MsElapsed,
				Expr::value(elapsed.as_millis() as i64),
			)
			.col_expr(
				job::Column::CompletedAt,
				Expr::value(Some(Utc::now().fixed_offset())),
			)
			.exec(self.conn())
			.await?;

		self.apalis_state.cancellation_tokens.remove(&self.job_id);
		self.finish_queue();

		Ok(())
	}

	/// A convenience method to mark a job as failed with a given status and message
	pub async fn fail(&self, status: JobStatus, message: &str) -> Result<(), JobError> {
		let elapsed = self.start.elapsed();
		self.report_progress(JobProgress::status_msg(status, message));

		job::Entity::update_many()
			.filter(job::Column::Id.eq(&self.job_id))
			.col_expr(job::Column::Status, Expr::value(status.to_string()))
			.col_expr(
				job::Column::MsElapsed,
				Expr::value(elapsed.as_millis() as i64),
			)
			.col_expr(
				job::Column::CompletedAt,
				Expr::value(Some(Utc::now().fixed_offset())),
			)
			.exec(self.conn())
			.await?;

		self.apalis_state.cancellation_tokens.remove(&self.job_id);
		self.finish_queue();

		Ok(())
	}

	/// A convenience method to mark a job as cancelled
	pub async fn cancel(&self) -> Result<(), JobError> {
		self.fail(JobStatus::Cancelled, "Job was cancelled").await
	}

	/// A convenience method to enqueue a follow-up job from this job's execution
	pub async fn enqueue(&self, job: StumpJob) -> Result<(), JobError> {
		self.apalis_state.enqueue_job(job).await.map_err(|_| {
			JobError::Unknown("Failed to enqueue follow-up job!".to_string())
		})?;
		Ok(())
	}
}

impl Drop for JobContext {
	fn drop(&mut self) {
		if !self.queue_finished.swap(true, Ordering::AcqRel) {
			self.apalis_state.queue_state.finished(self.job_name);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn queue_state_reports_queued_running_and_zero_transitions() {
		let (event_tx, mut events) = broadcast::channel(8);
		let state = JobQueueState::new(event_tx);
		let job =
			StumpJob::library_scan("library-1".to_owned(), "/library".to_owned(), None);

		state.enqueued(&job);
		assert_eq!(
			state.snapshot(),
			JobQueueStatus {
				count: 1,
				count_by_type: BTreeMap::from([("SCAN".to_owned(), 1)]),
			}
		);
		assert!(matches!(
			events.try_recv().expect("enqueue event"),
			CoreEvent::JobQueueStatus(JobQueueStatus { count: 1, .. })
		));

		state.started(job.name());
		assert_eq!(
			state.snapshot(),
			JobQueueStatus {
				count: 1,
				count_by_type: BTreeMap::from([("SCAN".to_owned(), 1)]),
			}
		);
		assert!(matches!(
			events.try_recv().expect("start event"),
			CoreEvent::JobQueueStatus(JobQueueStatus { count: 1, .. })
		));

		state.finished(job.name());
		assert_eq!(
			state.snapshot(),
			JobQueueStatus {
				count: 0,
				count_by_type: BTreeMap::new(),
			}
		);
		assert!(matches!(
			events.try_recv().expect("finish event"),
			CoreEvent::JobQueueStatus(JobQueueStatus { count: 0, .. })
		));
	}
}
