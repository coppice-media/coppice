//! GraphQL representation of a validated tier-2 read-aloud map.
//!
//! The worker crate stays transport-neutral, so GraphQL owns the public enum
//! mirror and converts it to the worker's wire enum at the resolver boundary.

use async_graphql::{Enum, Json, SimpleObject, ID};
use models::entity::media_sync_map;
use serde_json::Value;

/// Granularity of cues in a sync map or alignment request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Enum)]
pub enum AlignGranularity {
	#[default]
	Sentence,
	Word,
}

impl From<AlignGranularity> for stump_worker::AlignGranularity {
	fn from(granularity: AlignGranularity) -> Self {
		match granularity {
			AlignGranularity::Sentence => Self::Sentence,
			AlignGranularity::Word => Self::Word,
		}
	}
}

impl From<stump_worker::AlignGranularity> for AlignGranularity {
	fn from(granularity: stump_worker::AlignGranularity) -> Self {
		match granularity {
			stump_worker::AlignGranularity::Sentence => Self::Sentence,
			stump_worker::AlignGranularity::Word => Self::Word,
		}
	}
}

/// One persisted, validated `SyncMapV1` artifact.
#[derive(Debug, Clone, SimpleObject)]
pub struct SyncMap {
	pub id: ID,
	pub ebook_media_id: ID,
	pub audio_media_id: ID,
	pub granularity: AlignGranularity,
	pub generator: String,
	pub generator_version: String,
	pub algorithm: Option<String>,
	pub model: Option<String>,
	pub cue_count: i32,
	pub coverage: Option<f64>,
	pub source: String,
	pub job_id: Option<ID>,
	pub created_at: String,
	pub map: Json<Value>,
}

/// A ready-to-download derivative. This object is returned only when the
/// accepted map's deterministic cache file exists.
#[derive(Debug, Clone, SimpleObject)]
pub struct ReadAloudArtifact {
	pub url: String,
	pub mime_type: String,
	pub cache_key: String,
}

impl From<media_sync_map::Model> for SyncMap {
	fn from(model: media_sync_map::Model) -> Self {
		let granularity = match model.granularity.as_str() {
			"word" => AlignGranularity::Word,
			_ => AlignGranularity::Sentence,
		};
		Self {
			id: ID::from(model.id),
			ebook_media_id: ID::from(model.ebook_media_id),
			audio_media_id: ID::from(model.audio_media_id),
			granularity,
			generator: model.generator,
			generator_version: model.generator_version,
			algorithm: model.algorithm,
			model: model.model,
			cue_count: model.cue_count,
			coverage: model.coverage,
			source: model.source,
			job_id: model.job_id.map(ID::from),
			created_at: model.created_at.to_rfc3339(),
			map: Json(model.map),
		}
	}
}
