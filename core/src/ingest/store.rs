use std::{
	path::{Path, PathBuf},
	sync::Arc,
};

use chrono::Utc;
use metadata_integrations::MergeStrategy;
use models::txn::begin_write;
use models::{
	entity::{
		ingest_analysis_job, ingest_drop_item, ingest_metadata_application,
		ingest_metadata_candidate, ingest_plugin_setting, ingest_quality_report, library,
		library_config, media, series,
	},
	shared::enums::JobStatus,
};
use sea_orm::{
	entity::prelude::*,
	ActiveModelTrait,
	ActiveValue::{NotSet, Set},
	ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, QuerySelect,
};
use serde_json::{json, Value};
use tokio::{fs, io::AsyncRead};
use uuid::Uuid;

use super::{
	contract::{
		BookSnapshot, DropItemStatus, FieldPick, IngestMediaKind, IngestPageEntry,
		MetadataCandidate, MetadataField, QualityReport,
	},
	drop_folder,
	progress::{ProgressHub, ProgressStream, StoredProgressStream},
	providers::apply::{
		apply_to_media_txn_with_context, resolve_picks_for_context, validate_picks,
	},
	staging,
};
use crate::{
	config::StumpConfig,
	error::{CoreError, CoreResult},
	filesystem::{media::MediaBuilder, series::SeriesBuilder},
	utils::move_file,
};
use stump_media::{
	media::{get_page_count, process_metadata, ReadiumManifestGenerator},
	PathUtils,
};

pub type DropItemModel = ingest_drop_item::Model;
pub type AnalysisJobModel = ingest_analysis_job::Model;
pub type QualityReportModel = ingest_quality_report::Model;
pub type CandidateModel = ingest_metadata_candidate::Model;
pub type ApplicationModel = ingest_metadata_application::Model;
pub type PluginSettingModel = ingest_plugin_setting::Model;

/// What one analysis job runs against: a staged drop item, or an existing
/// library media row (library-wide rework).  Persisted in the job's `plan`
/// JSON; single-target jobs also fill the matching id column.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnalysisTarget {
	DropItem(String),
	Media(String),
}

impl AnalysisTarget {
	pub fn id(&self) -> &str {
		match self {
			AnalysisTarget::DropItem(id) | AnalysisTarget::Media(id) => id,
		}
	}
}

fn analysis_plan(targets: &[AnalysisTarget], providers: Option<&[String]>) -> Value {
	let targets = targets
		.iter()
		.map(|target| serde_json::to_value(target).expect("target is serializable"))
		.collect::<Vec<_>>();
	let mut plan = json!({ "targets": targets });
	if let Some(providers) = providers {
		plan["providers"] = json!(providers);
	}
	plan
}

/// Optional provider allowlist persisted with a library match job.
pub fn analysis_providers_from_plan(plan: &Value) -> Option<Vec<String>> {
	plan.get("providers")
		.and_then(Value::as_array)
		.map(|providers| {
			providers
				.iter()
				.filter_map(Value::as_str)
				.map(str::to_owned)
				.collect::<Vec<_>>()
		})
		.filter(|providers| !providers.is_empty())
}

pub fn analysis_targets_from_plan(
	plan: &Value,
	drop_item_id: Option<&str>,
) -> Vec<AnalysisTarget> {
	let parsed = plan
		.get("targets")
		.and_then(Value::as_array)
		.map(|targets| {
			targets
				.iter()
				.filter_map(|target| serde_json::from_value(target.clone()).ok())
				.collect::<Vec<AnalysisTarget>>()
		})
		.unwrap_or_default();
	if !parsed.is_empty() {
		return parsed;
	}
	drop_item_id
		.map(|id| vec![AnalysisTarget::DropItem(id.to_string())])
		.unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pagination {
	pub page: u64,
	pub page_size: u64,
}

impl Default for Pagination {
	fn default() -> Self {
		Self {
			page: 1,
			page_size: 50,
		}
	}
}

impl Pagination {
	pub fn offset(self) -> u64 {
		self.page.saturating_sub(1).saturating_mul(self.page_size)
	}

	pub fn limit(self) -> u64 {
		self.page_size.max(1)
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropFolderInfo {
	pub path_display: String,
	pub pending_files: u32,
	pub total_bytes: u64,
}

#[derive(Clone)]
pub struct IngestStore {
	config: Arc<StumpConfig>,
	conn: Arc<DatabaseConnection>,
	progress: Arc<ProgressHub>,
}

impl IngestStore {
	pub fn new(config: Arc<StumpConfig>, conn: Arc<DatabaseConnection>) -> Self {
		let progress = Arc::new(ProgressHub::new(
			conn.clone(),
			config.ingest.ingest_progress_retention,
		));
		Self {
			config,
			conn,
			progress,
		}
	}

	pub fn config(&self) -> &Arc<StumpConfig> {
		&self.config
	}

	pub fn conn(&self) -> &Arc<DatabaseConnection> {
		&self.conn
	}

	pub fn progress(&self) -> &Arc<ProgressHub> {
		&self.progress
	}

	pub async fn stage_upload<R>(
		&self,
		library_id: &str,
		created_by: Option<&str>,
		relative_path: Option<&str>,
		filename: &str,
		reader: R,
		idempotency_key: Option<&str>,
	) -> CoreResult<StagedUpload>
	where
		R: AsyncRead + Unpin + Send,
	{
		self.ensure_library(library_id).await?;
		let relative_path = staging::normalize_relative_path(relative_path)?;
		let filename = staging::sanitize_filename(filename)?;
		if let Some(key) = idempotency_key {
			if let Some(item) = ingest_drop_item::Entity::find()
				.filter(ingest_drop_item::Column::LibraryId.eq(library_id))
				.filter(ingest_drop_item::Column::IdempotencyKey.eq(key))
				.one(self.conn.as_ref())
				.await?
			{
				return Ok(StagedUpload::deduplicated(item));
			}
		}

		let staged = staging::stage_reader(
			&self.config.get_ingest_staging_dir(),
			library_id,
			&filename,
			reader,
		)
		.await?;
		if let Some(item) = self
			.find_identity(library_id, &staged.source_sha256, &filename)
			.await?
		{
			// Same bytes and name stage to the same deterministic path, so the
			// upload just replaced the existing item's file with identical
			// content; only a differently-located duplicate is removed.
			remove_duplicate_upload(&staged.path, &item.staging_path).await?;
			return Ok(StagedUpload::deduplicated(item));
		}

		let active = ingest_drop_item::ActiveModel {
			id: Set(Uuid::new_v4().to_string()),
			library_id: Set(library_id.to_string()),
			created_by: Set(created_by.map(str::to_owned)),
			source_filename: Set(filename.clone()),
			relative_path: Set(relative_path),
			byte_size: Set(i64::try_from(staged.byte_size).map_err(|_| {
				CoreError::BadRequest(
					"uploaded file is too large for the database".to_string(),
				)
			})?),
			source_sha256: Set(staged.source_sha256.clone()),
			media_kind: Set(media_kind_name(media_kind_for_filename(&filename))),
			staging_path: Set(staged.path.to_string_lossy().into_owned()),
			status: Set(DropItemStatus::Staged.as_str().to_string()),
			analysis_job_id: Set(None),
			quality_report_id: Set(None),
			media_id: Set(None),
			series_id: Set(None),
			pending_fields: Set(Some(Value::Object(Default::default()))),
			error: Set(None),
			idempotency_key: Set(idempotency_key.map(str::to_owned)),
			revision: Set(1),
			created_at: NotSet,
			updated_at: NotSet,
		};
		match active.insert(self.conn.as_ref()).await {
			Ok(item) => Ok(StagedUpload::fresh(item)),
			Err(error) => {
				// Lost a race with a concurrent identical upload.
				if let Some(item) = self
					.find_identity(library_id, &staged.source_sha256, &filename)
					.await?
				{
					remove_duplicate_upload(&staged.path, &item.staging_path).await?;
					Ok(StagedUpload::deduplicated(item))
				} else {
					staging::remove_if_exists(&staged.path).await?;
					Err(error.into())
				}
			},
		}
	}

	pub async fn scan_drop_folder(
		&self,
		library_id: &str,
	) -> CoreResult<Vec<DropItemModel>> {
		self.ensure_library(library_id).await?;
		let root = self.config.get_ingest_drop_dir().join(library_id);
		let files = drop_folder::list_files(&root).await?;
		let mut admitted = Vec::with_capacity(files.len());
		for file in files {
			let (source_sha256, byte_size) = staging::hash_file(&file.path).await?;
			if let Some(existing) = self
				.find_identity(library_id, &source_sha256, &file.filename)
				.await?
			{
				// A duplicate in the drop folder is consumed so later scans are
				// idempotent and do not repeatedly report the same file.
				staging::remove_if_exists(&file.path).await?;
				admitted.push(existing);
				continue;
			}
			let staged_path = staging::move_into_staging(
				&file.path,
				&self.config.get_ingest_staging_dir(),
				library_id,
				&file.filename,
				&source_sha256,
			)
			.await?;
			let relative_path = Path::new(&file.relative_path)
				.parent()
				.filter(|path| !path.as_os_str().is_empty())
				.map(|path| path.to_string_lossy().replace('\\', "/"));
			let active = ingest_drop_item::ActiveModel {
				id: Set(Uuid::new_v4().to_string()),
				library_id: Set(library_id.to_string()),
				created_by: Set(None),
				source_filename: Set(file.filename.clone()),
				relative_path: Set(relative_path),
				byte_size: Set(i64::try_from(byte_size).map_err(|_| {
					CoreError::BadRequest(
						"drop file is too large for the database".to_string(),
					)
				})?),
				source_sha256: Set(source_sha256.clone()),
				media_kind: Set(media_kind_name(media_kind_for_filename(&file.filename))),
				staging_path: Set(staged_path.to_string_lossy().into_owned()),
				status: Set(DropItemStatus::Staged.as_str().to_string()),
				analysis_job_id: Set(None),
				quality_report_id: Set(None),
				media_id: Set(None),
				series_id: Set(None),
				pending_fields: Set(Some(Value::Object(Default::default()))),
				error: Set(None),
				idempotency_key: Set(None),
				revision: Set(1),
				created_at: NotSet,
				updated_at: NotSet,
			};
			match active.insert(self.conn.as_ref()).await {
				Ok(item) => admitted.push(item),
				Err(error) => {
					if let Some(existing) = self
						.find_identity(library_id, &source_sha256, &file.filename)
						.await?
					{
						admitted.push(existing);
					} else {
						return Err(error.into());
					}
				},
			}
		}
		admitted.sort_by(|left, right| left.id.cmp(&right.id));
		Ok(admitted)
	}

	pub async fn drop_folder(&self, library_id: &str) -> CoreResult<DropFolderInfo> {
		self.ensure_library(library_id).await?;
		let path = self.config.get_ingest_drop_dir().join(library_id);
		let stats = drop_folder::stats(&path).await?;
		Ok(DropFolderInfo {
			path_display: path.to_string_lossy().into_owned(),
			pending_files: stats.pending_files,
			total_bytes: stats.total_bytes,
		})
	}

	pub async fn list_items(
		&self,
		library_id: Option<&str>,
		status: Option<DropItemStatus>,
		page: Pagination,
	) -> CoreResult<(Vec<DropItemModel>, u64)> {
		let mut query = ingest_drop_item::Entity::find();
		if let Some(library_id) = library_id {
			query = query.filter(ingest_drop_item::Column::LibraryId.eq(library_id));
		}
		if let Some(status) = status {
			query = query.filter(ingest_drop_item::Column::Status.eq(status.as_str()));
		}
		let total = query.clone().count(self.conn.as_ref()).await?;
		let items = query
			.order_by_asc(ingest_drop_item::Column::CreatedAt)
			.order_by_asc(ingest_drop_item::Column::Id)
			.offset(page.offset())
			.limit(page.limit())
			.all(self.conn.as_ref())
			.await?;
		Ok((items, total))
	}

	pub async fn item(&self, id: &str) -> CoreResult<Option<DropItemModel>> {
		ingest_drop_item::Entity::find_by_id(id)
			.one(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	pub async fn bulk_items(&self, ids: &[String]) -> CoreResult<Vec<DropItemModel>> {
		if ids.is_empty() {
			return Ok(Vec::new());
		}
		ingest_drop_item::Entity::find()
			.filter(ingest_drop_item::Column::Id.is_in(ids.to_vec()))
			.order_by_asc(ingest_drop_item::Column::CreatedAt)
			.order_by_asc(ingest_drop_item::Column::Id)
			.all(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	/// Items below `min_score`, lowest score first (missing report = 0), then
	/// oldest first, then id — one joined query, paginated in SQL.
	pub async fn rework_items(
		&self,
		min_score: u8,
		page: Pagination,
	) -> CoreResult<(Vec<DropItemModel>, u64)> {
		use sea_orm::{sea_query::Expr, JoinType, RelationTrait};

		// `quality_report_id` points at the latest report; unscored items rank
		// first as score 0.
		let score = || {
			Expr::expr(Expr::cust(
				"COALESCE(\"ingest_quality_reports\".\"score\", 0)",
			))
		};
		let query = ingest_drop_item::Entity::find()
			.join(
				JoinType::LeftJoin,
				ingest_drop_item::Relation::QualityReport.def(),
			)
			.filter(ingest_drop_item::Column::Status.is_not_in([
				DropItemStatus::Committed.as_str(),
				DropItemStatus::Rejected.as_str(),
			]))
			.filter(score().lt(i32::from(min_score)));
		let total = query.clone().count(self.conn.as_ref()).await?;
		let items = query
			.order_by(score(), sea_orm::Order::Asc)
			.order_by_asc(ingest_drop_item::Column::CreatedAt)
			.order_by_asc(ingest_drop_item::Column::Id)
			.offset(page.offset())
			.limit(page.limit())
			.all(self.conn.as_ref())
			.await?;
		Ok((items, total))
	}

	pub async fn snapshot(&self, item_id: &str) -> CoreResult<BookSnapshot> {
		let item = self
			.item(item_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("ingest item {item_id}")))?;
		let config = self.config.clone();
		tokio::task::spawn_blocking(move || build_snapshot(item, &config))
			.await
			.map_err(|error| CoreError::Unknown(error.to_string()))?
	}

	pub async fn media(&self, id: &str) -> CoreResult<Option<media::Model>> {
		media::Entity::find_by_id(id)
			.one(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	/// The library a media row belongs to, through its series.
	pub(crate) async fn media_library(
		&self,
		media_row: &media::Model,
	) -> CoreResult<String> {
		let series_id = media_row.series_id.as_deref().ok_or_else(|| {
			CoreError::BadRequest(format!("media {} has no series", media_row.id))
		})?;
		series::Entity::find_by_id(series_id)
			.one(self.conn.as_ref())
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("series {series_id}")))?
			.library_id
			.ok_or_else(|| {
				CoreError::BadRequest(format!("series {series_id} has no library"))
			})
	}

	/// Snapshot of an existing library file, built through the same
	/// `snapshot_from_source` path as staged drop items — one shared
	/// convention for parsing, pages, and digests.
	pub async fn media_snapshot(&self, media_id: &str) -> CoreResult<BookSnapshot> {
		let media_row = self
			.media(media_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("media {media_id}")))?;
		let path = PathBuf::from(&media_row.path);
		if !path.is_file() {
			return Err(CoreError::FileNotFound(media_row.path.clone()));
		}
		let library_id = self.media_library(&media_row).await?;
		// Same digest convention as staging: SHA-256 over the file bytes; the
		// `media.hash` column is optional and not guaranteed to be present.
		let (source_sha256, byte_size) = staging::hash_file(&path).await?;
		let source_filename = path
			.file_name()
			.and_then(|name| name.to_str())
			.map(str::to_owned)
			.ok_or_else(|| {
				CoreError::BadRequest(format!(
					"media path {} has no usable file name",
					media_row.path
				))
			})?;
		let config = self.config.clone();
		tokio::task::spawn_blocking(move || {
			snapshot_from_source(
				SnapshotSource {
					id: media_row.id,
					library_id,
					path,
					source_filename,
					source_sha256,
					byte_size,
					relative_path: String::new(),
				},
				&config,
			)
		})
		.await
		.map_err(|error| CoreError::Unknown(error.to_string()))?
	}

	pub async fn discard(&self, item_id: &str) -> CoreResult<DropItemModel> {
		let item = self
			.item(item_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("ingest item {item_id}")))?;
		staging::remove_if_exists(Path::new(&item.staging_path)).await?;
		item.clone()
			.into_active_model()
			.delete(self.conn.as_ref())
			.await?;
		Ok(item)
	}

	pub async fn reject(
		&self,
		item_id: &str,
		reason: Option<&str>,
	) -> CoreResult<DropItemModel> {
		let item = self
			.item(item_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("ingest item {item_id}")))?;
		if !matches!(
			DropItemStatus::parse(&item.status),
			Some(DropItemStatus::Committed | DropItemStatus::Rejected)
		) {
			let rejected_dir = self.config.get_ingest_staging_dir().join("rejected");
			fs::create_dir_all(&rejected_dir).await?;
			let target =
				rejected_dir.join(format!("{}-{}", item.id, item.source_filename));
			move_file(Path::new(&item.staging_path), &target).await?;
		}
		let revision = item.revision;
		let mut active = item.into_active_model();
		active.status = Set(DropItemStatus::Rejected.as_str().to_string());
		active.error = Set(reason.map(str::to_owned));
		active.revision = Set(revision.saturating_add(1));
		active
			.update(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}
	pub async fn approve(
		&self,
		item_id: &str,
		picks: Vec<FieldPick>,
		expected_revision: i32,
	) -> CoreResult<(String, DropItemModel)> {
		let item = self
			.item(item_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("ingest item {item_id}")))?;
		if item.revision != expected_revision {
			return Err(CoreError::BadRequest(format!(
				"ingest item {item_id} revision conflict: expected {expected_revision}, current {}",
				item.revision
			)));
		}
		if let Some(status) = DropItemStatus::parse(&item.status) {
			if status == DropItemStatus::Committed {
				return item
					.media_id
					.clone()
					.map(|media_id| (media_id, item))
					.ok_or_else(|| {
						CoreError::BadRequest(
							"committed ingest item has no media".to_string(),
						)
					});
			}
			if matches!(status, DropItemStatus::Rejected | DropItemStatus::Failed) {
				return Err(CoreError::BadRequest(format!(
					"cannot approve ingest item in {status:?} state"
				)));
			}
		}
		// Field values selected earlier via `applyIngestMetadata` live in the
		// item's pending projection until approval; explicit picks in this
		// call take precedence, everything else pending is applied as-is.
		let picks = merge_pending_fields(picks, item.pending_fields.as_ref());
		validate_picks(&picks)
			.map_err(|error| CoreError::BadRequest(error.to_string()))?;
		let candidates = self.candidates(item_id).await?;
		let resolved = resolve_picks_for_context(
			&picks,
			&candidates,
			None,
			&[],
			MergeStrategy::FillGaps,
			Some(item_id),
			Some(&item.source_sha256),
		)
		.map_err(|error| CoreError::BadRequest(error.to_string()))?;
		let library = self.ensure_library(&item.library_id).await?;
		let relative_path =
			staging::normalize_relative_path(item.relative_path.as_deref())?;
		let filename = staging::sanitize_filename(&item.source_filename)?;
		let library_root = PathBuf::from(&library.path);
		let destination = library_root
			.join(relative_path.as_deref().unwrap_or_default())
			.join(filename);
		if !destination.starts_with(&library_root) {
			return Err(CoreError::BadRequest(
				"destination escapes library root".to_string(),
			));
		}
		if !Path::new(&item.staging_path).is_file() {
			return Err(CoreError::FileNotFound(item.staging_path));
		}
		if fs::try_exists(&destination).await? {
			return Err(CoreError::BadRequest(format!(
				"destination already exists: {}",
				destination.display()
			)));
		}
		let series_path = destination
			.parent()
			.ok_or_else(|| {
				CoreError::BadRequest("destination has no series directory".to_string())
			})?
			.to_path_buf();
		fs::create_dir_all(&series_path).await?;

		let txn = begin_write(&self.conn).await?;
		let config = library_config::Entity::find_by_id(library.config_id)
			.one(&txn)
			.await?
			.ok_or_else(|| {
				CoreError::NotFound(format!("library config {}", library.config_id))
			})?;
		let series_path_string = series_path.to_string_lossy().into_owned();
		let existing_series = series::Entity::find()
			.filter(series::Column::Path.eq(series_path_string.clone()))
			.filter(series::Column::LibraryId.eq(item.library_id.clone()))
			.one(&txn)
			.await?;
		let series_id = if let Some(existing) = existing_series {
			existing.id
		} else {
			let series_path_for_build = series_path.clone();
			let library_id = item.library_id.clone();
			let built = tokio::task::spawn_blocking(move || {
				SeriesBuilder::new(&series_path_for_build, &library_id).build()
			})
			.await
			.map_err(|error| CoreError::Unknown(error.to_string()))??;
			let built_series = built.series.insert(&txn).await?;
			if let Some(metadata) = built.metadata {
				metadata.insert(&txn).await?;
			}
			built_series.id
		};

		// The media builder must see the file at its final library path, so the
		// move happens inside the transaction window; any failure after this
		// point rolls the database back (transaction drop) and moves the file
		// back to staging so the item stays approvable.
		move_file(Path::new(&item.staging_path), &destination).await?;
		let staging_path = item.staging_path.clone();
		let commit = async {
			let media_config = config.clone();
			let build_config = self.config.clone();
			let destination_for_build = destination.clone();
			let series_id_for_build = series_id.clone();
			let built_media = tokio::task::spawn_blocking(move || {
				MediaBuilder::new(
					&destination_for_build,
					&series_id_for_build,
					media_config,
					&build_config,
				)
				.build()
			})
			.await
			.map_err(|error| CoreError::Unknown(error.to_string()))??;
			let media_model = built_media.media.insert(&txn).await?;
			if let Some(metadata) = built_media.metadata {
				metadata.insert(&txn).await?;
			}
			let picks_value = serde_json::to_value(&picks)?;
			apply_to_media_txn_with_context(
				&txn,
				&media_model.id,
				resolved,
				item.created_by.as_deref().unwrap_or("system"),
				Some(item_id),
				expected_revision,
				"FILL_GAPS",
				picks_value,
			)
			.await
			.map_err(|error| CoreError::BadRequest(error.to_string()))?;
			let revision = item.revision;
			let mut updated_item = item.into_active_model();
			updated_item.status = Set(DropItemStatus::Committed.as_str().to_string());
			updated_item.media_id = Set(Some(media_model.id.clone()));
			updated_item.series_id = Set(Some(series_id));
			updated_item.error = Set(None);
			updated_item.revision = Set(revision.saturating_add(1));
			updated_item.update(&txn).await?;
			txn.commit().await?;
			Ok::<_, CoreError>(media_model.id)
		};
		let media_id = match commit.await {
			Ok(media_id) => media_id,
			Err(error) => {
				if let Err(restore_error) =
					move_file(&destination, Path::new(&staging_path)).await
				{
					tracing::error!(
						?restore_error,
						destination = %destination.display(),
						staging = %staging_path,
						"approve failed and the staged file could not be moved back"
					);
				}
				return Err(error);
			},
		};
		let committed = self
			.item(item_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("ingest item {item_id}")))?;
		Ok((media_id, committed))
	}

	pub async fn save_report(
		&self,
		drop_item_id: &str,
		report: &QualityReport,
	) -> CoreResult<QualityReportModel> {
		let item = self
			.item(drop_item_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("ingest item {drop_item_id}")))?;
		if item.source_sha256 != report.source_sha256 {
			return Err(CoreError::BadRequest(
				"quality report source digest mismatch".to_string(),
			));
		}
		let model = ingest_quality_report::ActiveModel {
			id: Set(Uuid::new_v4().to_string()),
			drop_item_id: Set(Some(drop_item_id.to_string())),
			media_id: Set(None),
			source_sha256: Set(report.source_sha256.clone()),
			algorithm_version: Set(report.algorithm_version.clone()),
			score: Set(i32::from(report.score)),
			checks: Set(serde_json::to_value(&report.checks)?),
			settings_snapshot: Set(serde_json::to_value(&report.settings_snapshot)?),
			created_at: NotSet,
		};
		let report_model = model.insert(self.conn.as_ref()).await?;
		let revision = item.revision;
		let mut item = item.into_active_model();
		item.quality_report_id = Set(Some(report_model.id.clone()));
		item.revision = Set(revision.saturating_add(1));
		item.update(self.conn.as_ref()).await?;
		Ok(report_model)
	}

	pub async fn report(
		&self,
		report_id: &str,
	) -> CoreResult<Option<QualityReportModel>> {
		ingest_quality_report::Entity::find_by_id(report_id)
			.one(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	pub async fn report_for_item(
		&self,
		item_id: &str,
	) -> CoreResult<Option<QualityReportModel>> {
		self.latest_report_for_item(item_id).await
	}

	pub async fn candidates(&self, item_id: &str) -> CoreResult<Vec<CandidateModel>> {
		ingest_metadata_candidate::Entity::find()
			.filter(ingest_metadata_candidate::Column::DropItemId.eq(item_id))
			.order_by_asc(ingest_metadata_candidate::Column::ProviderId)
			.order_by_asc(ingest_metadata_candidate::Column::Id)
			.all(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	pub async fn save_candidates(
		&self,
		item_id: &str,
		candidates: &[MetadataCandidate],
	) -> CoreResult<Vec<CandidateModel>> {
		let item = self
			.item(item_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("ingest item {item_id}")))?;
		let mut saved = Vec::with_capacity(candidates.len());
		for candidate in candidates {
			if candidate.source_sha256 != item.source_sha256 {
				continue;
			}
			let active = ingest_metadata_candidate::ActiveModel {
				id: Set(Uuid::new_v4().to_string()),
				drop_item_id: Set(Some(item_id.to_string())),
				media_id: Set(None),
				provider_id: Set(candidate.provider_id.clone()),
				provider_version: Set(candidate.provider_version.clone()),
				external_id: Set(candidate.external_id.clone()),
				source_sha256: Set(candidate.source_sha256.clone()),
				confidence: Set(candidate.confidence),
				fields: Set(serde_json::to_value(&candidate.fields)?),
				field_confidence: Set(serde_json::to_value(&candidate.field_confidence)?),
				provenance: Set(candidate.provenance.clone()),
				status: Set("PENDING".to_string()),
				created_at: NotSet,
				updated_at: NotSet,
			};
			saved.push(active.insert(self.conn.as_ref()).await?);
		}
		Ok(saved)
	}

	/// Latest quality report stored against a library media row.
	pub async fn report_for_media(
		&self,
		media_id: &str,
	) -> CoreResult<Option<QualityReportModel>> {
		ingest_quality_report::Entity::find()
			.filter(ingest_quality_report::Column::MediaId.eq(media_id))
			.order_by_desc(ingest_quality_report::Column::CreatedAt)
			.order_by_desc(ingest_quality_report::Column::Id)
			.one(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	pub async fn save_report_for_media(
		&self,
		media_id: &str,
		report: &QualityReport,
	) -> CoreResult<QualityReportModel> {
		self.media(media_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("media {media_id}")))?;
		let model = ingest_quality_report::ActiveModel {
			id: Set(Uuid::new_v4().to_string()),
			drop_item_id: Set(None),
			media_id: Set(Some(media_id.to_string())),
			source_sha256: Set(report.source_sha256.clone()),
			algorithm_version: Set(report.algorithm_version.clone()),
			score: Set(i32::from(report.score)),
			checks: Set(serde_json::to_value(&report.checks)?),
			settings_snapshot: Set(serde_json::to_value(&report.settings_snapshot)?),
			created_at: NotSet,
		};
		model
			.insert(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	/// Candidates stored against a library media row.
	pub async fn candidates_for_media(
		&self,
		media_id: &str,
	) -> CoreResult<Vec<CandidateModel>> {
		ingest_metadata_candidate::Entity::find()
			.filter(ingest_metadata_candidate::Column::MediaId.eq(media_id))
			.order_by_asc(ingest_metadata_candidate::Column::ProviderId)
			.order_by_asc(ingest_metadata_candidate::Column::Id)
			.all(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	pub async fn save_candidates_for_media(
		&self,
		media_id: &str,
		candidates: &[MetadataCandidate],
	) -> CoreResult<Vec<CandidateModel>> {
		self.media(media_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("media {media_id}")))?;
		let mut saved = Vec::with_capacity(candidates.len());
		for candidate in candidates {
			let active = ingest_metadata_candidate::ActiveModel {
				id: Set(Uuid::new_v4().to_string()),
				drop_item_id: Set(None),
				media_id: Set(Some(media_id.to_string())),
				provider_id: Set(candidate.provider_id.clone()),
				provider_version: Set(candidate.provider_version.clone()),
				external_id: Set(candidate.external_id.clone()),
				source_sha256: Set(candidate.source_sha256.clone()),
				confidence: Set(candidate.confidence),
				fields: Set(serde_json::to_value(&candidate.fields)?),
				field_confidence: Set(serde_json::to_value(&candidate.field_confidence)?),
				provenance: Set(candidate.provenance.clone()),
				status: Set("PENDING".to_string()),
				created_at: NotSet,
				updated_at: NotSet,
			};
			saved.push(active.insert(self.conn.as_ref()).await?);
		}
		Ok(saved)
	}

	pub async fn save_application(
		&self,
		application: ingest_metadata_application::ActiveModel,
	) -> CoreResult<ApplicationModel> {
		application
			.insert(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	pub async fn applications(
		&self,
		item_id: &str,
		page: Pagination,
	) -> CoreResult<(Vec<ApplicationModel>, u64)> {
		let query = ingest_metadata_application::Entity::find()
			.filter(ingest_metadata_application::Column::DropItemId.eq(item_id));

		let total = query.clone().count(self.conn.as_ref()).await?;
		let rows = query
			.order_by_desc(ingest_metadata_application::Column::CreatedAt)
			.order_by_desc(ingest_metadata_application::Column::Id)
			.offset(page.offset())
			.limit(page.limit())
			.all(self.conn.as_ref())
			.await?;
		Ok((rows, total))
	}

	pub async fn plugin_settings(
		&self,
		plugin_id: &str,
		kind: &str,
		library_id: Option<&str>,
		user_id: Option<&str>,
	) -> CoreResult<Option<PluginSettingModel>> {
		let mut query = ingest_plugin_setting::Entity::find()
			.filter(ingest_plugin_setting::Column::PluginId.eq(plugin_id))
			.filter(ingest_plugin_setting::Column::Kind.eq(kind));
		query = match library_id {
			Some(value) => {
				query.filter(ingest_plugin_setting::Column::LibraryId.eq(value))
			},
			None => query.filter(ingest_plugin_setting::Column::LibraryId.is_null()),
		};
		query = match user_id {
			Some(value) => query.filter(ingest_plugin_setting::Column::UserId.eq(value)),
			None => query.filter(ingest_plugin_setting::Column::UserId.is_null()),
		};
		query.one(self.conn.as_ref()).await.map_err(CoreError::from)
	}

	pub async fn set_plugin_settings(
		&self,
		setting: ingest_plugin_setting::ActiveModel,
	) -> CoreResult<PluginSettingModel> {
		setting
			.insert(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	pub async fn subscribe_progress(
		&self,
		library_id: Option<&str>,
		drop_item_id: Option<&str>,
		analysis_job_id: Option<&str>,
		after: Option<&str>,
	) -> Result<ProgressStream, super::progress::CursorExpired> {
		self.progress
			.subscribe(library_id, drop_item_id, analysis_job_id, after)
			.await
	}
	pub async fn subscribe_progress_stored(
		&self,
		library_id: Option<&str>,
		drop_item_id: Option<&str>,
		analysis_job_id: Option<&str>,
		after: Option<&str>,
	) -> Result<StoredProgressStream, super::progress::CursorExpired> {
		self.progress
			.subscribe_stored(library_id, drop_item_id, analysis_job_id, after)
			.await
	}

	pub(crate) async fn emit_progress(
		&self,
		event: super::contract::IngestProgressEvent,
	) -> CoreResult<super::contract::IngestProgressEvent> {
		self.progress.emit(event).await
	}

	async fn ensure_library(&self, library_id: &str) -> CoreResult<library::Model> {
		library::Entity::find_by_id(library_id)
			.one(self.conn.as_ref())
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("library {library_id}")))
	}

	async fn find_identity(
		&self,
		library_id: &str,
		source_sha256: &str,
		filename: &str,
	) -> CoreResult<Option<DropItemModel>> {
		ingest_drop_item::Entity::find()
			.filter(ingest_drop_item::Column::LibraryId.eq(library_id))
			.filter(ingest_drop_item::Column::SourceSha256.eq(source_sha256))
			.filter(ingest_drop_item::Column::SourceFilename.eq(filename))
			.one(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	async fn latest_report_for_item(
		&self,
		item_id: &str,
	) -> CoreResult<Option<QualityReportModel>> {
		ingest_quality_report::Entity::find()
			.filter(ingest_quality_report::Column::DropItemId.eq(item_id))
			.order_by_desc(ingest_quality_report::Column::CreatedAt)
			.order_by_desc(ingest_quality_report::Column::Id)
			.one(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}
	pub(crate) async fn latest_job_for_item(
		&self,
		item_id: &str,
	) -> CoreResult<Option<AnalysisJobModel>> {
		ingest_analysis_job::Entity::find()
			.filter(ingest_analysis_job::Column::DropItemId.eq(item_id))
			.order_by_desc(ingest_analysis_job::Column::CreatedAt)
			.order_by_desc(ingest_analysis_job::Column::Id)
			.one(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	pub(crate) async fn create_analysis_job(
		&self,
		item: &DropItemModel,
		force: bool,
	) -> CoreResult<AnalysisJobModel> {
		if !force {
			if let Some(existing) = self.latest_job_for_item(&item.id).await? {
				if existing.status.is_pending() {
					return Ok(existing);
				}
			}
		}
		let active = ingest_analysis_job::ActiveModel {
			id: Set(Uuid::new_v4().to_string()),
			drop_item_id: Set(Some(item.id.clone())),
			media_id: Set(None),
			job_id: Set(None),
			status: Set(JobStatus::Queued),
			phase: Set(phase_name(super::contract::AnalysisPhase::Staging)),
			priority: Set(0),
			attempts: Set(0),
			plan: Set(analysis_plan(
				&[AnalysisTarget::DropItem(item.id.clone())],
				None,
			)),
			error: Set(None),
			created_at: NotSet,
			started_at: Set(None),
			finished_at: Set(None),
		};
		let job = active.insert(self.conn.as_ref()).await?;
		let revision = item.revision;
		let mut item = item.clone().into_active_model();
		item.analysis_job_id = Set(Some(job.id.clone()));
		item.status = Set(DropItemStatus::Staged.as_str().to_string());
		item.revision = Set(revision.saturating_add(1));
		item.update(self.conn.as_ref()).await?;
		Ok(job)
	}

	/// One library-rework analysis job for a batch of existing media rows.
	/// A single-media job also fills the `media_id` column so the row cascades
	/// with its target; larger batches live only in the `plan` targets.
	pub(crate) async fn create_media_analysis_job(
		&self,
		media_ids: &[String],
		providers: Option<&[String]>,
	) -> CoreResult<AnalysisJobModel> {
		let targets: Vec<AnalysisTarget> = media_ids
			.iter()
			.map(|id| AnalysisTarget::Media(id.clone()))
			.collect();
		let active = ingest_analysis_job::ActiveModel {
			id: Set(Uuid::new_v4().to_string()),
			drop_item_id: Set(None),
			media_id: Set(match media_ids {
				[single] => Some(single.clone()),
				_ => None,
			}),
			job_id: Set(None),
			status: Set(JobStatus::Queued),
			phase: Set(phase_name(super::contract::AnalysisPhase::Staging)),
			priority: Set(0),
			attempts: Set(0),
			plan: Set(analysis_plan(&targets, providers)),
			error: Set(None),
			created_at: NotSet,
			started_at: Set(None),
			finished_at: Set(None),
		};
		Ok(active.insert(self.conn.as_ref()).await?)
	}

	/// Resolved targets of an analysis job: the persisted `plan` targets when
	/// present, otherwise the drop-item column for rows written before
	/// library-wide rework existed.
	pub(crate) fn job_targets(
		&self,
		job: &AnalysisJobModel,
	) -> CoreResult<Vec<AnalysisTarget>> {
		let targets = analysis_targets_from_plan(&job.plan, job.drop_item_id.as_deref());
		if targets.is_empty() {
			return Err(CoreError::InternalError(format!(
				"analysis job {} has no target",
				job.id
			)));
		}
		Ok(targets)
	}

	/// A pending job whose media-target set is exactly `media_ids`, so a
	/// repeated library-rework request does not enqueue a duplicate batch.
	pub(crate) async fn latest_pending_media_job_matching(
		&self,
		media_ids: &[String],
	) -> CoreResult<Option<AnalysisJobModel>> {
		let jobs = ingest_analysis_job::Entity::find()
			.filter(
				ingest_analysis_job::Column::Status
					.is_in([JobStatus::Queued, JobStatus::Running]),
			)
			.filter(ingest_analysis_job::Column::DropItemId.is_null())
			.order_by_desc(ingest_analysis_job::Column::CreatedAt)
			.all(self.conn.as_ref())
			.await?;
		Ok(jobs.into_iter().find(|job| {
			analysis_targets_from_plan(&job.plan, job.drop_item_id.as_deref())
				.iter()
				.filter_map(|target| match target {
					AnalysisTarget::Media(id) => Some(id.as_str()),
					AnalysisTarget::DropItem(_) => None,
				})
				.eq(media_ids.iter().map(String::as_str))
		}))
	}

	pub(crate) async fn queue_jobs(
		&self,
		status: Option<JobStatus>,
		page: Pagination,
	) -> CoreResult<(Vec<AnalysisJobModel>, u64)> {
		let mut query = ingest_analysis_job::Entity::find();
		if let Some(status) = status {
			query = query.filter(ingest_analysis_job::Column::Status.eq(status));
		}
		let total = query.clone().count(self.conn.as_ref()).await?;
		let jobs = query
			.order_by_asc(ingest_analysis_job::Column::Priority)
			.order_by_asc(ingest_analysis_job::Column::CreatedAt)
			.order_by_asc(ingest_analysis_job::Column::Id)
			.offset(page.offset())
			.limit(page.limit())
			.all(self.conn.as_ref())
			.await?;
		Ok((jobs, total))
	}

	pub(crate) async fn job(&self, job_id: &str) -> CoreResult<Option<AnalysisJobModel>> {
		ingest_analysis_job::Entity::find_by_id(job_id)
			.one(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	pub(crate) async fn update_job_status(
		&self,
		job_id: &str,
		status: JobStatus,
		error: Option<&str>,
	) -> CoreResult<AnalysisJobModel> {
		let job = self
			.job(job_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("analysis job {job_id}")))?;
		let mut active = job.into_active_model();
		active.status = Set(status);
		if error.is_some() {
			active.error = Set(error.map(str::to_owned));
		}
		if status.is_resolved() {
			active.finished_at = Set(Some(Utc::now().into()));
		}
		active
			.update(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	pub(crate) async fn start_job(&self, job_id: &str) -> CoreResult<AnalysisJobModel> {
		let job = self
			.job(job_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("analysis job {job_id}")))?;
		let attempts = job.attempts;
		let mut active = job.into_active_model();
		active.status = Set(JobStatus::Running);
		active.attempts = Set(attempts.saturating_add(1));
		active.started_at = Set(Some(Utc::now().into()));
		active.error = Set(None);
		active
			.update(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	pub(crate) async fn update_job_phase(
		&self,
		job_id: &str,
		phase: super::contract::AnalysisPhase,
		_done: bool,
		score: Option<u8>,
	) -> CoreResult<AnalysisJobModel> {
		let job = self
			.job(job_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("analysis job {job_id}")))?;
		let mut active = job.into_active_model();
		active.phase = Set(phase_name(phase));
		if let Some(score) = score {
			active.priority = Set(i32::from(score));
		}
		active
			.update(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	pub(crate) async fn finish_job(
		&self,
		job_id: &str,
		score: u8,
		_has_candidates: bool,
	) -> CoreResult<AnalysisJobModel> {
		let job = self
			.job(job_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("analysis job {job_id}")))?;
		let mut active = job.into_active_model();
		active.status = Set(JobStatus::Completed);
		active.phase = Set(phase_name(super::contract::AnalysisPhase::Done));
		active.priority = Set(i32::from(score));
		active.finished_at = Set(Some(Utc::now().into()));
		active
			.update(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	pub(crate) async fn fail_job(
		&self,
		job_id: &str,
		error: &str,
	) -> CoreResult<AnalysisJobModel> {
		let job = self
			.job(job_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("analysis job {job_id}")))?;
		let mut active = job.into_active_model();
		active.status = Set(JobStatus::Failed);
		active.error = Set(Some(error.to_string()));
		active.finished_at = Set(Some(Utc::now().into()));
		active
			.update(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}

	pub(crate) async fn set_item_analysis_result(
		&self,
		item_id: &str,
		score: u8,
		has_candidates: bool,
	) -> CoreResult<DropItemModel> {
		let item = self
			.item(item_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("ingest item {item_id}")))?;
		let status = if score < 100 || has_candidates {
			DropItemStatus::AwaitingReview
		} else {
			DropItemStatus::Ready
		};
		let revision = item.revision;
		let mut active = item.into_active_model();
		active.status = Set(status.as_str().to_string());
		active.error = Set(None);
		active.revision = Set(revision.saturating_add(1));
		active
			.update(self.conn.as_ref())
			.await
			.map_err(CoreError::from)
	}
}
fn phase_name(phase: super::contract::AnalysisPhase) -> String {
	serde_json::to_string(&phase)
		.expect("AnalysisPhase is always serializable")
		.trim_matches('"')
		.to_string()
}
fn media_kind_name(kind: IngestMediaKind) -> String {
	serde_json::to_string(&kind)
		.expect("IngestMediaKind is always serializable")
		.trim_matches('"')
		.to_string()
}

fn media_kind_for_filename(filename: &str) -> IngestMediaKind {
	match Path::new(filename)
		.extension()
		.and_then(|extension| extension.to_str())
		.unwrap_or_default()
		.to_ascii_lowercase()
		.as_str()
	{
		"cbz" | "zip" => IngestMediaKind::ComicArchive,
		"cbr" | "rar" => IngestMediaKind::ComicRarArchive,
		"epub" => IngestMediaKind::Epub,
		"pdf" => IngestMediaKind::Pdf,
		_ => IngestMediaKind::Unknown,
	}
}

/// File identity consumed by the shared snapshot builder: one `SnapshotSource`
/// per ingest target, whether a staged drop item or an existing library file.
struct SnapshotSource {
	/// Opaque target id: drop item id for staged runs, media id for library
	/// rework runs.
	id: String,
	library_id: String,
	path: PathBuf,
	source_filename: String,
	source_sha256: String,
	byte_size: u64,
	relative_path: String,
}

fn build_snapshot(item: DropItemModel, config: &StumpConfig) -> CoreResult<BookSnapshot> {
	let staged_path = PathBuf::from(&item.staging_path);
	if !staged_path.is_file() {
		return Err(CoreError::FileNotFound(item.staging_path));
	}
	snapshot_from_source(
		SnapshotSource {
			id: item.id,
			library_id: item.library_id,
			path: staged_path,
			source_filename: item.source_filename,
			source_sha256: item.source_sha256,
			byte_size: item.byte_size.max(0) as u64,
			relative_path: item.relative_path.unwrap_or_default(),
		},
		config,
	)
}

/// The one snapshot path shared by staged and library targets: embedded
/// metadata via `process_metadata`, pages via the same format adapters the
/// processor selection uses.
fn snapshot_from_source(
	source: SnapshotSource,
	config: &StumpConfig,
) -> CoreResult<BookSnapshot> {
	let media_kind = media_kind_for_filename(&source.source_filename);
	let embedded_metadata = process_metadata(&source.path)
		.map_err(|error| CoreError::Unknown(error.to_string()))?;
	let pages = match media_kind {
		IngestMediaKind::ComicArchive => archive_pages(&source.path)?,
		IngestMediaKind::ComicRarArchive => {
			let count =
				get_page_count(source.path.to_str().unwrap_or_default(), &config.media)
					.map_err(|error| CoreError::Unknown(error.to_string()))?;
			(0..count.max(0))
				.map(|index| IngestPageEntry {
					index: index as u32,
					path: format!("page-{}", index + 1),
					size: None,
					is_image: true,
				})
				.collect()
		},
		IngestMediaKind::Epub => epub_pages(&source.path)?,
		IngestMediaKind::Pdf => {
			let count =
				get_page_count(source.path.to_str().unwrap_or_default(), &config.media)
					.map_err(|error| CoreError::Unknown(error.to_string()))?;
			(0..count.max(0))
				.map(|index| IngestPageEntry {
					index: index as u32,
					path: format!("page-{}", index + 1),
					size: None,
					is_image: true,
				})
				.collect()
		},
		IngestMediaKind::Unknown => Vec::new(),
	};
	Ok(BookSnapshot {
		drop_item_id: source.id,
		library_id: source.library_id,
		staged_path: source.path,
		source_sha256: source.source_sha256,
		byte_size: source.byte_size,
		source_filename: source.source_filename,
		relative_path: source.relative_path,
		media_kind,
		embedded_metadata,
		pages,
		analysis: None,
	})
}

fn archive_pages(path: &Path) -> CoreResult<Vec<IngestPageEntry>> {
	let file = std::fs::File::open(path)?;
	let mut archive = zip::ZipArchive::new(file)
		.map_err(|error| CoreError::Unknown(error.to_string()))?;
	let mut names = archive.file_names().map(str::to_owned).collect::<Vec<_>>();
	names.sort_by(|left, right| alphanumeric_sort::compare_path(left, right));
	let mut pages = Vec::new();
	for name in names {
		let entry = archive
			.by_name(&name)
			.map_err(|error| CoreError::Unknown(error.to_string()))?;
		if entry.is_dir() {
			continue;
		}
		let entry_path = Path::new(&name);
		if entry_path.is_hidden_file() {
			continue;
		}
		let file_name = entry_path
			.file_name()
			.and_then(|name| name.to_str())
			.unwrap_or_default();
		if file_name.eq_ignore_ascii_case("comicinfo.xml")
			|| file_name.eq_ignore_ascii_case("thumbs.db")
			|| !entry_path.is_img()
		{
			continue;
		}
		pages.push(IngestPageEntry {
			index: pages.len() as u32,
			path: name,
			size: Some(entry.size()),
			is_image: true,
		});
	}
	Ok(pages)
}

fn epub_pages(path: &Path) -> CoreResult<Vec<IngestPageEntry>> {
	let path = path.to_string_lossy();
	let manifest = ReadiumManifestGenerator::new(path.as_ref(), "");
	let spine = manifest
		.enumerate_spine_for_positions()
		.map_err(|error| CoreError::Unknown(error.to_string()))?;
	Ok(spine
		.into_iter()
		.map(|entry| IngestPageEntry {
			index: entry.spine_index as u32,
			path: entry.package_path,
			size: Some(entry.size as u64),
			is_image: entry.media_type.starts_with("image/"),
		})
		.collect())
}

/// Result of staging one upload: the item, and whether it already existed.
#[derive(Debug, Clone)]
pub struct StagedUpload {
	pub item: DropItemModel,
	pub deduplicated: bool,
}

impl StagedUpload {
	fn fresh(item: DropItemModel) -> Self {
		Self {
			item,
			deduplicated: false,
		}
	}

	fn deduplicated(item: DropItemModel) -> Self {
		Self {
			item,
			deduplicated: true,
		}
	}
}

/// Remove a freshly staged duplicate unless it *is* the existing item's file.
async fn remove_duplicate_upload(staged: &Path, existing: &str) -> CoreResult<()> {
	if staged != Path::new(existing) {
		staging::remove_if_exists(staged).await?;
	}
	Ok(())
}

/// Explicit picks win; every pending field not named by a pick becomes a
/// `MANUAL` pick (or `CLEAR` for a JSON null).
fn merge_pending_fields(
	mut picks: Vec<FieldPick>,
	pending: Option<&Value>,
) -> Vec<FieldPick> {
	let Some(Value::Object(pending)) = pending else {
		return picks;
	};
	let explicit: std::collections::HashSet<MetadataField> =
		picks.iter().map(FieldPick::field).collect();
	for (key, value) in pending {
		let Ok(field) =
			serde_json::from_value::<MetadataField>(Value::String(key.clone()))
		else {
			continue;
		};
		if explicit.contains(&field) {
			continue;
		}
		picks.push(if value.is_null() {
			FieldPick::Clear { field }
		} else {
			FieldPick::Manual {
				field,
				value: value.clone(),
			}
		});
	}
	picks
}

#[cfg(test)]
mod tests {
	use super::*;
	use migrations::MigratorTrait;
	use sea_orm::Database;

	#[tokio::test]
	async fn media_kind_detection_is_extension_based() {
		assert_eq!(
			media_kind_for_filename("book.cbz"),
			IngestMediaKind::ComicArchive
		);
		assert_eq!(media_kind_for_filename("book.epub"), IngestMediaKind::Epub);
		assert_eq!(media_kind_for_filename("book.pdf"), IngestMediaKind::Pdf);
		assert_eq!(
			media_kind_for_filename("book.txt"),
			IngestMediaKind::Unknown
		);
	}

	#[tokio::test]
	async fn list_items_is_empty_on_migrated_database() {
		let conn = Arc::new(Database::connect("sqlite::memory:").await.unwrap());
		migrations::Migrator::up(conn.as_ref(), None).await.unwrap();
		let store = IngestStore::new(Arc::new(StumpConfig::debug()), conn);
		let result = store.list_items(None, None, Pagination::default()).await;
		assert!(result.is_ok());
	}

	/// Lowest score first with unscored items ranking as 0, committed and
	/// rejected items excluded, and pagination applied in SQL (the previous
	/// implementation loaded every row and passed `u64::MAX` as a LIMIT, which
	/// SQLite's binder rejects with a panic).
	#[tokio::test]
	async fn rework_items_order_lowest_score_first_and_paginate() {
		use models::{
			entity::{ingest_drop_item, library, library_config},
			shared::enums::FileStatus,
		};

		let conn = Arc::new(Database::connect("sqlite::memory:").await.unwrap());
		migrations::Migrator::up(conn.as_ref(), None).await.unwrap();
		let config = <library_config::ActiveModel as std::default::Default>::default()
			.insert(conn.as_ref())
			.await
			.unwrap();
		library::ActiveModel {
			id: Set("library".to_string()),
			name: Set("Library".to_string()),
			path: Set("/tmp/library".to_string()),
			status: Set(FileStatus::Ready),
			config_id: Set(config.id),
			..Default::default()
		}
		.insert(conn.as_ref())
		.await
		.unwrap();
		let store = IngestStore::new(Arc::new(StumpConfig::debug()), conn.clone());

		// (id, status, score) — `None` means no report yet.
		let fixtures = [
			("high", DropItemStatus::AwaitingReview, Some(92)),
			("low", DropItemStatus::AwaitingReview, Some(40)),
			("unscored", DropItemStatus::Staged, None),
			("done", DropItemStatus::Committed, Some(10)),
			("gone", DropItemStatus::Rejected, Some(5)),
			("perfect", DropItemStatus::Ready, Some(100)),
		];
		for (id, status, score) in fixtures {
			ingest_drop_item::ActiveModel {
				id: Set(id.to_string()),
				library_id: Set("library".to_string()),
				source_filename: Set(format!("{id}.epub")),
				byte_size: Set(1),
				source_sha256: Set(id.to_string()),
				media_kind: Set("EPUB".to_string()),
				staging_path: Set(format!("/tmp/{id}.epub")),
				status: Set(status.as_str().to_string()),
				revision: Set(1),
				..Default::default()
			}
			.insert(conn.as_ref())
			.await
			.unwrap();
			if let Some(score) = score {
				let report = QualityReport {
					source_sha256: id.to_string(),
					algorithm_version: crate::ingest::contract::QUALITY_ALGORITHM_VERSION
						.to_string(),
					score,
					checks: Vec::new(),
					settings_snapshot: Default::default(),
				};
				store.save_report(id, &report).await.unwrap();
			}
		}

		let (items, total) = store
			.rework_items(100, Pagination::default())
			.await
			.unwrap();
		assert_eq!(
			total, 3,
			"committed, rejected, and score-100 items are excluded"
		);
		assert_eq!(
			items
				.iter()
				.map(|item| item.id.as_str())
				.collect::<Vec<_>>(),
			["unscored", "low", "high"]
		);

		let (page, total) = store
			.rework_items(
				100,
				Pagination {
					page: 2,
					page_size: 2,
				},
			)
			.await
			.unwrap();
		assert_eq!(total, 3);
		assert_eq!(page.len(), 1);
		assert_eq!(page[0].id, "high");

		let (strict, _) = store.rework_items(50, Pagination::default()).await.unwrap();
		assert_eq!(
			strict
				.iter()
				.map(|item| item.id.as_str())
				.collect::<Vec<_>>(),
			["unscored", "low"]
		);
	}
	#[test]
	fn pending_fields_merge_behind_explicit_picks() {
		let pending =
			json!({"TITLE": "Pending", "SERIES": "Saga", "TAGS": null, "NOPE": 1});
		let picks = merge_pending_fields(
			vec![FieldPick::Manual {
				field: MetadataField::Title,
				value: json!("Explicit"),
			}],
			Some(&pending),
		);
		assert_eq!(picks.len(), 3);
		assert!(
			matches!(&picks[0], FieldPick::Manual { field: MetadataField::Title, value } if value == "Explicit")
		);
		assert!(picks.iter().any(|pick| matches!(pick, FieldPick::Manual { field: MetadataField::Series, value } if value == "Saga")));
		assert!(picks.iter().any(|pick| matches!(
			pick,
			FieldPick::Clear {
				field: MetadataField::Tags
			}
		)));
		assert_eq!(merge_pending_fields(Vec::new(), None).len(), 0);
	}
	/// Re-uploading identical bytes under the same name must return the
	/// existing item, flag it as deduplicated, and leave its staged file in
	/// place (the duplicate stages to the same deterministic path).
	#[tokio::test]
	async fn duplicate_upload_keeps_the_existing_staged_file() {
		use models::{
			entity::{library, library_config},
			shared::enums::FileStatus,
		};

		let conn = Arc::new(Database::connect("sqlite::memory:").await.unwrap());
		migrations::Migrator::up(conn.as_ref(), None).await.unwrap();
		let config = <library_config::ActiveModel as std::default::Default>::default()
			.insert(conn.as_ref())
			.await
			.unwrap();
		library::ActiveModel {
			id: Set("library".to_string()),
			name: Set("Library".to_string()),
			path: Set("/tmp/library".to_string()),
			status: Set(FileStatus::Ready),
			config_id: Set(config.id),
			..Default::default()
		}
		.insert(conn.as_ref())
		.await
		.unwrap();
		let temporary = tempfile::tempdir().unwrap();
		let mut stump_config = StumpConfig::debug();
		stump_config.ingest.ingest_staging_dir = Some(
			temporary
				.path()
				.join("staging")
				.to_string_lossy()
				.into_owned(),
		);
		let store = IngestStore::new(Arc::new(stump_config), conn);

		let first = store
			.stage_upload("library", None, None, "book.cbz", &b"identical"[..], None)
			.await
			.unwrap();
		assert!(!first.deduplicated);
		assert!(Path::new(&first.item.staging_path).is_file());

		let second = store
			.stage_upload("library", None, None, "book.cbz", &b"identical"[..], None)
			.await
			.unwrap();
		assert!(second.deduplicated);
		assert_eq!(second.item.id, first.item.id);
		assert!(
			Path::new(&first.item.staging_path).is_file(),
			"dedup must not delete the existing item's staged file"
		);
	}

	/// Applying field picks against a library media target: candidates saved
	/// with `media_id` resolve and `apply_to_media` (the same path approve
	/// uses) writes `media_metadata` and the audit row for the media.
	#[tokio::test]
	async fn apply_metadata_writes_media_metadata_for_media_target() {
		use crate::ingest::providers::apply::apply_to_media;
		use metadata_integrations::MergeStrategy;
		use models::{
			entity::{
				ingest_metadata_application, library, library_config, media, series,
			},
			shared::enums::FileStatus,
		};
		use sea_orm::{
			ActiveModelTrait, ColumnTrait, Database, EntityTrait, QueryFilter, Set,
		};

		let conn = Arc::new(Database::connect("sqlite::memory:").await.unwrap());
		migrations::Migrator::up(conn.as_ref(), None).await.unwrap();
		let config = <library_config::ActiveModel as std::default::Default>::default()
			.insert(conn.as_ref())
			.await
			.unwrap();
		library::ActiveModel {
			id: Set("library".to_string()),
			name: Set("Library".to_string()),
			path: Set("/tmp/library".to_string()),
			status: Set(FileStatus::Ready),
			config_id: Set(config.id),
			..Default::default()
		}
		.insert(conn.as_ref())
		.await
		.unwrap();
		series::ActiveModel {
			id: Set("series".to_string()),
			name: Set("Saga".to_string()),
			path: Set("/tmp/library/Saga".to_string()),
			status: Set(FileStatus::Ready),
			library_id: Set(Some("library".to_string())),
			..Default::default()
		}
		.insert(conn.as_ref())
		.await
		.unwrap();
		media::ActiveModel {
			id: Set("media-1".to_string()),
			name: Set("Saga 001".to_string()),
			size: Set(1024),
			extension: Set("cbz".to_string()),
			pages: Set(2),
			hash: Set(Some("fixture-digest".to_string())),
			path: Set("/tmp/library/Saga/Saga 001.cbz".to_string()),
			series_id: Set(Some("series".to_string())),
			..Default::default()
		}
		.insert(conn.as_ref())
		.await
		.unwrap();
		let store = IngestStore::new(Arc::new(StumpConfig::debug()), conn.clone());

		let candidate = MetadataCandidate {
			provider_id: "test-provider".to_string(),
			provider_version: "1".to_string(),
			external_id: Some("external-1".to_string()),
			source_sha256: "fixture-digest".to_string(),
			confidence: 0.9,
			fields: [(MetadataField::Title, json!("Saga Vol 1"))]
				.into_iter()
				.collect(),
			field_confidence: [(MetadataField::Title, 0.9)].into_iter().collect(),
			provenance: json!({"source": "test"}),
		};
		let saved = store
			.save_candidates_for_media("media-1", &[candidate])
			.await
			.unwrap();
		assert_eq!(saved.len(), 1);
		assert_eq!(saved[0].media_id.as_deref(), Some("media-1"));
		assert_eq!(saved[0].drop_item_id, None);

		let candidates = store.candidates_for_media("media-1").await.unwrap();
		let picks = vec![FieldPick::Candidate {
			field: MetadataField::Title,
			candidate_id: saved[0].id.clone(),
		}];
		let resolved = resolve_picks_for_context(
			&picks,
			&candidates,
			None,
			&[],
			MergeStrategy::FillGaps,
			None,
			None,
		)
		.unwrap();
		apply_to_media(store.conn(), "media-1", resolved, "tester")
			.await
			.unwrap();

		let metadata = models::entity::media_metadata::Entity::find()
			.filter(models::entity::media_metadata::Column::MediaId.eq("media-1"))
			.one(conn.as_ref())
			.await
			.unwrap()
			.expect("media metadata row created for the media target");
		assert_eq!(metadata.title.as_deref(), Some("Saga Vol 1"));

		let application = ingest_metadata_application::Entity::find()
			.filter(ingest_metadata_application::Column::MediaId.eq("media-1"))
			.one(conn.as_ref())
			.await
			.unwrap()
			.expect("application audit row recorded for the media target");
		assert_eq!(application.drop_item_id, None);
		assert_eq!(application.media_id.as_deref(), Some("media-1"));
		assert_eq!(application.actor, "tester");
	}
}
