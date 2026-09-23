use std::{
	collections::{BTreeMap, BTreeSet},
	net::{IpAddr, SocketAddr, ToSocketAddrs},
	path::PathBuf,
	time::Duration,
};

use metadata_integrations::MergeStrategy;
use models::entity::{
	ingest_metadata_application, ingest_metadata_candidate, media, media_metadata,
	media_tag, tag,
};
use models::shared::image::ImageMetadata;
use models::txn::begin_write;
use rust_decimal::{prelude::FromPrimitive, Decimal};
use sea_orm::{
	prelude::*, ActiveModelTrait, ColumnTrait, DatabaseConnection, DatabaseTransaction,
	EntityTrait, IntoActiveModel, QueryFilter, Set,
};
use serde_json::{json, Value};
use stump_media::{generate_image_metadata_from_bytes, ContentType, MediaConfig};
use tokio::fs;
use uuid::Uuid;

use crate::contract::{FieldPick, MetadataField};

pub type CandidateModel = ingest_metadata_candidate::Model;

/// The bounds and destination used when a provider's cover URL is selected.
///
/// Cover files are written below Stump's existing thumbnail directory and the
/// media row points at the content-addressed path only after all validation has
/// completed.  The caller owns the returned [`CoverWrite`] until its database
/// transaction commits.
#[derive(Debug, Clone)]
pub struct CoverApplyConfig {
	pub media: MediaConfig,
	pub max_bytes: usize,
	pub timeout: Duration,
}

impl CoverApplyConfig {
	pub fn new(media: MediaConfig, max_bytes: usize) -> Self {
		Self {
			media,
			max_bytes,
			timeout: Duration::from_secs(30),
		}
	}
}

/// A filesystem change staged alongside a metadata transaction.
///
/// The new content is content-addressed, so a rollback only removes the file
/// when this operation created it.  On commit, the previous thumbnail is
/// removed if it lives in Stump's thumbnail directory.
pub struct CoverWrite {
	new_path: Option<PathBuf>,
	created_new: bool,
	previous_path: Option<String>,
	thumbnail_meta: Option<ImageMetadata>,
	thumbnail_dir: PathBuf,
}

impl CoverWrite {
	pub async fn commit(self) {
		if let Some(previous) = self.previous_path {
			let previous = PathBuf::from(previous);
			if self.new_path.as_deref() != Some(previous.as_path())
				&& previous.starts_with(&self.thumbnail_dir)
			{
				let _ = fs::remove_file(previous).await;
			}
		}
	}

	pub async fn rollback(self) {
		if self.created_new {
			if let Some(path) = self.new_path {
				let _ = fs::remove_file(path).await;
			}
		}
	}
}

/// Fields produced by resolving one editor recipe.  `cleared` is separate from
/// `values` so an explicit CLEAR is never confused with a missing candidate
/// value.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedFields {
	pub values: BTreeMap<MetadataField, Value>,
	pub cleared: Vec<MetadataField>,
}

/// Canonical fields persisted by the media metadata/tag tables or the
/// consumer-visible media artwork reference.  Identifiers and status remain
/// candidate-only until their dedicated storage paths exist.
pub const STORABLE_FIELDS: &[MetadataField] = &[
	MetadataField::Title,
	MetadataField::SortTitle,
	MetadataField::Series,
	MetadataField::SeriesIndex,
	MetadataField::Authors,
	MetadataField::Narrators,
	MetadataField::Publisher,
	MetadataField::PublishedDate,
	MetadataField::Language,
	MetadataField::Summary,
	MetadataField::Tags,
	MetadataField::CoverUrl,
	MetadataField::Genres,
	MetadataField::Isbn,
	MetadataField::AgeRating,
	MetadataField::PageCount,
];

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ApplyError {
	#[error("unsupported metadata fields cannot be applied: {0:?}")]
	UnsupportedFields(Vec<MetadataField>),
	#[error("invalid metadata pick: {0}")]
	InvalidPick(String),
	#[error("candidate {0} was not supplied")]
	CandidateNotFound(String),
	#[error("candidate {candidate_id} does not match the selected item/source digest")]
	CandidateMismatch { candidate_id: String },
	#[error("candidate {candidate_id} does not contain field {field:?}")]
	CandidateFieldMissing {
		candidate_id: String,
		field: MetadataField,
	},
	#[error("manual value for {field:?} has the wrong type")]
	InvalidValue { field: MetadataField },
	#[error("cover application requires the host media configuration")]
	CoverConfigurationRequired,
	#[error("cover download failed: {0}")]
	CoverDownload(String),
	#[error("cover exceeds the maximum size of {max_bytes} bytes")]
	CoverTooLarge { max_bytes: usize },
	#[error("cover image is invalid: {0}")]
	CoverInvalid(String),
	#[error("media {0} was not found while applying its cover")]
	MediaNotFound(String),
	#[error("database operation failed: {0}")]
	Database(String),
}

/// Validate the structural rules of a field-pick recipe before looking up
/// candidates or touching the database.
pub fn validate_picks(picks: &[FieldPick]) -> Result<(), ApplyError> {
	let unsupported = picks
		.iter()
		.map(FieldPick::field)
		.filter(|field| !STORABLE_FIELDS.contains(field))
		.collect::<BTreeSet<_>>();
	if !unsupported.is_empty() {
		return Err(ApplyError::UnsupportedFields(
			unsupported.into_iter().collect(),
		));
	}
	let mut seen = BTreeSet::new();
	for pick in picks {
		if !seen.insert(pick.field()) {
			return Err(ApplyError::InvalidPick(format!(
				"field {:?} appears more than once",
				pick.field()
			)));
		}
		match pick {
			FieldPick::KeepExisting { .. } | FieldPick::Clear { .. } => {},
			FieldPick::Candidate { candidate_id, .. } => {
				if candidate_id.trim().is_empty() {
					return Err(ApplyError::InvalidPick(
						"CANDIDATE requires a non-empty candidateId".to_string(),
					));
				}
			},
			FieldPick::Manual { field, value } => {
				if value.is_null() {
					return Err(ApplyError::InvalidPick(format!(
						"MANUAL for {field:?} requires a non-null value; use CLEAR to clear"
					)));
				}
				validate_value(*field, value)?;
			},
		}
	}
	Ok(())
}
fn validate_resolved_fields(resolved: &ResolvedFields) -> Result<(), ApplyError> {
	let unsupported = resolved
		.values
		.keys()
		.chain(resolved.cleared.iter())
		.filter(|field| !STORABLE_FIELDS.contains(field))
		.copied()
		.collect::<BTreeSet<_>>();
	if unsupported.is_empty() {
		Ok(())
	} else {
		Err(ApplyError::UnsupportedFields(
			unsupported.into_iter().collect(),
		))
	}
}

/// Resolve an editor recipe against the candidate rows for one item.  Candidate
/// rows are required to describe one target and one source digest; mixed rows
/// are rejected, which prevents stale results being merged accidentally.
pub fn resolve_picks(
	picks: &[FieldPick],
	candidates: &[CandidateModel],
	existing: Option<&media_metadata::Model>,
	locked: &[MetadataField],
	strategy: MergeStrategy,
) -> Result<ResolvedFields, ApplyError> {
	resolve_picks_for_context(picks, candidates, existing, locked, strategy, None, None)
}

/// Context-aware variant used by the staged store.  Unlike the compatibility
/// [`resolve_picks`] helper, this can compare the selected candidates with the
/// drop-item id and source digest loaded in the same transaction.
pub fn resolve_picks_for_context(
	picks: &[FieldPick],
	candidates: &[CandidateModel],
	existing: Option<&media_metadata::Model>,
	locked: &[MetadataField],
	strategy: MergeStrategy,
	expected_drop_item_id: Option<&str>,
	expected_source_sha256: Option<&str>,
) -> Result<ResolvedFields, ApplyError> {
	validate_picks(picks)?;
	let locked: BTreeSet<_> = locked.iter().copied().collect();
	let context = candidate_context(candidates)?;
	if let Some(expected) = expected_drop_item_id {
		if context
			.as_ref()
			.is_some_and(|(drop_item, _, _)| drop_item.as_deref() != Some(expected))
		{
			return Err(ApplyError::CandidateMismatch {
				candidate_id: "selected candidates".to_string(),
			});
		}
	}
	if let Some(expected) = expected_source_sha256 {
		if context
			.as_ref()
			.is_some_and(|(_, _, source)| *source != expected)
		{
			return Err(ApplyError::CandidateMismatch {
				candidate_id: "selected candidates".to_string(),
			});
		}
	}

	let mut values = BTreeMap::new();
	let mut cleared = Vec::new();
	for pick in picks {
		let field = pick.field();
		// A locked field always wins, including over an explicit CLEAR.  The
		// existing value is therefore left untouched and no resolved operation is
		// emitted for it.
		if locked.contains(&field) {
			continue;
		}
		match pick {
			FieldPick::KeepExisting { .. } => {},
			FieldPick::Clear { .. } => cleared.push(field),
			FieldPick::Manual { value, .. } => {
				// Manual values are explicit user overrides, matching the existing
				// FieldMerger override semantics and independent of merge strategy.
				values.insert(field, value.clone());
			},
			FieldPick::Candidate { candidate_id, .. } => {
				let candidate = candidates
					.iter()
					.find(|candidate| candidate.id == *candidate_id)
					.ok_or_else(|| ApplyError::CandidateNotFound(candidate_id.clone()))?;
				let fields: BTreeMap<MetadataField, Value> =
					serde_json::from_value(candidate.fields.clone()).map_err(|_| {
						ApplyError::CandidateMismatch {
							candidate_id: candidate_id.clone(),
						}
					})?;
				let value = fields.get(&field).ok_or_else(|| {
					ApplyError::CandidateFieldMissing {
						candidate_id: candidate_id.clone(),
						field,
					}
				})?;
				validate_value(field, value)?;
				if should_apply_candidate(field, existing, strategy) {
					let value = merge_candidate_value(field, value, existing, strategy)?;
					values.insert(field, value);
				}
			},
		}
	}
	Ok(ResolvedFields { values, cleared })
}

type CandidateContext<'a> = Option<(Option<&'a str>, Option<&'a str>, &'a str)>;

fn candidate_context(
	candidates: &[CandidateModel],
) -> Result<CandidateContext<'_>, ApplyError> {
	let Some(first) = candidates.first() else {
		return Ok(None);
	};
	let context = (
		first.drop_item_id.as_deref(),
		first.media_id.as_deref(),
		first.source_sha256.as_str(),
	);
	for candidate in candidates.iter().skip(1) {
		if candidate.drop_item_id.as_deref() != context.0
			|| candidate.media_id.as_deref() != context.1
			|| candidate.source_sha256 != context.2
		{
			return Err(ApplyError::CandidateMismatch {
				candidate_id: candidate.id.clone(),
			});
		}
	}
	if context.0.is_none() && context.1.is_none() {
		return Err(ApplyError::CandidateMismatch {
			candidate_id: first.id.clone(),
		});
	}
	Ok(Some(context))
}

fn should_apply_candidate(
	field: MetadataField,
	existing: Option<&media_metadata::Model>,
	strategy: MergeStrategy,
) -> bool {
	let Some(existing) = existing else {
		return true;
	};
	match strategy {
		MergeStrategy::FillGaps | MergeStrategy::FillAndMergeLists => {
			!field_is_present(field, existing)
		},
		MergeStrategy::PreferExternal | MergeStrategy::PreferExternalAndMergeLists => {
			true
		},
	}
}

fn merge_candidate_value(
	field: MetadataField,
	value: &Value,
	existing: Option<&media_metadata::Model>,
	strategy: MergeStrategy,
) -> Result<Value, ApplyError> {
	if !matches!(
		strategy,
		MergeStrategy::FillAndMergeLists | MergeStrategy::PreferExternalAndMergeLists
	) || !is_list_field(field)
	{
		return Ok(value.clone());
	}
	let Some(existing) = existing.and_then(|model| existing_list(field, model)) else {
		return Ok(value.clone());
	};
	let Some(incoming) = value.as_array() else {
		return Err(ApplyError::InvalidValue { field });
	};
	let mut merged: BTreeSet<String> = existing.into_iter().collect();
	for item in incoming {
		let value = item
			.as_str()
			.ok_or(ApplyError::InvalidValue { field })?
			.trim();
		if !value.is_empty() {
			merged.insert(value.to_string());
		}
	}
	Ok(json!(merged.into_iter().collect::<Vec<_>>()))
}

pub(crate) fn is_list_field(field: MetadataField) -> bool {
	matches!(
		field,
		MetadataField::Authors
			| MetadataField::Narrators
			| MetadataField::Tags
			| MetadataField::Genres
	)
}

pub(crate) fn existing_list(
	field: MetadataField,
	model: &media_metadata::Model,
) -> Option<Vec<String>> {
	let value = match field {
		MetadataField::Authors => model.writers.as_deref(),
		MetadataField::Narrators => model.narrators.as_deref(),
		MetadataField::Tags => None,
		MetadataField::Genres => model.genres.as_deref(),
		_ => None,
	}?;
	let values: Vec<_> = value
		.split(',')
		.map(str::trim)
		.filter(|value| !value.is_empty())
		.map(str::to_string)
		.collect();
	(!values.is_empty()).then_some(values)
}

pub(crate) fn field_is_present(
	field: MetadataField,
	model: &media_metadata::Model,
) -> bool {
	match field {
		MetadataField::Title => model.title.as_deref().is_some_and(non_empty),
		MetadataField::SortTitle => model.title_sort.as_deref().is_some_and(non_empty),
		MetadataField::Series => model.series.as_deref().is_some_and(non_empty),
		MetadataField::SeriesIndex => model.number.is_some(),
		MetadataField::Authors => model.writers.as_deref().is_some_and(non_empty),
		MetadataField::Narrators => model.narrators.as_deref().is_some_and(non_empty),
		MetadataField::Publisher => model.publisher.as_deref().is_some_and(non_empty),
		MetadataField::PublishedDate => {
			model.year.is_some() || model.month.is_some() || model.day.is_some()
		},
		MetadataField::Language => model.language.as_deref().is_some_and(non_empty),
		MetadataField::Summary => model.summary.as_deref().is_some_and(non_empty),
		MetadataField::Tags => false,
		MetadataField::Genres => model.genres.as_deref().is_some_and(non_empty),
		MetadataField::Isbn => model.identifier_isbn.as_deref().is_some_and(non_empty),
		MetadataField::AgeRating => model.age_rating.is_some(),
		MetadataField::PageCount => model.page_count.is_some(),
		MetadataField::CoverUrl | MetadataField::Identifiers | MetadataField::Status => {
			false
		},
	}
}

fn non_empty(value: &str) -> bool {
	!value.trim().is_empty()
}

fn cover_url(value: &Value) -> Option<&str> {
	value
		.as_str()
		.or_else(|| value.get("url").and_then(Value::as_str))
		.filter(|url| !url.trim().is_empty())
}

fn validate_value(field: MetadataField, value: &Value) -> Result<(), ApplyError> {
	let valid = match field {
		MetadataField::Title
		| MetadataField::SortTitle
		| MetadataField::Series
		| MetadataField::Publisher
		| MetadataField::Language
		| MetadataField::Summary
		| MetadataField::Isbn
		| MetadataField::Status
		| MetadataField::PublishedDate => value.is_string(),
		MetadataField::CoverUrl => cover_url(value).is_some(),
		MetadataField::SeriesIndex => value.as_f64().is_some_and(f64::is_finite),
		MetadataField::AgeRating | MetadataField::PageCount => value.as_i64().is_some(),
		MetadataField::Authors
		| MetadataField::Narrators
		| MetadataField::Tags
		| MetadataField::Genres => value
			.as_array()
			.is_some_and(|values| values.iter().all(Value::is_string)),
		MetadataField::Identifiers => value.is_object(),
	};
	if valid {
		Ok(())
	} else {
		Err(ApplyError::InvalidValue { field })
	}
}

/// Apply resolved values to a media metadata row and record the audit entry.
/// The operation is one transaction; tag links are created additively, while
/// an explicit CLEAR removes links for the Tags field.
pub async fn apply_to_media(
	conn: &DatabaseConnection,
	media_id: &str,
	resolved: ResolvedFields,
	actor: &str,
) -> Result<(), ApplyError> {
	validate_resolved_fields(&resolved)?;
	let txn = begin_write(conn).await.map_err(db_error)?;
	let cover_write = apply_to_media_txn_with_context_and_cover(
		&txn,
		media_id,
		resolved,
		actor,
		None,
		0,
		"RESOLVED",
		json!({}),
		None,
	)
	.await?;
	match txn.commit().await {
		Ok(()) => {
			if let Some(write) = cover_write {
				write.commit().await;
			}
			Ok(())
		},
		Err(error) => {
			if let Some(write) = cover_write {
				write.rollback().await;
			}
			Err(db_error(error))
		},
	}
}

/// Prepare a selected cover before opening a database write transaction.
pub async fn prepare_cover_for_media(
	conn: &DatabaseConnection,
	media_id: &str,
	resolved: &ResolvedFields,
	cover_config: &CoverApplyConfig,
) -> Result<Option<CoverWrite>, ApplyError> {
	validate_resolved_fields(resolved)?;
	if !cover_requested(resolved) {
		return Ok(None);
	}
	let media_row = media::Entity::find_by_id(media_id.to_string())
		.one(conn)
		.await
		.map_err(db_error)?
		.ok_or_else(|| ApplyError::MediaNotFound(media_id.to_string()))?;
	prepare_cover_with_previous(
		media_id,
		resolved,
		media_row.thumbnail_path,
		cover_config,
	)
	.await
}

/// Prepare a cover for a newly-created media row before its insert transaction.
pub async fn prepare_cover_for_new_media(
	media_id: &str,
	resolved: &ResolvedFields,
	cover_config: &CoverApplyConfig,
) -> Result<Option<CoverWrite>, ApplyError> {
	validate_resolved_fields(resolved)?;
	if !cover_requested(resolved) {
		return Ok(None);
	}
	prepare_cover_with_previous(media_id, resolved, None, cover_config).await
}

fn cover_requested(resolved: &ResolvedFields) -> bool {
	resolved.values.contains_key(&MetadataField::CoverUrl)
		|| resolved.cleared.contains(&MetadataField::CoverUrl)
}

async fn prepare_cover_with_previous(
	media_id: &str,
	resolved: &ResolvedFields,
	previous_path: Option<String>,
	cover_config: &CoverApplyConfig,
) -> Result<Option<CoverWrite>, ApplyError> {
	if let Some(url) = resolved
		.values
		.get(&MetadataField::CoverUrl)
		.and_then(cover_url)
	{
		Ok(Some(
			stage_cover(media_id, url, previous_path, cover_config).await?,
		))
	} else {
		Ok(Some(CoverWrite {
			new_path: None,
			created_new: false,
			previous_path,
			thumbnail_meta: None,
			thumbnail_dir: cover_config.media.get_thumbnails_dir().to_path_buf(),
		}))
	}
}

/// Apply resolved values and a selected cover to an existing library media
/// row. Cover download, validation, and file staging complete before the
/// metadata transaction is acquired.
pub async fn apply_to_media_with_cover(
	conn: &DatabaseConnection,
	media_id: &str,
	resolved: ResolvedFields,
	actor: &str,
	cover_config: &CoverApplyConfig,
) -> Result<(), ApplyError> {
	validate_resolved_fields(&resolved)?;
	let cover_write =
		prepare_cover_for_media(conn, media_id, &resolved, cover_config).await?;
	let txn = match begin_write(conn).await {
		Ok(txn) => txn,
		Err(error) => {
			if let Some(write) = cover_write {
				write.rollback().await;
			}
			return Err(db_error(error));
		},
	};
	let cover_write = apply_to_media_txn_with_context_and_cover(
		&txn,
		media_id,
		resolved,
		actor,
		None,
		0,
		"RESOLVED",
		json!({}),
		cover_write,
	)
	.await?;
	match txn.commit().await {
		Ok(()) => {
			if let Some(write) = cover_write {
				write.commit().await;
			}
			Ok(())
		},
		Err(error) => {
			if let Some(write) = cover_write {
				write.rollback().await;
			}
			Err(db_error(error))
		},
	}
}

/// Transaction-safe variant used by the staged store's approve operation.  It
/// deliberately accepts an existing [`DatabaseTransaction`] so media
/// metadata, tags, artwork, and the application audit row commit together
/// with the source-file move.
pub async fn apply_to_media_txn(
	txn: &DatabaseTransaction,
	media_id: &str,
	resolved: ResolvedFields,
	actor: &str,
) -> Result<(), ApplyError> {
	let picks = json!({
		"values": &resolved.values,
		"cleared": &resolved.cleared,
	});
	apply_to_media_txn_with_context(
		txn, media_id, resolved, actor, None, 0, "RESOLVED", picks,
	)
	.await
}

/// Apply values and record an audit row with the staged item's complete
/// context.  The caller can commit this transaction alongside the source-file
/// move.  Cover fields require a prepared write so callers cannot accidentally
/// persist a URL without downloading artwork.
#[allow(clippy::too_many_arguments)] // Public transaction API mirrors the audit context contract.
pub async fn apply_to_media_txn_with_context(
	txn: &DatabaseTransaction,
	media_id: &str,
	resolved: ResolvedFields,
	actor: &str,
	drop_item_id: Option<&str>,
	expected_revision: i32,
	strategy: &str,
	picks: Value,
) -> Result<(), ApplyError> {
	apply_to_media_txn_with_context_and_cover(
		txn,
		media_id,
		resolved,
		actor,
		drop_item_id,
		expected_revision,
		strategy,
		picks,
		None,
	)
	.await
	.map(|_| ())
}

/// Cover-aware transaction variant. The write was prepared before the
/// transaction began and must be committed after the database transaction
/// commits, or rolled back when the transaction is abandoned.
#[allow(clippy::too_many_arguments)]
pub async fn apply_to_media_txn_with_context_and_cover(
	txn: &DatabaseTransaction,
	media_id: &str,
	resolved: ResolvedFields,
	actor: &str,
	drop_item_id: Option<&str>,
	expected_revision: i32,
	strategy: &str,
	picks: Value,
	mut cover_write: Option<CoverWrite>,
) -> Result<Option<CoverWrite>, ApplyError> {
	if let Err(error) = validate_resolved_fields(&resolved) {
		if let Some(write) = cover_write.take() {
			write.rollback().await;
		}
		return Err(error);
	}
	let cover_requested = cover_requested(&resolved);
	if cover_requested && cover_write.is_none() {
		return Err(ApplyError::CoverConfigurationRequired);
	}

	let result: Result<(), ApplyError> = async {
		let values_for_audit = resolved.values.clone();
		let cleared_for_audit = resolved.cleared.clone();
		let existing = media_metadata::Entity::find()
			.filter(media_metadata::Column::MediaId.eq(media_id))
			.one(txn)
			.await
			.map_err(db_error)?;
		let mut active = existing
			.clone()
			.map(|model| model.into_active_model())
			.unwrap_or_else(|| media_metadata::ActiveModel {
				media_id: Set(Some(media_id.to_string())),
				..Default::default()
			});
		for (field, value) in &resolved.values {
			apply_value(&mut active, *field, value)?;
		}
		for field in &resolved.cleared {
			clear_value(&mut active, *field);
		}
		if existing.is_some() {
			media_metadata::Entity::update(active)
				.exec(txn)
				.await
				.map_err(db_error)?;
		} else {
			media_metadata::Entity::insert(active)
				.exec(txn)
				.await
				.map_err(db_error)?;
		}

		if let Some(value) = resolved.values.get(&MetadataField::Tags) {
			let tags = list_strings(value, MetadataField::Tags)?;
			ensure_tags(txn, media_id, &tags).await?;
		}
		if resolved.cleared.contains(&MetadataField::Tags) {
			media_tag::Entity::delete_many()
				.filter(media_tag::Column::MediaId.eq(media_id))
				.exec(txn)
				.await
				.map_err(db_error)?;
		}

		if let Some(write) = cover_write.as_ref() {
			let mut active = media::ActiveModel {
				id: Set(media_id.to_string()),
				..Default::default()
			};
			active.thumbnail_path = Set(write
				.new_path
				.as_ref()
				.map(|path| path.to_string_lossy().into_owned()));
			active.thumbnail_meta = Set(write.thumbnail_meta.clone());
			active.update(txn).await.map_err(db_error)?;
		}

		let applied_fields = json!(resolved
			.values
			.keys()
			.map(|field| serde_json::to_value(field).unwrap_or(Value::Null))
			.chain(
				resolved
					.cleared
					.iter()
					.map(|field| serde_json::to_value(field).unwrap_or(Value::Null)),
			)
			.collect::<Vec<_>>());
		// `ActiveModelTrait::insert` (not `Entity::insert(..).exec`) so the
		// entity's `before_save` hook assigns the id and timestamp.
		ingest_metadata_application::ActiveModel {
			drop_item_id: Set(drop_item_id.map(ToOwned::to_owned)),
			media_id: Set(Some(media_id.to_string())),
			expected_revision: Set(expected_revision),
			strategy: Set(strategy.to_string()),
			picks: Set({
				let mut audit_picks = serde_json::Map::new();
				audit_picks.insert("request".to_string(), picks);
				audit_picks.insert("resolvedValues".to_string(), json!(values_for_audit));
				audit_picks
					.insert("resolvedCleared".to_string(), json!(cleared_for_audit));
				Value::Object(audit_picks)
			}),
			actor: Set(actor.to_string()),
			applied_fields: Set(applied_fields),
			failed_fields: Set(json!([])),
			..Default::default()
		}
		.insert(txn)
		.await
		.map_err(db_error)?;
		Ok(())
	}
	.await;

	if let Err(error) = result {
		if let Some(write) = cover_write.take() {
			write.rollback().await;
		}
		return Err(error);
	}
	Ok(cover_write)
}

async fn stage_cover(
	media_id: &str,
	url: &str,
	previous_path: Option<String>,
	config: &CoverApplyConfig,
) -> Result<CoverWrite, ApplyError> {
	let (bytes, content_type) = download_cover(url, config).await?;
	if bytes.len() > config.max_bytes {
		return Err(ApplyError::CoverTooLarge {
			max_bytes: config.max_bytes,
		});
	}
	let metadata = generate_image_metadata_from_bytes(bytes.clone())
		.await
		.map_err(|error| ApplyError::CoverInvalid(error.to_string()))?;
	let dimensions = metadata.dimensions.as_ref().ok_or_else(|| {
		ApplyError::CoverInvalid("image dimensions are unavailable".to_string())
	})?;
	const MAX_COVER_DIMENSION: u32 = 16_384;
	const MAX_COVER_PIXELS: u64 = 100_000_000;
	let pixels = u64::from(dimensions.width).saturating_mul(u64::from(dimensions.height));
	if dimensions.width == 0
		|| dimensions.height == 0
		|| dimensions.width > MAX_COVER_DIMENSION
		|| dimensions.height > MAX_COVER_DIMENSION
		|| pixels > MAX_COVER_PIXELS
	{
		return Err(ApplyError::CoverInvalid(format!(
			"image dimensions {}x{} exceed supported bounds",
			dimensions.width, dimensions.height
		)));
	}

	let digest = {
		let mut context = ring::digest::Context::new(&ring::digest::SHA256);
		context.update(&bytes);
		data_encoding::HEXLOWER.encode(context.finish().as_ref())
	};
	let thumbnail_dir = config.media.get_thumbnails_dir().to_path_buf();
	fs::create_dir_all(&thumbnail_dir)
		.await
		.map_err(|error| ApplyError::CoverDownload(error.to_string()))?;
	let safe_id = media_id
		.chars()
		.map(|character| {
			if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
				character
			} else {
				'_'
			}
		})
		.collect::<String>();
	let final_path =
		thumbnail_dir.join(format!("{safe_id}-{digest}.{}", content_type.extension()));
	let created_new = if fs::try_exists(&final_path)
		.await
		.map_err(|error| ApplyError::CoverDownload(error.to_string()))?
	{
		false
	} else {
		let temporary_path = thumbnail_dir.join(format!(".cover-{}", Uuid::new_v4()));
		let result = async {
			fs::write(&temporary_path, &bytes)
				.await
				.map_err(|error| ApplyError::CoverDownload(error.to_string()))?;
			fs::rename(&temporary_path, &final_path)
				.await
				.map_err(|error| ApplyError::CoverDownload(error.to_string()))?;
			Ok::<_, ApplyError>(())
		}
		.await;
		if let Err(error) = result {
			let _ = fs::remove_file(&temporary_path).await;
			return Err(error);
		}
		true
	};

	Ok(CoverWrite {
		new_path: Some(final_path),
		created_new,
		previous_path,
		thumbnail_meta: Some(metadata),
		thumbnail_dir,
	})
}

async fn download_cover(
	url: &str,
	config: &CoverApplyConfig,
) -> Result<(Vec<u8>, ContentType), ApplyError> {
	let parsed = reqwest::Url::parse(url)
		.map_err(|error| ApplyError::CoverDownload(format!("invalid URL: {error}")))?;
	let mut response = fetch_cover_response(parsed, config).await?;
	if !response.status().is_success() {
		return Err(ApplyError::CoverDownload(format!(
			"HTTP {}",
			response.status()
		)));
	}
	if response
		.content_length()
		.is_some_and(|size| size > config.max_bytes as u64)
	{
		return Err(ApplyError::CoverTooLarge {
			max_bytes: config.max_bytes,
		});
	}
	let declared_type = response
		.headers()
		.get(reqwest::header::CONTENT_TYPE)
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.split(';').next())
		.map(str::trim)
		.filter(|value| !value.is_empty())
		.map(ToOwned::to_owned);
	let mut bytes = Vec::new();
	while let Some(chunk) = response
		.chunk()
		.await
		.map_err(|error| ApplyError::CoverDownload(error.to_string()))?
	{
		if bytes.len().saturating_add(chunk.len()) > config.max_bytes {
			return Err(ApplyError::CoverTooLarge {
				max_bytes: config.max_bytes,
			});
		}
		bytes.extend_from_slice(&chunk);
	}
	let content_type = ContentType::from_bytes(&bytes);
	if !content_type.is_decodable_image() {
		return Err(ApplyError::CoverInvalid(
			"downloaded bytes are not a supported image".to_string(),
		));
	}
	if let Some(declared) = declared_type {
		let declared_content_type = ContentType::from(declared.as_str());
		let is_octet_stream = declared.eq_ignore_ascii_case("application/octet-stream");
		if !is_octet_stream && !declared_content_type.is_decodable_image() {
			return Err(ApplyError::CoverInvalid(format!(
				"unsupported MIME type {declared}"
			)));
		}
		if declared_content_type.is_decodable_image()
			&& declared_content_type != content_type
		{
			return Err(ApplyError::CoverInvalid(format!(
				"MIME type {declared} does not match image bytes {}",
				content_type.mime_type()
			)));
		}
	}
	let header = &bytes[..bytes.len().min(64 * 1024)];
	let dimensions = imagesize::blob_size(header).map_err(|error| {
		ApplyError::CoverInvalid(format!("invalid dimensions: {error}"))
	})?;
	if dimensions.width == 0 || dimensions.height == 0 {
		return Err(ApplyError::CoverInvalid(
			"image dimensions must be non-zero".to_string(),
		));
	}
	Ok((bytes, content_type))
}

async fn fetch_cover_response(
	mut url: reqwest::Url,
	config: &CoverApplyConfig,
) -> Result<reqwest::Response, ApplyError> {
	for redirect in 0..=5 {
		let (host, address) = validated_cover_endpoint(&url).await?;
		let client = reqwest::Client::builder()
			.timeout(config.timeout)
			.no_proxy()
			.redirect(reqwest::redirect::Policy::none())
			.resolve(&host, address)
			.build()
			.map_err(|error| ApplyError::CoverDownload(error.to_string()))?;
		let response = client
			.get(url.clone())
			.header(reqwest::header::ACCEPT, "image/*")
			.send()
			.await
			.map_err(|error| ApplyError::CoverDownload(error.to_string()))?;
		if !response.status().is_redirection() {
			return Ok(response);
		}
		if redirect == 5 {
			return Err(ApplyError::CoverDownload(
				"cover URL exceeded the redirect limit".to_string(),
			));
		}
		let location = response
			.headers()
			.get(reqwest::header::LOCATION)
			.and_then(|value| value.to_str().ok())
			.ok_or_else(|| {
				ApplyError::CoverDownload(
					"redirect response has no valid Location".to_string(),
				)
			})?;
		url = url.join(location).map_err(|error| {
			ApplyError::CoverDownload(format!("invalid redirect URL: {error}"))
		})?;
	}
	Err(ApplyError::CoverDownload(
		"cover URL redirect resolution failed".to_string(),
	))
}

async fn validated_cover_endpoint(
	url: &reqwest::Url,
) -> Result<(String, SocketAddr), ApplyError> {
	if !matches!(url.scheme(), "http" | "https")
		|| url.host_str().is_none()
		|| !url.username().is_empty()
		|| url.password().is_some()
	{
		return Err(ApplyError::CoverDownload(
			"cover URL must use http or https without credentials".to_string(),
		));
	}
	let host = url
		.host_str()
		.ok_or_else(|| ApplyError::CoverDownload("cover URL has no host".to_string()))?;
	if !is_allowed_cover_hostname(host) {
		return Err(ApplyError::CoverDownload(
			"cover URL host is not a public destination".to_string(),
		));
	}
	let port = url.port_or_known_default().ok_or_else(|| {
		ApplyError::CoverDownload("cover URL has no supported port".to_string())
	})?;
	if let Ok(ip) = host.parse::<IpAddr>() {
		if !is_public_ip(ip) {
			return Err(ApplyError::CoverDownload(
				"cover URL host is not a public destination".to_string(),
			));
		}
		return Ok((host.to_string(), SocketAddr::new(ip, port)));
	}
	let host_for_lookup = host.to_string();
	let addresses = tokio::task::spawn_blocking(move || {
		(host_for_lookup.as_str(), port)
			.to_socket_addrs()
			.map(|addresses| addresses.map(|address| address.ip()).collect::<Vec<_>>())
	})
	.await
	.map_err(|error| ApplyError::CoverDownload(error.to_string()))?
	.map_err(|error| {
		ApplyError::CoverDownload(format!("cover host lookup failed: {error}"))
	})?;
	let ip = addresses
		.into_iter()
		.find(|ip| is_public_ip(*ip))
		.ok_or_else(|| {
			ApplyError::CoverDownload(
				"cover URL host resolved only to non-public addresses".to_string(),
			)
		})?;
	Ok((host.to_string(), SocketAddr::new(ip, port)))
}

fn is_allowed_cover_hostname(host: &str) -> bool {
	let host = host.trim_end_matches('.').to_ascii_lowercase();
	!host.is_empty()
		&& !host.contains('%')
		&& host != "localhost"
		&& !host.ends_with(".localhost")
		&& !host.ends_with(".local")
		&& !host.ends_with(".localdomain")
		&& !host.ends_with(".internal")
		&& !host.ends_with(".lan")
		&& !host.ends_with(".home.arpa")
		&& !host.ends_with(".test")
		&& !host.ends_with(".invalid")
		&& !host.ends_with(".example")
}

fn is_public_ip(ip: IpAddr) -> bool {
	match ip {
		IpAddr::V4(ip) => {
			let [a, b, c, _] = ip.octets();
			!ip.is_private()
				&& !ip.is_loopback()
				&& !ip.is_link_local()
				&& !ip.is_unspecified()
				&& !ip.is_multicast()
				&& !ip.is_broadcast()
				&& a != 0 && a < 224
				&& !(a == 100 && (64..=127).contains(&b))
				&& !(a == 192 && b == 0 && c <= 2)
				&& !(a == 192 && b == 0 && c == 9)
				&& !(a == 192 && b == 0 && c == 10)
				&& !(a == 192 && b == 88 && c == 99)
				&& !(a == 198 && b == 18)
				&& !(a == 198 && b == 19)
				&& !(a == 198 && b == 51 && c == 100)
				&& !(a == 203 && b == 0 && c == 113)
		},
		IpAddr::V6(ip) => {
			let segments = ip.segments();
			if segments[..5].iter().all(|segment| *segment == 0) && segments[5] == 0xffff
			{
				let mapped = std::net::Ipv4Addr::new(
					(segments[6] >> 8) as u8,
					segments[6] as u8,
					(segments[7] >> 8) as u8,
					segments[7] as u8,
				);
				return is_public_ip(IpAddr::V4(mapped));
			}
			(segments[0] & 0xe000) == 0x2000
				&& !((segments[0] & 0xfe00) == 0xfc00)
				&& !((segments[0] & 0xffc0) == 0xfe80)
				&& !(segments[0] == 0x2001 && segments[1] == 0x0db8)
				&& !(segments[0] == 0x2002)
				&& !ip.is_loopback()
				&& !ip.is_unspecified()
				&& !ip.is_multicast()
		},
	}
}

async fn ensure_tags(
	txn: &DatabaseTransaction,
	media_id: &str,
	tag_names: &[String],
) -> Result<(), ApplyError> {
	for name in tag_names {
		let Some(name) = (!name.trim().is_empty()).then(|| name.trim().to_string())
		else {
			continue;
		};
		let tag_model = match tag::Entity::find()
			.filter(tag::Column::Name.eq(name.clone()))
			.one(txn)
			.await
			.map_err(db_error)?
		{
			Some(tag) => tag,
			None => tag::ActiveModel {
				name: Set(name),
				..Default::default()
			}
			.insert(txn)
			.await
			.map_err(db_error)?,
		};
		let linked = media_tag::Entity::find()
			.filter(media_tag::Column::MediaId.eq(media_id))
			.filter(media_tag::Column::TagId.eq(tag_model.id))
			.one(txn)
			.await
			.map_err(db_error)?;
		if linked.is_none() {
			media_tag::ActiveModel {
				media_id: Set(media_id.to_string()),
				tag_id: Set(tag_model.id),
				..Default::default()
			}
			.insert(txn)
			.await
			.map_err(db_error)?;
		}
	}
	Ok(())
}

fn list_strings(value: &Value, field: MetadataField) -> Result<Vec<String>, ApplyError> {
	value
		.as_array()
		.ok_or(ApplyError::InvalidValue { field })?
		.iter()
		.map(|value| {
			value
				.as_str()
				.map(|value| value.trim().to_string())
				.ok_or(ApplyError::InvalidValue { field })
		})
		.collect()
}

fn apply_value(
	active: &mut media_metadata::ActiveModel,
	field: MetadataField,
	value: &Value,
) -> Result<(), ApplyError> {
	validate_value(field, value)?;
	match field {
		MetadataField::Title => active.title = Set(value.as_str().map(str::to_string)),
		MetadataField::SortTitle => {
			active.title_sort = Set(value.as_str().map(str::to_string))
		},
		MetadataField::Series => active.series = Set(value.as_str().map(str::to_string)),
		MetadataField::SeriesIndex => {
			active.number = Set(value.as_f64().and_then(Decimal::from_f64));
		},
		MetadataField::Authors => active.writers = Set(Some(join_strings(value, field)?)),
		MetadataField::Narrators => {
			active.narrators = Set(Some(join_strings(value, field)?))
		},
		MetadataField::Publisher => {
			active.publisher = Set(value.as_str().map(str::to_string))
		},
		MetadataField::PublishedDate => {
			apply_date(active, value.as_str().unwrap_or_default())?
		},
		MetadataField::Language => {
			active.language = Set(value.as_str().map(str::to_string))
		},
		MetadataField::Summary => {
			active.summary = Set(value.as_str().map(str::to_string))
		},
		MetadataField::Genres => active.genres = Set(Some(join_strings(value, field)?)),
		MetadataField::Isbn => {
			active.identifier_isbn = Set(value.as_str().map(str::to_string))
		},
		MetadataField::AgeRating => {
			active.age_rating = Set(value.as_i64().map(|value| value as i32))
		},
		MetadataField::PageCount => {
			active.page_count = Set(value.as_i64().map(|value| value as i32))
		},
		MetadataField::Tags
		| MetadataField::CoverUrl
		| MetadataField::Identifiers
		| MetadataField::Status => {},
	}
	Ok(())
}

fn clear_value(active: &mut media_metadata::ActiveModel, field: MetadataField) {
	match field {
		MetadataField::Title => active.title = Set(None),
		MetadataField::SortTitle => active.title_sort = Set(None),
		MetadataField::Series => active.series = Set(None),
		MetadataField::SeriesIndex => active.number = Set(None),
		MetadataField::Authors => active.writers = Set(None),
		MetadataField::Narrators => active.narrators = Set(None),
		MetadataField::Publisher => active.publisher = Set(None),
		MetadataField::PublishedDate => {
			active.year = Set(None);
			active.month = Set(None);
			active.day = Set(None);
		},
		MetadataField::Language => active.language = Set(None),
		MetadataField::Summary => active.summary = Set(None),
		MetadataField::Genres => active.genres = Set(None),
		MetadataField::Isbn => active.identifier_isbn = Set(None),
		MetadataField::AgeRating => active.age_rating = Set(None),
		MetadataField::PageCount => active.page_count = Set(None),
		MetadataField::Tags
		| MetadataField::CoverUrl
		| MetadataField::Identifiers
		| MetadataField::Status => {},
	}
}

fn join_strings(value: &Value, field: MetadataField) -> Result<String, ApplyError> {
	Ok(list_strings(value, field)?.join(", "))
}

fn apply_date(
	active: &mut media_metadata::ActiveModel,
	value: &str,
) -> Result<(), ApplyError> {
	let mut parts = value.split('-');
	let year = parts
		.next()
		.and_then(|part| part.parse::<i32>().ok())
		.ok_or(ApplyError::InvalidValue {
			field: MetadataField::PublishedDate,
		})?;
	let month = parts.next().and_then(|part| part.parse::<i32>().ok());
	let day = parts.next().and_then(|part| part.parse::<i32>().ok());
	if parts.next().is_some() || month.is_some_and(|value| !(1..=12).contains(&value)) {
		return Err(ApplyError::InvalidValue {
			field: MetadataField::PublishedDate,
		});
	}
	if day.is_some_and(|value| !(1..=31).contains(&value)) {
		return Err(ApplyError::InvalidValue {
			field: MetadataField::PublishedDate,
		});
	}
	active.year = Set(Some(year));
	active.month = Set(month);
	active.day = Set(day);
	Ok(())
}

fn db_error(error: DbErr) -> ApplyError {
	ApplyError::Database(error.to_string())
}

#[cfg(test)]
mod tests {
	use chrono::Utc;
	use models::entity::ingest_metadata_candidate;

	use super::*;

	fn candidate(
		id: &str,
		drop_item_id: &str,
		digest: &str,
		fields: Value,
	) -> CandidateModel {
		ingest_metadata_candidate::Model {
			id: id.to_string(),
			drop_item_id: Some(drop_item_id.to_string()),
			media_id: None,
			provider_id: "provider".to_string(),
			provider_version: "1".to_string(),
			external_id: Some("external".to_string()),
			source_sha256: digest.to_string(),
			confidence: 1.0,
			fields,
			field_confidence: json!({}),
			provenance: json!({}),
			status: "PENDING".to_string(),
			created_at: Utc::now().into(),
			updated_at: Utc::now().into(),
		}
	}

	#[test]
	fn cover_url_is_storable_but_not_a_metadata_column() {
		assert!(validate_picks(&[FieldPick::Manual {
			field: MetadataField::CoverUrl,
			value: json!("https://example.test/cover.jpg"),
		}])
		.is_ok());
		assert!(STORABLE_FIELDS.contains(&MetadataField::Tags));
		assert!(STORABLE_FIELDS.contains(&MetadataField::CoverUrl));
	}
	#[test]
	fn cover_endpoint_rejects_loopback_and_local_domains() {
		assert!(!is_allowed_cover_hostname("localhost"));
		assert!(!is_allowed_cover_hostname("covers.local"));
		assert!(!is_allowed_cover_hostname("service.internal"));
		assert!(!is_public_ip("127.0.0.1".parse().unwrap()));
		assert!(!is_public_ip("10.0.0.7".parse().unwrap()));
		assert!(!is_public_ip("::1".parse().unwrap()));
		assert!(!is_public_ip("fd00::7".parse().unwrap()));
		assert!(is_public_ip("93.184.216.34".parse().unwrap()));
		let redirect = reqwest::Url::parse("http://127.0.0.1/private-cover").unwrap();
		assert!(!is_public_ip(redirect.host_str().unwrap().parse().unwrap()));
	}

	#[test]
	fn pick_validation_table() {
		let cases = [
			(
				vec![FieldPick::KeepExisting {
					field: MetadataField::Title,
				}],
				true,
			),
			(
				vec![
					FieldPick::KeepExisting {
						field: MetadataField::Title,
					},
					FieldPick::Clear {
						field: MetadataField::Title,
					},
				],
				false,
			),
			(
				vec![FieldPick::Candidate {
					field: MetadataField::Title,
					candidate_id: String::new(),
				}],
				false,
			),
			(
				vec![FieldPick::Manual {
					field: MetadataField::Title,
					value: json!(42),
				}],
				false,
			),
			(
				vec![FieldPick::Manual {
					field: MetadataField::Title,
					value: json!("title"),
				}],
				true,
			),
			(
				vec![FieldPick::Clear {
					field: MetadataField::Title,
				}],
				true,
			),
		];
		for (picks, expected) in cases {
			assert_eq!(validate_picks(&picks).is_ok(), expected);
		}
	}

	#[test]
	fn resolution_precedence_lock_clear_candidate_manual_keep() {
		let all_fields = json!({
			"TITLE": "candidate title",
			"SUMMARY": "candidate summary",
			"SERIES": "candidate series",
			"PUBLISHER": "candidate publisher"
		});
		let candidate = candidate("candidate", "drop", "digest", all_fields);
		let existing = media_metadata::Model {
			title: Some("existing title".to_string()),
			summary: Some("existing summary".to_string()),
			..Default::default()
		};
		let picks = vec![
			FieldPick::Candidate {
				field: MetadataField::Title,
				candidate_id: "candidate".to_string(),
			},
			FieldPick::Clear {
				field: MetadataField::Summary,
			},
			FieldPick::Manual {
				field: MetadataField::Series,
				value: json!("manual series"),
			},
			FieldPick::KeepExisting {
				field: MetadataField::Publisher,
			},
		];
		let resolved = resolve_picks(
			&picks,
			&[candidate],
			Some(&existing),
			&[MetadataField::Title],
			MergeStrategy::PreferExternal,
		)
		.unwrap();
		assert!(!resolved.values.contains_key(&MetadataField::Title));
		assert_eq!(resolved.cleared, vec![MetadataField::Summary]);
		assert_eq!(resolved.values[&MetadataField::Series], "manual series");
		assert!(!resolved.values.contains_key(&MetadataField::Publisher));
	}

	#[test]
	fn fill_gaps_and_overwrite_strategies() {
		let candidate =
			candidate("candidate", "drop", "digest", json!({"TITLE": "incoming"}));
		let existing = media_metadata::Model {
			title: Some("existing".to_string()),
			..Default::default()
		};
		let picks = vec![FieldPick::Candidate {
			field: MetadataField::Title,
			candidate_id: "candidate".to_string(),
		}];
		let fill = resolve_picks(
			&picks,
			std::slice::from_ref(&candidate),
			Some(&existing),
			&[],
			MergeStrategy::FillGaps,
		)
		.unwrap();
		assert!(fill.values.is_empty());
		let overwrite = resolve_picks(
			&picks,
			&[candidate],
			Some(&existing),
			&[],
			MergeStrategy::PreferExternal,
		)
		.unwrap();
		assert_eq!(overwrite.values[&MetadataField::Title], "incoming");
	}

	#[test]
	fn stale_digest_or_mixed_context_is_rejected() {
		let first = candidate("first", "drop", "digest-a", json!({"TITLE": "a"}));
		let second = candidate("second", "drop", "digest-b", json!({"TITLE": "b"}));
		let picks = vec![FieldPick::Candidate {
			field: MetadataField::Title,
			candidate_id: "first".to_string(),
		}];
		assert!(matches!(
			resolve_picks(&picks, &[first, second], None, &[], MergeStrategy::FillGaps,),
			Err(ApplyError::CandidateMismatch { .. })
		));
	}
}
