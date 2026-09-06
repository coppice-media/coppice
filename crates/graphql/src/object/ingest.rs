use std::str::FromStr;

use async_graphql::{Context, Error, Json, Object, Result, SimpleObject, ID};
use models::{
	entity::{
		ingest_analysis_job, ingest_drop_item, ingest_metadata_candidate,
		ingest_plugin_setting, ingest_quality_report, media, series,
	},
	shared::enums::JobStatus,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
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
