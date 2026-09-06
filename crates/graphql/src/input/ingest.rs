use async_graphql::{Enum, InputObject, Json, Upload};
use metadata_integrations::{MergeStrategy, MetadataField as ExistingMetadataField};
use serde_json::Value;
use stump_ingest::contract::{FieldPick, MetadataField};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Enum)]
pub enum IngestMetadataFieldMode {
	KeepExisting,
	Candidate,
	Manual,
	Clear,
}

#[derive(Debug, Clone, InputObject)]
pub struct IngestMetadataFieldSelectionInput {
	pub field: ExistingMetadataField,
	pub mode: IngestMetadataFieldMode,
	pub candidate_id: Option<async_graphql::ID>,
	pub value: Option<Json<Value>>,
}

impl IngestMetadataFieldSelectionInput {
	/// Convert the public metadata enum to the smaller, provider-neutral ingest
	/// field set and reject mode/value combinations before touching persistence.
	pub fn into_field_pick(self) -> Result<FieldPick, String> {
		let field = map_metadata_field(self.field)?;
		let candidate_id = self.candidate_id.map(|id| id.to_string());
		let value = self.value.map(|json| json.0);

		match self.mode {
			IngestMetadataFieldMode::KeepExisting => {
				if candidate_id.is_some() || value.is_some() {
					return Err(format!(
						"KEEP_EXISTING selection for {field:?} cannot include candidateId or value"
					));
				}
				Ok(FieldPick::KeepExisting { field })
			},
			IngestMetadataFieldMode::Candidate => {
				let candidate_id = candidate_id.ok_or_else(|| {
					format!("CANDIDATE selection for {field:?} requires candidateId")
				})?;
				if value.is_some() {
					return Err(format!(
						"CANDIDATE selection for {field:?} cannot include value"
					));
				}
				Ok(FieldPick::Candidate {
					field,
					candidate_id,
				})
			},
			IngestMetadataFieldMode::Manual => {
				let value = value.ok_or_else(|| {
					format!("MANUAL selection for {field:?} requires value")
				})?;
				if candidate_id.is_some() {
					return Err(format!(
						"MANUAL selection for {field:?} cannot include candidateId"
					));
				}
				Ok(FieldPick::Manual { field, value })
			},
			IngestMetadataFieldMode::Clear => {
				if candidate_id.is_some() || value.is_some() {
					return Err(format!(
						"CLEAR selection for {field:?} cannot include candidateId or value"
					));
				}
				Ok(FieldPick::Clear { field })
			},
		}
	}
}

/// Convert the public metadata enum (which the legacy metadata mutations and
/// the per-row lock lists also use) to the canonical ingest field enum. The
/// table itself lives with the enum in `stump_ingest::contract`; unsupported
/// public-only fields are rejected rather than folded onto the wrong field.
pub fn map_metadata_field(field: ExistingMetadataField) -> Result<MetadataField, String> {
	MetadataField::from_public(field).ok_or_else(|| {
		format!("metadata field {field:?} is not supported by staged ingest")
	})
}

#[derive(Debug, InputObject)]
pub struct IngestUploadFileInput {
	pub file: Upload,
	pub relative_path: Option<String>,
}

#[derive(Debug, InputObject)]
pub struct StageIngestUploadsInput {
	pub library_id: async_graphql::ID,
	pub files: Vec<IngestUploadFileInput>,
	pub idempotency_key: Option<String>,
	#[graphql(default = true)]
	pub start_analysis: bool,
}

#[derive(Debug, InputObject)]
pub struct EnqueueIngestAnalysisInput {
	pub drop_item_ids: Vec<async_graphql::ID>,
	#[graphql(default)]
	pub force: bool,
}

/// Exactly one of `drop_item_id` (staged item) or `media_id` (library-wide
/// rework target) must be provided.
#[derive(Debug, InputObject)]
pub struct ApplyIngestMetadataInput {
	pub drop_item_id: Option<async_graphql::ID>,
	pub media_id: Option<async_graphql::ID>,
	pub selections: Vec<IngestMetadataFieldSelectionInput>,
	pub strategy: Option<MergeStrategy>,
}

#[derive(Debug, InputObject)]
pub struct BulkApplyIngestMetadataInput {
	pub drop_item_ids: Vec<async_graphql::ID>,
	pub selections: Vec<IngestMetadataFieldSelectionInput>,
	pub strategy: Option<MergeStrategy>,
}

#[derive(Debug, InputObject)]
pub struct SetIngestProviderSettingsInput {
	pub provider_id: String,
	pub enabled: Option<bool>,
	pub opted_in: Option<bool>,
	pub settings: Option<Json<Value>>,
}

#[derive(Debug, InputObject)]
pub struct SetIngestQualityCheckSettingsInput {
	pub check_id: String,
	pub enabled: Option<bool>,
	pub settings: Option<Json<Value>>,
}

#[cfg(test)]
mod tests {
	use super::*;

	fn selection(
		mode: IngestMetadataFieldMode,
		candidate_id: Option<&str>,
		value: Option<Value>,
	) -> IngestMetadataFieldSelectionInput {
		IngestMetadataFieldSelectionInput {
			field: ExistingMetadataField::Title,
			mode,
			candidate_id: candidate_id.map(Into::into),
			value: value.map(Json),
		}
	}

	#[test]
	fn rejects_invalid_pick_payloads() {
		assert!(selection(IngestMetadataFieldMode::Candidate, None, None)
			.into_field_pick()
			.is_err());
		assert!(selection(IngestMetadataFieldMode::Manual, None, None)
			.into_field_pick()
			.is_err());
		assert!(selection(
			IngestMetadataFieldMode::KeepExisting,
			Some("candidate"),
			None,
		)
		.into_field_pick()
		.is_err());
		assert!(
			selection(IngestMetadataFieldMode::Clear, None, Some(Value::Null),)
				.into_field_pick()
				.is_err()
		);
	}

	#[test]
	fn converts_valid_manual_pick() {
		let pick = selection(
			IngestMetadataFieldMode::Manual,
			None,
			Some(Value::String("title".to_owned())),
		)
		.into_field_pick()
		.unwrap();
		assert!(matches!(
			pick,
			FieldPick::Manual {
				field: MetadataField::Title,
				..
			}
		));
	}
}
