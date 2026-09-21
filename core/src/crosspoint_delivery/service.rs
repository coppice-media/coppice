//! Durable CrossPoint delivery queue execution.
//!
//! The service is intentionally a single in-process coordinator.  It keeps one
//! semaphore per device (so a target receives one transfer at a time), while
//! independent devices can progress concurrently.  Queue and attempt rows are
//! the source of truth; a restart moves interrupted work back to `QUEUED` and
//! every upload is rebuilt from the immutable source/profile snapshots.

use std::{
	collections::HashMap,
	path::{Path, PathBuf},
	str::FromStr,
	sync::{
		atomic::{AtomicBool, Ordering as AtomicOrdering},
		Arc,
	},
	time::Duration as StdDuration,
};

use chrono::{DateTime, Duration, Utc};
use models::entity::{
	crosspoint_delivery_attempt, crosspoint_delivery_queue, crosspoint_device_target,
	media,
};
use ring::digest::{Context, SHA256};
use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::{sea_query::Expr, Condition, DbErr, Order};
use sea_orm::{
	ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
	QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use stump_crosspoint::{
	profile::{
		CrosspointProfileError, CrosspointTargetModel, CrosspointTransferProfile,
		CrosspointTransferProfileInput,
	},
	storage::{
		DeliveryStatus, TargetFingerprint, CROSSPOINT_HTTP_PORT, CROSSPOINT_WS_PORT,
	},
	transfer::{
		CrossPointClient, CrossPointEndpoint, CrossPointError, CrossPointIdentity,
		CrossPointModel, TransferConfig,
	},
};
use stump_media::{
	generate,
	transform::{EpubDeviceTarget, EpubOptimizer, EpubOptimizerOptions},
};
use thiserror::Error;
use tokio::{
	fs,
	sync::{Mutex, Semaphore},
	task::JoinHandle,
	time::sleep,
};
use uuid::Uuid;
const DEFAULT_MAX_ATTEMPTS: u32 = 4;
const DEFAULT_RETRY_DELAY_SECS: u64 = 2;
const DEFAULT_POLL_INTERVAL_SECS: u64 = 2;
const MAX_DUE_ROWS: u64 = 64;
const DELIVERY_CACHE_DIR: &str = "crosspoint-delivery";

/// The immutable source state captured when a delivery is queued.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRevision {
	pub path: String,
	pub hash: String,
	pub filename: String,
	pub bytes: u64,
}

impl SourceRevision {
	/// Build a source snapshot from the currently indexed media row.
	pub async fn from_media(media: &media::Model) -> Result<Self, DeliveryError> {
		let path = media.path.clone();
		let bytes = u64::try_from(media.size).map_err(|_| {
			DeliveryError::InvalidSource(format!(
				"media {} has a negative size",
				media.id
			))
		})?;
		let filename = Path::new(&media.path)
			.file_name()
			.and_then(|name| name.to_str())
			.ok_or_else(|| {
				DeliveryError::InvalidSource(format!(
					"media {} has no safe filename",
					media.id
				))
			})?
			.to_string();
		let hash = match media.hash.clone().filter(|hash| !hash.trim().is_empty()) {
			Some(hash) => hash,
			None => sampled_hash(path.clone(), bytes).await?,
		};
		Ok(Self {
			path,
			hash,
			filename,
			bytes,
		})
	}

	fn parse(value: &str) -> Result<Self, DeliveryError> {
		let revision: Self = serde_json::from_str(value).map_err(|error| {
			DeliveryError::InvalidSource(format!("invalid source revision: {error}"))
		})?;
		revision.validate()?;
		Ok(revision)
	}

	fn validate(&self) -> Result<(), DeliveryError> {
		if self.path.trim().is_empty()
			|| self.hash.trim().is_empty()
			|| self.filename.is_empty()
		{
			return Err(DeliveryError::InvalidSource(
				"source revision requires path, hash, and filename".to_string(),
			));
		}
		validate_single_component(&self.filename, "source filename")?;
		Ok(())
	}

	fn canonical_json(&self) -> Result<String, DeliveryError> {
		serde_json::to_string(self).map_err(DeliveryError::Json)
	}
}

/// Normalize a profile patch into the bounded defaults used by target
/// settings.  The GraphQL layer can expose this pure operation without
/// constructing a queue worker.
pub fn normalize_transfer_profile(
	input: CrosspointTransferProfileInput,
) -> Result<CrosspointTransferProfile, DeliveryError> {
	input.normalized().map_err(|error: CrosspointProfileError| {
		DeliveryError::InvalidProfile(error.to_string())
	})
}

/// Convert a normalized profile into the EPUB optimizer options used for one
/// target.  Transfer controls stay in the queue snapshot and are not silently
/// discarded.
fn optimizer_options(profile: &CrosspointTransferProfile) -> EpubOptimizerOptions {
	EpubOptimizerOptions {
		enabled: profile.optimizer_enabled,
		device_target: match profile.target_model {
			CrosspointTargetModel::Auto => EpubDeviceTarget::Auto,
			CrosspointTargetModel::X3 => EpubDeviceTarget::X3,
			CrosspointTargetModel::X4 => EpubDeviceTarget::X4,
		},
		jpeg_quality: profile.jpeg_quality,
		grayscale: profile.grayscale,
		auto_crop: profile.auto_crop,
		split_text: profile.split_large_paragraphs,
		remove_fonts: profile.remove_fonts,
	}
}

fn transfer_config(profile: &CrosspointTransferProfile) -> TransferConfig {
	TransferConfig {
		timeout: StdDuration::from_secs(profile.timeout_seconds),
		chunk_size: profile.chunk_bytes as usize,
		max_upload_bytes: profile.max_upload_bytes,
	}
}

fn resolve_profile(
	profile: CrosspointTransferProfile,
	model: CrossPointModel,
) -> Result<CrosspointTransferProfile, DeliveryError> {
	profile
		.resolve_for_model(model)
		.map_err(DeliveryError::InvalidProfile)
}

fn profile_from_target(
	target: &crosspoint_device_target::Model,
) -> Result<CrosspointTransferProfile, DeliveryError> {
	let profile: CrosspointTransferProfile =
		serde_json::from_value(target.profile_json.clone()).map_err(|error| {
			DeliveryError::InvalidProfile(format!("invalid target profile: {error}"))
		})?;
	let profile = profile
		.validate()
		.map_err(|error| DeliveryError::InvalidProfile(error.to_string()))?;
	if profile.digest() != target.profile_digest {
		return Err(DeliveryError::InvalidTarget(
			"target profile digest does not match saved profile".to_string(),
		));
	}
	Ok(profile)
}

/// Timing and retry policy for the durable queue.
#[derive(Clone, Debug)]
pub struct DeliveryConfig {
	pub max_attempts: u32,
	pub retry_delay: Duration,
	pub poll_interval: StdDuration,
	pub transfer: TransferConfig,
}

impl Default for DeliveryConfig {
	fn default() -> Self {
		Self {
			max_attempts: DEFAULT_MAX_ATTEMPTS,
			retry_delay: Duration::seconds(DEFAULT_RETRY_DELAY_SECS as i64),
			poll_interval: StdDuration::from_secs(DEFAULT_POLL_INTERVAL_SECS),
			transfer: TransferConfig::default(),
		}
	}
}

impl DeliveryConfig {
	pub fn validated(mut self) -> Result<Self, DeliveryError> {
		if self.max_attempts == 0 {
			return Err(DeliveryError::InvalidConfig(
				"max_attempts must be greater than zero".to_string(),
			));
		}
		if self.retry_delay < Duration::zero() {
			return Err(DeliveryError::InvalidConfig(
				"retry_delay cannot be negative".to_string(),
			));
		}
		if self.poll_interval.is_zero() {
			return Err(DeliveryError::InvalidConfig(
				"poll_interval must be greater than zero".to_string(),
			));
		}
		self.transfer = self.transfer.validated().map_err(DeliveryError::Transfer)?;
		Ok(self)
	}
}

#[derive(Debug, Error)]
pub enum DeliveryError {
	#[error("CrossPoint delivery database error: {0}")]
	Database(#[from] DbErr),
	#[error("CrossPoint delivery I/O error: {0}")]
	Io(#[from] std::io::Error),
	#[error("CrossPoint delivery JSON error: {0}")]
	Json(#[from] serde_json::Error),
	#[error("CrossPoint transfer failed: {0}")]
	Transfer(#[from] CrossPointError),
	#[error("CrossPoint EPUB optimization failed: {0}")]
	Optimization(String),
	#[error("invalid CrossPoint delivery source: {0}")]
	InvalidSource(String),
	#[error("invalid CrossPoint delivery target: {0}")]
	InvalidTarget(String),
	#[error("invalid CrossPoint delivery profile: {0}")]
	InvalidProfile(String),
	#[error("invalid CrossPoint delivery configuration: {0}")]
	InvalidConfig(String),
	#[error("CrossPoint delivery worker failed: {0}")]
	Worker(String),
}

/// Outcome of a completed local/remote transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryOutcome {
	pub queue_id: String,
	pub bytes: u64,
	pub attempts: u32,
}

#[derive(Clone, Debug)]
struct PreparedSource {
	path: PathBuf,
	bytes: u64,
}

#[derive(Clone)]
pub struct CrossPointDeliveryService {
	conn: Arc<DatabaseConnection>,
	cache_dir: PathBuf,
	config: DeliveryConfig,
	device_gates: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
	running: Arc<AtomicBool>,
}

impl CrossPointDeliveryService {
	/// Construct a queue service.  `cache_dir` should be Stump's transform
	/// cache root; staged files are kept below a CrossPoint-specific child.
	pub fn new(
		conn: Arc<DatabaseConnection>,
		cache_dir: impl Into<PathBuf>,
		config: DeliveryConfig,
	) -> Result<Self, DeliveryError> {
		let config = config.validated()?;
		Ok(Self {
			conn,
			cache_dir: cache_dir.into().join(DELIVERY_CACHE_DIR),
			config,
			device_gates: Arc::new(Mutex::new(HashMap::new())),
			running: Arc::new(AtomicBool::new(true)),
		})
	}

	pub fn config(&self) -> &DeliveryConfig {
		&self.config
	}

	/// Recover interrupted rows after process startup.  Rows that exhausted
	/// their retry budget remain failed; rows with budget left are queued now.
	pub async fn recover_inflight(&self) -> Result<u64, DeliveryError> {
		let rows = crosspoint_delivery_queue::Entity::find()
			.filter(
				Condition::any()
					.add(
						crosspoint_delivery_queue::Column::Status
							.eq(DeliveryStatus::Preparing.as_str()),
					)
					.add(
						crosspoint_delivery_queue::Column::Status
							.eq(DeliveryStatus::Transferring.as_str()),
					),
			)
			.all(self.conn.as_ref())
			.await?;
		let mut recovered = 0;
		for row in rows {
			let now = Utc::now().fixed_offset();
			if row.attempts >= row.max_attempts {
				self.update_queue(
					&row.id,
					DeliveryStatus::Failed,
					None,
					Some(
						"delivery worker restarted after retry budget was exhausted"
							.to_string(),
					),
					Some(now),
					None,
				)
				.await?;
			} else {
				self.update_queue(
					&row.id,
					DeliveryStatus::Queued,
					Some(now),
					Some("delivery recovered after worker restart".to_string()),
					None,
					None,
				)
				.await?;
			}
			recovered += 1;
		}
		Ok(recovered)
	}

	/// Start the durable polling loop.  Call [`Self::stop`] during shutdown.
	pub fn start(self: Arc<Self>) -> JoinHandle<()> {
		tokio::spawn(async move {
			if let Err(error) = self.recover_inflight().await {
				tracing::error!(error = %error, "failed to recover CrossPoint deliveries");
			}
			while self.running.load(AtomicOrdering::Acquire) {
				match self.process_due().await {
					Ok(0) => sleep(self.config.poll_interval).await,
					Ok(_) => tokio::task::yield_now().await,
					Err(error) => {
						tracing::error!(error = %error, "CrossPoint delivery poll failed");
						sleep(self.config.poll_interval).await;
					},
				}
			}
		})
	}

	pub fn stop(&self) {
		self.running.store(false, AtomicOrdering::Release);
	}

	/// Queue one media item.  This is the same path used by one-book, bulk,
	/// and dedicated-device sends, so all callers get the same idempotency and
	/// immutable snapshot behavior.
	pub async fn enqueue_for_media(
		&self,
		user_id: impl Into<String>,
		device_id: impl Into<String>,
		media_id: impl Into<String>,
		destination_path: &str,
		profile_input: Option<CrosspointTransferProfileInput>,
	) -> Result<crosspoint_delivery_queue::Model, DeliveryError> {
		let user_id = user_id.into();
		let device_id = device_id.into();
		let media_id = media_id.into();
		let media = media::Entity::find_by_id(&media_id)
			.one(self.conn.as_ref())
			.await?
			.ok_or_else(|| {
				DeliveryError::InvalidSource(format!("media {media_id} was not found"))
			})?;
		let target = crosspoint_device_target::Entity::find_by_id(&device_id)
			.one(self.conn.as_ref())
			.await?
			.ok_or_else(|| {
				DeliveryError::InvalidTarget(format!("device {device_id} was not found"))
			})?;
		if target.user_id != user_id {
			return Err(DeliveryError::InvalidTarget(
				"target does not belong to the requesting user".to_string(),
			));
		}
		let (identity, _) = verified_target(&target)?;
		let profile = match profile_input {
			Some(input) => normalize_transfer_profile(input)?,
			None => profile_from_target(&target)?,
		};
		let profile = resolve_profile(profile, identity.model)?;
		let profile_digest = profile.digest();
		let source = SourceRevision::from_media(&media).await?;
		let source_revision = source.canonical_json()?;
		let profile_json = serde_json::to_string(&profile)?;
		let destination_path = compose_remote_path(&target.root_path, destination_path)?;
		let idempotency_key = idempotency_key(
			&user_id,
			&device_id,
			&media_id,
			&source.hash,
			&profile_digest,
			&destination_path,
		);
		let now = Utc::now().fixed_offset();
		let row = crosspoint_delivery_queue::ActiveModel {
			id: Set(Uuid::new_v4().to_string()),
			user_id: Set(user_id),
			device_id: Set(device_id),
			media_id: Set(media_id),
			source_revision: Set(source_revision),
			profile_digest: Set(profile_digest),
			profile_json: Set(profile_json),
			destination_path: Set(destination_path),
			idempotency_key: Set(idempotency_key.clone()),
			status: Set(DeliveryStatus::Queued.as_str().to_string()),
			attempts: Set(0),
			max_attempts: Set(profile.max_attempts() as i32),
			next_attempt_at: Set(None),
			last_error: Set(None),
			queued_at: Set(now),
			started_at: Set(None),
			completed_at: Set(None),
			updated_at: Set(now),
		};
		crosspoint_delivery_queue::Entity::insert(row)
			.on_conflict(
				sea_orm::sea_query::OnConflict::column(
					crosspoint_delivery_queue::Column::IdempotencyKey,
				)
				.do_nothing()
				.to_owned(),
			)
			.exec(self.conn.as_ref())
			.await?;
		crosspoint_delivery_queue::Entity::find()
			.filter(crosspoint_delivery_queue::Column::IdempotencyKey.eq(idempotency_key))
			.one(self.conn.as_ref())
			.await?
			.ok_or_else(|| {
				DeliveryError::Worker(
					"queue insert succeeded but row was not found".to_string(),
				)
			})
	}

	/// Process all currently due rows.  Device semaphores make this parallel
	/// across devices and serial within each device.
	pub async fn process_due(&self) -> Result<usize, DeliveryError> {
		let now = Utc::now().fixed_offset();
		let rows = crosspoint_delivery_queue::Entity::find()
			.filter(
				crosspoint_delivery_queue::Column::Status
					.eq(DeliveryStatus::Queued.as_str()),
			)
			.filter(
				Condition::any()
					.add(crosspoint_delivery_queue::Column::NextAttemptAt.is_null())
					.add(crosspoint_delivery_queue::Column::NextAttemptAt.lte(now)),
			)
			.order_by(crosspoint_delivery_queue::Column::QueuedAt, Order::Asc)
			.limit(MAX_DUE_ROWS)
			.all(self.conn.as_ref())
			.await?;
		let count = rows.len();
		let mut workers = Vec::with_capacity(count);
		for row in rows {
			let service = self.clone();
			let gate = self.device_gate(&row.device_id).await;
			workers.push(tokio::spawn(async move {
				let _permit = gate.acquire_owned().await.map_err(|_| {
					DeliveryError::Worker("device gate closed".to_string())
				})?;
				service.process_one(&row.id).await
			}));
		}
		for worker in workers {
			worker
				.await
				.map_err(|error| DeliveryError::Worker(error.to_string()))??;
		}
		Ok(count)
	}

	/// Process one queue row if it is still due.  A stale/terminal id is a
	/// no-op, which keeps retries and duplicate wakeups idempotent.
	pub async fn process_one(
		&self,
		queue_id: &str,
	) -> Result<Option<DeliveryOutcome>, DeliveryError> {
		if crosspoint_delivery_queue::Entity::find_by_id(queue_id)
			.one(self.conn.as_ref())
			.await?
			.is_none_or(|row| row.status != DeliveryStatus::Queued.as_str())
		{
			return Ok(None);
		}
		let now = Utc::now().fixed_offset();
		let claimed = crosspoint_delivery_queue::Entity::update_many()
			.filter(crosspoint_delivery_queue::Column::Id.eq(queue_id))
			.filter(
				crosspoint_delivery_queue::Column::Status
					.eq(DeliveryStatus::Queued.as_str()),
			)
			.filter(
				Condition::any()
					.add(crosspoint_delivery_queue::Column::NextAttemptAt.is_null())
					.add(crosspoint_delivery_queue::Column::NextAttemptAt.lte(now)),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::Status,
				Expr::value(DeliveryStatus::Preparing.as_str()),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::Attempts,
				Expr::col(crosspoint_delivery_queue::Column::Attempts).add(1),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::StartedAt,
				Expr::value(now),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::UpdatedAt,
				Expr::value(now),
			)
			.exec(self.conn.as_ref())
			.await?;
		if claimed.rows_affected == 0 {
			return Ok(None);
		}
		let row = crosspoint_delivery_queue::Entity::find_by_id(queue_id)
			.one(self.conn.as_ref())
			.await?
			.ok_or_else(|| {
				DeliveryError::Worker(format!("claimed queue row {queue_id} disappeared"))
			})?;
		let attempt_id = Uuid::new_v4().to_string();
		crosspoint_delivery_attempt::ActiveModel {
			id: Set(attempt_id.clone()),
			queue_id: Set(row.id.clone()),
			attempt_no: Set(row.attempts),
			status: Set(DeliveryStatus::Preparing.as_str().to_string()),
			error: Set(None),
			bytes: Set(0),
			started_at: Set(now),
			finished_at: Set(None),
		}
		.insert(self.conn.as_ref())
		.await?;

		let prepared = self.prepare_source(&row).await;
		let result = match prepared {
			Ok(prepared) => {
				self.update_queue_status(&row.id, DeliveryStatus::Transferring)
					.await?;
				let transfer = self.transfer(&row, &prepared).await;
				let _ = fs::remove_file(&prepared.path).await;
				transfer.map(|receipt| (receipt, prepared.bytes))
			},
			Err(error) => Err(error),
		};
		match result {
			Ok((_receipt, bytes)) => {
				self.finish_attempt(&attempt_id, DeliveryStatus::Completed, None, bytes)
					.await?;
				self.update_queue(
					&row.id,
					DeliveryStatus::Completed,
					None,
					None,
					Some(Utc::now().fixed_offset()),
					None,
				)
				.await?;
				Ok(Some(DeliveryOutcome {
					queue_id: row.id,
					bytes,
					attempts: row.attempts as u32,
				}))
			},
			Err(error) => {
				let profile_retry_delay =
					serde_json::from_str::<CrosspointTransferProfile>(&row.profile_json)
						.ok()
						.and_then(|profile| profile.validate().ok())
						.map(|profile| {
							Duration::seconds(profile.retry_delay_seconds as i64)
						});
				let retry = is_retryable(&error)
					&& profile_retry_delay.is_some()
					&& row.attempts < row.max_attempts;
				let now = Utc::now().fixed_offset();
				let next = profile_retry_delay.map(|delay| now + delay);
				let status = if retry {
					DeliveryStatus::Queued
				} else {
					DeliveryStatus::Failed
				};
				self.finish_attempt(
					&attempt_id,
					DeliveryStatus::Failed,
					Some(error.to_string()),
					0,
				)
				.await?;
				self.update_queue(
					&row.id,
					status,
					if retry { next } else { None },
					Some(error.to_string()),
					if retry { None } else { Some(now) },
					None,
				)
				.await?;
				if retry {
					tracing::warn!(queue_id = %row.id, attempt = row.attempts, error = %error, "CrossPoint delivery will retry");
				} else {
					tracing::error!(queue_id = %row.id, attempt = row.attempts, error = %error, "CrossPoint delivery failed permanently");
				}
				Ok(None)
			},
		}
	}

	/// Retry a retained failed row.  Cancelled rows remain an explicit user
	/// decision and are not silently resurrected.
	pub async fn retry(&self, queue_id: &str) -> Result<bool, DeliveryError> {
		let now = Utc::now().fixed_offset();
		let update = crosspoint_delivery_queue::Entity::update_many()
			.filter(crosspoint_delivery_queue::Column::Id.eq(queue_id))
			.filter(
				crosspoint_delivery_queue::Column::Status
					.eq(DeliveryStatus::Failed.as_str()),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::Status,
				Expr::value(DeliveryStatus::Queued.as_str()),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::NextAttemptAt,
				Expr::value(None::<DateTimeWithTimeZone>),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::LastError,
				Expr::value(None::<String>),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::CompletedAt,
				Expr::value(None::<DateTimeWithTimeZone>),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::StartedAt,
				Expr::value(None::<DateTimeWithTimeZone>),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::UpdatedAt,
				Expr::value(now),
			)
			.exec(self.conn.as_ref())
			.await?;
		Ok(update.rows_affected > 0)
	}

	/// Cancel only local queued work.  An active transfer cannot be cancelled
	/// safely because the firmware has no authenticated remote cancellation.
	pub async fn cancel(&self, queue_id: &str) -> Result<bool, DeliveryError> {
		let now = Utc::now().fixed_offset();
		let update = crosspoint_delivery_queue::Entity::update_many()
			.filter(crosspoint_delivery_queue::Column::Id.eq(queue_id))
			.filter(
				crosspoint_delivery_queue::Column::Status
					.eq(DeliveryStatus::Queued.as_str()),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::Status,
				Expr::value(DeliveryStatus::Cancelled.as_str()),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::NextAttemptAt,
				Expr::value(None::<DateTimeWithTimeZone>),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::CompletedAt,
				Expr::value(Some(now)),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::UpdatedAt,
				Expr::value(now),
			)
			.exec(self.conn.as_ref())
			.await?;
		Ok(update.rows_affected > 0)
	}

	async fn device_gate(&self, device_id: &str) -> Arc<Semaphore> {
		let mut gates = self.device_gates.lock().await;
		gates
			.entry(device_id.to_string())
			.or_insert_with(|| Arc::new(Semaphore::new(1)))
			.clone()
	}

	async fn prepare_source(
		&self,
		row: &crosspoint_delivery_queue::Model,
	) -> Result<PreparedSource, DeliveryError> {
		let source = SourceRevision::parse(&row.source_revision)?;
		let media = media::Entity::find_by_id(&row.media_id)
			.one(self.conn.as_ref())
			.await?
			.ok_or_else(|| {
				DeliveryError::InvalidSource(format!(
					"media {} was not found",
					row.media_id
				))
			})?;
		validate_source_snapshot(&media, &source).await?;
		let target = crosspoint_device_target::Entity::find_by_id(&row.device_id)
			.one(self.conn.as_ref())
			.await?
			.ok_or_else(|| {
				DeliveryError::InvalidTarget(format!(
					"device {} was not found",
					row.device_id
				))
			})?;
		if target.user_id != row.user_id {
			return Err(DeliveryError::InvalidTarget(
				"target no longer belongs to the queued delivery user".to_string(),
			));
		}
		let (identity, root_path) = verified_target(&target)?;
		validate_remote_path(&row.destination_path, &root_path)?;
		let profile: CrosspointTransferProfile = serde_json::from_str(&row.profile_json)
			.map_err(|error| DeliveryError::InvalidProfile(error.to_string()))?;
		let profile = profile
			.validate()
			.map_err(|error| DeliveryError::InvalidProfile(error.to_string()))?;
		if profile.digest() != row.profile_digest {
			return Err(DeliveryError::InvalidProfile(format!(
				"profile digest mismatch: queue={}, calculated={}",
				row.profile_digest,
				profile.digest()
			)));
		}
		let profile = resolve_profile(profile, identity.model)?;
		let optimizer = EpubOptimizer::new(optimizer_options(&profile))
			.map_err(|error| DeliveryError::InvalidProfile(error.to_string()))?;
		fs::create_dir_all(&self.cache_dir).await?;
		let extension = Path::new(&source.filename)
			.extension()
			.and_then(|value| value.to_str())
			.unwrap_or("bin");
		let final_path = self
			.cache_dir
			.join(format!("{}-{}.{}", row.id, row.profile_digest, extension));
		let part_path = final_path.with_extension(format!("{extension}.part"));
		let raw_path = final_path.with_extension(format!("{extension}.raw.part"));
		let _ = fs::remove_file(&part_path).await;
		let _ = fs::remove_file(&raw_path).await;
		if optimizer.options().enabled && extension.eq_ignore_ascii_case("epub") {
			fs::copy(&source.path, &raw_path).await?;
			verify_staged_copy(&raw_path, &source).await?;
			let source_path = raw_path.clone();
			let destination = part_path.clone();
			let model = identity.model.to_string();
			let summary = tokio::task::spawn_blocking(move || {
				optimizer
					.optimize_path(&source_path, &destination, Some(&model))
					.map_err(|error| DeliveryError::Optimization(error.to_string()))
			})
			.await
			.map_err(|error| DeliveryError::Worker(error.to_string()))??;
			let _ = fs::remove_file(&raw_path).await;
			if summary.source_bytes != source.bytes {
				let _ = fs::remove_file(&part_path).await;
				return Err(DeliveryError::InvalidSource(
					"source changed while staging optimized delivery".to_string(),
				));
			}
		} else {
			fs::copy(&source.path, &part_path).await?;
			verify_staged_copy(&part_path, &source).await?;
		}
		let _ = fs::remove_file(&final_path).await;
		fs::rename(&part_path, &final_path).await?;
		let bytes = fs::metadata(&final_path).await?.len();
		if bytes > profile.max_upload_bytes {
			let _ = fs::remove_file(&final_path).await;
			return Err(DeliveryError::Transfer(CrossPointError::SizeLimit {
				size: bytes,
				maximum: profile.max_upload_bytes,
			}));
		}
		Ok(PreparedSource {
			path: final_path,
			bytes,
		})
	}

	async fn transfer(
		&self,
		row: &crosspoint_delivery_queue::Model,
		prepared: &PreparedSource,
	) -> Result<stump_crosspoint::transfer::UploadReceipt, DeliveryError> {
		let source = SourceRevision::parse(&row.source_revision)?;
		let target = crosspoint_device_target::Entity::find_by_id(&row.device_id)
			.one(self.conn.as_ref())
			.await?
			.ok_or_else(|| {
				DeliveryError::InvalidTarget(format!(
					"device {} was not found",
					row.device_id
				))
			})?;
		if target.user_id != row.user_id {
			return Err(DeliveryError::InvalidTarget(
				"target no longer belongs to the queued delivery user".to_string(),
			));
		}
		let (identity, root_path) = verified_target(&target)?;
		let profile: CrosspointTransferProfile =
			serde_json::from_str::<CrosspointTransferProfile>(&row.profile_json)
				.map_err(|error| DeliveryError::InvalidProfile(error.to_string()))?
				.validate()
				.map_err(|error| DeliveryError::InvalidProfile(error.to_string()))?;
		if profile.digest() != row.profile_digest {
			return Err(DeliveryError::InvalidProfile(
				"profile digest does not match queue snapshot".to_string(),
			));
		}
		let profile = resolve_profile(profile, identity.model)?;
		let client = CrossPointClient::new(transfer_config(&profile))?;
		let destination = compose_remote_path(&root_path, &row.destination_path)?;
		client
			.upload_for_identity(
				&CrossPointEndpoint::new(&target.host_or_ip)?,
				&identity,
				&prepared.path,
				&source.filename,
				&destination,
			)
			.await
			.map_err(DeliveryError::Transfer)
	}

	async fn update_queue_status(
		&self,
		queue_id: &str,
		status: DeliveryStatus,
	) -> Result<(), DeliveryError> {
		let now = Utc::now().fixed_offset();
		crosspoint_delivery_queue::Entity::update_many()
			.filter(crosspoint_delivery_queue::Column::Id.eq(queue_id))
			.col_expr(
				crosspoint_delivery_queue::Column::Status,
				Expr::value(status.as_str()),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::UpdatedAt,
				Expr::value(now),
			)
			.exec(self.conn.as_ref())
			.await?;
		Ok(())
	}

	async fn update_queue(
		&self,
		queue_id: &str,
		status: DeliveryStatus,
		next_attempt_at: Option<DateTime<chrono::FixedOffset>>,
		last_error: Option<String>,
		completed_at: Option<DateTime<chrono::FixedOffset>>,
		started_at: Option<DateTime<chrono::FixedOffset>>,
	) -> Result<(), DeliveryError> {
		let now = Utc::now().fixed_offset();
		crosspoint_delivery_queue::Entity::update_many()
			.filter(crosspoint_delivery_queue::Column::Id.eq(queue_id))
			.col_expr(
				crosspoint_delivery_queue::Column::Status,
				Expr::value(status.as_str()),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::NextAttemptAt,
				Expr::value(next_attempt_at),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::LastError,
				Expr::value(last_error),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::CompletedAt,
				Expr::value(completed_at),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::StartedAt,
				Expr::value(started_at),
			)
			.col_expr(
				crosspoint_delivery_queue::Column::UpdatedAt,
				Expr::value(now),
			)
			.exec(self.conn.as_ref())
			.await?;
		Ok(())
	}

	async fn finish_attempt(
		&self,
		attempt_id: &str,
		status: DeliveryStatus,
		error: Option<String>,
		bytes: u64,
	) -> Result<(), DeliveryError> {
		let finished_at = Utc::now().fixed_offset();
		crosspoint_delivery_attempt::Entity::update_many()
			.filter(crosspoint_delivery_attempt::Column::Id.eq(attempt_id))
			.col_expr(
				crosspoint_delivery_attempt::Column::Status,
				Expr::value(status.as_str()),
			)
			.col_expr(
				crosspoint_delivery_attempt::Column::Error,
				Expr::value(error),
			)
			.col_expr(
				crosspoint_delivery_attempt::Column::Bytes,
				Expr::value(i64::try_from(bytes).unwrap_or(i64::MAX)),
			)
			.col_expr(
				crosspoint_delivery_attempt::Column::FinishedAt,
				Expr::value(Some(finished_at)),
			)
			.exec(self.conn.as_ref())
			.await?;
		Ok(())
	}
}

fn verified_target(
	target: &crosspoint_device_target::Model,
) -> Result<(CrossPointIdentity, String), DeliveryError> {
	if target.revoked_at.is_some() {
		return Err(DeliveryError::InvalidTarget(
			"target has been revoked".to_string(),
		));
	}
	if target.http_port != i32::from(CROSSPOINT_HTTP_PORT)
		|| target.ws_port != i32::from(CROSSPOINT_WS_PORT)
	{
		return Err(DeliveryError::InvalidTarget(
			"target ports must remain CrossPoint's fixed 80/81".to_string(),
		));
	}
	if target.verified_at.is_none() {
		return Err(DeliveryError::InvalidTarget(
			"target has not completed /api/status verification".to_string(),
		));
	}
	let fingerprint_value = target.fingerprint.clone().ok_or_else(|| {
		DeliveryError::InvalidTarget("target fingerprint is missing".to_string())
	})?;
	let fingerprint: TargetFingerprint = serde_json::from_value(fingerprint_value)
		.map_err(|error| {
			DeliveryError::InvalidTarget(format!("invalid target fingerprint: {error}"))
		})?;
	if !fingerprint.is_present() || !fingerprint.is_supported_model() {
		return Err(DeliveryError::InvalidTarget(
			"target fingerprint must contain a supported model and serial".to_string(),
		));
	}
	let model = CrossPointModel::from_str(&fingerprint.model)
		.map_err(|error| DeliveryError::InvalidTarget(error.to_string()))?;
	let identity = CrossPointIdentity::new(model, fingerprint.serial)
		.map_err(DeliveryError::Transfer)?;
	// Constructing the endpoint here rejects DNS, public, loopback, link-local,
	// and multicast addresses before any local staging or network I/O.
	CrossPointEndpoint::new(&target.host_or_ip).map_err(DeliveryError::Transfer)?;
	validate_remote_path("/", &target.root_path)?;
	Ok((identity, target.root_path.clone()))
}

async fn validate_source_snapshot(
	media: &media::Model,
	source: &SourceRevision,
) -> Result<(), DeliveryError> {
	if media.path != source.path {
		return Err(DeliveryError::InvalidSource(
			"indexed media path differs from queued source revision".to_string(),
		));
	}
	if media.size < 0 || media.size as u64 != source.bytes {
		return Err(DeliveryError::InvalidSource(
			"indexed media size differs from queued source revision".to_string(),
		));
	}
	if let Some(hash) = &media.hash {
		if hash != &source.hash {
			return Err(DeliveryError::InvalidSource(
				"indexed media hash differs from queued source revision".to_string(),
			));
		}
	}
	let metadata = fs::metadata(&source.path).await?;
	if !metadata.is_file() || metadata.len() != source.bytes {
		return Err(DeliveryError::InvalidSource(
			"source file is missing or its size changed".to_string(),
		));
	}
	let actual = sampled_hash(source.path.clone(), source.bytes).await?;
	if actual != source.hash {
		return Err(DeliveryError::InvalidSource(
			"source file hash differs from queued source revision".to_string(),
		));
	}
	Ok(())
}

async fn sampled_hash(path: String, bytes: u64) -> Result<String, DeliveryError> {
	tokio::task::spawn_blocking(move || generate(&path, bytes))
		.await
		.map_err(|error| DeliveryError::Worker(error.to_string()))?
		.map_err(DeliveryError::Io)
}

async fn verify_staged_copy(
	path: &Path,
	source: &SourceRevision,
) -> Result<(), DeliveryError> {
	let metadata = fs::metadata(path).await?;
	if !metadata.is_file() || metadata.len() != source.bytes {
		let _ = fs::remove_file(path).await;
		return Err(DeliveryError::InvalidSource(
			"source changed while staging delivery".to_string(),
		));
	}
	let actual = sampled_hash(path.to_string_lossy().into_owned(), source.bytes).await?;
	if actual != source.hash {
		let _ = fs::remove_file(path).await;
		return Err(DeliveryError::InvalidSource(
			"source changed while staging delivery".to_string(),
		));
	}
	Ok(())
}

fn is_retryable(error: &DeliveryError) -> bool {
	matches!(
		error,
		DeliveryError::Transfer(
			CrossPointError::Status(_)
				| CrossPointError::Handshake(_)
				| CrossPointError::Timeout
				| CrossPointError::Io(_)
		)
	)
}

fn idempotency_key(
	user_id: &str,
	device_id: &str,
	media_id: &str,
	source_hash: &str,
	profile_digest: &str,
	destination_path: &str,
) -> String {
	let mut context = Context::new(&SHA256);
	context.update(b"stump-crosspoint-delivery-v1\0");
	for value in [
		user_id,
		device_id,
		media_id,
		source_hash,
		profile_digest,
		destination_path,
	] {
		context.update(value.as_bytes());
		context.update(&[0]);
	}
	data_encoding::HEXLOWER.encode(context.finish().as_ref())
}

fn validate_remote_path(destination: &str, root: &str) -> Result<(), DeliveryError> {
	if !root.is_empty() && !root.starts_with('/') {
		return Err(DeliveryError::InvalidTarget(
			"target root_path must be absolute".to_string(),
		));
	}
	let _ = remote_components(root, "target root_path")?;
	let _ = remote_components(destination, "destination path")?;
	Ok(())
}

fn compose_remote_path(root: &str, destination: &str) -> Result<String, DeliveryError> {
	let root_components = remote_components(root, "target root_path")?;
	let destination_components = remote_components(destination, "destination path")?;
	let components = if destination_components.starts_with(&root_components) {
		destination_components
	} else {
		root_components
			.into_iter()
			.chain(destination_components)
			.collect()
	};
	if components.is_empty() {
		Ok("/".to_string())
	} else {
		Ok(format!("/{}", components.join("/")))
	}
}

fn remote_components<'a>(
	path: &'a str,
	field: &str,
) -> Result<Vec<&'a str>, DeliveryError> {
	if path.contains('\\') || path.chars().any(char::is_control) {
		return Err(DeliveryError::InvalidTarget(format!(
			"{field} contains a forbidden separator or control character"
		)));
	}
	let mut components = Vec::new();
	for component in path.split('/') {
		if component.is_empty() {
			continue;
		}
		validate_single_component(component, field)?;
		components.push(component);
	}
	Ok(components)
}

fn validate_single_component(value: &str, field: &str) -> Result<(), DeliveryError> {
	if value.is_empty() || value == "." || value == ".." || value.contains(':') {
		return Err(DeliveryError::InvalidTarget(format!(
			"{field} contains an unsafe path component"
		)));
	}
	if value.chars().any(|character| character.is_control())
		|| value.contains('/')
		|| value.contains('\\')
	{
		return Err(DeliveryError::InvalidTarget(format!(
			"{field} contains a forbidden separator or control character"
		)));
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::{compose_remote_path, idempotency_key, SourceRevision};

	#[test]
	fn source_revision_is_canonical_and_filename_safe() {
		let revision = SourceRevision {
			path: "/library/book.epub".to_string(),
			hash: "abc".to_string(),
			filename: "book.epub".to_string(),
			bytes: 12,
		};
		assert_eq!(
			revision.canonical_json().unwrap(),
			r#"{"path":"/library/book.epub","hash":"abc","filename":"book.epub","bytes":12}"#
		);
		assert!(SourceRevision {
			filename: "../book.epub".to_string(),
			..revision
		}
		.validate()
		.is_err());
	}

	#[test]
	fn remote_paths_cannot_escape_target_root() {
		assert_eq!(
			compose_remote_path("/books", "/fiction").unwrap(),
			"/books/fiction"
		);
		assert_eq!(
			compose_remote_path("/books", "/books/fiction").unwrap(),
			"/books/fiction"
		);
		assert!(compose_remote_path("/books", "/../etc").is_err());
	}

	#[test]
	fn idempotency_key_is_stable() {
		assert_eq!(
			idempotency_key("u", "d", "m", "h", "p", "/books"),
			idempotency_key("u", "d", "m", "h", "p", "/books")
		);
		assert_ne!(
			idempotency_key("u", "d", "m", "h", "p", "/books"),
			idempotency_key("u", "d", "m", "h", "p", "/other")
		);
	}
}
