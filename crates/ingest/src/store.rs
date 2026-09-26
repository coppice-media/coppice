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
	archive,
	contract::{
		AssembledAudio, AudioAnalysis, BookSnapshot, DropItemStatus, FieldPick,
		FixAction, IngestMediaKind, IngestPageEntry, MetadataCandidate, MetadataField,
		QualityReport,
	},
	drop_folder,
	policy::AudioPolicy,
	preprocess::{HookOutcome, PreprocessHook},
	progress::{ProgressHub, ProgressStream, StoredProgressStream},
	providers::apply::{
		apply_to_media_txn_with_context_and_cover, prepare_cover_for_new_media,
		resolve_picks_for_context, validate_picks, CoverApplyConfig, CoverWrite,
	},
	staging,
};
use crate::{
	config::IngestSettings,
	error::{IngestError, IngestResult},
	event::{IngestEvent, IngestEventSink},
	host::RowFactory,
};
use stump_media::{
	media::{get_page_count, process_metadata, ReadiumManifestGenerator},
	move_file, PathUtils,
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
	config: Arc<IngestSettings>,
	conn: Arc<DatabaseConnection>,
	progress: Arc<ProgressHub>,
	/// Where drop-item row changes are announced. `None` in tests, which
	/// assert on rows instead.
	events: Option<Arc<dyn IngestEventSink>>,
}

impl IngestStore {
	pub fn new(config: Arc<IngestSettings>, conn: Arc<DatabaseConnection>) -> Self {
		let progress =
			Arc::new(ProgressHub::new(conn.clone(), config.progress_retention));
		Self {
			config,
			conn,
			progress,
			events: None,
		}
	}

	/// Attach the host's event sink so every persisted drop-item revision is
	/// announced. Called by [`crate::IngestServices`] at construction.
	pub fn with_event_sink(mut self, events: Arc<dyn IngestEventSink>) -> Self {
		self.events = Some(events);
		self
	}

	pub fn config(&self) -> &Arc<IngestSettings> {
		&self.config
	}

	pub fn conn(&self) -> &Arc<DatabaseConnection> {
		&self.conn
	}

	pub fn progress(&self) -> &Arc<ProgressHub> {
		&self.progress
	}

	/// Announce a drop-item row the store just persisted. Every write to
	/// `ingest_drop_item` bumps `revision`, so one call per successful write
	/// is the complete change feed for the row.
	fn announce(&self, item: &DropItemModel) {
		let Some(sink) = &self.events else {
			return;
		};
		let Some(status) = DropItemStatus::parse(&item.status) else {
			tracing::warn!(
				item_id = %item.id,
				status = %item.status,
				"Drop item carries an unknown status; not announcing the change"
			);
			return;
		};
		sink.emit(IngestEvent::ItemChanged {
			library_id: item.library_id.clone(),
			item_id: item.id.clone(),
			status,
			revision: item.revision,
		});
	}

	/// Update a drop item and announce the row that landed. Every mutation of
	/// `ingest_drop_item` outside a transaction goes through here, so the
	/// change feed cannot drift from what was written.
	async fn persist(
		&self,
		active: ingest_drop_item::ActiveModel,
	) -> IngestResult<DropItemModel> {
		let item = active.update(self.conn.as_ref()).await?;
		self.announce(&item);
		Ok(item)
	}

	pub async fn stage_upload<R>(
		&self,
		library_id: &str,
		created_by: Option<&str>,
		relative_path: Option<&str>,
		filename: &str,
		reader: R,
		idempotency_key: Option<&str>,
	) -> IngestResult<StagedUpload>
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
			&self.config.staging_dir,
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

		let active = NewDropItem {
			library_id: library_id.to_string(),
			created_by: created_by.map(str::to_owned),
			source_filename: filename.clone(),
			relative_path,
			byte_size: staged.byte_size,
			source_sha256: staged.source_sha256.clone(),
			media_kind: media_kind_for_filename(&filename),
			staging_path: staged.path.to_string_lossy().into_owned(),
			idempotency_key: idempotency_key.map(str::to_owned),
			..NewDropItem::default()
		}
		.into_active_model()?;
		match active.insert(self.conn.as_ref()).await {
			Ok(item) => {
				self.announce(&item);
				Ok(StagedUpload::fresh(item))
			},
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

	/// Copy a publication directory into staged ingest while retaining the
	/// source tree. An optional expected digest is checked before a row is
	/// admitted; this is used when materializing a verified source-worker item.
	pub async fn stage_directory_copy(
		&self,
		library_id: &str,
		created_by: Option<&str>,
		relative_path: Option<&str>,
		filename: &str,
		source: &Path,
		expected_sha256: Option<&str>,
		idempotency_key: Option<&str>,
	) -> IngestResult<StagedUpload> {
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

		let (source_sha256, byte_size) = staging::hash_dir(source).await?;
		if byte_size == 0 {
			return Err(IngestError::BadRequest(
				"cannot stage an empty publication directory".to_string(),
			));
		}
		if expected_sha256.is_some_and(|expected| expected != source_sha256) {
			return Err(IngestError::BadRequest(
				"materialized directory digest does not match its verified source"
					.to_string(),
			));
		}
		if let Some(item) = self
			.find_identity(library_id, &source_sha256, &filename)
			.await?
		{
			return Ok(StagedUpload::deduplicated(item));
		}

		let staged_path = staging::copy_dir_into_staging(
			source,
			&self.config.staging_dir,
			library_id,
			&filename,
			&source_sha256,
		)
		.await?;
		let active = NewDropItem {
			library_id: library_id.to_string(),
			created_by: created_by.map(str::to_owned),
			source_filename: filename.clone(),
			relative_path,
			byte_size,
			source_sha256: source_sha256.clone(),
			media_kind: IngestMediaKind::Audio,
			staging_path: staged_path.to_string_lossy().into_owned(),
			idempotency_key: idempotency_key.map(str::to_owned),
			..NewDropItem::default()
		}
		.into_active_model()?;
		match active.insert(self.conn.as_ref()).await {
			Ok(item) => {
				self.announce(&item);
				Ok(StagedUpload::fresh(item))
			},
			Err(error) => {
				if let Some(item) = self
					.find_identity(library_id, &source_sha256, &filename)
					.await?
				{
					Ok(StagedUpload::deduplicated(item))
				} else {
					staging::remove_staged(&staged_path).await?;
					Err(error.into())
				}
			},
		}
	}

	/// Admit everything sitting in a library's drop folder.
	///
	/// A dropped file is normally one publication. A dropped `.zip`/`.rar`/
	/// `.7z` may instead be a *delivery* — a folder of MP3 parts, an EPUB
	/// beside its MOBI and cover, several books — and those explode into one
	/// item per publication through [`Self::explode_archive`]. The question is
	/// answered from the container's member list, so a comic archive is never
	/// extracted to discover it was a comic.
	pub async fn scan_drop_folder(
		&self,
		library_id: &str,
	) -> IngestResult<Vec<DropItemModel>> {
		self.ensure_library(library_id).await?;
		let root = self.config.drop_dir.join(library_id);
		let files = drop_folder::list_files(&root).await?;
		let mut admitted = Vec::with_capacity(files.len());
		for file in files {
			if let Some(exploded) = self.try_explode(library_id, &file).await? {
				admitted.extend(exploded);
				continue;
			}
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
				&self.config.staging_dir,
				library_id,
				&file.filename,
				&source_sha256,
			)
			.await?;
			let active = NewDropItem {
				library_id: library_id.to_string(),
				source_filename: file.filename.clone(),
				relative_path: parent_of(&file.relative_path),
				byte_size,
				source_sha256: source_sha256.clone(),
				media_kind: media_kind_for_filename(&file.filename),
				staging_path: staged_path.to_string_lossy().into_owned(),
				..NewDropItem::default()
			}
			.into_active_model()?;
			match active.insert(self.conn.as_ref()).await {
				Ok(item) => {
					self.announce(&item);
					admitted.push(item);
				},
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

	/// Explode a dropped container, or decline and let the normal path stage
	/// it as one book.
	///
	/// `Ok(None)` means "not a delivery": the extension is not a container's,
	/// or its member list holds no publication (a comic archive, an
	/// EPUB-shaped zip). Anything else is exploded, and a *failed* explosion
	/// is `Ok(Some(vec![failed item]))` rather than an error, because a
	/// missing `unrar` must fail one drop and not the whole scan.
	async fn try_explode(
		&self,
		library_id: &str,
		file: &drop_folder::DropFile,
	) -> IngestResult<Option<Vec<DropItemModel>>> {
		let Some(format) = archive::ArchiveFormat::for_filename(&file.filename) else {
			return Ok(None);
		};
		let path = file.path.clone();
		let members = match tokio::task::spawn_blocking(move || {
			archive::list_members(format, &path)
		})
		.await
		.map_err(|error| IngestError::Unknown(error.to_string()))?
		{
			Ok(members) => members,
			// An unreadable container is a failed drop, not a book: a
			// `.rar` whose header cannot be read has no pages either.
			Err(error) => {
				return Ok(Some(vec![
					self.fail_drop(library_id, file, &error.to_string()).await?,
				]));
			},
		};
		if !archive::holds_publications(&members) {
			return Ok(None);
		}
		Ok(Some(self.explode_archive(library_id, file, format).await?))
	}

	/// Extract a container into staging and admit one drop item per
	/// publication, all sharing a drop group.
	///
	/// The archive is removed on success: it was the *delivery*, and leaving
	/// it in the drop folder would re-explode it on every scan. On failure it
	/// is left exactly where it was, so fixing the extractor and rescanning is
	/// the whole recovery.
	pub async fn explode_archive(
		&self,
		library_id: &str,
		file: &drop_folder::DropFile,
		format: archive::ArchiveFormat,
	) -> IngestResult<Vec<DropItemModel>> {
		let scratch = self
			.config
			.staging_dir
			.join(library_id)
			.join(format!(".explode-{}", Uuid::new_v4()));
		let source = file.path.clone();
		let destination = scratch.clone();
		let exploded = tokio::task::spawn_blocking(move || {
			archive::extract(format, &source, &destination)?;
			archive::classify(&destination)
		})
		.await
		.map_err(|error| IngestError::Unknown(error.to_string()))?;
		let exploded = match exploded {
			Ok(exploded) => exploded,
			Err(error) => {
				let _ = fs::remove_dir_all(&scratch).await;
				return Ok(vec![
					self.fail_drop(library_id, file, &error.to_string()).await?,
				]);
			},
		};
		if exploded.items.is_empty() {
			let _ = fs::remove_dir_all(&scratch).await;
			let reason = if exploded.notes.is_empty() {
				format!("{} held no publications", file.filename)
			} else {
				format!(
					"{} held no publications: {}",
					file.filename,
					exploded.notes.join("; ")
				)
			};
			return Ok(vec![self.fail_drop(library_id, file, &reason).await?]);
		}

		// One group per successful explosion, so a re-drop of the same archive
		// produces a new group whose members deduplicate onto the existing
		// items individually.
		let drop_group_id = Uuid::new_v4().to_string();
		let container_directory = parent_of(&file.relative_path);
		let mut admitted = Vec::with_capacity(exploded.items.len());
		for item in exploded.items {
			match self
				.admit_exploded(
					library_id,
					&drop_group_id,
					container_directory.as_deref(),
					item,
				)
				.await
			{
				Ok(model) => admitted.push(model),
				Err(error) => {
					// One unstageable member must not swallow the rest of the
					// delivery, and it must not vanish either.
					tracing::error!(
						?error,
						archive = %file.filename,
						"Failed to admit one member of an exploded archive"
					);
					admitted.push(
						self.fail_drop(library_id, file, &error.to_string()).await?,
					);
				},
			}
		}
		if !exploded.notes.is_empty() {
			tracing::info!(
				archive = %file.filename,
				notes = ?exploded.notes,
				"Archive members that became neither an item nor a sidecar"
			);
		}
		let _ = fs::remove_dir_all(&scratch).await;
		staging::remove_if_exists(&file.path).await?;
		Ok(admitted)
	}

	/// Stage one exploded publication and insert its row.
	async fn admit_exploded(
		&self,
		library_id: &str,
		drop_group_id: &str,
		container_directory: Option<&str>,
		item: archive::ExplodedItem,
	) -> IngestResult<DropItemModel> {
		let is_folder = item.path.is_dir();
		let (source_sha256, byte_size) = if is_folder {
			staging::hash_dir(&item.path).await?
		} else {
			staging::hash_file(&item.path).await?
		};
		if let Some(existing) = self
			.find_identity(library_id, &source_sha256, &item.filename)
			.await?
		{
			return Ok(existing);
		}
		let staged_path = if is_folder {
			staging::move_dir_into_staging(
				&item.path,
				&self.config.staging_dir,
				library_id,
				&item.filename,
				&source_sha256,
			)
			.await?
		} else {
			staging::move_into_staging(
				&item.path,
				&self.config.staging_dir,
				library_id,
				&item.filename,
				&source_sha256,
			)
			.await?
		};
		// A folder book's sidecars moved with it; a file's are staged beside
		// it under the same digest prefix so they are found and removed with
		// the item.
		let sidecars = if is_folder {
			staging::sidecars_in(&staged_path).await?
		} else {
			self.stage_sidecars(library_id, &source_sha256, &item.sidecars)
				.await?
		};
		let relative_path =
			join_relative(container_directory, item.relative_path.as_deref());
		let active = NewDropItem {
			library_id: library_id.to_string(),
			source_filename: item.filename,
			relative_path,
			byte_size,
			source_sha256,
			media_kind: item.kind,
			staging_path: staged_path.to_string_lossy().into_owned(),
			drop_group_id: Some(drop_group_id.to_string()),
			sidecar_paths: sidecars,
			..NewDropItem::default()
		}
		.into_active_model()?;
		let model = active.insert(self.conn.as_ref()).await?;
		self.announce(&model);
		Ok(model)
	}

	/// Move an item's sidecars beside its staged file, keeping their names.
	async fn stage_sidecars(
		&self,
		library_id: &str,
		sha256: &str,
		sidecars: &[PathBuf],
	) -> IngestResult<Vec<String>> {
		let mut staged = Vec::with_capacity(sidecars.len());
		for sidecar in sidecars {
			let Some(name) = sidecar.file_name().and_then(|name| name.to_str()) else {
				continue;
			};
			let target = staging::move_into_staging(
				sidecar,
				&self.config.staging_dir,
				library_id,
				name,
				sha256,
			)
			.await?;
			staged.push(target.to_string_lossy().into_owned());
		}
		staged.sort();
		Ok(staged)
	}

	/// One failed drop item for a container that could not be exploded.
	///
	/// The archive itself is left in the drop folder: the reason names the
	/// missing tool or the broken container, and a rescan after fixing either
	/// is the retry.
	async fn fail_drop(
		&self,
		library_id: &str,
		file: &drop_folder::DropFile,
		reason: &str,
	) -> IngestResult<DropItemModel> {
		if let Some(existing) = ingest_drop_item::Entity::find()
			.filter(ingest_drop_item::Column::LibraryId.eq(library_id))
			.filter(ingest_drop_item::Column::SourceFilename.eq(file.filename.clone()))
			.filter(ingest_drop_item::Column::Status.eq(DropItemStatus::Failed.as_str()))
			.one(self.conn.as_ref())
			.await?
		{
			// A rescan of an archive that still cannot be extracted updates
			// the reason rather than accumulating one row per scan.
			let revision = existing.revision;
			let mut active = existing.into_active_model();
			active.error = Set(Some(reason.to_string()));
			active.revision = Set(revision.saturating_add(1));
			return self.persist(active).await;
		}
		let active = NewDropItem {
			library_id: library_id.to_string(),
			source_filename: file.filename.clone(),
			relative_path: parent_of(&file.relative_path),
			byte_size: file.byte_size,
			// The archive is still in the drop folder, so the row records
			// where it is rather than a staging path it never reached.
			source_sha256: String::new(),
			media_kind: media_kind_for_filename(&file.filename),
			staging_path: file.path.to_string_lossy().into_owned(),
			status: DropItemStatus::Failed,
			error: Some(reason.to_string()),
			..NewDropItem::default()
		}
		.into_active_model()?;
		let model = active.insert(self.conn.as_ref()).await?;
		self.announce(&model);
		Ok(model)
	}

	/// The other items that arrived in the same delivery, oldest first.
	pub async fn group_siblings(
		&self,
		item: &DropItemModel,
	) -> IngestResult<Vec<DropItemModel>> {
		let Some(group) = item.drop_group_id.as_deref() else {
			return Ok(Vec::new());
		};
		Ok(ingest_drop_item::Entity::find()
			.filter(ingest_drop_item::Column::DropGroupId.eq(group))
			.filter(ingest_drop_item::Column::Id.ne(item.id.clone()))
			.order_by_asc(ingest_drop_item::Column::CreatedAt)
			.order_by_asc(ingest_drop_item::Column::Id)
			.all(self.conn.as_ref())
			.await?)
	}

	pub async fn drop_folder(&self, library_id: &str) -> IngestResult<DropFolderInfo> {
		self.ensure_library(library_id).await?;
		let path = self.config.drop_dir.join(library_id);
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
	) -> IngestResult<(Vec<DropItemModel>, u64)> {
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

	pub async fn item(&self, id: &str) -> IngestResult<Option<DropItemModel>> {
		ingest_drop_item::Entity::find_by_id(id)
			.one(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}

	pub async fn bulk_items(&self, ids: &[String]) -> IngestResult<Vec<DropItemModel>> {
		if ids.is_empty() {
			return Ok(Vec::new());
		}
		ingest_drop_item::Entity::find()
			.filter(ingest_drop_item::Column::Id.is_in(ids.to_vec()))
			.order_by_asc(ingest_drop_item::Column::CreatedAt)
			.order_by_asc(ingest_drop_item::Column::Id)
			.all(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}

	/// Items below `min_score`, lowest score first (missing report = 0), then
	/// oldest first, then id — one joined query, paginated in SQL.
	pub async fn rework_items(
		&self,
		min_score: u8,
		page: Pagination,
	) -> IngestResult<(Vec<DropItemModel>, u64)> {
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

	pub async fn snapshot(&self, item_id: &str) -> IngestResult<BookSnapshot> {
		let item = self
			.item(item_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("ingest item {item_id}")))?;
		let config = self.config.clone();
		tokio::task::spawn_blocking(move || build_snapshot(item, &config))
			.await
			.map_err(|error| IngestError::Unknown(error.to_string()))?
	}

	pub async fn media(&self, id: &str) -> IngestResult<Option<media::Model>> {
		media::Entity::find_by_id(id)
			.one(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}

	/// The library a media row belongs to, through its series.
	pub(crate) async fn media_library(
		&self,
		media_row: &media::Model,
	) -> IngestResult<String> {
		let series_id = media_row.series_id.as_deref().ok_or_else(|| {
			IngestError::BadRequest(format!("media {} has no series", media_row.id))
		})?;
		series::Entity::find_by_id(series_id)
			.one(self.conn.as_ref())
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("series {series_id}")))?
			.library_id
			.ok_or_else(|| {
				IngestError::BadRequest(format!("series {series_id} has no library"))
			})
	}

	/// Snapshot of an existing library file, built through the same
	/// `snapshot_from_source` path as staged drop items — one shared
	/// convention for parsing, pages, and digests.
	pub async fn media_snapshot(&self, media_id: &str) -> IngestResult<BookSnapshot> {
		let media_row = self
			.media(media_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("media {media_id}")))?;
		let path = PathBuf::from(&media_row.path);
		if !path.is_file() {
			return Err(IngestError::FileNotFound(media_row.path.clone()));
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
				IngestError::BadRequest(format!(
					"media path {} has no usable file name",
					media_row.path
				))
			})?;
		// A library rework target is a path on disk; its kind is what the
		// extension says, which is exactly what the scanner recorded for it.
		let media_kind = media_kind_for_filename(&source_filename);
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
					media_kind,
				},
				&config,
			)
		})
		.await
		.map_err(|error| IngestError::Unknown(error.to_string()))?
	}

	/// Remove one staged item and everything it owns.
	///
	/// A folder audiobook's target is a directory and an item's sidecars are
	/// files it owns rather than files it is; both go with the row, or
	/// discarding an archive drop would leave its cover art and its MP3 parts
	/// in staging forever.
	pub async fn discard(&self, item_id: &str) -> IngestResult<DropItemModel> {
		let item = self
			.item(item_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("ingest item {item_id}")))?;
		staging::remove_staged(Path::new(&item.staging_path)).await?;
		for sidecar in Self::sidecars(&item) {
			staging::remove_staged(Path::new(&sidecar)).await?;
		}
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
	) -> IngestResult<DropItemModel> {
		let item = self
			.item(item_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("ingest item {item_id}")))?;
		if !matches!(
			DropItemStatus::parse(&item.status),
			Some(DropItemStatus::Committed | DropItemStatus::Rejected)
		) {
			let rejected_dir = self.config.staging_dir.join("rejected");
			fs::create_dir_all(&rejected_dir).await?;
			let target =
				rejected_dir.join(format!("{}-{}", item.id, item.source_filename));
			let staged = PathBuf::from(&item.staging_path);
			if staged.is_dir() {
				staging::move_dir(&staged, &target).await?;
			} else if staged.is_file() {
				move_file(&staged, &target).await?;
			}
		}
		let revision = item.revision;
		let mut active = item.into_active_model();
		active.status = Set(DropItemStatus::Rejected.as_str().to_string());
		active.error = Set(reason.map(str::to_owned));
		active.revision = Set(revision.saturating_add(1));
		self.persist(active).await
	}
	/// Commit a staged item into its library: the file moves to its final
	/// path, `rows` builds the series and media rows the same way a scan
	/// would, and the picked metadata is applied in the same transaction.
	pub async fn approve(
		&self,
		item_id: &str,
		picks: Vec<FieldPick>,
		expected_revision: i32,
		rows: Arc<dyn RowFactory>,
	) -> IngestResult<(String, DropItemModel)> {
		let item = self
			.item(item_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("ingest item {item_id}")))?;
		if item.revision != expected_revision {
			return Err(IngestError::BadRequest(format!(
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
						IngestError::BadRequest(
							"committed ingest item has no media".to_string(),
						)
					});
			}
			if matches!(status, DropItemStatus::Rejected | DropItemStatus::Failed) {
				return Err(IngestError::BadRequest(format!(
					"cannot approve ingest item in {status:?} state"
				)));
			}
		}
		// Field values selected earlier via `applyIngestMetadata` live in the
		// item's pending projection until approval; explicit picks in this
		// call take precedence, everything else pending is applied as-is.
		let picks = merge_pending_fields(picks, item.pending_fields.as_ref());
		validate_picks(&picks)
			.map_err(|error| IngestError::BadRequest(error.to_string()))?;
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
		.map_err(|error| IngestError::BadRequest(error.to_string()))?;
		let library = self.ensure_library(&item.library_id).await?;
		let relative_path =
			staging::normalize_relative_path(item.relative_path.as_deref())?;
		let filename = staging::sanitize_filename(&item.source_filename)?;
		let library_root = PathBuf::from(&library.path);
		let destination = library_root
			.join(relative_path.as_deref().unwrap_or_default())
			.join(filename);
		if !destination.starts_with(&library_root) {
			return Err(IngestError::BadRequest(
				"destination escapes library root".to_string(),
			));
		}
		let staged = PathBuf::from(&item.staging_path);
		// A folder audiobook commits as a directory of parts, which is the
		// shape the scanner already recognises as one book.
		let staged_is_dir = staged.is_dir();
		if !staged.is_file() && !staged_is_dir {
			return Err(IngestError::FileNotFound(item.staging_path));
		}
		if fs::try_exists(&destination).await? {
			return Err(IngestError::BadRequest(format!(
				"destination already exists: {}",
				destination.display()
			)));
		}
		let series_path = destination
			.parent()
			.ok_or_else(|| {
				IngestError::BadRequest("destination has no series directory".to_string())
			})?
			.to_path_buf();
		fs::create_dir_all(&series_path).await?;

		let config = library_config::Entity::find_by_id(library.config_id)
			.one(self.conn.as_ref())
			.await?
			.ok_or_else(|| {
				IngestError::NotFound(format!("library config {}", library.config_id))
			})?;
		let series_path_string = series_path.to_string_lossy().into_owned();
		let existing_series = series::Entity::find()
			.filter(series::Column::Path.eq(series_path_string.clone()))
			.filter(series::Column::LibraryId.eq(item.library_id.clone()))
			.one(self.conn.as_ref())
			.await?;

		// The media builder and cover preparation both run before the write
		// transaction.  This keeps network/image work out of the SQLite lock.
		let staging_path = item.staging_path.clone();
		let sidecars = Self::sidecars(&item);
		let drop_group_id = item.drop_group_id.clone();
		let media_kind = media_kind_for_item(&item);
		let created_by = item.created_by.clone();
		if staged_is_dir {
			staging::move_dir(&staged, &destination).await?;
		} else {
			move_file(&staged, &destination).await?;
		}
		let moved_sidecars =
			move_sidecars_beside(&sidecars, &destination, staged_is_dir).await;
		let cover_config = CoverApplyConfig::new(
			self.config.media.clone(),
			self.config.max_image_upload_size,
		);
		let commit = async {
			let (series_id, new_series) = if let Some(existing) = existing_series {
				(existing.id, None)
			} else {
				let series_path_for_build = series_path.clone();
				let library_id = item.library_id.clone();
				let factory = rows.clone();
				let (mut series_model, mut series_metadata) =
					tokio::task::spawn_blocking(move || {
						factory.series_rows(&series_path_for_build, &library_id)
					})
					.await
					.map_err(|error| IngestError::Unknown(error.to_string()))??;
				let series_id = match &series_model.id {
					Set(id) => id.clone(),
					_ => Uuid::new_v4().to_string(),
				};
				series_model.id = Set(series_id.clone());
				if let Some(metadata) = &mut series_metadata {
					metadata.series_id = Set(series_id.clone());
				}
				(series_id, Some((series_model, series_metadata)))
			};
			let media_config = config.clone();
			let destination_for_build = destination.clone();
			let series_id_for_build = series_id.clone();
			let factory = rows.clone();
			let (mut media_active, mut media_meta) =
				tokio::task::spawn_blocking(move || {
					factory.media_rows(
						&destination_for_build,
						&series_id_for_build,
						media_config,
					)
				})
				.await
				.map_err(|error| IngestError::Unknown(error.to_string()))??;
			let media_id = match &media_active.id {
				Set(id) => id.clone(),
				_ => Uuid::new_v4().to_string(),
			};
			media_active.id = Set(media_id.clone());
			if let Some(metadata) = &mut media_meta {
				metadata.media_id = Set(Some(media_id.clone()));
			}
			let mut cover_write =
				prepare_cover_for_new_media(&media_id, &resolved, &cover_config)
					.await
					.map_err(|error| IngestError::BadRequest(error.to_string()))?;
			let txn = match begin_write(&self.conn).await {
				Ok(txn) => txn,
				Err(error) => {
					if let Some(write) = cover_write.take() {
						write.rollback().await;
					}
					return Err(error.into());
				},
			};
			let result: Result<(String, Option<CoverWrite>), IngestError> = async {
				if let Some((series_model, series_metadata)) = new_series {
					series_model.insert(&txn).await?;
					if let Some(metadata) = series_metadata {
						metadata.insert(&txn).await?;
					}
				}
				let media_model = media_active.insert(&txn).await?;
				if let Some(metadata) = media_meta {
					metadata.insert(&txn).await?;
				}
				let picks_value = serde_json::to_value(&picks)?;
				let applied_cover = apply_to_media_txn_with_context_and_cover(
					&txn,
					&media_model.id,
					resolved,
					item.created_by.as_deref().unwrap_or("system"),
					Some(item_id),
					expected_revision,
					"FILL_GAPS",
					picks_value,
					cover_write.take(),
				)
				.await
				.map_err(|error| IngestError::BadRequest(error.to_string()))?;
				let committed: Result<String, IngestError> = async {
					let revision = item.revision;
					let mut updated_item = item.into_active_model();
					updated_item.status =
						Set(DropItemStatus::Committed.as_str().to_string());
					updated_item.media_id = Set(Some(media_model.id.clone()));
					updated_item.series_id = Set(Some(series_id));
					updated_item.sidecar_paths = Set((!moved_sidecars.is_empty())
						.then(|| serde_json::to_value(&moved_sidecars))
						.transpose()?);
					updated_item.error = Set(None);
					updated_item.revision = Set(revision.saturating_add(1));
					updated_item.update(&txn).await?;
					txn.commit().await?;
					Ok(media_model.id)
				}
				.await;
				match committed {
					Ok(media_id) => Ok((media_id, applied_cover)),
					Err(error) => {
						if let Some(write) = applied_cover {
							write.rollback().await;
						}
						Err(error)
					},
				}
			}
			.await;
			match result {
				Ok(value) => Ok(value),
				Err(error) => {
					if let Some(write) = cover_write.take() {
						write.rollback().await;
					}
					Err(error)
				},
			}
		};
		let (media_id, cover_write) = match commit.await {
			Ok(result) => result,
			Err(error) => {
				let restore = if staged_is_dir {
					staging::move_dir(&destination, Path::new(&staging_path)).await
				} else {
					move_file(&destination, Path::new(&staging_path))
						.await
						.map_err(IngestError::from)
				};
				if let Err(restore_error) = restore {
					tracing::error!(
						?restore_error,
						destination = %destination.display(),
						staging = %staging_path,
						"approve failed and the staged file could not be moved back"
					);
				}
				restore_moved_sidecars(&moved_sidecars, &sidecars).await;
				return Err(error);
			},
		};
		if let Some(write) = cover_write {
			write.commit().await;
		}
		if let Some(group) = drop_group_id {
			self.suggest_group_pairs(&group, item_id, &media_id, media_kind, created_by)
				.await;
		}
		// The commit ran in a transaction, so the change is announced from the
		// re-read row once it is durable.
		let committed = self
			.item(item_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("ingest item {item_id}")))?;
		self.announce(&committed);
		Ok((media_id, committed))
	}

	/// Record an edition-pair suggestion for every already-committed sibling
	/// of this item's drop group that is the complementary kind.
	///
	/// Runs after the commit transaction, deliberately: the books are in the
	/// library either way, and a suggestion nobody asked for must never be
	/// able to roll back an ingest. Every failure is logged and swallowed.
	async fn suggest_group_pairs(
		&self,
		drop_group_id: &str,
		item_id: &str,
		media_id: &str,
		media_kind: IngestMediaKind,
		created_by: Option<String>,
	) {
		let siblings = match ingest_drop_item::Entity::find()
			.filter(ingest_drop_item::Column::DropGroupId.eq(drop_group_id))
			.filter(ingest_drop_item::Column::Id.ne(item_id.to_string()))
			.filter(ingest_drop_item::Column::MediaId.is_not_null())
			.all(self.conn.as_ref())
			.await
		{
			Ok(siblings) => siblings,
			Err(error) => {
				tracing::warn!(%error, drop_group_id, "Could not read drop group siblings");
				return;
			},
		};
		for sibling in siblings {
			let Some(sibling_media_id) = sibling.media_id.as_deref() else {
				continue;
			};
			if !crate::pairing::is_edition_pair(media_kind, media_kind_for_item(&sibling))
			{
				continue;
			}
			// The pair belongs to whoever committed it; a drop with no user
			// (the folder watcher) pairs for the sibling's owner, and a
			// sibling with none either is a server-owned drop.
			let Some(user_id) = created_by.clone().or_else(|| sibling.created_by.clone())
			else {
				tracing::debug!(
					drop_group_id,
					"Drop group pair has no owning user; no suggestion written"
				);
				continue;
			};
			match crate::pairing::suggest_same_drop_pair(
				self.conn.as_ref(),
				&user_id,
				media_id,
				sibling_media_id,
			)
			.await
			{
				Ok(outcome) => tracing::info!(
					drop_group_id,
					media_id,
					sibling_media_id,
					?outcome,
					"Recorded a same-drop edition pair suggestion"
				),
				Err(error) => tracing::warn!(
					%error,
					drop_group_id,
					"Could not record a same-drop edition pair suggestion"
				),
			}
		}
	}

	pub async fn save_report(
		&self,
		drop_item_id: &str,
		report: &QualityReport,
	) -> IngestResult<QualityReportModel> {
		let item = self.item(drop_item_id).await?.ok_or_else(|| {
			IngestError::NotFound(format!("ingest item {drop_item_id}"))
		})?;
		if item.source_sha256 != report.source_sha256 {
			return Err(IngestError::BadRequest(
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
		self.persist(item).await?;
		Ok(report_model)
	}

	pub async fn report(
		&self,
		report_id: &str,
	) -> IngestResult<Option<QualityReportModel>> {
		ingest_quality_report::Entity::find_by_id(report_id)
			.one(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}

	pub async fn report_for_item(
		&self,
		item_id: &str,
	) -> IngestResult<Option<QualityReportModel>> {
		self.latest_report_for_item(item_id).await
	}

	pub async fn candidates(&self, item_id: &str) -> IngestResult<Vec<CandidateModel>> {
		ingest_metadata_candidate::Entity::find()
			.filter(ingest_metadata_candidate::Column::DropItemId.eq(item_id))
			.order_by_asc(ingest_metadata_candidate::Column::ProviderId)
			.order_by_asc(ingest_metadata_candidate::Column::Id)
			.all(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}

	pub async fn save_candidates(
		&self,
		item_id: &str,
		candidates: &[MetadataCandidate],
	) -> IngestResult<Vec<CandidateModel>> {
		let item = self
			.item(item_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("ingest item {item_id}")))?;
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
	) -> IngestResult<Option<QualityReportModel>> {
		ingest_quality_report::Entity::find()
			.filter(ingest_quality_report::Column::MediaId.eq(media_id))
			.order_by_desc(ingest_quality_report::Column::CreatedAt)
			.order_by_desc(ingest_quality_report::Column::Id)
			.one(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}

	pub async fn save_report_for_media(
		&self,
		media_id: &str,
		report: &QualityReport,
	) -> IngestResult<QualityReportModel> {
		self.media(media_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("media {media_id}")))?;
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
			.map_err(IngestError::from)
	}

	/// Candidates stored against a library media row.
	pub async fn candidates_for_media(
		&self,
		media_id: &str,
	) -> IngestResult<Vec<CandidateModel>> {
		ingest_metadata_candidate::Entity::find()
			.filter(ingest_metadata_candidate::Column::MediaId.eq(media_id))
			.order_by_asc(ingest_metadata_candidate::Column::ProviderId)
			.order_by_asc(ingest_metadata_candidate::Column::Id)
			.all(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}

	pub async fn save_candidates_for_media(
		&self,
		media_id: &str,
		candidates: &[MetadataCandidate],
	) -> IngestResult<Vec<CandidateModel>> {
		self.media(media_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("media {media_id}")))?;
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
	) -> IngestResult<ApplicationModel> {
		application
			.insert(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}

	pub async fn applications(
		&self,
		item_id: &str,
		page: Pagination,
	) -> IngestResult<(Vec<ApplicationModel>, u64)> {
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
	) -> IngestResult<Option<PluginSettingModel>> {
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
		query
			.one(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}

	pub async fn set_plugin_settings(
		&self,
		setting: ingest_plugin_setting::ActiveModel,
	) -> IngestResult<PluginSettingModel> {
		setting
			.insert(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
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
	) -> IngestResult<super::contract::IngestProgressEvent> {
		self.progress.emit(event).await
	}

	async fn ensure_library(&self, library_id: &str) -> IngestResult<library::Model> {
		library::Entity::find_by_id(library_id)
			.one(self.conn.as_ref())
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("library {library_id}")))
	}

	async fn find_identity(
		&self,
		library_id: &str,
		source_sha256: &str,
		filename: &str,
	) -> IngestResult<Option<DropItemModel>> {
		ingest_drop_item::Entity::find()
			.filter(ingest_drop_item::Column::LibraryId.eq(library_id))
			.filter(ingest_drop_item::Column::SourceSha256.eq(source_sha256))
			.filter(ingest_drop_item::Column::SourceFilename.eq(filename))
			.one(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}

	async fn latest_report_for_item(
		&self,
		item_id: &str,
	) -> IngestResult<Option<QualityReportModel>> {
		ingest_quality_report::Entity::find()
			.filter(ingest_quality_report::Column::DropItemId.eq(item_id))
			.order_by_desc(ingest_quality_report::Column::CreatedAt)
			.order_by_desc(ingest_quality_report::Column::Id)
			.one(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}
	pub(crate) async fn latest_job_for_item(
		&self,
		item_id: &str,
	) -> IngestResult<Option<AnalysisJobModel>> {
		ingest_analysis_job::Entity::find()
			.filter(ingest_analysis_job::Column::DropItemId.eq(item_id))
			.order_by_desc(ingest_analysis_job::Column::CreatedAt)
			.order_by_desc(ingest_analysis_job::Column::Id)
			.one(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}

	pub(crate) async fn create_analysis_job(
		&self,
		item: &DropItemModel,
		force: bool,
	) -> IngestResult<AnalysisJobModel> {
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
		self.persist(item).await?;
		Ok(job)
	}

	/// One library-rework analysis job for a batch of existing media rows.
	/// A single-media job also fills the `media_id` column so the row cascades
	/// with its target; larger batches live only in the `plan` targets.
	pub(crate) async fn create_media_analysis_job(
		&self,
		media_ids: &[String],
		providers: Option<&[String]>,
	) -> IngestResult<AnalysisJobModel> {
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
	) -> IngestResult<Vec<AnalysisTarget>> {
		let targets = analysis_targets_from_plan(&job.plan, job.drop_item_id.as_deref());
		if targets.is_empty() {
			return Err(IngestError::InternalError(format!(
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
	) -> IngestResult<Option<AnalysisJobModel>> {
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
	) -> IngestResult<(Vec<AnalysisJobModel>, u64)> {
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

	pub(crate) async fn job(
		&self,
		job_id: &str,
	) -> IngestResult<Option<AnalysisJobModel>> {
		ingest_analysis_job::Entity::find_by_id(job_id)
			.one(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}

	pub(crate) async fn update_job_status(
		&self,
		job_id: &str,
		status: JobStatus,
		error: Option<&str>,
	) -> IngestResult<AnalysisJobModel> {
		let job = self
			.job(job_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("analysis job {job_id}")))?;
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
			.map_err(IngestError::from)
	}

	pub(crate) async fn start_job(&self, job_id: &str) -> IngestResult<AnalysisJobModel> {
		let job = self
			.job(job_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("analysis job {job_id}")))?;
		let attempts = job.attempts;
		let mut active = job.into_active_model();
		active.status = Set(JobStatus::Running);
		active.attempts = Set(attempts.saturating_add(1));
		active.started_at = Set(Some(Utc::now().into()));
		active.error = Set(None);
		active
			.update(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}

	pub(crate) async fn update_job_phase(
		&self,
		job_id: &str,
		phase: super::contract::AnalysisPhase,
		_done: bool,
		score: Option<u8>,
	) -> IngestResult<AnalysisJobModel> {
		let job = self
			.job(job_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("analysis job {job_id}")))?;
		let mut active = job.into_active_model();
		active.phase = Set(phase_name(phase));
		if let Some(score) = score {
			active.priority = Set(i32::from(score));
		}
		active
			.update(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}

	pub(crate) async fn finish_job(
		&self,
		job_id: &str,
		score: u8,
		_has_candidates: bool,
	) -> IngestResult<AnalysisJobModel> {
		let job = self
			.job(job_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("analysis job {job_id}")))?;
		let mut active = job.into_active_model();
		active.status = Set(JobStatus::Completed);
		active.phase = Set(phase_name(super::contract::AnalysisPhase::Done));
		active.priority = Set(i32::from(score));
		active.finished_at = Set(Some(Utc::now().into()));
		active
			.update(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}

	pub(crate) async fn fail_job(
		&self,
		job_id: &str,
		error: &str,
	) -> IngestResult<AnalysisJobModel> {
		let job = self
			.job(job_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("analysis job {job_id}")))?;
		let mut active = job.into_active_model();
		active.status = Set(JobStatus::Failed);
		active.error = Set(Some(error.to_string()));
		active.finished_at = Set(Some(Utc::now().into()));
		active
			.update(self.conn.as_ref())
			.await
			.map_err(IngestError::from)
	}

	pub(crate) async fn set_item_analysis_result(
		&self,
		item_id: &str,
		score: u8,
		has_candidates: bool,
	) -> IngestResult<DropItemModel> {
		let item = self
			.item(item_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("ingest item {item_id}")))?;
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
		self.persist(active).await
	}

	/// Run the configured preprocess hook for `item`, at most once in the
	/// item's lifetime, and return the item analysis should work from.
	///
	/// The hook may rewrite the staged file in place or replace it at the same
	/// path, so a successful run re-hashes the file and persists the post-hook
	/// digest and size: the staged path is never renamed, and `source_sha256`
	/// always describes the bytes analysis actually saw. A failing hook fails
	/// the item with its stderr tail as the reason and leaves
	/// `preprocessed_at` unset, so fixing the command and retrying runs it
	/// again.
	pub(crate) async fn run_preprocess(
		&self,
		item: &DropItemModel,
	) -> IngestResult<DropItemModel> {
		if item.preprocessed_at.is_some() {
			return Ok(item.clone());
		}
		let Some(hook) = PreprocessHook::from_config(&self.config)? else {
			return Ok(item.clone());
		};
		let path = PathBuf::from(&item.staging_path);
		if let HookOutcome::Fail(reason) =
			hook.run(&item.id, &item.library_id, &path).await
		{
			self.fail_item(&item.id, &reason).await?;
			return Err(IngestError::InternalError(reason));
		}

		// Exit 0 with the staged file gone is the classic hook mistake: it
		// wrote its output somewhere else. Fail the item where the reason can
		// still name the hook, not later as a bare missing-file error.
		let readable = fs::metadata(&path)
			.await
			.map(|metadata| metadata.is_file())
			.unwrap_or(false);
		if !readable {
			let reason = format!(
				"preprocess hook succeeded but left no file at {}",
				path.display()
			);
			self.fail_item(&item.id, &reason).await?;
			return Err(IngestError::InternalError(reason));
		}

		let (source_sha256, byte_size) = staging::hash_file(&path).await?;
		let byte_size = i64::try_from(byte_size).map_err(|_| {
			IngestError::BadRequest(
				"preprocessed file is too large for the database".to_string(),
			)
		})?;
		tracing::debug!(
			item_id = %item.id,
			rewritten = source_sha256 != item.source_sha256,
			"Ingest preprocess hook completed"
		);
		let revision = item.revision;
		let mut active = item.clone().into_active_model();
		active.source_sha256 = Set(source_sha256);
		active.byte_size = Set(byte_size);
		active.preprocessed_at = Set(Some(Utc::now().into()));
		active.revision = Set(revision.saturating_add(1));
		self.persist(active).await
	}

	/// Terminal failure of one item, with the reason the editor displays.
	async fn fail_item(
		&self,
		item_id: &str,
		reason: &str,
	) -> IngestResult<DropItemModel> {
		let item = self
			.item(item_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("ingest item {item_id}")))?;
		let revision = item.revision;
		let mut active = item.into_active_model();
		active.status = Set(DropItemStatus::Failed.as_str().to_string());
		active.error = Set(Some(reason.to_string()));
		active.revision = Set(revision.saturating_add(1));
		self.persist(active).await
	}

	/// Probe an audio item and persist the result on the row.
	///
	/// Every screen that shows a chapter list, a track table, or a duration
	/// reads this column; recomputing it would mean demuxing 62 MP3s per page
	/// view. Non-audio items are returned untouched, and a probe that fails is
	/// *not* a failed item: the audio quality family already reports an
	/// unreadable publication as a finding with the demuxer's own message, and
	/// failing here would hide that behind a bare error.
	pub(crate) async fn run_audio_analysis(
		&self,
		item: &DropItemModel,
	) -> IngestResult<DropItemModel> {
		if media_kind_for_item(item) != IngestMediaKind::Audio {
			return Ok(item.clone());
		}
		let path = PathBuf::from(&item.staging_path);
		let probed =
			tokio::task::spawn_blocking(move || stump_media::audio::probe(&path))
				.await
				.map_err(|error| IngestError::Unknown(error.to_string()))?;
		let probed = match probed {
			Ok(probed) => probed,
			Err(error) => {
				tracing::warn!(
					item_id = %item.id,
					%error,
					"Audio probe failed; the quality report carries the finding"
				);
				return Ok(item.clone());
			},
		};
		let analysis = AudioAnalysis::from(&probed);
		self.save_audio_analysis(item, analysis).await
	}

	async fn save_audio_analysis(
		&self,
		item: &DropItemModel,
		analysis: AudioAnalysis,
	) -> IngestResult<DropItemModel> {
		let revision = item.revision;
		let mut active = item.clone().into_active_model();
		active.audio_analysis = Set(Some(serde_json::to_value(&analysis)?));
		active.revision = Set(revision.saturating_add(1));
		self.persist(active).await
	}

	/// Run a quality finding's named repair tool against a staged item.
	///
	/// The finding names the tool *and* the options that address it
	/// ([`crate::contract::FixAction`]), so a fix cannot drift from what the
	/// tool accepts and the editor never has to know a command line. The
	/// report is read back from the database rather than recomputed: the
	/// button a librarian pressed belongs to the findings they were looking
	/// at.
	///
	/// Two shapes of outcome, both handled here:
	///
	/// * the tool wrote a **new file** (`audio-assemble` produces an M4B) —
	///   it becomes the item's file, the old target's contents become
	///   sidecars or are removed per policy, and the row is re-hashed;
	/// * the tool **rewrote in place** (`audio-chapters`, `meta-edit`) — the
	///   path is unchanged and the row is re-hashed, exactly as the
	///   preprocess hook's rewrite is.
	///
	/// A tool that produced nothing leaves the item untouched and returns it:
	/// the next report simply carries the same finding, which is a truthful
	/// answer rather than a fabricated success.
	pub(crate) async fn run_quality_fix(
		&self,
		item_id: &str,
		check_id: &str,
		fix: &FixAction,
	) -> IngestResult<DropItemModel> {
		let item = self
			.item(item_id)
			.await?
			.ok_or_else(|| IngestError::NotFound(format!("ingest item {item_id}")))?;
		// A fix belongs to a finding: the button a librarian pressed must be
		// on the report they were looking at, so an item with no report — or
		// a report that never ran this check — is refused rather than
		// silently rewritten.
		let report = self.report_for_item(item_id).await?.ok_or_else(|| {
			IngestError::BadRequest(format!(
				"ingest item {item_id} has no quality report to fix"
			))
		})?;
		let stored: QualityReport = serde_json::from_value(report.checks.clone())?;
		if !stored
			.checks
			.iter()
			.any(|check| check.outcome.check_id == check_id)
		{
			return Err(IngestError::BadRequest(format!(
				"quality check {check_id} did not run for this item"
			)));
		}

		// `audio-assemble` is the one fix whose output replaces the
		// publication, and the assemble path already owns re-hashing it,
		// recording it, and deciding what happens to the parts. Routing the
		// button through it means the manual fix and the policy-driven one
		// cannot behave differently.
		if fix.tool == "audio-assemble" {
			let policy = self.audio_policy(&item.library_id).await?;
			let forced = AudioPolicy {
				auto_assemble: true,
				..policy
			};
			let analysed = self.run_audio_analysis(&item).await?;
			return self.run_audio_assemble(&analysed, &forced).await;
		}

		let staged = PathBuf::from(&item.staging_path);
		let tool_id = fix.tool.clone();
		let options = fix.options.clone();
		let target = staged.clone();
		let ran = tokio::task::spawn_blocking(move || {
			run_tool_in_place(&tool_id, &target, options)
		})
		.await
		.map_err(|error| IngestError::Unknown(error.to_string()))??;
		if !ran {
			return Ok(item);
		}

		// The tool rewrote the staged bytes; the row must describe what is
		// there now, or a commit would move a file whose digest the audit
		// trail disagrees with.
		let updated = if staged.is_dir() {
			let (source_sha256, byte_size) = staging::hash_dir(&staged).await?;
			self.rehash(&item, source_sha256, byte_size).await?
		} else {
			let (source_sha256, byte_size) = staging::hash_file(&staged).await?;
			self.rehash(&item, source_sha256, byte_size).await?
		};
		self.run_audio_analysis(&updated).await
	}

	async fn rehash(
		&self,
		item: &DropItemModel,
		source_sha256: String,
		byte_size: u64,
	) -> IngestResult<DropItemModel> {
		let byte_size = i64::try_from(byte_size).map_err(|_| {
			IngestError::BadRequest("file is too large for the database".to_string())
		})?;
		let revision = item.revision;
		let mut active = item.clone().into_active_model();
		active.source_sha256 = Set(source_sha256);
		active.byte_size = Set(byte_size);
		active.revision = Set(revision.saturating_add(1));
		self.persist(active).await
	}

	/// The persisted probe result, when this item is an analysed audiobook.
	pub fn audio_analysis(item: &DropItemModel) -> Option<AudioAnalysis> {
		let value = item.audio_analysis.clone()?;
		match serde_json::from_value(value) {
			Ok(analysis) => Some(analysis),
			Err(error) => {
				tracing::warn!(
					item_id = %item.id,
					%error,
					"Stored audio analysis could not be read; treating the item as unanalysed"
				);
				None
			},
		}
	}

	/// The staged files an item owns without them being it.
	pub fn sidecars(item: &DropItemModel) -> Vec<String> {
		item.sidecar_paths
			.as_ref()
			.and_then(|value| serde_json::from_value::<Vec<String>>(value.clone()).ok())
			.unwrap_or_default()
	}

	/// Assemble a split audiobook into one canonical M4B and make it the
	/// item's file.
	///
	/// Runs only for [`IngestMediaKind::Audio`] items with more than one part
	/// and only when the library's [`AudioPolicy::auto_assemble`] is on. The
	/// output lands beside the parts in staging, is re-hashed onto the row the
	/// same way the preprocess hook's rewrite is — the digest always describes
	/// the bytes the library will receive — and the parts become sidecars when
	/// `keep_original` is set, or are removed when it is not.
	///
	/// A tool result that assembled nothing (no ffmpeg for a transcode, a
	/// verification failure) leaves the item exactly as it was and records the
	/// reason in the log: an unassembled folder book is still a book, and the
	/// `single_file` quality finding already says it is split.
	pub(crate) async fn run_audio_assemble(
		&self,
		item: &DropItemModel,
		policy: &AudioPolicy,
	) -> IngestResult<DropItemModel> {
		if !policy.auto_assemble || media_kind_for_item(item) != IngestMediaKind::Audio {
			return Ok(item.clone());
		}
		let Some(analysis) = Self::audio_analysis(item) else {
			return Ok(item.clone());
		};
		// One container is already the canonical shape; re-muxing it would
		// rewrite a file to produce the same file.
		if analysis.tracks.len() < 2 || analysis.assembled.is_some() {
			return Ok(item.clone());
		}

		let source = PathBuf::from(&item.staging_path);
		let outcome = tokio::task::spawn_blocking(move || assemble_audio(&source))
			.await
			.map_err(|error| IngestError::Unknown(error.to_string()))?;
		let assembled = match outcome {
			Ok(Some(assembled)) => assembled,
			Ok(None) => return Ok(item.clone()),
			Err(error) => {
				tracing::warn!(
					item_id = %item.id,
					%error,
					"audio-assemble produced no output; the item keeps its parts"
				);
				return Ok(item.clone());
			},
		};

		let (source_sha256, byte_size) = staging::hash_file(&assembled.output).await?;
		let byte_size = i64::try_from(byte_size).map_err(|_| {
			IngestError::BadRequest(
				"assembled audiobook is too large for the database".to_string(),
			)
		})?;
		let parts_directory = PathBuf::from(&item.staging_path);
		let mut sidecars = Vec::new();
		if policy.keep_original {
			sidecars = staging::files_in(&parts_directory).await?;
		} else {
			staging::remove_staged(&parts_directory).await?;
		}
		sidecars.sort();

		let filename = assembled
			.output
			.file_name()
			.map(|name| name.to_string_lossy().into_owned())
			.unwrap_or_default();
		let analysis = AudioAnalysis {
			assembled: Some(AssembledAudio {
				filename: filename.clone(),
				byte_size: byte_size.max(0) as u64,
				duration_ms: assembled.duration_ms,
				chapters: assembled.chapters,
				faststart: assembled.faststart,
				parts_kept: policy.keep_original,
				method: assembled.method,
			}),
			..analysis
		};

		let revision = item.revision;
		let mut active = item.clone().into_active_model();
		active.source_filename = Set(filename);
		active.media_kind = Set(media_kind_name(IngestMediaKind::Audio));
		active.staging_path = Set(assembled.output.to_string_lossy().into_owned());
		active.source_sha256 = Set(source_sha256);
		active.byte_size = Set(byte_size);
		active.sidecar_paths = Set(Some(serde_json::to_value(&sidecars)?));
		active.audio_analysis = Set(Some(serde_json::to_value(&analysis)?));
		active.revision = Set(revision.saturating_add(1));
		self.persist(active).await
	}

	/// The library's effective audio policy, for the assemble decision.
	///
	/// A stored override that does not parse is not a reason to rewrite an
	/// operator's audio: the server default (both auto-fixes off, sources
	/// kept) is what an unreadable document means.
	pub(crate) async fn audio_policy(
		&self,
		library_id: &str,
	) -> IngestResult<AudioPolicy> {
		match crate::policy::effective_policy(self.conn.as_ref(), library_id).await {
			Ok(effective) => Ok(*effective.policy.audio()),
			Err(error) => {
				tracing::warn!(
					library_id,
					%error,
					"Library metadata policy could not be read; using the server default"
				);
				Ok(*AudioPolicy::server_default())
			},
		}
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
		// The extensions `media::AUDIO_EXTENSIONS` defines, so a book the
		// scanner treats as audio is a book ingest treats as audio.
		extension if models::entity::media::AUDIO_EXTENSIONS.contains(&extension) => {
			IngestMediaKind::Audio
		},
		_ => IngestMediaKind::Unknown,
	}
}

/// Everything that distinguishes one new drop item row from another.
///
/// Three call sites create drop items — an upload, a plain drop-folder file,
/// and one publication of an exploded archive — and they differ in four
/// fields out of nineteen. One builder is what keeps a new column (a drop
/// group, a sidecar list) from being silently absent from two of them.
#[derive(Debug, Default)]
struct NewDropItem {
	library_id: String,
	created_by: Option<String>,
	source_filename: String,
	relative_path: Option<String>,
	byte_size: u64,
	source_sha256: String,
	media_kind: IngestMediaKind,
	staging_path: String,
	idempotency_key: Option<String>,
	drop_group_id: Option<String>,
	sidecar_paths: Vec<String>,
	status: DropItemStatus,
	error: Option<String>,
}

impl NewDropItem {
	fn into_active_model(self) -> IngestResult<ingest_drop_item::ActiveModel> {
		Ok(ingest_drop_item::ActiveModel {
			id: Set(Uuid::new_v4().to_string()),
			library_id: Set(self.library_id),
			created_by: Set(self.created_by),
			source_filename: Set(self.source_filename),
			relative_path: Set(self.relative_path),
			byte_size: Set(i64::try_from(self.byte_size).map_err(|_| {
				IngestError::BadRequest(
					"dropped file is too large for the database".to_string(),
				)
			})?),
			source_sha256: Set(self.source_sha256),
			media_kind: Set(media_kind_name(self.media_kind)),
			staging_path: Set(self.staging_path),
			status: Set(self.status.as_str().to_string()),
			analysis_job_id: Set(None),
			quality_report_id: Set(None),
			media_id: Set(None),
			series_id: Set(None),
			pending_fields: Set(Some(Value::Object(Default::default()))),
			error: Set(self.error),
			idempotency_key: Set(self.idempotency_key),
			preprocessed_at: Set(None),
			drop_group_id: Set(self.drop_group_id),
			sidecar_paths: Set((!self.sidecar_paths.is_empty())
				.then(|| serde_json::to_value(&self.sidecar_paths))
				.transpose()?),
			audio_analysis: Set(None),
			revision: Set(1),
			created_at: NotSet,
			updated_at: NotSet,
		})
	}
}

/// The directory part of a drop-folder relative path, `/`-separated.
fn parent_of(relative_path: &str) -> Option<String> {
	Path::new(relative_path)
		.parent()
		.filter(|path| !path.as_os_str().is_empty())
		.map(|path| path.to_string_lossy().replace('\\', "/"))
}

/// The relative path an exploded item commits under: the container's own
/// directory in the drop folder, then the item's directory inside the
/// container. A nested delivery keeps its shape; a flat one has neither.
fn join_relative(container: Option<&str>, inside: Option<&str>) -> Option<String> {
	match (container, inside) {
		(Some(container), Some(inside)) => Some(format!("{container}/{inside}")),
		(Some(only), None) | (None, Some(only)) => Some(only.to_string()),
		(None, None) => None,
	}
}

/// Move an item's sidecars next to its committed publication.
///
/// A folder audiobook's sidecars moved with the directory, so there is
/// nothing to do; a single file's land in its parent directory, which is
/// where the scanner looks for a `cover.jpg`.
///
/// A sidecar that cannot be moved is dropped from the item's list rather than
/// failing the commit: the book is in the library, and a lost `.nfo` is not
/// worth refusing it over. The returned list is what actually exists.
async fn move_sidecars_beside(
	sidecars: &[String],
	destination: &Path,
	staged_is_dir: bool,
) -> Vec<String> {
	if staged_is_dir {
		// They travelled inside the directory; their paths are now relative
		// to the committed publication and are recomputed by the scanner.
		return Vec::new();
	}
	let Some(parent) = destination.parent() else {
		return Vec::new();
	};
	let mut moved = Vec::with_capacity(sidecars.len());
	for sidecar in sidecars {
		let source = Path::new(sidecar);
		let Some(name) = source.file_name() else {
			continue;
		};
		let target = parent.join(name);
		if fs::try_exists(&target).await.unwrap_or(false) {
			// Something is already there — a cover from a previous commit in
			// the same series directory. Leaving it alone is the only safe
			// answer; the staged copy is removed with the drop item.
			continue;
		}
		match move_file(source, &target).await {
			Ok(()) => moved.push(target.to_string_lossy().into_owned()),
			Err(error) => tracing::warn!(
				?error,
				sidecar = %sidecar,
				"Could not move an ingest sidecar beside its book"
			),
		}
	}
	moved.sort();
	moved
}

async fn restore_moved_sidecars(moved: &[String], originals: &[String]) {
	for target in moved {
		let Some(target_name) = Path::new(target).file_name() else {
			continue;
		};
		let Some(original) = originals.iter().find(|original| {
			Path::new(original.as_str()).file_name() == Some(target_name)
		}) else {
			continue;
		};
		if let Err(error) = move_file(Path::new(target), Path::new(original)).await {
			tracing::warn!(
				?error,
				target,
				original = %original,
				"Could not restore an ingest sidecar after approval rollback"
			);
		}
	}
}

/// Run one repair tool over `target`, returning whether it applied anything.
///
/// Blocking: every tool demuxes or rewrites a file, and some shell out.
/// An unknown tool id is an error rather than a silent no-op — the ids come
/// from the check registry, so one that the build cannot find means the two
/// have drifted and that is worth surfacing.
fn run_tool_in_place(tool_id: &str, target: &Path, options: Value) -> IngestResult<bool> {
	let tool = stump_tools::find(tool_id).ok_or_else(|| {
		IngestError::BadRequest(format!("this build has no `{tool_id}` tool"))
	})?;
	let mut input = stump_tools::ToolInput::new(vec![target.to_path_buf()]);
	if !options.is_null() {
		input = input.with_options(options);
	}
	let plan = tool
		.plan(&input)
		.map_err(|error| IngestError::InternalError(error.to_string()))?;
	let report = tool
		.apply(&plan, &mut stump_tools::NoopProgress)
		.map_err(|error| IngestError::InternalError(error.to_string()))?;
	for (action, reason) in &report.skipped {
		tracing::warn!(
			tool = tool_id,
			kind = %action.kind,
			reason = %reason,
			"A quality fix skipped an action"
		);
	}
	Ok(!report.applied.is_empty())
}

pub(crate) fn media_kind_for_item(item: &DropItemModel) -> IngestMediaKind {
	serde_json::from_value(Value::String(item.media_kind.clone()))
		.unwrap_or(IngestMediaKind::Unknown)
}

/// What one `audio-assemble` run produced.
struct AssembleOutcome {
	output: PathBuf,
	duration_ms: i64,
	chapters: usize,
	faststart: bool,
	method: String,
}

/// Plan and apply `audio-assemble` over one staged publication.
///
/// Blocking by nature: it demuxes every part and may shell out to ffmpeg for
/// hours on a 60-part book, so callers run it on the blocking pool.
///
/// `Ok(None)` is a truthful "nothing was assembled": the tool plans a
/// transcode it cannot run (no ffmpeg), or refuses its own output because the
/// duration, the chapter list, or the box order did not match the plan. Either
/// way the parts are untouched and the caller keeps the item as it was.
fn assemble_audio(source: &Path) -> IngestResult<Option<AssembleOutcome>> {
	use stump_tools::Tool;

	let tool = stump_tools::audio_assemble::AudioAssemble;
	let input = stump_tools::ToolInput::new(vec![source.to_path_buf()]);
	let plan = tool
		.plan(&input)
		.map_err(|error| IngestError::InternalError(error.to_string()))?;
	let report = tool
		.apply(&plan, &mut stump_tools::NoopProgress)
		.map_err(|error| IngestError::InternalError(error.to_string()))?;
	if let Some((action, reason)) = report.skipped.first() {
		tracing::warn!(
			kind = %action.kind,
			reason = %reason,
			source = %source.display(),
			"audio-assemble skipped a publication"
		);
	}
	let Some(action) = report.applied.first() else {
		return Ok(None);
	};
	let Some(output) = action.target.clone() else {
		return Ok(None);
	};
	// The plan's detail is the tool's own description of what it wrote; the
	// output is re-probed instead so the row records what is actually on disk.
	let probed = stump_media::audio::probe(&output)?;
	let faststart = crate::quality::audio::moov_before_mdat(&output)
		.ok()
		.flatten()
		.unwrap_or(false);
	Ok(Some(AssembleOutcome {
		output,
		duration_ms: probed.duration_ms,
		chapters: probed.chapters.len(),
		faststart,
		method: action.kind.clone(),
	}))
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
	/// The kind the *row* recorded, never re-derived from the file name here.
	/// A folder audiobook's name is a directory with no extension, so a
	/// filename rule would call the one kind that can be a directory
	/// `Unknown` and every audio quality check would report
	/// `NOT_APPLICABLE` for the books the family exists for.
	media_kind: IngestMediaKind,
}

fn build_snapshot(
	item: DropItemModel,
	config: &IngestSettings,
) -> IngestResult<BookSnapshot> {
	let staged_path = PathBuf::from(&item.staging_path);
	let media_kind = media_kind_for_item(&item);
	// A folder audiobook is one publication whose target is a directory —
	// the only kind for which that is true. Every other kind is a file, and a
	// directory standing in for one is a broken row rather than a book.
	let staged = if media_kind == IngestMediaKind::Audio {
		staged_path.is_file() || staged_path.is_dir()
	} else {
		staged_path.is_file()
	};
	if !staged {
		return Err(IngestError::FileNotFound(item.staging_path));
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
			media_kind,
		},
		config,
	)
}

/// The one snapshot path shared by staged and library targets: embedded
/// metadata via `process_metadata`, pages via the same format adapters the
/// processor selection uses.
fn snapshot_from_source(
	source: SnapshotSource,
	config: &IngestSettings,
) -> IngestResult<BookSnapshot> {
	let media_kind = source.media_kind;
	let embedded_metadata = process_metadata(&source.path)
		.map_err(|error| IngestError::Unknown(error.to_string()))?;
	let pages = match media_kind {
		IngestMediaKind::ComicArchive => archive_pages(&source.path)?,
		IngestMediaKind::ComicRarArchive => {
			let count =
				get_page_count(source.path.to_str().unwrap_or_default(), &config.media)
					.map_err(|error| IngestError::Unknown(error.to_string()))?;
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
					.map_err(|error| IngestError::Unknown(error.to_string()))?;
			(0..count.max(0))
				.map(|index| IngestPageEntry {
					index: index as u32,
					path: format!("page-{}", index + 1),
					size: None,
					is_image: true,
				})
				.collect()
		},
		// A recording is addressed in time, not in pages.
		IngestMediaKind::Audio | IngestMediaKind::Unknown => Vec::new(),
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

fn archive_pages(path: &Path) -> IngestResult<Vec<IngestPageEntry>> {
	let file = std::fs::File::open(path)?;
	let mut archive = zip::ZipArchive::new(file)
		.map_err(|error| IngestError::Unknown(error.to_string()))?;
	let mut names = archive.file_names().map(str::to_owned).collect::<Vec<_>>();
	names.sort_by(|left, right| alphanumeric_sort::compare_path(left, right));
	let mut pages = Vec::new();
	for name in names {
		let entry = archive
			.by_name(&name)
			.map_err(|error| IngestError::Unknown(error.to_string()))?;
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

fn epub_pages(path: &Path) -> IngestResult<Vec<IngestPageEntry>> {
	let path = path.to_string_lossy();
	let manifest = ReadiumManifestGenerator::new(path.as_ref(), "");
	let spine = manifest
		.enumerate_spine_for_positions()
		.map_err(|error| IngestError::Unknown(error.to_string()))?;
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
async fn remove_duplicate_upload(staged: &Path, existing: &str) -> IngestResult<()> {
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
		let store = IngestStore::new(Arc::new(IngestSettings::debug()), conn);
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
		let store = IngestStore::new(Arc::new(IngestSettings::debug()), conn.clone());

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
					algorithm_version: crate::contract::QUALITY_ALGORITHM_VERSION
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
		let settings = IngestSettings::rooted_at(temporary.path());
		let store = IngestStore::new(Arc::new(settings), conn);

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
		use crate::providers::apply::apply_to_media;
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
		let store = IngestStore::new(Arc::new(IngestSettings::debug()), conn.clone());

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

	/// A staged item on a real file, plus the store that owns it. The hook
	/// tests all need the same fixture: one library, one item, one file whose
	/// bytes the hook is free to rewrite.
	#[cfg(unix)]
	async fn preprocess_fixture(
		hook_body: &str,
		contents: &[u8],
	) -> (tempfile::TempDir, IngestStore, DropItemModel) {
		use models::{
			entity::{ingest_drop_item, library, library_config},
			shared::enums::FileStatus,
		};
		use std::os::unix::fs::PermissionsExt;

		let conn = Arc::new(Database::connect("sqlite::memory:").await.unwrap());
		migrations::Migrator::up(conn.as_ref(), None).await.unwrap();
		let library_config =
			<library_config::ActiveModel as std::default::Default>::default()
				.insert(conn.as_ref())
				.await
				.unwrap();
		library::ActiveModel {
			id: Set("library".to_string()),
			name: Set("Library".to_string()),
			path: Set("/tmp/library".to_string()),
			status: Set(FileStatus::Ready),
			config_id: Set(library_config.id),
			..Default::default()
		}
		.insert(conn.as_ref())
		.await
		.unwrap();

		let temporary = tempfile::tempdir().unwrap();
		let hook = temporary.path().join("hook.sh");
		std::fs::write(&hook, hook_body).unwrap();
		std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
		let staged = temporary.path().join("book.cbz");
		std::fs::write(&staged, contents).unwrap();

		let mut config = IngestSettings::debug();
		config.preprocess_command = Some(hook.to_string_lossy().into_owned());
		config.preprocess_timeout_secs = 30;
		let store = IngestStore::new(Arc::new(config), conn.clone());

		let item = ingest_drop_item::ActiveModel {
			id: Set("item-1".to_string()),
			library_id: Set("library".to_string()),
			source_filename: Set("book.cbz".to_string()),
			byte_size: Set(contents.len() as i64),
			source_sha256: Set(staging::hash_file(&staged).await.unwrap().0),
			media_kind: Set("COMIC_ARCHIVE".to_string()),
			staging_path: Set(staged.to_string_lossy().into_owned()),
			status: Set(DropItemStatus::Staged.as_str().to_string()),
			revision: Set(1),
			..Default::default()
		}
		.insert(conn.as_ref())
		.await
		.unwrap();
		(temporary, store, item)
	}

	/// The hook rewrites the staged file in place, so the item must carry the
	/// digest and size of the bytes analysis will actually read.
	#[cfg(unix)]
	#[tokio::test]
	async fn preprocess_hook_rewrite_is_rehashed_onto_the_item() {
		let (_temporary, store, item) =
			preprocess_fixture("#!/bin/sh\nprintf 'rewritten' > \"$1\"\n", b"original")
				.await;

		let updated = store.run_preprocess(&item).await.unwrap();
		let path = Path::new(&updated.staging_path);
		let (expected_sha, expected_size) = staging::hash_file(path).await.unwrap();
		assert_eq!(std::fs::read(path).unwrap(), b"rewritten");
		assert_eq!(updated.source_sha256, expected_sha);
		assert_ne!(updated.source_sha256, item.source_sha256);
		assert_eq!(updated.byte_size, expected_size as i64);
		assert!(updated.preprocessed_at.is_some());

		let persisted = store.item("item-1").await.unwrap().unwrap();
		assert_eq!(persisted.source_sha256, expected_sha);
		assert_eq!(persisted.byte_size, expected_size as i64);
	}

	/// A non-zero exit fails the item, and the reason the editor lists is the
	/// hook's own stderr tail.
	#[cfg(unix)]
	#[tokio::test]
	async fn preprocess_hook_failure_fails_the_item_with_the_stderr_tail() {
		let (_temporary, store, item) = preprocess_fixture(
			"#!/bin/sh\necho 'unsupported input format' >&2\nexit 3\n",
			b"original",
		)
		.await;

		let error = store
			.run_preprocess(&item)
			.await
			.expect_err("a non-zero exit must fail the analysis");
		assert!(
			error.to_string().contains("unsupported input format"),
			"{error}"
		);

		let failed = store.item("item-1").await.unwrap().unwrap();
		assert_eq!(failed.status, DropItemStatus::Failed.as_str());
		let reason = failed.error.expect("failure reason is stored on the item");
		assert!(reason.contains("exited with 3"), "{reason}");
		assert!(reason.contains("unsupported input format"), "{reason}");
		assert!(
			failed.preprocessed_at.is_none(),
			"a failed hook must stay retryable"
		);
		assert_eq!(
			failed.source_sha256, item.source_sha256,
			"a failed hook must not re-hash the item"
		);
	}

	/// Re-analysis must not run the hook again: a conversion pipeline is not
	/// idempotent, and running it over its own output corrupts the file.
	#[cfg(unix)]
	#[tokio::test]
	async fn preprocess_hook_runs_once_per_item() {
		let (_temporary, store, item) =
			preprocess_fixture("#!/bin/sh\nprintf 'x' >> \"$1\"\n", b"original").await;

		let first = store.run_preprocess(&item).await.unwrap();
		let second = store.run_preprocess(&first).await.unwrap();

		assert_eq!(
			std::fs::read(&first.staging_path).unwrap(),
			b"originalx",
			"the hook ran exactly once"
		);
		assert_eq!(second.source_sha256, first.source_sha256);
		assert_eq!(second.preprocessed_at, first.preprocessed_at);
		assert_eq!(second.revision, first.revision);
	}

	/// A hook that writes its output elsewhere and removes the input exits 0,
	/// so the item must fail naming the hook rather than surfacing a bare
	/// missing-file error from the next phase.
	#[cfg(unix)]
	#[tokio::test]
	async fn preprocess_hook_that_removes_the_file_fails_the_item() {
		let (_temporary, store, item) =
			preprocess_fixture("#!/bin/sh\nrm \"$1\"\n", b"original").await;

		store
			.run_preprocess(&item)
			.await
			.expect_err("a vanished staged file must fail the analysis");

		let failed = store.item("item-1").await.unwrap().unwrap();
		assert_eq!(failed.status, DropItemStatus::Failed.as_str());
		assert!(
			failed
				.error
				.expect("failure reason is stored on the item")
				.contains("left no file"),
			"the reason must name the hook"
		);
		assert!(failed.preprocessed_at.is_none());
	}
}
