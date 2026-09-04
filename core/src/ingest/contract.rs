//! Shared contract for the staged ingest workflow (drop folder → analysis →
//! quality report → metadata candidates → field-level apply → commit).
//!
//! This file is the compile-time seam between the store/coordinator, the
//! quality checks, the provider façade, and GraphQL.  It is deliberately free
//! of database types: every value here is derived from a staged file and is
//! safe to hand to a plugin.  See `docs/content/docs/developer/modular-ingest.mdx`
//! for the design this implements.

use std::{collections::BTreeMap, path::PathBuf};

use async_trait::async_trait;
use models::shared::analysis::MediaAnalysisData;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use stump_media::media::ProcessedMediaMetadata;

/// Algorithm version stamped on every quality report produced by the
/// built-in checks.  Bump when a check definition, weight, or threshold
/// changes so old scores are never silently reinterpreted.
pub const QUALITY_ALGORITHM_VERSION: &str = "ingest-quality-1";

/// Media container kinds the ingest layer understands.  Mirrors the
/// processor selection in `filesystem::media::process` without exposing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IngestMediaKind {
	/// ZIP-backed comic (`.cbz`, `.zip`).
	ComicArchive,
	/// RAR-backed comic (`.cbr`, `.rar`).
	ComicRarArchive,
	Epub,
	Pdf,
	Unknown,
}

impl IngestMediaKind {
	/// Whether the format is page/image oriented (comics, PDF) rather than
	/// reflowable text (EPUB).
	pub fn is_paged(self) -> bool {
		matches!(self, Self::ComicArchive | Self::ComicRarArchive | Self::Pdf)
	}
}

/// One entry of a paged/archive container in the format adapter's canonical
/// reading order (ZIP natural path order, EPUB spine order, PDF page order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestPageEntry {
	/// Zero-based canonical index.
	pub index: u32,
	/// Archive path / spine href / synthetic `page-N` for PDFs.
	pub path: String,
	/// Uncompressed size when known.
	pub size: Option<u64>,
	/// Whether the processor classifies the entry as an image page.
	pub is_image: bool,
}

/// Read-only view of one staged file.  Plugins receive this and nothing else;
/// in particular they never get a database handle or a mutable library path.
#[derive(Debug, Clone)]
pub struct BookSnapshot {
	/// Opaque drop-item id (never a filesystem path).
	pub drop_item_id: String,
	pub library_id: String,
	/// Absolute path of the immutable staged copy.  Plugins may open it
	/// read-only; they must not rename, rewrite, or delete it.
	pub staged_path: PathBuf,
	/// Lower-case hex SHA-256 of the full staged bytes.
	pub source_sha256: String,
	pub byte_size: u64,
	/// Original file name as supplied by the client/watcher (no directories).
	pub source_filename: String,
	/// Normalized relative path inside the drop folder, `/`-separated,
	/// never containing `..`; empty when the item was uploaded flat.
	pub relative_path: String,
	pub media_kind: IngestMediaKind,
	/// Embedded metadata as parsed by the existing `FileProcessor`.
	pub embedded_metadata: Option<ProcessedMediaMetadata>,
	/// Page/archive index in canonical order; empty when not applicable.
	pub pages: Vec<IngestPageEntry>,
	/// Page dimensions/content types when an analysis has already run.
	pub analysis: Option<MediaAnalysisData>,
}

/// Status of one quality finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QualityStatus {
	Pass,
	Warn,
	Fail,
	NotApplicable,
}

/// The single scored outcome of one check.  `normalized_score` is `0..=1`
/// and must be `1.0` for `Pass`, `0.0` for `Fail`, the check's documented
/// fractional value for `Warn`, and ignored for `NotApplicable`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityCheckOutcome {
	pub check_id: String,
	pub label: String,
	pub status: QualityStatus,
	pub normalized_score: f64,
	/// Bounded structured evidence (counts, digests, dimensions, parse
	/// tuples).  Never network results, timestamps, or free-form prose.
	pub evidence: Value,
}

/// Errors a check may raise for its own failure (as opposed to a `Fail`
/// finding about the book).  A raised error marks the report attempt failed.
#[derive(Debug, thiserror::Error)]
pub enum QualityCheckError {
	#[error("quality check {check_id} failed: {message}")]
	Internal { check_id: String, message: String },
	#[error(transparent)]
	Io(#[from] std::io::Error),
}

/// A deterministic quality check.  Implementations must be pure functions of
/// the snapshot plus their persisted settings: no network, no clock, no
/// randomness, no provider output.
#[async_trait]
pub trait QualityCheck: Send + Sync {
	/// Stable id, e.g. `cover_present`.
	fn id(&self) -> &'static str;
	fn name(&self) -> &'static str;
	fn version(&self) -> &'static str;
	/// Fixed weight; the built-in weights sum to 100.
	fn weight(&self) -> u16;
	fn settings(&self) -> &[SettingDefinition];
	async fn run(
		&self,
		book: &BookSnapshot,
		settings: &SettingValues,
	) -> Result<QualityCheckOutcome, QualityCheckError>;
}

/// A finished report for one snapshot: ordered outcomes and the integer score
/// computed by [`score_report`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityReport {
	pub source_sha256: String,
	pub algorithm_version: String,
	/// `0..=100`.
	pub score: u8,
	/// In registry order; each carries its registered weight and the
	/// contribution `100 * w * q / Σ_applicable w`.
	pub checks: Vec<QualityReportCheck>,
	/// Effective setting snapshot per check id at run time.
	pub settings_snapshot: BTreeMap<String, SettingValues>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityReportCheck {
	#[serde(flatten)]
	pub outcome: QualityCheckOutcome,
	pub weight: u16,
	pub contribution: f64,
}

/// Deterministic scorer: `NOT_APPLICABLE` outcomes are excluded from both
/// sums; a report with no applicable checks scores `0`.
pub fn score_report(
	outcomes: &[(QualityCheckOutcome, u16)],
) -> (u8, Vec<QualityReportCheck>) {
	let applicable_weight: f64 = outcomes
		.iter()
		.filter(|(outcome, _)| outcome.status != QualityStatus::NotApplicable)
		.map(|(_, weight)| f64::from(*weight))
		.sum();
	let mut total = 0.0;
	let checks = outcomes
		.iter()
		.map(|(outcome, weight)| {
			let contribution = if outcome.status == QualityStatus::NotApplicable
				|| applicable_weight == 0.0
			{
				0.0
			} else {
				100.0 * f64::from(*weight) * outcome.normalized_score / applicable_weight
			};
			total += contribution;
			QualityReportCheck {
				outcome: outcome.clone(),
				weight: *weight,
				contribution,
			}
		})
		.collect();
	(total.round().clamp(0.0, 100.0) as u8, checks)
}

/// Setting schema entry exposed to the UI for a provider or check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingDefinition {
	pub key: &'static str,
	pub label: &'static str,
	pub description: &'static str,
	pub kind: SettingKind,
	pub default: Value,
	pub required: bool,
	/// Secret values are stored encrypted and never returned to clients.
	pub secret: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SettingKind {
	Bool,
	Int,
	Float,
	String,
	Enum,
	Json,
}

/// Effective setting values keyed by `SettingDefinition::key`.
pub type SettingValues = BTreeMap<String, Value>;

/// Canonical metadata fields a candidate may supply and a field pick may
/// target.  Values are JSON so providers can carry lists (authors, tags).
#[derive(
	Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MetadataField {
	Title,
	SortTitle,
	Series,
	SeriesIndex,
	Authors,
	Publisher,
	PublishedDate,
	Language,
	Summary,
	Tags,
	Genres,
	Isbn,
	Identifiers,
	AgeRating,
	PageCount,
	CoverUrl,
}

/// Stable identity of a provider's notion of "this book" (an external id
/// plus the evidence that produced it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderIdentity {
	pub provider_id: String,
	pub external_id: String,
	pub display: String,
	/// `0..=1`.
	pub confidence: f64,
	/// Provider-neutral explanation, e.g. `{"title_similarity":0.93}`.
	pub factors: Value,
}

/// One provider's proposal for a snapshot.  Evidence only: never applied
/// until a user (or an explicitly enabled auto-apply policy) picks fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetadataCandidate {
	pub provider_id: String,
	pub provider_version: String,
	pub external_id: Option<String>,
	pub source_sha256: String,
	/// `0..=1`.
	pub confidence: f64,
	pub fields: BTreeMap<MetadataField, Value>,
	pub field_confidence: BTreeMap<MetadataField, f64>,
	/// Model/prompt/version or search query that produced the candidate.
	pub provenance: Value,
}

/// Capabilities a provider descriptor advertises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProviderCapability {
	Identify,
	Lookup,
	Tags,
	AiEnrichment,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
	#[error("provider {provider_id} is not configured: {message}")]
	NotConfigured {
		provider_id: String,
		message: String,
	},
	#[error("provider {provider_id} request failed: {message}")]
	Request {
		provider_id: String,
		message: String,
	},
	#[error("provider {provider_id} rate limited")]
	RateLimited { provider_id: String },
}

/// Higher-level façade over `metadata_integrations::MetadataProvider`:
/// `identify` finds candidates for "which external record is this", and
/// `lookup` expands one identity into a field-level candidate.
#[async_trait]
pub trait IngestMetadataProvider: Send + Sync {
	fn id(&self) -> &'static str;
	fn name(&self) -> &'static str;
	fn version(&self) -> &'static str;
	fn supported_media_kinds(&self) -> &[IngestMediaKind];
	fn capabilities(&self) -> &[ProviderCapability];
	fn settings(&self) -> &[SettingDefinition];
	async fn identify(
		&self,
		book: &BookSnapshot,
		settings: &SettingValues,
	) -> Result<Vec<ProviderIdentity>, ProviderError>;
	async fn lookup(
		&self,
		book: &BookSnapshot,
		identity: &ProviderIdentity,
		settings: &SettingValues,
	) -> Result<Vec<MetadataCandidate>, ProviderError>;
}

/// How one field is resolved by an apply/merge request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FieldPick {
	KeepExisting {
		field: MetadataField,
	},
	Candidate {
		field: MetadataField,
		candidate_id: String,
	},
	Manual {
		field: MetadataField,
		value: Value,
	},
	Clear {
		field: MetadataField,
	},
}

impl FieldPick {
	pub fn field(&self) -> MetadataField {
		match self {
			Self::KeepExisting { field }
			| Self::Candidate { field, .. }
			| Self::Manual { field, .. }
			| Self::Clear { field } => *field,
		}
	}
}

/// Lifecycle of a drop item.  Terminal states for an attempt are
/// `Rejected` and `Failed`; a retry starts a new analysis attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DropItemStatus {
	Received,
	Staged,
	Analyzing,
	AwaitingReview,
	Ready,
	Committed,
	Rejected,
	Failed,
}

impl DropItemStatus {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Received => "RECEIVED",
			Self::Staged => "STAGED",
			Self::Analyzing => "ANALYZING",
			Self::AwaitingReview => "AWAITING_REVIEW",
			Self::Ready => "READY",
			Self::Committed => "COMMITTED",
			Self::Rejected => "REJECTED",
			Self::Failed => "FAILED",
		}
	}

	pub fn parse(value: &str) -> Option<Self> {
		Some(match value {
			"RECEIVED" => Self::Received,
			"STAGED" => Self::Staged,
			"ANALYZING" => Self::Analyzing,
			"AWAITING_REVIEW" => Self::AwaitingReview,
			"READY" => Self::Ready,
			"COMMITTED" => Self::Committed,
			"REJECTED" => Self::Rejected,
			"FAILED" => Self::Failed,
			_ => return None,
		})
	}
}

/// Phase of an analysis job, in execution order.
#[derive(
	Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnalysisPhase {
	Staging,
	Parsing,
	Pages,
	Quality,
	Identify,
	Lookup,
	Done,
}

/// One typed progress event.  Both the GraphQL subscription and the SSE
/// endpoint serialize exactly this shape; the client shares one reducer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestProgressEvent {
	/// Monotonic opaque cursor (zero-padded decimal sequence).
	pub cursor: String,
	pub library_id: String,
	pub drop_item_id: String,
	pub analysis_job_id: Option<String>,
	pub phase: AnalysisPhase,
	pub status: DropItemStatus,
	pub completed: u32,
	pub total: u32,
	pub score: Option<u8>,
	pub message: String,
}

#[cfg(test)]
mod tests {
	use super::*;

	fn outcome(id: &str, status: QualityStatus, q: f64) -> QualityCheckOutcome {
		QualityCheckOutcome {
			check_id: id.to_owned(),
			label: id.to_owned(),
			status,
			normalized_score: q,
			evidence: Value::Null,
		}
	}

	#[test]
	fn score_excludes_not_applicable_and_rounds() {
		let (score, checks) = score_report(&[
			(outcome("a", QualityStatus::Pass, 1.0), 20),
			(outcome("b", QualityStatus::Warn, 0.5), 10),
			(outcome("c", QualityStatus::NotApplicable, 0.0), 70),
		]);
		// applicable weight 30: (20*1 + 10*0.5)/30 = 0.8333 -> 83
		assert_eq!(score, 83);
		assert_eq!(checks[2].contribution, 0.0);
		assert!((checks[0].contribution + checks[1].contribution - 83.333).abs() < 0.01);
	}

	#[test]
	fn no_applicable_checks_scores_zero() {
		let (score, _) =
			score_report(&[(outcome("a", QualityStatus::NotApplicable, 0.0), 100)]);
		assert_eq!(score, 0);
	}

	#[test]
	fn drop_item_status_round_trips() {
		for status in [
			DropItemStatus::Received,
			DropItemStatus::AwaitingReview,
			DropItemStatus::Failed,
		] {
			assert_eq!(DropItemStatus::parse(status.as_str()), Some(status));
		}
		assert_eq!(DropItemStatus::parse("nope"), None);
	}
}
