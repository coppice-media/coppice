//! Annotation export wiring: the debounce hub, the [`AnnotationSyncJob`],
//! and the settings-value encryption helpers.
//!
//! The canonical model and the concrete sinks live in
//! `stump_annotation_sync`; this module hosts them onto Stump's job runtime:
//!
//! - every accepted annotation or reading-head change calls
//!   [`Ctx::note_annotation_activity`], which (re)arms a per-user deadline in
//!   the [`AnnotationSyncDebouncer`];
//! - a daemon loop (spawned by `StumpCore`) drains due users and enqueues
//!   [`StumpJob::AnnotationSync`];
//! - the job loads the user's enabled `annotation_sink_configs` rows, builds
//!   the canonical batch (incremental by liseur CAS `seq`, full rebuild for
//!   native books), and runs each enabled sink, persisting the sink state and
//!   the last-run/last-error bookkeeping.
//!
//! Annotation changes deliberately do not become `CoreEvent`s: that stream is
//! broadcast to every GraphQL subscriber regardless of user (the same reason
//! reading heads use the separate `ReadingHeadChanged` channel).

use std::{
	collections::HashMap,
	sync::{Arc, Mutex},
	time::{Duration, Instant},
};

use models::entity::annotation_sink_config;
use sea_orm::{prelude::*, ActiveValue::Set, QueryFilter};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stump_annotation_sync::{sink::SinkState, BuildOptions, ExportBatch};
use stump_api_types::settings::{SettingDefinition, SettingValues};
use stump_jobs::{
	JobContext, JobError, JobExecuteLog, JobLifecycle, JobOutputExt, JobTaskOutput,
	WorkingState,
};

use crate::{
	config::StumpConfig,
	job::{stump_job::StumpJob, JobServices},
	utils::encryption::{decrypt_string, encrypt_string, fetch_encryption_key},
	CoreResult, Ctx,
};

/// JSON marker wrapping encrypted secret setting values.
const ENCRYPTED_MARKER: &str = "__stump_encrypted";

// ---------------------------------------------------------------------------
// Debouncer
// ---------------------------------------------------------------------------

/// Per-user debounce deadlines for annotation export runs. An export is
/// scheduled once the user has been quiet for the configured delay; further
/// activity keeps pushing the deadline out.
pub struct AnnotationSyncDebouncer {
	deadlines: Mutex<HashMap<String, Instant>>,
	delay: Duration,
}

impl AnnotationSyncDebouncer {
	pub fn new(secs: u64) -> Self {
		Self {
			deadlines: Mutex::new(HashMap::new()),
			delay: Duration::from_secs(secs),
		}
	}

	/// (Re)arms the deadline for `user_id`.
	pub fn note(&self, user_id: &str) {
		let mut deadlines = self
			.deadlines
			.lock()
			.expect("annotation sync debounce poisoned");
		deadlines.insert(user_id.to_owned(), Instant::now() + self.delay);
	}

	/// Removes and returns every user whose deadline has passed.
	pub fn take_due(&self, now: Instant) -> Vec<String> {
		let mut deadlines = self
			.deadlines
			.lock()
			.expect("annotation sync debounce poisoned");
		let due: Vec<String> = deadlines
			.iter()
			.filter(|(_, deadline)| **deadline <= now)
			.map(|(user_id, _)| user_id.clone())
			.collect();
		for user_id in &due {
			deadlines.remove(user_id);
		}
		due.sort();
		due
	}

	/// Whether an export is currently scheduled for `user_id`.
	pub fn is_pending(&self, user_id: &str) -> bool {
		self.deadlines
			.lock()
			.expect("annotation sync debounce poisoned")
			.contains_key(user_id)
	}
}

/// Spawns the daemon loop that turns due debounce deadlines into
/// [`StumpJob::AnnotationSync`] enqueues. Called from `StumpCore`; harmless to
/// call without a running Tokio runtime (it logs and returns).
pub fn spawn_debounce_loop(ctx: Ctx) {
	if tokio::runtime::Handle::try_current().is_err() {
		tracing::debug!("annotation sync debounce loop not started: no tokio runtime");
		return;
	}

	tokio::spawn(async move {
		let mut interval = tokio::time::interval(Duration::from_secs(1));
		interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			interval.tick().await;
			if !ctx.background_jobs_enabled() {
				continue;
			}
			for user_id in ctx.annotation_debounce.take_due(Instant::now()) {
				tracing::debug!(user_id = %user_id, "Annotation sync debounce elapsed");
				if let Err(error) = ctx.enqueue(StumpJob::AnnotationSync { user_id }).await {
					tracing::error!(error = ?error, "Failed to enqueue annotation sync");
				}
			}
		}
	});
}

// ---------------------------------------------------------------------------
// Settings-value encryption helpers (host side; sinks see plaintext)
// ---------------------------------------------------------------------------

/// Encrypts every secret-marked value in `values` with the server encryption
/// key. Encrypted values are stored as `{"__stump_encrypted": "<base64>"}`.
pub fn encrypt_sink_values(
	definitions: &[SettingDefinition],
	values: &SettingValues,
	encryption_key: &str,
) -> CoreResult<SettingValues> {
	values
		.iter()
		.map(|(key, value)| {
			let is_secret = definitions
				.iter()
				.any(|definition| definition.key == key && definition.secret);
			if !is_secret {
				return Ok((key.clone(), value.clone()));
			}
			let Some(plain) = value.as_str() else {
				return Ok((key.clone(), value.clone()));
			};
			let encrypted = encrypt_string(plain, &encryption_key.to_owned())?;
			Ok((
				key.clone(),
				serde_json::json!({ ENCRYPTED_MARKER: encrypted }),
			))
		})
		.collect()
}

/// Reverses [`encrypt_sink_values`]. Values without the marker pass through.
pub fn decrypt_sink_values(
	values: &SettingValues,
	encryption_key: &str,
) -> CoreResult<SettingValues> {
	values
		.iter()
		.map(|(key, value)| {
			let Some(marker_value) = value.get(ENCRYPTED_MARKER).and_then(Value::as_str) else {
				return Ok((key.clone(), value.clone()));
			};
			let plain = decrypt_string(marker_value, &encryption_key.to_owned())?;
			Ok((key.clone(), Value::String(plain)))
		})
		.collect()
}

// ---------------------------------------------------------------------------
// Job
// ---------------------------------------------------------------------------

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
	pub books: usize,
	pub sinks: Vec<AnnotationSinkRun>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct AnnotationSinkRun {
	pub sink_id: String,
	pub books: usize,
	pub error: Option<String>,
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
		ctx.report_progress(stump_jobs::JobProgress::status_msg(
			models::shared::enums::JobStatus::Running,
			"Loading annotation sinks",
		));

		// Secrets are only decryptable when the server encryption key exists;
		// sinks without secret settings are unaffected.
		self.encryption_key = match fetch_encryption_key(ctx.conn()).await {
			Ok(key) => Some(Arc::new(key)),
			Err(_) => None,
		};

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
				logs: Vec::new(),
			});
		}

		// Build one batch from the oldest cursor so no sink misses data; the
		// cursor in each row's state advances to the same high-water mark.
		let from_seq = rows
			.iter()
			.map(|row| {
				row.state
					.as_ref()
					.and_then(|state| serde_json::from_value::<SinkState>(state.clone()).ok())
					.map(|state| state.liseur_seq)
					.unwrap_or(0)
			})
			.min()
			.unwrap_or(0);

		let batch = stump_annotation_sync::build_export_batch(
			ctx.conn(),
			&self.user_id,
			BuildOptions {
				liseur_from_seq: from_seq,
			},
		)
		.await
		.map_err(|error| JobError::Unknown(error.to_string()))?;
		output.books = batch.books.len();
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
		let batch = self
			.batch
			.as_ref()
			.ok_or_else(|| JobError::TaskFailed("annotation batch was not initialized".to_string()))?;

		let definitions = stump_annotation_sync::registry::catalog()
			.into_iter()
			.find(|descriptor| descriptor.id == task.sink_id)
			.ok_or_else(|| JobError::TaskFailed(format!("unknown annotation sink {}", task.sink_id)))?
			.settings
			.to_vec();

		let configured: SettingValues = task
			.settings
			.clone()
			.and_then(|value| serde_json::from_value::<SettingValues>(value).ok())
			.unwrap_or_default();
		let values = match &self.encryption_key {
			Some(key) => crate::annotation_sync::decrypt_sink_values(&configured, key)
				.map_err(|error| JobError::Unknown(error.to_string()))?,
			None => configured,
		};

		let state: SinkState = task
			.state
			.clone()
			.and_then(|value| serde_json::from_value(value).ok())
			.unwrap_or_default();

		let root = ctx.config().get_annotation_sync_root();
		let sink = stump_annotation_sync::registry::sink(&task.sink_id, &root, &values)
			.map_err(|error| JobError::Unknown(error.to_string()))?;

		let result = sink.export(batch, &state).await;
		let new_state = match &result {
			Ok(new_state) => SinkState {
				liseur_seq: batch.liseur_high_water,
				data: new_state.data.clone(),
			},
			Err(_) => state.clone(),
		};

		let (last_error, error_for_output) = match &result {
			Ok(_) => (None, None),
			Err(error) => (Some(error.to_string()), Some(error.to_string())),
		};

		annotation_sink_config::Entity::update_many()
			.filter(annotation_sink_config::Column::UserId.eq(&batch.user_id))
			.filter(annotation_sink_config::Column::SinkId.eq(&task.sink_id))
			.col_expr(
				annotation_sink_config::Column::State,
				sea_orm::sea_query::Expr::value(
					serde_json::to_value(&new_state)
						.map_err(|error| JobError::Unknown(error.to_string()))?,
				),
			)
			.col_expr(
				annotation_sink_config::Column::LastRunAt,
				sea_orm::sea_query::Expr::value(chrono::Utc::now().fixed_offset()),
			)
			.col_expr(
				annotation_sink_config::Column::LastError,
				sea_orm::sea_query::Expr::value(last_error),
			)
			.exec(ctx.conn())
			.await?;

		match result {
			Ok(_) => Ok(JobTaskOutput {
				output: AnnotationSyncOutput {
					user_id: batch.user_id.clone(),
					books: batch.books.len(),
					sinks: vec![AnnotationSinkRun {
						sink_id: task.sink_id.clone(),
						books: batch.books.len(),
						error: None,
					}],
				},
				subtasks: Vec::new(),
				logs: vec![JobExecuteLog::new(
					format!("Exported {} books to sink {}", batch.books.len(), task.sink_id),
					models::shared::enums::LogLevel::Info,
				)],
			}),
			Err(error) => Err(JobError::Unknown(error.to_string())),
		}
	}
}

/// Resolves the export root a sink would use for `user_id`; exposed for the
/// GraphQL status surface.
pub fn sink_root(config: &StumpConfig) -> std::path::PathBuf {
	config.get_annotation_sync_root()
}

/// Result of resolving a sink config row for status rendering.
pub struct SinkStatusRow {
	pub sink_id: String,
	pub enabled: bool,
	pub last_run_at: Option<sea_orm::prelude::DateTimeWithTimeZone>,
	pub last_error: Option<String>,
}

/// Loads the per-sink status rows for a user (enabled and disabled).
pub async fn sink_status_rows(
	conn: &DatabaseConnection,
	user_id: &str,
) -> CoreResult<Vec<SinkStatusRow>> {
	let rows = annotation_sink_config::Entity::find()
		.filter(annotation_sink_config::Column::UserId.eq(user_id))
		.all(conn)
		.await?;
	Ok(rows
		.into_iter()
		.map(|row| SinkStatusRow {
			sink_id: row.sink_id,
			enabled: row.enabled,
			last_run_at: row.last_run_at,
			last_error: row.last_error,
		})
		.collect())
}

/// Upserts one sink config row for a user. Values must already be encrypted.
pub async fn upsert_sink_config(
	conn: &DatabaseConnection,
	user_id: &str,
	sink_id: &str,
	settings: Option<Value>,
	enabled: bool,
) -> CoreResult<()> {
	let existing = annotation_sink_config::Entity::find()
		.filter(annotation_sink_config::Column::UserId.eq(user_id))
		.filter(annotation_sink_config::Column::SinkId.eq(sink_id))
		.one(conn)
		.await?;

	if let Some(row) = existing {
		let mut active: annotation_sink_config::ActiveModel = row.into();
		active.settings = Set(settings);
		active.enabled = Set(enabled);
		active.updated_at = Set(chrono::Utc::now().fixed_offset());
		active.update(conn).await?;
	} else {
		let active = annotation_sink_config::ActiveModel {
			user_id: Set(user_id.to_owned()),
			sink_id: Set(sink_id.to_owned()),
			settings: Set(settings),
			enabled: Set(enabled),
			last_run_at: Set(None),
			last_error: Set(None),
			state: Set(None),
			updated_at: Set(chrono::Utc::now().fixed_offset()),
		};
		active.insert(conn).await?;
	}

	Ok(())
}

/// Merges per-sink task outputs into the run output; the run-level book
/// count comes from the batch built in `init`.
impl JobOutputExt for AnnotationSyncOutput {
	fn update(&mut self, updated: Self) {
		self.user_id = updated.user_id;
		self.books = self.books.max(updated.books);
		self.sinks.extend(updated.sinks);
	}
}
