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
use metadata_integrations::MetadataField as PublicMetadataField;
use models::shared::analysis::MediaAnalysisData;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stump_api_types::settings::{SettingDefinition, SettingValues};

use stump_media::{audio::ProbedAudio, media::ProcessedMediaMetadata};

/// Algorithm version stamped on every quality report produced by the
/// built-in checks.  Bump when a check definition, weight, or threshold
/// changes so old scores are never silently reinterpreted.
pub const QUALITY_ALGORITHM_VERSION: &str = "ingest-quality-2";

/// Media container kinds the ingest layer understands.  Mirrors the
/// processor selection in `filesystem::media::process` without exposing it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IngestMediaKind {
	/// ZIP-backed comic (`.cbz`, `.zip`).
	ComicArchive,
	/// RAR-backed comic (`.cbr`, `.rar`).
	ComicRarArchive,
	Epub,
	Pdf,
	/// An audiobook: one MP4/MP3/Opus/FLAC container, or one folder of parts.
	/// The only kind whose target can be a directory, because a folder
	/// audiobook is one publication rather than one file per book.
	Audio,
	/// The kind a file the processor selection does not recognise gets, and
	/// the default: a MOBI/AZW3 is read by the processor but has neither a
	/// page lane nor a time lane.
	#[default]
	Unknown,
}

impl IngestMediaKind {
	/// Whether the format is page/image oriented (comics, PDF) rather than
	/// reflowable text (EPUB) or time-addressed (audio).
	pub fn is_paged(self) -> bool {
		matches!(self, Self::ComicArchive | Self::ComicRarArchive | Self::Pdf)
	}

	/// Whether the publication is addressed in time rather than in pages or
	/// resources. Every page-oriented check reports `NOT_APPLICABLE` for one
	/// of these, and every audio check reports `NOT_APPLICABLE` for the rest.
	pub fn is_audio(self) -> bool {
		matches!(self, Self::Audio)
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
	/// Opaque ingest target id (never a filesystem path): the drop item id
	/// for staged runs, the media id for library-wide rework runs.
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

/// What the analysis phase learned about an audio publication.
///
/// The probe is the expensive part of an audiobook (symphonia walks every
/// container, and a folder book means one walk per part), so its result is
/// persisted on the drop item rather than recomputed per screen. Every time
/// value is milliseconds from the start of the *publication*, which is the
/// unit `reading_heads.position_ms` is already in.
///
/// Cover *bytes* are deliberately absent: a 2 MB JPEG on every drop-item row
/// would make listing a drop folder a multi-megabyte query. The content type
/// and size are what a client needs to decide whether to fetch it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioAnalysis {
	pub duration_ms: i64,
	/// The publication codec, or `mixed` when a folder book's parts disagree.
	pub codec: String,
	pub sample_rate: Option<i32>,
	pub channels: Option<i32>,
	pub bitrate: Option<i32>,
	/// Where the chapter marks came from, as
	/// [`stump_media::audio::ChapterSource`] spells it. Provenance, never a
	/// quality tier: `per_track` means Stump synthesized the list from file
	/// boundaries, which is exactly what a librarian needs to know before
	/// trusting it.
	pub chapter_source: String,
	pub tracks: Vec<AudioAnalysisTrack>,
	pub chapters: Vec<AudioAnalysisChapter>,
	pub cover_content_type: Option<String>,
	pub cover_byte_size: Option<u64>,
	pub title: Option<String>,
	pub author: Option<String>,
	pub narrator: Option<String>,
	pub album: Option<String>,
	pub description: Option<String>,
	pub genre: Option<String>,
	pub year: Option<i32>,
	/// Set once `audio-assemble` produced this item's current file.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub assembled: Option<AssembledAudio>,
}

/// One file of a probed audio publication, in playback order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioAnalysisTrack {
	/// File name of the part. A path is never exposed: the staging layout is
	/// server-owned and a client has no use for it.
	pub filename: String,
	pub duration_ms: i64,
	/// Offset of this part's first sample within the publication.
	pub start_offset_ms: i64,
	pub byte_size: i64,
	pub codec: String,
	pub bitrate: Option<i32>,
	pub title: Option<String>,
	pub track_number: Option<u32>,
}

/// One chapter mark, relative to the publication.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioAnalysisChapter {
	pub title: Option<String>,
	pub start_ms: i64,
	pub end_ms: Option<i64>,
}

/// The M4B an `audio-assemble` run produced for a drop item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssembledAudio {
	/// File name of the assembled book, which is now the item's file.
	pub filename: String,
	pub byte_size: u64,
	pub duration_ms: i64,
	pub chapters: usize,
	/// Whether the output puts `moov` ahead of `mdat`, so playback starts
	/// without downloading the whole file.
	pub faststart: bool,
	/// Whether the source parts were kept beside the output as sidecars
	/// (`AudioPolicy::keep_original`).
	pub parts_kept: bool,
	/// `assemble-remux` (lossless) or `assemble-transcode` (re-encoded).
	pub method: String,
}

impl From<&ProbedAudio> for AudioAnalysis {
	fn from(probed: &ProbedAudio) -> Self {
		// `ProbedTrack` deliberately carries no start offset: it is the
		// running sum of the preceding durations and is assigned exactly once,
		// here, so no two callers can disagree about where a part begins.
		let mut start_offset_ms = 0_i64;
		let mut tracks = Vec::with_capacity(probed.tracks.len());
		for track in &probed.tracks {
			tracks.push(AudioAnalysisTrack {
				filename: track
					.path
					.file_name()
					.map(|name| name.to_string_lossy().into_owned())
					.unwrap_or_default(),
				duration_ms: track.duration_ms,
				start_offset_ms,
				byte_size: track.byte_size,
				codec: track.codec.clone(),
				bitrate: track.bitrate,
				title: track.title.clone(),
				track_number: track.track_number,
			});
			start_offset_ms = start_offset_ms.saturating_add(track.duration_ms);
		}
		Self {
			duration_ms: probed.duration_ms,
			codec: probed.codec.clone(),
			sample_rate: probed.sample_rate,
			channels: probed.channels,
			bitrate: probed.bitrate,
			chapter_source: probed.chapter_source.to_string(),
			tracks,
			chapters: probed
				.chapters
				.iter()
				.map(|chapter| AudioAnalysisChapter {
					title: chapter.title.clone(),
					start_ms: chapter.start_ms,
					end_ms: chapter.end_ms,
				})
				.collect(),
			cover_content_type: probed
				.cover
				.as_ref()
				.map(|(content_type, _)| content_type.to_string()),
			cover_byte_size: probed.cover.as_ref().map(|(_, bytes)| bytes.len() as u64),
			title: probed.title.clone(),
			author: probed.author.clone(),
			narrator: probed.narrator.clone(),
			album: probed.album.clone(),
			description: probed.description.clone(),
			genre: probed.genre.clone(),
			year: probed.year,
			assembled: None,
		}
	}
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

/// The tool that repairs what a check found.
///
/// A finding a librarian cannot act on is a complaint, not a check. Every
/// audio check names the tool that fixes it, so a failing report is one step
/// away from a plan: the ids are [`crate::quality`]'s side of the
/// `stump_tools` registry (`audio-assemble`, `audio-chapters`, `meta-edit`),
/// and `options` is the tool option blob that addresses *this* finding.
///
/// Deliberately not a command line: the tool is invoked through the tool
/// registry with these options, so a fix cannot drift from what the tool
/// actually accepts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixAction {
	/// A `stump_tools` tool id.
	pub tool: String,
	/// Human-readable statement of what running it would do.
	pub summary: String,
	/// The tool's JSON options, or `null` for its defaults.
	#[serde(default, skip_serializing_if = "Value::is_null")]
	pub options: Value,
}

impl FixAction {
	pub fn new(tool: &str, summary: &str) -> Self {
		Self {
			tool: tool.to_string(),
			summary: summary.to_string(),
			options: Value::Null,
		}
	}

	#[must_use]
	pub fn with_options(mut self, options: Value) -> Self {
		self.options = options;
		self
	}
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
	/// The tool that repairs a failing outcome of this check, when one exists.
	///
	/// `None` means "no tool in this build can fix it" — which is the honest
	/// answer for a duplicate-detection or filename-parsing finding, where the
	/// decision is the librarian's.
	fn fix(&self) -> Option<FixAction> {
		None
	}
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
	/// The audiobook's readers. A separate credit from `Authors`: an
	/// audiobook's author wrote it and its narrator did not, and folding the
	/// two would put a reader in the writers column.
	Narrators,
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
	/// Publication status of the series a book belongs to, spelled exactly as
	/// the legacy apply path writes `series_metadata.status` (`Ongoing`,
	/// `Completed`, `Hiatus`, `Cancelled`, `Upcoming`).  Candidate-only: the
	/// staged apply path writes book rows, so a series status is evidence for
	/// the policy and the editor until series apply exists.
	Status,
}

impl MetadataField {
	/// The one mapping from the public metadata vocabulary
	/// ([`PublicMetadataField`], which the legacy fetch path, the per-row lock
	/// lists, and every GraphQL input speak) onto the smaller,
	/// provider-neutral ingest field set.
	///
	/// `None` means staged ingest has no equivalent field, and a caller must
	/// reject rather than write the wrong column: several public fields
	/// deliberately fold onto one ingest field (every credit role becomes
	/// `Authors`), so a silent fallback would put a colorist in the writers
	/// column.
	pub fn from_public(field: PublicMetadataField) -> Option<Self> {
		Some(match field {
			PublicMetadataField::Title => Self::Title,
			PublicMetadataField::TitleSort => Self::SortTitle,
			PublicMetadataField::Summary => Self::Summary,
			PublicMetadataField::Series => Self::Series,
			PublicMetadataField::Number => Self::SeriesIndex,
			PublicMetadataField::Artists
			| PublicMetadataField::Writers
			| PublicMetadataField::Editors
			| PublicMetadataField::Inkers
			| PublicMetadataField::Letterers
			| PublicMetadataField::Colorists
			| PublicMetadataField::CoverArtists
			| PublicMetadataField::Pencillers
			| PublicMetadataField::Teams => Self::Authors,
			PublicMetadataField::Narrators => Self::Narrators,
			PublicMetadataField::Publisher | PublicMetadataField::Imprint => {
				Self::Publisher
			},
			PublicMetadataField::Year | PublicMetadataField::ReleaseDate => {
				Self::PublishedDate
			},
			PublicMetadataField::Language => Self::Language,
			PublicMetadataField::Tags => Self::Tags,
			PublicMetadataField::Genres => Self::Genres,
			PublicMetadataField::Isbn => Self::Isbn,
			PublicMetadataField::AgeRating => Self::AgeRating,
			PublicMetadataField::PageCount => Self::PageCount,
			PublicMetadataField::Cover => Self::CoverUrl,
			PublicMetadataField::Status => Self::Status,
			PublicMetadataField::Links
			| PublicMetadataField::ComicId
			| PublicMetadataField::IdentifierAmazon
			| PublicMetadataField::IdentifierCalibre
			| PublicMetadataField::IdentifierGoogle
			| PublicMetadataField::IdentifierMobiAsin
			| PublicMetadataField::IdentifierUuid => Self::Identifiers,
			_ => return None,
		})
	}

	/// The public field an ingest field is named by, for clients that speak
	/// only the public vocabulary.  It is the inverse of [`Self::from_public`]
	/// on the canonical spelling of each fold: `Authors` names `WRITERS`, and
	/// `Identifiers` names `LINKS`.
	pub fn to_public(self) -> PublicMetadataField {
		match self {
			Self::Title => PublicMetadataField::Title,
			Self::SortTitle => PublicMetadataField::TitleSort,
			Self::Series => PublicMetadataField::Series,
			Self::SeriesIndex => PublicMetadataField::Number,
			Self::Authors => PublicMetadataField::Writers,
			Self::Narrators => PublicMetadataField::Narrators,
			Self::Publisher => PublicMetadataField::Publisher,
			Self::PublishedDate => PublicMetadataField::ReleaseDate,
			Self::Language => PublicMetadataField::Language,
			Self::Summary => PublicMetadataField::Summary,
			Self::Tags => PublicMetadataField::Tags,
			Self::Genres => PublicMetadataField::Genres,
			Self::Isbn => PublicMetadataField::Isbn,
			Self::Identifiers => PublicMetadataField::Links,
			Self::AgeRating => PublicMetadataField::AgeRating,
			Self::PageCount => PublicMetadataField::PageCount,
			Self::CoverUrl => PublicMetadataField::Cover,
			Self::Status => PublicMetadataField::Status,
		}
	}

	/// Every ingest field, in declaration order.  The policy editor renders
	/// one row per field, so the list is part of the contract rather than
	/// something each client re-types.
	pub const ALL: &'static [Self] = &[
		Self::Title,
		Self::SortTitle,
		Self::Series,
		Self::SeriesIndex,
		Self::Authors,
		Self::Publisher,
		Self::PublishedDate,
		Self::Language,
		Self::Summary,
		Self::Tags,
		Self::Genres,
		Self::Isbn,
		Self::Identifiers,
		Self::AgeRating,
		Self::PageCount,
		Self::CoverUrl,
		Self::Status,
	];
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

/// A free-text catalog search request for the editor.  `text` is the raw user
/// query; `media_kind` optionally restricts which providers participate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchQuery {
	pub text: String,
	pub media_kind: Option<IngestMediaKind>,
	pub limit: u8,
}

/// One media-level hit from a provider catalog search (an issue, volume, or
/// book -- never a series).  `score` is `0.0..=1.0` from the provider's own
/// scorer, falling back to title similarity when the provider does not score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
	pub provider_id: String,
	pub external_id: String,
	pub title: String,
	pub year: Option<i32>,
	pub cover_url: Option<String>,
	pub summary: Option<String>,
	/// `0.0..=1.0`.
	pub score: f32,
}

/// Capabilities a provider descriptor advertises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProviderCapability {
	Identify,
	Lookup,
	Search,
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
	#[error("provider {provider_id} does not support this operation")]
	Unsupported { provider_id: String },
}

/// Higher-level façade over `metadata_integrations::MetadataProvider`:
/// `identify` finds candidates for "which external record is this", `lookup`
/// expands one identity into a field-level candidate, and `search` performs a
/// free-text catalog search (editor-driven, media-level hits only).
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

	/// Free-text search across the provider's catalog.  Default: unsupported.
	async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>, ProviderError> {
		let _ = query;
		Err(ProviderError::Unsupported {
			provider_id: self.id().to_string(),
		})
	}
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DropItemStatus {
	Received,
	/// The immutable staged copy exists and analysis has not run. Where every
	/// admitted item starts, and therefore the default.
	#[default]
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

	#[test]
	fn every_ingest_field_round_trips_through_the_public_vocabulary() {
		for field in MetadataField::ALL.iter().copied() {
			assert_eq!(
				MetadataField::from_public(field.to_public()),
				Some(field),
				"{field:?} does not survive to_public/from_public"
			);
		}
		// A public field with no ingest column must be refused, not folded.
		assert_eq!(
			MetadataField::from_public(PublicMetadataField::Format),
			None
		);
		assert_eq!(
			MetadataField::from_public(PublicMetadataField::StoryArc),
			None
		);
	}
}
