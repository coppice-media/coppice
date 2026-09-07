use std::str::FromStr;

use async_graphql::{Context, Error, Json, Object, Result, SimpleObject, ID};
use models::{
	entity::{
		ingest_analysis_job, ingest_drop_item, ingest_metadata_candidate,
		ingest_plugin_setting, ingest_quality_report, media, series,
	},
	shared::enums::JobStatus,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{json, Value};
use stump_api_types::settings::{SettingDefinition, SettingKind};
use stump_ingest::contract::{
	AnalysisPhase, DropItemStatus, IngestProgressEvent as CoreIngestProgressEvent,
	QualityReportCheck, QualityStatus,
};

use crate::{
	data::CoreContext,
	object::{media::Media, series::Series},
	pagination::PaginationInfo,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq, async_graphql::Enum)]
pub enum IngestDropItemStatus {
	Received,
	Staged,
	Analyzing,
	AwaitingReview,
	Ready,
	Committed,
	Rejected,
	Failed,
}

impl From<DropItemStatus> for IngestDropItemStatus {
	fn from(status: DropItemStatus) -> Self {
		match status {
			DropItemStatus::Received => Self::Received,
			DropItemStatus::Staged => Self::Staged,
			DropItemStatus::Analyzing => Self::Analyzing,
			DropItemStatus::AwaitingReview => Self::AwaitingReview,
			DropItemStatus::Ready => Self::Ready,
			DropItemStatus::Committed => Self::Committed,
			DropItemStatus::Rejected => Self::Rejected,
			DropItemStatus::Failed => Self::Failed,
		}
	}
}

impl FromStr for IngestDropItemStatus {
	type Err = Error;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		DropItemStatus::parse(value)
			.map(Into::into)
			.ok_or_else(|| Error::new(format!("Unknown ingest item status: {value}")))
	}
}

impl From<IngestDropItemStatus> for DropItemStatus {
	fn from(status: IngestDropItemStatus) -> Self {
		match status {
			IngestDropItemStatus::Received => Self::Received,
			IngestDropItemStatus::Staged => Self::Staged,
			IngestDropItemStatus::Analyzing => Self::Analyzing,
			IngestDropItemStatus::AwaitingReview => Self::AwaitingReview,
			IngestDropItemStatus::Ready => Self::Ready,
			IngestDropItemStatus::Committed => Self::Committed,
			IngestDropItemStatus::Rejected => Self::Rejected,
			IngestDropItemStatus::Failed => Self::Failed,
		}
	}
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, async_graphql::Enum)]
pub enum IngestAnalysisPhase {
	Staging,
	Analysis,
	Quality,
	Identify,
	Lookup,
	Review,
	Commit,
	Done,
}

impl From<AnalysisPhase> for IngestAnalysisPhase {
	fn from(phase: AnalysisPhase) -> Self {
		match phase {
			AnalysisPhase::Staging => Self::Staging,
			AnalysisPhase::Parsing | AnalysisPhase::Pages => Self::Analysis,
			AnalysisPhase::Quality => Self::Quality,
			AnalysisPhase::Identify => Self::Identify,
			AnalysisPhase::Lookup => Self::Lookup,
			AnalysisPhase::Done => Self::Done,
		}
	}
}

impl FromStr for IngestAnalysisPhase {
	type Err = Error;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		match value {
			"STAGING" => Ok(Self::Staging),
			"PARSING" | "PAGES" | "ANALYSIS" => Ok(Self::Analysis),
			"QUALITY" => Ok(Self::Quality),
			"IDENTIFY" => Ok(Self::Identify),
			"LOOKUP" => Ok(Self::Lookup),
			"REVIEW" => Ok(Self::Review),
			"COMMIT" => Ok(Self::Commit),
			"DONE" => Ok(Self::Done),
			_ => Err(Error::new(format!(
				"Unknown ingest analysis phase: {value}"
			))),
		}
	}
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, async_graphql::Enum)]
pub enum IngestQualityStatus {
	Pass,
	Warn,
	Fail,
	NotApplicable,
}

impl From<QualityStatus> for IngestQualityStatus {
	fn from(status: QualityStatus) -> Self {
		match status {
			QualityStatus::Pass => Self::Pass,
			QualityStatus::Warn => Self::Warn,
			QualityStatus::Fail => Self::Fail,
			QualityStatus::NotApplicable => Self::NotApplicable,
		}
	}
}

#[derive(Clone, Debug)]
pub struct IngestDropItem {
	pub model: ingest_drop_item::Model,
}

impl From<ingest_drop_item::Model> for IngestDropItem {
	fn from(model: ingest_drop_item::Model) -> Self {
		Self { model }
	}
}

#[Object]
impl IngestDropItem {
	async fn id(&self) -> ID {
		self.model.id.clone().into()
	}

	async fn library_id(&self) -> ID {
		self.model.library_id.clone().into()
	}

	async fn created_by(&self) -> ID {
		self.model.created_by.clone().unwrap_or_default().into()
	}

	async fn filename(&self) -> &str {
		&self.model.source_filename
	}

	async fn relative_path(&self) -> Option<&str> {
		self.model.relative_path.as_deref()
	}

	async fn size_bytes(&self) -> i32 {
		self.model.byte_size.clamp(0, i32::MAX as i64) as i32
	}

	async fn source_sha256(&self) -> &str {
		&self.model.source_sha256
	}

	async fn media_type(&self) -> &str {
		&self.model.media_kind
	}

	async fn status(&self) -> Result<IngestDropItemStatus> {
		self.model.status.parse()
	}

	async fn revision(&self) -> i32 {
		self.model.revision
	}

	async fn analysis_job(&self, ctx: &Context<'_>) -> Result<Option<IngestAnalysisJob>> {
		let Some(id) = &self.model.analysis_job_id else {
			return Ok(None);
		};
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		Ok(ingest_analysis_job::Entity::find_by_id(id)
			.one(conn)
			.await?
			.map(IngestAnalysisJob::from))
	}

	async fn quality_report(
		&self,
		ctx: &Context<'_>,
	) -> Result<Option<IngestQualityReport>> {
		let Some(id) = &self.model.quality_report_id else {
			return Ok(None);
		};
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		Ok(ingest_quality_report::Entity::find_by_id(id)
			.one(conn)
			.await?
			.map(IngestQualityReport::from))
	}

	async fn metadata_candidates(
		&self,
		ctx: &Context<'_>,
	) -> Result<Vec<IngestMetadataCandidate>> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let models = ingest_metadata_candidate::Entity::find()
			.filter(
				ingest_metadata_candidate::Column::DropItemId.eq(self.model.id.clone()),
			)
			.all(conn)
			.await?;
		Ok(models
			.into_iter()
			.map(IngestMetadataCandidate::from)
			.collect())
	}

	async fn pending_fields(&self) -> Json<Value> {
		Json(
			self.model
				.pending_fields
				.clone()
				.unwrap_or_else(|| json!({})),
		)
	}

	async fn media(&self, ctx: &Context<'_>) -> Result<Option<Media>> {
		let Some(id) = &self.model.media_id else {
			return Ok(None);
		};
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let model = media::ModelWithMetadata::find_for_user(user)
			.filter(media::Column::Id.eq(id.clone()))
			.into_model::<media::ModelWithMetadata>()
			.one(conn)
			.await?;
		Ok(model.map(Media::from))
	}

	async fn series(&self, ctx: &Context<'_>) -> Result<Option<Series>> {
		let Some(id) = &self.model.series_id else {
			return Ok(None);
		};
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let model = series::ModelWithMetadata::find_for_user(user)
			.filter(series::Column::Id.eq(id.clone()))
			.into_model::<series::ModelWithMetadata>()
			.one(conn)
			.await?;
		Ok(model.map(Series::from))
	}

	async fn error(&self) -> Option<&str> {
		self.model.error.as_deref()
	}

	/// The delivery this item arrived in, when one dropped archive produced
	/// several items. `null` for a drop of one file.
	async fn drop_group_id(&self) -> Option<ID> {
		self.model.drop_group_id.clone().map(ID::from)
	}

	/// The other items of the same delivery, oldest first.
	///
	/// Empty for an item that arrived on its own, which is most of them, so a
	/// client may render the strip unconditionally.
	async fn drop_group_siblings(
		&self,
		ctx: &Context<'_>,
	) -> Result<Vec<IngestDropGroupSibling>> {
		let Some(group) = self.model.drop_group_id.as_deref() else {
			return Ok(Vec::new());
		};
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();
		// A pair belongs to a user: the link rows are per-user, so the state
		// shown is the viewer's own, never somebody else's decision.
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let siblings = ingest_drop_item::Entity::find()
			.filter(ingest_drop_item::Column::DropGroupId.eq(group))
			.filter(ingest_drop_item::Column::Id.ne(self.model.id.clone()))
			.order_by_asc(ingest_drop_item::Column::CreatedAt)
			.order_by_asc(ingest_drop_item::Column::Id)
			.all(conn)
			.await?;
		let own_kind = ingest_media_kind(&self.model.media_kind);
		let mut out = Vec::with_capacity(siblings.len());
		for sibling in siblings {
			let candidate = stump_ingest::pairing::is_edition_pair(
				own_kind,
				ingest_media_kind(&sibling.media_kind),
			);
			let state = if !candidate {
				IngestEditionPairState::NotAPair
			} else {
				match (self.model.media_id.as_deref(), sibling.media_id.as_deref()) {
					(Some(left), Some(right)) => {
						pair_state(conn, &user.id, left, right).await?
					},
					// A suggestion needs two media rows; until both sides
					// commit there is nothing to suggest yet.
					_ => IngestEditionPairState::PendingCommit,
				}
			};
			let quality_score = match sibling.quality_report_id.as_deref() {
				Some(id) => ingest_quality_report::Entity::find_by_id(id)
					.one(conn)
					.await?
					.map(|report| report.score),
				None => None,
			};
			out.push(IngestDropGroupSibling {
				id: sibling.id.into(),
				filename: sibling.source_filename,
				media_type: sibling.media_kind,
				status: sibling.status.parse()?,
				quality_score,
				media_id: sibling.media_id.map(ID::from),
				edition_pair_candidate: candidate,
				pair_state: state,
			});
		}
		Ok(out)
	}

	/// Staged files this item owns without being them: cover art, notes, and
	/// the source parts kept beside an assembled M4B.
	async fn sidecars(&self) -> Vec<String> {
		stump_ingest::store::IngestStore::sidecars(&self.model)
			.into_iter()
			.filter_map(|path| {
				// File names only: the staging layout is server-owned.
				std::path::Path::new(&path)
					.file_name()
					.map(|name| name.to_string_lossy().into_owned())
			})
			.collect()
	}

	/// The audio probe's result. `null` for every non-audio item and for an
	/// audio item whose analysis has not run yet.
	async fn audio(&self) -> Option<IngestDropItemAudio> {
		stump_ingest::store::IngestStore::audio_analysis(&self.model)
			.map(IngestDropItemAudio::from)
	}

	async fn created_at(&self) -> sea_orm::prelude::DateTimeWithTimeZone {
		self.model.created_at
	}

	async fn updated_at(&self) -> sea_orm::prelude::DateTimeWithTimeZone {
		self.model.updated_at
	}
}

#[derive(Clone, Debug)]
pub struct IngestAnalysisJob {
	pub model: ingest_analysis_job::Model,
}

impl From<ingest_analysis_job::Model> for IngestAnalysisJob {
	fn from(model: ingest_analysis_job::Model) -> Self {
		Self { model }
	}
}

#[Object]
impl IngestAnalysisJob {
	async fn id(&self) -> ID {
		self.model.id.clone().into()
	}

	async fn drop_item_id(&self) -> Option<ID> {
		self.model.drop_item_id.clone().map(ID::from)
	}

	/// Media targets of a library rework job; empty for staged drop-item
	/// jobs.
	async fn media_ids(&self) -> Vec<ID> {
		stump_ingest::store::analysis_targets_from_plan(
			&self.model.plan,
			self.model.drop_item_id.as_deref(),
		)
		.into_iter()
		.filter_map(|target| match target {
			stump_ingest::store::AnalysisTarget::Media(id) => Some(ID::from(id)),
			stump_ingest::store::AnalysisTarget::DropItem(_) => None,
		})
		.collect()
	}

	async fn job_id(&self) -> Option<&str> {
		self.model.job_id.as_deref()
	}

	async fn status(&self) -> JobStatus {
		self.model.status
	}

	async fn phase(&self) -> Result<IngestAnalysisPhase> {
		self.model.phase.parse()
	}

	async fn priority_score(&self) -> f64 {
		self.model.priority as f64
	}

	async fn attempt(&self) -> i32 {
		self.model.attempts
	}

	async fn queued_at(&self) -> sea_orm::prelude::DateTimeWithTimeZone {
		self.model.created_at
	}

	async fn started_at(&self) -> Option<sea_orm::prelude::DateTimeWithTimeZone> {
		self.model.started_at
	}

	async fn completed_at(&self) -> Option<sea_orm::prelude::DateTimeWithTimeZone> {
		self.model.finished_at
	}

	async fn error(&self) -> Option<&str> {
		self.model.error.as_deref()
	}
}

#[derive(Clone, Debug)]
pub struct IngestQualityReport {
	pub model: ingest_quality_report::Model,
}

impl From<ingest_quality_report::Model> for IngestQualityReport {
	fn from(model: ingest_quality_report::Model) -> Self {
		Self { model }
	}
}

impl IngestQualityReport {
	pub fn checks_from_json(value: &Value) -> Vec<QualityReportCheck> {
		serde_json::from_value(value.clone()).unwrap_or_default()
	}
}

#[Object]
impl IngestQualityReport {
	async fn id(&self) -> ID {
		self.model.id.clone().into()
	}

	async fn drop_item_id(&self) -> Option<ID> {
		self.model.drop_item_id.clone().map(ID::from)
	}

	/// Set when the report was produced by a library rework run.
	async fn media_id(&self) -> Option<ID> {
		self.model.media_id.clone().map(ID::from)
	}

	async fn source_sha256(&self) -> &str {
		&self.model.source_sha256
	}

	async fn algorithm_version(&self) -> &str {
		&self.model.algorithm_version
	}

	async fn score(&self) -> i32 {
		self.model.score.clamp(0, 100)
	}

	async fn checks(&self) -> Vec<IngestQualityCheckResult> {
		Self::checks_from_json(&self.model.checks)
			.into_iter()
			.map(IngestQualityCheckResult::from)
			.collect()
	}

	async fn generated_at(&self) -> sea_orm::prelude::DateTimeWithTimeZone {
		self.model.created_at
	}
}

#[derive(Clone, Debug)]
pub struct IngestQualityCheckResult {
	pub check: QualityReportCheck,
}

impl From<QualityReportCheck> for IngestQualityCheckResult {
	fn from(check: QualityReportCheck) -> Self {
		Self { check }
	}
}

#[Object]
impl IngestQualityCheckResult {
	async fn check_id(&self) -> &str {
		&self.check.outcome.check_id
	}

	async fn label(&self) -> &str {
		&self.check.outcome.label
	}

	async fn status(&self) -> IngestQualityStatus {
		self.check.outcome.status.into()
	}

	async fn weight(&self) -> i32 {
		self.check.weight as i32
	}

	async fn normalized_score(&self) -> f64 {
		self.check.outcome.normalized_score
	}

	async fn contribution(&self) -> f64 {
		self.check.contribution
	}

	async fn evidence(&self) -> Json<Value> {
		Json(self.check.outcome.evidence.clone())
	}

	/// The tool that repairs this finding, when one exists in this build.
	///
	/// Resolved from the check *registry* rather than stored on the report: a
	/// check declares its repair once, so a persisted report can never offer
	/// a fix naming a tool this build does not have.
	async fn fix(&self, ctx: &Context<'_>) -> Result<Option<IngestQualityFixAction>> {
		let check_id = self.check.outcome.check_id.as_str();
		Ok(ctx
			.data::<CoreContext>()?
			.ingest()
			.quality
			.checks()
			.iter()
			.find(|check| check.id() == check_id)
			.and_then(|check| check.fix())
			.map(IngestQualityFixAction::from))
	}
}

/// The audio probe's result for one staged publication.
///
/// A projection of the persisted `ingest_drop_items.audio_analysis`, never a
/// fresh probe: demuxing a 62-part folder book is 62 container walks, and a
/// screen that renders a chapter list must not cost that.
#[derive(Debug, Clone, SimpleObject)]
pub struct IngestDropItemAudio {
	pub duration_ms: i32,
	/// `h:mm:ss`, the field a librarian actually reads.
	pub duration: String,
	/// The publication codec, or `mixed` when a folder book's parts disagree.
	pub codec: String,
	pub sample_rate: Option<i32>,
	pub channels: Option<i32>,
	pub bitrate: Option<i32>,
	/// Where the chapter marks came from: `mp4_chpl`, `mp4_chapter_track`,
	/// `id3_chap`, `vorbis_comment`, `per_track` (synthesized from file
	/// boundaries), or `none`. Provenance, never a quality tier.
	pub chapter_source: String,
	pub tracks: Vec<IngestAudioTrack>,
	pub chapters: Vec<IngestAudioChapter>,
	/// Set when a part carries embedded artwork. The bytes are not served
	/// here: a cover on every row would make listing a drop folder a
	/// multi-megabyte query.
	pub cover_content_type: Option<String>,
	pub cover_byte_size: Option<i32>,
	pub title: Option<String>,
	pub author: Option<String>,
	pub narrator: Option<String>,
	pub album: Option<String>,
	pub description: Option<String>,
	pub genre: Option<String>,
	pub year: Option<i32>,
	/// The M4B an assemble produced, when one has run for this item.
	pub assembled: Option<IngestAssembledAudio>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct IngestAudioTrack {
	/// File name of the part. The staging layout is server-owned, so no path
	/// is exposed.
	pub filename: String,
	pub duration_ms: i32,
	pub duration: String,
	/// Offset of this part's first sample within the publication, which is
	/// the unit a reading position is expressed in.
	pub start_offset_ms: i32,
	pub byte_size: i32,
	pub codec: String,
	pub bitrate: Option<i32>,
	pub title: Option<String>,
	pub track_number: Option<i32>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct IngestAudioChapter {
	pub title: Option<String>,
	pub start_ms: i32,
	pub start: String,
	pub end_ms: Option<i32>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct IngestAssembledAudio {
	pub filename: String,
	pub byte_size: i32,
	pub duration_ms: i32,
	pub duration: String,
	pub chapters: i32,
	/// Whether `moov` precedes `mdat`, so playback starts without
	/// downloading the whole file.
	pub faststart: bool,
	/// Whether the source parts were kept beside the output.
	pub parts_kept: bool,
	/// `assemble-remux` (lossless) or `assemble-transcode` (re-encoded).
	pub method: String,
}

/// How far along the edition pairing of two items of one drop group is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, async_graphql::Enum)]
pub enum IngestEditionPairState {
	/// The two kinds do not pair: two ebooks are a format duplicate, and two
	/// audiobooks are two books.
	NotAPair,
	/// A pair once both sides are in the library. A suggestion needs two
	/// media rows, and a staged item has none.
	PendingCommit,
	/// The suggestion exists and is waiting for the user.
	Suggested,
	/// The user accepted it, or the liseur lane already asserted the work.
	Confirmed,
	/// The user declined it. Kept, because pairing is recomputed on every
	/// book-page query.
	Rejected,
}

/// The kind a drop item's stored `media_kind` names.
///
/// An unrecognised value reads as `Unknown`, which pairs with audio and with
/// nothing else — the same answer the ingest crate gives.
fn ingest_media_kind(value: &str) -> stump_ingest::contract::IngestMediaKind {
	serde_json::from_value(Value::String(value.to_string()))
		.unwrap_or(stump_ingest::contract::IngestMediaKind::Unknown)
}

/// Where the edition pairing of two committed media rows currently stands,
/// for the user asking.
///
/// Read from the link rows the pairing lane owns, so the strip cannot claim a
/// suggestion that was never written or hide one the user already rejected.
/// A pair is two links naming one work: it is confirmed only when *both*
/// sides are, rejected as soon as either is, and pending until both exist.
async fn pair_state(
	conn: &sea_orm::DatabaseConnection,
	user_id: &str,
	left_media_id: &str,
	right_media_id: &str,
) -> Result<IngestEditionPairState> {
	use models::domain::edition_pair::{self, PairStatus};

	let left = edition_pair::link_for_media(conn, user_id, left_media_id).await?;
	let right = edition_pair::link_for_media(conn, user_id, right_media_id).await?;
	let (Some(left), Some(right)) = (left, right) else {
		return Ok(IngestEditionPairState::PendingCommit);
	};
	if left.work_id != right.work_id {
		// Both belong to works, but not to the same one: nothing has paired
		// them, and re-homing a link is the pairing lane's decision, not a
		// display concern.
		return Ok(IngestEditionPairState::PendingCommit);
	}
	let statuses = [
		PairStatus::from_stored(&left.pair_status),
		PairStatus::from_stored(&right.pair_status),
	];
	Ok(if statuses.contains(&PairStatus::Rejected) {
		IngestEditionPairState::Rejected
	} else if statuses.contains(&PairStatus::Suggested) {
		IngestEditionPairState::Suggested
	} else {
		IngestEditionPairState::Confirmed
	})
}

/// One other item of the same archive drop.
#[derive(Debug, Clone, SimpleObject)]
pub struct IngestDropGroupSibling {
	pub id: ID,
	pub filename: String,
	pub media_type: String,
	pub status: IngestDropItemStatus,
	pub quality_score: Option<i32>,
	/// Set once this sibling committed into the library.
	pub media_id: Option<ID>,
	/// Whether this sibling and the item are an audio/text edition pair, so
	/// committing both records a same-drop pair suggestion.
	pub edition_pair_candidate: bool,
	pub pair_state: IngestEditionPairState,
}

/// `h:mm:ss` from milliseconds, the one spelling every audio field uses.
fn human_duration(duration_ms: i64) -> String {
	let total = duration_ms.max(0) / 1_000;
	format!(
		"{}:{:02}:{:02}",
		total / 3_600,
		(total % 3_600) / 60,
		total % 60
	)
}

fn clamp_i32(value: i64) -> i32 {
	value.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

impl From<stump_ingest::contract::AudioAnalysis> for IngestDropItemAudio {
	fn from(analysis: stump_ingest::contract::AudioAnalysis) -> Self {
		Self {
			duration_ms: clamp_i32(analysis.duration_ms),
			duration: human_duration(analysis.duration_ms),
			codec: analysis.codec,
			sample_rate: analysis.sample_rate,
			channels: analysis.channels,
			bitrate: analysis.bitrate,
			chapter_source: analysis.chapter_source,
			tracks: analysis
				.tracks
				.into_iter()
				.map(IngestAudioTrack::from)
				.collect(),
			chapters: analysis
				.chapters
				.into_iter()
				.map(IngestAudioChapter::from)
				.collect(),
			cover_content_type: analysis.cover_content_type,
			cover_byte_size: analysis.cover_byte_size.map(|size| clamp_i32(size as i64)),
			title: analysis.title,
			author: analysis.author,
			narrator: analysis.narrator,
			album: analysis.album,
			description: analysis.description,
			genre: analysis.genre,
			year: analysis.year,
			assembled: analysis.assembled.map(IngestAssembledAudio::from),
		}
	}
}

impl From<stump_ingest::contract::AudioAnalysisTrack> for IngestAudioTrack {
	fn from(track: stump_ingest::contract::AudioAnalysisTrack) -> Self {
		Self {
			filename: track.filename,
			duration_ms: clamp_i32(track.duration_ms),
			duration: human_duration(track.duration_ms),
			start_offset_ms: clamp_i32(track.start_offset_ms),
			byte_size: clamp_i32(track.byte_size),
			codec: track.codec,
			bitrate: track.bitrate,
			title: track.title,
			track_number: track.track_number.map(|number| number as i32),
		}
	}
}

impl From<stump_ingest::contract::AudioAnalysisChapter> for IngestAudioChapter {
	fn from(chapter: stump_ingest::contract::AudioAnalysisChapter) -> Self {
		Self {
			title: chapter.title,
			start_ms: clamp_i32(chapter.start_ms),
			start: human_duration(chapter.start_ms),
			end_ms: chapter.end_ms.map(clamp_i32),
		}
	}
}

impl From<stump_ingest::contract::AssembledAudio> for IngestAssembledAudio {
	fn from(assembled: stump_ingest::contract::AssembledAudio) -> Self {
		Self {
			filename: assembled.filename,
			byte_size: clamp_i32(assembled.byte_size as i64),
			duration_ms: clamp_i32(assembled.duration_ms),
			duration: human_duration(assembled.duration_ms),
			chapters: assembled.chapters as i32,
			faststart: assembled.faststart,
			parts_kept: assembled.parts_kept,
			method: assembled.method,
		}
	}
}

#[derive(Clone, Debug)]
pub struct IngestMetadataCandidate {
	pub model: ingest_metadata_candidate::Model,
}

impl From<ingest_metadata_candidate::Model> for IngestMetadataCandidate {
	fn from(model: ingest_metadata_candidate::Model) -> Self {
		Self { model }
	}
}

#[Object]
impl IngestMetadataCandidate {
	async fn id(&self) -> ID {
		self.model.id.clone().into()
	}

	async fn drop_item_id(&self) -> Option<ID> {
		self.model.drop_item_id.clone().map(ID::from)
	}

	/// Set when the candidate was produced by a library rework run.
	async fn media_id(&self) -> Option<ID> {
		self.model.media_id.clone().map(ID::from)
	}

	async fn provider(&self) -> &str {
		&self.model.provider_id
	}

	async fn provider_version(&self) -> &str {
		&self.model.provider_version
	}

	async fn model(&self) -> Option<String> {
		self.model
			.provenance
			.get("model")
			.and_then(Value::as_str)
			.map(ToOwned::to_owned)
	}

	async fn confidence(&self) -> f64 {
		self.model.confidence
	}

	async fn fields(&self) -> Json<Value> {
		Json(self.model.fields.clone())
	}

	async fn field_confidences(&self) -> Option<Json<Value>> {
		Some(Json(self.model.field_confidence.clone()))
	}

	async fn source_sha256(&self) -> &str {
		&self.model.source_sha256
	}

	async fn provenance(&self) -> Json<Value> {
		Json(self.model.provenance.clone())
	}

	async fn status(&self) -> Result<IngestCandidateStatus> {
		self.model.status.parse()
	}

	async fn created_at(&self) -> sea_orm::prelude::DateTimeWithTimeZone {
		self.model.created_at
	}
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, async_graphql::Enum)]
pub enum IngestCandidateStatus {
	Pending,
	Accepted,
	Rejected,
}

impl FromStr for IngestCandidateStatus {
	type Err = Error;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		match value {
			"PENDING" => Ok(Self::Pending),
			"ACCEPTED" => Ok(Self::Accepted),
			"REJECTED" => Ok(Self::Rejected),
			_ => Err(Error::new(format!(
				"Unknown ingest candidate status: {value}"
			))),
		}
	}
}

#[derive(Debug, Clone, SimpleObject)]
pub struct IngestDropFolder {
	pub library_id: ID,
	pub display_path: String,
	pub enabled: bool,
	pub pending_count: i32,
	pub last_discovered_at: Option<sea_orm::prelude::DateTimeWithTimeZone>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct IngestReworkReason {
	pub check_id: String,
	pub status: IngestQualityStatus,
	pub message: String,
}

#[derive(Debug, Clone)]
pub struct IngestReworkItem {
	pub item: IngestDropItem,
	pub reasons: Vec<IngestReworkReason>,
}

#[Object]
impl IngestReworkItem {
	async fn item(&self) -> &IngestDropItem {
		&self.item
	}

	async fn reasons(&self) -> &[IngestReworkReason] {
		&self.reasons
	}
}

#[derive(Debug, SimpleObject)]
pub struct PaginatedIngestDropItemResponse {
	pub nodes: Vec<IngestDropItem>,
	pub page_info: PaginationInfo,
}

#[derive(Debug, SimpleObject)]
pub struct PaginatedIngestAnalysisJobResponse {
	pub nodes: Vec<IngestAnalysisJob>,
	pub page_info: PaginationInfo,
}

#[derive(Debug, SimpleObject)]
pub struct PaginatedIngestReworkResponse {
	pub nodes: Vec<IngestReworkItem>,
	pub page_info: PaginationInfo,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct IngestBulkApplyFailure {
	pub drop_item_id: ID,
	pub message: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct IngestBulkApplyPayload {
	pub applied: Vec<IngestDropItem>,
	pub failures: Vec<IngestBulkApplyFailure>,
}

/// Result of `applyIngestMetadata`: exactly one side is set, depending on
/// whether the input targeted a drop item or a library media row.
#[derive(Debug, Clone, SimpleObject)]
pub struct IngestApplyPayload {
	pub drop_item: Option<IngestDropItem>,
	pub media: Option<crate::object::media::Media>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct IngestProviderDescriptor {
	pub id: String,
	pub name: String,
	pub version: String,
	pub configured: bool,
	pub capabilities: Vec<IngestProviderCapability>,
	pub supported_media_types: Vec<String>,
	pub enabled_by_default: bool,
	/// Whether this provider needs an API credential; keyless providers can
	/// be enabled without one.
	pub requires_api_token: bool,
	pub settings: Vec<IngestSettingDefinition>,
	pub help_url: Option<String>,
}

impl From<stump_ingest::providers::ProviderDescriptor> for IngestProviderDescriptor {
	fn from(descriptor: stump_ingest::providers::ProviderDescriptor) -> Self {
		let help_url = provider_help_url(&descriptor.id);
		Self {
			id: descriptor.id,
			name: descriptor.name,
			version: descriptor.version,
			configured: descriptor.configured,
			capabilities: descriptor
				.capabilities
				.into_iter()
				.map(IngestProviderCapability::from)
				.collect(),
			supported_media_types: descriptor.supported_media_types,
			enabled_by_default: descriptor.enabled_default,
			requires_api_token: descriptor.requires_api_token,
			settings: descriptor
				.settings
				.into_iter()
				.map(IngestSettingDefinition::from)
				.collect(),
			help_url,
		}
	}
}

/// Where a user obtains or manages the credential for providers whose keys
/// live in the encrypted `metadata_provider_configs` store rather than in
/// ingest plugin settings.  Providers without a key (embedded, keyless
/// remotes) resolve to `None`.
fn provider_help_url(provider_id: &str) -> Option<String> {
	const COMIC_VINE_HELP_URL: &str = "https://comicvine.gamespot.com/api/";
	const HARDCOVER_HELP_URL: &str = "https://hardcover.app/account/api";
	const MAL_HELP_URL: &str = "https://myanimelist.net/apiconfig";
	const METRON_HELP_URL: &str = "https://metron.cloud/accounts/signup/";
	const GOOGLE_BOOKS_HELP_URL: &str =
		"https://console.cloud.google.com/apis/credentials";
	match provider_id {
		"comic_vine" => Some(COMIC_VINE_HELP_URL.to_string()),
		"hardcover" => Some(HARDCOVER_HELP_URL.to_string()),
		"mal" => Some(MAL_HELP_URL.to_string()),
		"metron" => Some(METRON_HELP_URL.to_string()),
		"googlebooks" => Some(GOOGLE_BOOKS_HELP_URL.to_string()),
		_ => None,
	}
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, async_graphql::Enum)]
pub enum IngestProviderCapability {
	Identify,
	Lookup,
	Search,
	Tags,
	AiEnrichment,
}

impl From<stump_ingest::contract::ProviderCapability> for IngestProviderCapability {
	fn from(capability: stump_ingest::contract::ProviderCapability) -> Self {
		use stump_ingest::contract::ProviderCapability as Core;
		match capability {
			Core::Identify => Self::Identify,
			Core::Lookup => Self::Lookup,
			Core::Search => Self::Search,
			Core::Tags => Self::Tags,
			Core::AiEnrichment => Self::AiEnrichment,
		}
	}
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, async_graphql::Enum)]
pub enum IngestMediaKind {
	ComicArchive,
	ComicRarArchive,
	Epub,
	Pdf,
	/// An audiobook: one container, or one folder of parts.
	Audio,
	Unknown,
}

impl From<stump_ingest::contract::IngestMediaKind> for IngestMediaKind {
	fn from(kind: stump_ingest::contract::IngestMediaKind) -> Self {
		use stump_ingest::contract::IngestMediaKind as Core;
		match kind {
			Core::ComicArchive => Self::ComicArchive,
			Core::ComicRarArchive => Self::ComicRarArchive,
			Core::Epub => Self::Epub,
			Core::Pdf => Self::Pdf,
			Core::Audio => Self::Audio,
			Core::Unknown => Self::Unknown,
		}
	}
}

impl From<IngestMediaKind> for stump_ingest::contract::IngestMediaKind {
	fn from(kind: IngestMediaKind) -> Self {
		use stump_ingest::contract::IngestMediaKind as Core;
		match kind {
			IngestMediaKind::ComicArchive => Core::ComicArchive,
			IngestMediaKind::ComicRarArchive => Core::ComicRarArchive,
			IngestMediaKind::Epub => Core::Epub,
			IngestMediaKind::Pdf => Core::Pdf,
			IngestMediaKind::Audio => Core::Audio,
			IngestMediaKind::Unknown => Core::Unknown,
		}
	}
}

/// One flattened result of a provider search.  Evidence only: the editor
/// turns a chosen hit into a persisted candidate via `lookupIngestCandidate`.
#[derive(Debug, Clone, SimpleObject)]
pub struct IngestSearchHit {
	pub provider_id: String,
	pub external_id: String,
	pub title: String,
	pub year: Option<i32>,
	pub cover_url: Option<String>,
	pub summary: Option<String>,
	pub score: f64,
}

impl From<stump_ingest::contract::SearchHit> for IngestSearchHit {
	fn from(hit: stump_ingest::contract::SearchHit) -> Self {
		Self {
			provider_id: hit.provider_id,
			external_id: hit.external_id,
			title: hit.title,
			year: hit.year,
			cover_url: hit.cover_url,
			summary: hit.summary,
			score: f64::from(hit.score),
		}
	}
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, async_graphql::Enum)]
pub enum IngestSettingValueType {
	Boolean,
	Integer,
	Number,
	String,
	Json,
}

impl From<SettingKind> for IngestSettingValueType {
	fn from(kind: SettingKind) -> Self {
		match kind {
			SettingKind::Bool => Self::Boolean,
			SettingKind::Int => Self::Integer,
			SettingKind::Float => Self::Number,
			SettingKind::String | SettingKind::Enum => Self::String,
			SettingKind::Json => Self::Json,
		}
	}
}

#[derive(Debug, Clone, SimpleObject)]
pub struct IngestSettingDefinition {
	pub key: String,
	pub label: String,
	pub value_type: IngestSettingValueType,
	pub required: bool,
	pub secret: bool,
	pub default_value: Option<Json<Value>>,
	pub description: Option<String>,
	pub help_url: Option<String>,
}

impl From<SettingDefinition> for IngestSettingDefinition {
	fn from(definition: SettingDefinition) -> Self {
		Self {
			key: definition.key.to_owned(),
			label: definition.label.to_owned(),
			value_type: definition.kind.into(),
			required: definition.required,
			secret: definition.secret,
			default_value: Some(Json(definition.default)),
			description: Some(definition.description.to_owned()),
			help_url: definition.help_url.map(str::to_owned),
		}
	}
}

#[derive(Debug, Clone, SimpleObject)]
pub struct IngestProviderSetting {
	pub key: String,
	pub configured: bool,
	pub secret: bool,
	pub value: Option<Json<Value>>,
}

fn settings_for_definitions(
	definitions: &[IngestSettingDefinition],
	values: Option<&Value>,
) -> Vec<IngestProviderSetting> {
	let values = values.and_then(Value::as_object);
	definitions
		.iter()
		.map(|definition| {
			let value = values.and_then(|values| values.get(&definition.key));
			IngestProviderSetting {
				key: definition.key.clone(),
				configured: value.is_some_and(|value| {
					!value.is_null() && !value.as_str().is_some_and(str::is_empty)
				}),
				secret: definition.secret,
				value: (!definition.secret)
					.then(|| value.cloned().map(Json))
					.flatten(),
			}
		})
		.collect()
}

#[derive(Debug, Clone)]
pub struct IngestProviderSettings {
	pub provider: IngestProviderDescriptor,
	pub enabled: bool,
	pub opted_in: bool,
	pub settings: Vec<IngestProviderSetting>,
	pub updated_at: Option<sea_orm::prelude::DateTimeWithTimeZone>,
}

impl IngestProviderSettings {
	pub fn from_parts(
		provider: IngestProviderDescriptor,
		model: Option<ingest_plugin_setting::Model>,
	) -> Self {
		let enabled = model
			.as_ref()
			.map(|model| model.enabled)
			.unwrap_or(provider.enabled_by_default);
		let opted_in = model.as_ref().is_some_and(|model| model.opted_in);
		let values = model.as_ref().and_then(|model| model.values.as_ref());
		let settings = settings_for_definitions(&provider.settings, values);
		Self {
			provider,
			enabled,
			opted_in,
			settings,
			updated_at: model.map(|model| model.updated_at),
		}
	}
}

#[Object]
impl IngestProviderSettings {
	async fn provider(&self) -> &IngestProviderDescriptor {
		&self.provider
	}

	async fn enabled(&self) -> bool {
		self.enabled
	}

	async fn opted_in(&self) -> bool {
		self.opted_in
	}

	async fn settings(&self) -> &[IngestProviderSetting] {
		&self.settings
	}

	async fn updated_at(&self) -> Option<sea_orm::prelude::DateTimeWithTimeZone> {
		self.updated_at
	}
}

/// One repair a failing quality check points at.
///
/// A finding a librarian cannot act on is a complaint, not a check: the id is
/// a `stump_tools` tool and `options` is the option blob that addresses this
/// finding, so a client can offer "fix it" beside the row.
#[derive(Debug, Clone, SimpleObject)]
pub struct IngestQualityFixAction {
	/// A `stump_tools` tool id, e.g. `audio-assemble`.
	pub tool: String,
	pub summary: String,
	/// The tool's JSON options, or `null` for its defaults.
	pub options: Option<serde_json::Value>,
}

impl From<stump_ingest::contract::FixAction> for IngestQualityFixAction {
	fn from(fix: stump_ingest::contract::FixAction) -> Self {
		Self {
			tool: fix.tool,
			summary: fix.summary,
			options: (!fix.options.is_null()).then_some(fix.options),
		}
	}
}

#[derive(Debug, Clone, SimpleObject)]
pub struct IngestQualityCheckDescriptor {
	pub id: String,
	pub name: String,
	pub version: String,
	pub available: bool,
	pub weight: i32,
	pub enabled: bool,
	pub supported_media_types: Vec<String>,
	pub settings: Vec<IngestSettingDefinition>,
	/// The tool that repairs a failing outcome, when one exists in this
	/// build. `null` for a finding whose decision is the librarian's, such as
	/// a duplicate or an unparseable filename.
	pub fix: Option<IngestQualityFixAction>,
}

impl From<stump_ingest::quality::CheckDescriptor> for IngestQualityCheckDescriptor {
	fn from(descriptor: stump_ingest::quality::CheckDescriptor) -> Self {
		Self {
			id: descriptor.id,
			name: descriptor.name,
			version: descriptor.version,
			available: true,
			weight: descriptor.weight as i32,
			enabled: true,
			supported_media_types: Vec::new(),
			settings: descriptor
				.settings
				.into_iter()
				.map(IngestSettingDefinition::from)
				.collect(),
			fix: descriptor.fix.map(IngestQualityFixAction::from),
		}
	}
}

#[derive(Debug, Clone)]
pub struct IngestQualityCheckSettings {
	pub check_id: String,
	pub enabled: bool,
	pub settings: Vec<IngestProviderSetting>,
	pub updated_at: Option<sea_orm::prelude::DateTimeWithTimeZone>,
}

impl IngestQualityCheckSettings {
	pub fn from_parts(
		descriptor: IngestQualityCheckDescriptor,
		model: Option<ingest_plugin_setting::Model>,
	) -> Self {
		let enabled = model.as_ref().is_none_or(|model| model.enabled);
		let values = model.as_ref().and_then(|model| model.values.as_ref());
		Self {
			check_id: descriptor.id,
			enabled,
			settings: settings_for_definitions(&descriptor.settings, values),
			updated_at: model.map(|model| model.updated_at),
		}
	}
}

#[Object]
impl IngestQualityCheckSettings {
	async fn check_id(&self) -> &str {
		&self.check_id
	}

	async fn enabled(&self) -> bool {
		self.enabled
	}

	async fn settings(&self) -> &[IngestProviderSetting] {
		&self.settings
	}

	async fn updated_at(&self) -> Option<sea_orm::prelude::DateTimeWithTimeZone> {
		self.updated_at
	}
}

#[derive(Debug, Clone)]
pub struct IngestProgressEvent {
	pub event: CoreIngestProgressEvent,
	pub emitted_at: sea_orm::prelude::DateTimeWithTimeZone,
}

impl IngestProgressEvent {
	pub fn from_stored(
		event: CoreIngestProgressEvent,
		emitted_at: sea_orm::prelude::DateTimeWithTimeZone,
	) -> Self {
		Self { event, emitted_at }
	}
}

#[Object]
impl IngestProgressEvent {
	async fn event_id(&self) -> ID {
		self.event.cursor.clone().into()
	}

	async fn emitted_at(&self) -> sea_orm::prelude::DateTimeWithTimeZone {
		self.emitted_at
	}

	async fn library_id(&self) -> ID {
		self.event.library_id.clone().into()
	}

	async fn drop_item_id(&self) -> ID {
		self.event.drop_item_id.clone().into()
	}

	async fn analysis_job_id(&self) -> Option<ID> {
		self.event.analysis_job_id.clone().map(Into::into)
	}

	async fn status(&self) -> IngestDropItemStatus {
		self.event.status.into()
	}

	async fn phase(&self) -> IngestAnalysisPhase {
		self.event.phase.into()
	}

	async fn completed(&self) -> i32 {
		self.event.completed.min(i32::MAX as u32) as i32
	}

	async fn total(&self) -> i32 {
		self.event.total.min(i32::MAX as u32) as i32
	}

	async fn score(&self) -> Option<f64> {
		self.event.score.map(f64::from)
	}

	async fn message(&self) -> Option<&str> {
		Some(self.event.message.as_str())
	}
}

#[derive(Debug, Clone, SimpleObject)]
pub struct StageIngestUploadsPayload {
	pub items: Vec<IngestDropItem>,
	pub deduplicated: i32,
}
