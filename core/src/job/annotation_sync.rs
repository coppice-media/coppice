//! [`AnnotationSyncJob`]: exports one user's annotations to every enabled
//! sink. Queued by the debounce loop and `runAnnotationSync`; see
//! `core/src/annotation_sync.rs` for the host wiring around it.
//!
//! One task per enabled `annotation_sink_configs` row. A failing sink records
//! its error on the row (`last_error`) and in the run output, and the
//! remaining sinks still run; the job then fails in `finalize` so the job
//! record reflects the partial failure.

use std::sync::Arc;

use models::{
	entity::annotation_sink_config,
	shared::enums::{JobStatus, LogLevel},
};
use sea_orm::{prelude::*, sea_query::Expr, QueryFilter};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stump_annotation_sync::{
	build_export_batch, registry, BuildOptions, ExportBatch, SinkState,
};
use stump_api_types::settings::SettingValues;
use stump_jobs::{
	JobContext, JobError, JobExecuteLog, JobLifecycle, JobOutputExt, JobProgress,
	JobTaskOutput, WorkingState,
};

use crate::{
	annotation_sync::decrypt_sink_values, job::JobServices,
	utils::encryption::fetch_encryption_key,
};

/// Per-sink work item for one annotation sync run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationSyncTask {
	pub sink_id: String,
	/// The row's configured values (secret values still encrypted).
	pub settings: Option<Value>,
	/// The row's persisted sink state.
	pub state: Option<Value>,
}

/// Output persisted with the job record when the run completes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct AnnotationSyncOutput {
	pub user_id: String,
	/// Books in the batch handed to every sink.
	pub books: u32,
	pub sinks: Vec<AnnotationSinkRun>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct AnnotationSinkRun {
	pub sink_id: String,
	/// The sink's error message when it failed; the row's `last_error`.
	pub error: Option<String>,
}

impl JobOutputExt for AnnotationSyncOutput {
	fn update(&mut self, updated: Self) {
		self.user_id = updated.user_id;
		self.books = self.books.max(updated.books);
		self.sinks.extend(updated.sinks);
	}
}

/// Exports one user's annotations to every enabled sink.
pub struct AnnotationSyncJob {
	user_id: String,
	batch: Option<ExportBatch>,
	encryption_key: Option<Arc<String>>,
}

impl AnnotationSyncJob {
	pub fn new(user_id: String) -> Self {
		Self {
			user_id,
			batch: None,
			encryption_key: None,
		}
	}
}

#[async_trait::async_trait]
impl JobLifecycle for AnnotationSyncJob {
	const NAME: &'static str = "annotation_sync";

	type Context = JobServices;
	type Output = AnnotationSyncOutput;
	type Task = AnnotationSyncTask;

	fn description(&self) -> Option<String> {
		Some(format!("Export annotations for user {}", self.user_id))
	}

	async fn init(
		&mut self,
		ctx: &JobContext<Self::Context>,
	) -> Result<WorkingState<Self::Output, Self::Task>, JobError> {
		ctx.report_progress(JobProgress::status_msg(
			JobStatus::Running,
			"Loading annotation sinks",
		));

		let rows = annotation_sink_config::Entity::find()
			.filter(annotation_sink_config::Column::UserId.eq(&self.user_id))
			.filter(annotation_sink_config::Column::Enabled.eq(true))
			.all(ctx.conn())
			.await?;

		let mut output = AnnotationSyncOutput {
			user_id: self.user_id.clone(),
			books: 0,
			sinks: Vec::new(),
		};

		if rows.is_empty() {
			return Ok(WorkingState {
				output: Some(output),
				tasks: Default::default(),
				logs: vec![JobExecuteLog::new(
					"No enabled annotation sinks; nothing to export".to_string(),
					LogLevel::Info,
				)],
			});
		}

		// Secrets are only decryptable when the server encryption key exists;
		// sinks without secret settings are unaffected (checked per task).
		self.encryption_key = fetch_encryption_key(ctx.conn()).await.ok().map(Arc::new);

		// Build one batch from the oldest cursor so no sink misses data; every
		// successful sink's cursor advances to the same high-water mark.
		let from_seq = rows
			.iter()
			.map(|row| sink_state(row.state.as_ref()).liseur_seq)
			.min()
			.unwrap_or(0);

		let batch = build_export_batch(
			ctx.conn(),
			&self.user_id,
			BuildOptions {
				liseur_from_seq: from_seq,
			},
		)
		.await
		.map_err(|error| JobError::InitFailed(error.to_string()))?;
		output.books = batch.books.len() as u32;
		self.batch = Some(batch);

		let tasks = rows
			.into_iter()
			.map(|row| AnnotationSyncTask {
				sink_id: row.sink_id,
				settings: row.settings,
				state: row.state,
			})
			.collect();

		Ok(WorkingState {
			output: Some(output),
			tasks,
			logs: Vec::new(),
		})
	}

	async fn execute_task(
		&self,
		ctx: &JobContext<Self::Context>,
		task: Self::Task,
	) -> Result<JobTaskOutput<Self>, JobError> {
		let batch = self.batch.as_ref().ok_or_else(|| {
			JobError::TaskFailed("annotation batch was not initialized".to_string())
		})?;
		let state = sink_state(task.state.as_ref());

		let result = self.run_sink(ctx, batch, &task, &state).await;
		let (new_state, error) = match result {
			Ok(new_state) => (
				SinkState {
					liseur_seq: batch.liseur_high_water,
					data: new_state.data,
				},
				None,
			),
			Err(error) => (state, Some(error)),
		};

		annotation_sink_config::Entity::update_many()
			.filter(annotation_sink_config::Column::UserId.eq(&batch.user_id))
			.filter(annotation_sink_config::Column::SinkId.eq(&task.sink_id))
			.col_expr(
				annotation_sink_config::Column::State,
				Expr::value(
					serde_json::to_value(&new_state)
						.map_err(|error| JobError::TaskFailed(error.to_string()))?,
				),
			)
			.col_expr(
				annotation_sink_config::Column::LastRunAt,
				Expr::value(Some(chrono::Utc::now().fixed_offset())),
			)
			.col_expr(
				annotation_sink_config::Column::LastError,
				Expr::value(error.clone()),
			)
			.exec(ctx.conn())
			.await?;

		let log = match &error {
			None => JobExecuteLog::new(
				format!(
					"Exported {} books to sink {}",
					batch.books.len(),
					task.sink_id
				),
				LogLevel::Info,
			),
			Some(error) => {
				JobExecuteLog::error(format!("Sink {} failed: {error}", task.sink_id))
			},
		};

		Ok(JobTaskOutput {
			output: AnnotationSyncOutput {
				user_id: batch.user_id.clone(),
				books: batch.books.len() as u32,
				sinks: vec![AnnotationSinkRun {
					sink_id: task.sink_id,
					error,
				}],
			},
			subtasks: Vec::new(),
			logs: vec![log],
		})
	}

	async fn finalize(
		&self,
		_ctx: &JobContext<Self::Context>,
		output: &Self::Output,
	) -> Result<(), JobError> {
		let failed: Vec<&str> = output
			.sinks
			.iter()
			.filter(|run| run.error.is_some())
			.map(|run| run.sink_id.as_str())
			.collect();
		if failed.is_empty() {
			Ok(())
		} else {
			Err(JobError::TaskFailed(format!(
				"annotation sink(s) failed: {}",
				failed.join(", ")
			)))
		}
	}
}

impl AnnotationSyncJob {
	/// Resolves the sink for one task and exports the batch through it.
	async fn run_sink(
		&self,
		ctx: &JobContext<JobServices>,
		batch: &ExportBatch,
		task: &AnnotationSyncTask,
		state: &SinkState,
	) -> Result<SinkState, String> {
		let configured: SettingValues = task
			.settings
			.as_ref()
			.and_then(|value| serde_json::from_value(value.clone()).ok())
			.unwrap_or_default();
		let values = decrypt_sink_values(
			&configured,
			self.encryption_key.as_deref().map(String::as_str),
		)
		.map_err(|error| error.to_string())?;

		let root = ctx.config().get_annotation_sync_root();
		let sink = registry::sink(&task.sink_id, &root, &values)
			.map_err(|error| error.to_string())?;
		sink.export(batch, state)
			.await
			.map_err(|error| error.to_string())
	}
}

/// The persisted sink state, or the default when the row has none yet (or
/// it no longer parses).
fn sink_state(value: Option<&Value>) -> SinkState {
	value
		.and_then(|value| serde_json::from_value(value.clone()).ok())
		.unwrap_or_default()
}
