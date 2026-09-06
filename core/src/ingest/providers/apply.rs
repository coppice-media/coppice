use std::collections::{BTreeMap, BTreeSet};

use metadata_integrations::MergeStrategy;
use models::entity::{
	ingest_metadata_application, ingest_metadata_candidate, media_metadata, media_tag,
	tag,
};
use models::txn::begin_write;
use rust_decimal::{prelude::FromPrimitive, Decimal};
use sea_orm::{
	prelude::*, ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait,
	IntoActiveModel, QueryFilter, Set,
};
use serde_json::{json, Value};

use crate::ingest::contract::{FieldPick, MetadataField};

pub type CandidateModel = ingest_metadata_candidate::Model;

/// Fields produced by resolving one editor recipe.  `cleared` is separate from
/// `values` so an explicit CLEAR is never confused with a missing candidate
/// value.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedFields {
	pub values: BTreeMap<MetadataField, Value>,
	pub cleared: Vec<MetadataField>,
}

/// Canonical fields currently persisted by the media metadata/tag tables.
/// Cover URLs and arbitrary identifier objects stay candidate-only until the
/// library has a dedicated storage/apply path.
pub const STORABLE_FIELDS: &[MetadataField] = &[
	MetadataField::Title,
	MetadataField::SortTitle,
	MetadataField::Series,
	MetadataField::SeriesIndex,
	MetadataField::Authors,
	MetadataField::Publisher,
	MetadataField::PublishedDate,
	MetadataField::Language,
	MetadataField::Summary,
	MetadataField::Tags,
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

fn is_list_field(field: MetadataField) -> bool {
	matches!(
		field,
		MetadataField::Authors | MetadataField::Tags | MetadataField::Genres
	)
}

fn existing_list(
	field: MetadataField,
	model: &media_metadata::Model,
) -> Option<Vec<String>> {
	let value = match field {
		MetadataField::Authors => model.writers.as_deref(),
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

fn field_is_present(field: MetadataField, model: &media_metadata::Model) -> bool {
	match field {
		MetadataField::Title => model.title.as_deref().is_some_and(non_empty),
		MetadataField::SortTitle => model.title_sort.as_deref().is_some_and(non_empty),
		MetadataField::Series => model.series.as_deref().is_some_and(non_empty),
		MetadataField::SeriesIndex => model.number.is_some(),
		MetadataField::Authors => model.writers.as_deref().is_some_and(non_empty),
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
		MetadataField::CoverUrl | MetadataField::Identifiers => false,
	}
}

fn non_empty(value: &str) -> bool {
	!value.trim().is_empty()
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
		| MetadataField::CoverUrl
		| MetadataField::PublishedDate => value.is_string(),
		MetadataField::SeriesIndex => value.as_f64().is_some_and(f64::is_finite),
		MetadataField::AgeRating | MetadataField::PageCount => value.as_i64().is_some(),
		MetadataField::Authors | MetadataField::Tags | MetadataField::Genres => value
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
	apply_to_media_txn(&txn, media_id, resolved, actor).await?;
	txn.commit().await.map_err(db_error)?;
	Ok(())
}

/// Transaction-safe variant used by the staged store's approve operation.  It
/// deliberately accepts an existing [`DatabaseTransaction`] so media metadata,
/// tags, and the application audit row commit together with the source move.
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
/// move and drop-item revision update.
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
	validate_resolved_fields(&resolved)?;
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
			audit_picks.insert("resolvedCleared".to_string(), json!(cleared_for_audit));
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
		MetadataField::Tags | MetadataField::CoverUrl | MetadataField::Identifiers => {},
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
		MetadataField::Tags | MetadataField::CoverUrl | MetadataField::Identifiers => {},
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
	fn unsupported_fields_fail_before_application() {
		let error = validate_picks(&[FieldPick::Manual {
			field: MetadataField::CoverUrl,
			value: json!("https://example.test/cover.jpg"),
		}])
		.unwrap_err();
		assert_eq!(
			error,
			ApplyError::UnsupportedFields(vec![MetadataField::CoverUrl])
		);
		assert!(STORABLE_FIELDS.contains(&MetadataField::Tags));
		assert!(!STORABLE_FIELDS.contains(&MetadataField::CoverUrl));
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
