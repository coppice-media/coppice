use async_graphql::{Json, Object, SimpleObject};
use models::entity::{
	book_request, book_request_gateway_setting, book_request_grab, book_request_handoff,
	book_request_release,
};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Eq, PartialEq, async_graphql::Enum)]
pub enum BookRequestStatus {
	Pending,
	Searching,
	AwaitingApproval,
	NeedsSelection,
	Approved,
	Grabbed,
	Importing,
	Queued,
	Completed,
	Rejected,
	Failed,
}

impl BookRequestStatus {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Pending => "PENDING",
			Self::Searching => "SEARCHING",
			Self::AwaitingApproval => "AWAITING_APPROVAL",
			Self::NeedsSelection => "NEEDS_SELECTION",
			Self::Approved => "APPROVED",
			Self::Grabbed => "GRABBED",
			Self::Importing => "IMPORTING",
			Self::Queued => "QUEUED",
			Self::Completed => "COMPLETED",
			Self::Rejected => "REJECTED",
			Self::Failed => "FAILED",
		}
	}
}
impl From<&str> for BookRequestStatus {
	fn from(value: &str) -> Self {
		match value {
			"SEARCHING" => Self::Searching,
			"AWAITING_APPROVAL" => Self::AwaitingApproval,
			"NEEDS_SELECTION" => Self::NeedsSelection,
			"APPROVED" => Self::Approved,
			"IMPORTING" => Self::Importing,
			"QUEUED" => Self::Queued,
			"COMPLETED" => Self::Completed,
			"REJECTED" => Self::Rejected,
			"FAILED" => Self::Failed,
			_ => Self::Pending,
		}
	}
}

#[derive(Debug, Clone)]
pub struct BookRequest {
	pub model: book_request::Model,
}
impl From<book_request::Model> for BookRequest {
	fn from(model: book_request::Model) -> Self {
		Self { model }
	}
}
#[Object]
impl BookRequest {
	async fn id(&self) -> &str {
		&self.model.id
	}
	async fn requester_id(&self) -> &str {
		&self.model.requester_id
	}
	async fn internal_media_id(&self) -> Option<&str> {
		self.model.internal_media_id.as_deref()
	}
	async fn internal_work_id(&self) -> Option<&str> {
		self.model.internal_work_id.as_deref()
	}
	async fn source_provider(&self) -> Option<&str> {
		self.model.source_provider.as_deref()
	}
	async fn remote_id(&self) -> Option<&str> {
		self.model.remote_id.as_deref()
	}
	async fn external_key(&self) -> Option<&str> {
		self.model.external_key.as_deref()
	}
	async fn title(&self) -> &str {
		&self.model.title
	}
	async fn authors(&self) -> Option<&str> {
		self.model.authors.as_deref()
	}
	async fn cover_url(&self) -> Option<&str> {
		self.model.cover_url.as_deref()
	}
	async fn destination_shelf_id(&self) -> Option<&str> {
		self.model.destination_shelf_id.as_deref()
	}
	async fn destination_device_id(&self) -> Option<&str> {
		self.model.destination_device_id.as_deref()
	}
	async fn status(&self) -> BookRequestStatus {
		self.model.status.as_str().into()
	}
	async fn approval_policy(&self) -> &str {
		&self.model.approval_policy
	}
	async fn automation_enabled(&self) -> bool {
		self.model.automation_enabled
	}
	async fn scoring_floor(&self) -> i32 {
		self.model.scoring_floor
	}
	async fn verification_threshold(&self) -> i32 {
		self.model.verification_threshold
	}
	async fn max_retries(&self) -> i32 {
		self.model.max_retries
	}
	async fn retries(&self) -> i32 {
		self.model.retries
	}
	async fn approved_by(&self) -> Option<&str> {
		self.model.approved_by.as_deref()
	}
	async fn rejected_by(&self) -> Option<&str> {
		self.model.rejected_by.as_deref()
	}
	async fn failure_code(&self) -> Option<&str> {
		self.model.failure_code.as_deref()
	}
	async fn failure_message(&self) -> Option<&str> {
		self.model.failure_message.as_deref()
	}
	async fn created_at(&self) -> chrono::DateTime<chrono::FixedOffset> {
		self.model.created_at.into()
	}
	async fn updated_at(&self) -> chrono::DateTime<chrono::FixedOffset> {
		self.model.updated_at.into()
	}
	async fn approved_at(&self) -> Option<chrono::DateTime<chrono::FixedOffset>> {
		self.model.approved_at.map(Into::into)
	}
	async fn completed_at(&self) -> Option<chrono::DateTime<chrono::FixedOffset>> {
		self.model.completed_at.map(Into::into)
	}
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookRequestRelease {
	pub id: String,
	pub search_id: String,
	pub request_id: String,
	pub source_provider: String,
	pub remote_id: String,
	pub external_key: Option<String>,
	pub title: String,
	pub authors: Option<String>,
	pub format: Option<String>,
	pub language: Option<String>,
	pub edition: Option<String>,
	pub quality: Option<String>,
	pub size_bytes: Option<i64>,
	pub seeders: Option<i32>,
	pub preview_name: Option<String>,
	pub preview_mime: Option<String>,
	pub preview_bytes: Option<i64>,
	pub score: i32,
	pub score_components: Json<Value>,
	pub rank: i32,
	pub selected: bool,
}
impl From<book_request_release::Model> for BookRequestRelease {
	fn from(model: book_request_release::Model) -> Self {
		Self {
			id: model.id,
			search_id: model.search_id,
			request_id: model.request_id,
			source_provider: model.source_provider,
			remote_id: model.remote_id,
			external_key: model.external_key,
			title: model.title,
			authors: model.authors,
			format: model.format,
			language: model.language,
			edition: model.edition,
			quality: model.quality,
			size_bytes: model.size_bytes,
			seeders: model.seeders,
			preview_name: model.preview_name,
			preview_mime: model.preview_mime,
			preview_bytes: model.preview_bytes,
			score: model.score,
			score_components: Json(model.score_components),
			rank: model.rank,
			selected: model.selected,
		}
	}
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookRequestGrab {
	pub id: String,
	pub request_id: String,
	pub release_id: String,
	/// Opaque sidecar handle; it is not a tracker URL or a download URL.
	pub opaque_id: String,
	pub status: String,
	pub attempts: i32,
	pub max_attempts: i32,
	pub failure_code: Option<String>,
	pub failure_message: Option<String>,
	pub started_at: Option<chrono::DateTime<chrono::FixedOffset>>,
	pub finished_at: Option<chrono::DateTime<chrono::FixedOffset>>,
	pub last_polled_at: Option<chrono::DateTime<chrono::FixedOffset>>,
	pub next_poll_at: Option<chrono::DateTime<chrono::FixedOffset>>,
	pub created_at: chrono::DateTime<chrono::FixedOffset>,
	pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}
impl From<book_request_grab::Model> for BookRequestGrab {
	fn from(model: book_request_grab::Model) -> Self {
		Self {
			id: model.id,
			request_id: model.request_id,
			release_id: model.release_id,
			opaque_id: model.opaque_id,
			status: model.status,
			attempts: model.attempts,
			max_attempts: model.max_attempts,
			failure_code: model.failure_code,
			failure_message: model.failure_message,
			started_at: model.started_at.map(Into::into),
			finished_at: model.finished_at.map(Into::into),
			last_polled_at: model.last_polled_at.map(Into::into),
			next_poll_at: model.next_poll_at.map(Into::into),
			created_at: model.created_at.into(),
			updated_at: model.updated_at.into(),
		}
	}
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookRequestHandoff {
	pub id: String,
	pub request_id: String,
	pub grab_id: String,
	pub library_id: String,
	pub relative_path: String,
	pub sha256: String,
	pub byte_size: i64,
	pub status: String,
	pub drop_item_id: Option<String>,
	pub shelf_id: Option<String>,
	pub device_id: Option<String>,
	pub error_code: Option<String>,
	pub error_message: Option<String>,
}
impl From<book_request_handoff::Model> for BookRequestHandoff {
	fn from(model: book_request_handoff::Model) -> Self {
		Self {
			id: model.id,
			request_id: model.request_id,
			grab_id: model.grab_id,
			library_id: model.library_id,
			relative_path: model.relative_path,
			sha256: model.sha256,
			byte_size: model.byte_size,
			status: model.status,
			drop_item_id: model.drop_item_id,
			shelf_id: model.shelf_id,
			device_id: model.device_id,
			error_code: model.error_code,
			error_message: model.error_message,
		}
	}
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookRequestGatewaySettings {
	pub id: String,
	pub endpoint: String,
	pub enabled: bool,
	pub require_approval: bool,
	pub automation_enabled: bool,
	pub scoring_floor: i32,
	pub verification_threshold: i32,
	pub max_retries: i32,
	pub handoff_root: Option<String>,
	pub has_token: bool,
	pub token_redacted: String,
	pub updated_by: Option<String>,
	pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}
impl From<book_request_gateway_setting::Model> for BookRequestGatewaySettings {
	fn from(model: book_request_gateway_setting::Model) -> Self {
		let has_token = !model.encrypted_token.is_empty();
		Self {
			id: model.id,
			endpoint: model.endpoint,
			enabled: model.enabled,
			require_approval: model.require_approval,
			automation_enabled: model.automation_enabled,
			scoring_floor: model.scoring_floor,
			verification_threshold: model.verification_threshold,
			max_retries: model.max_retries,
			handoff_root: model.handoff_root,
			has_token,
			token_redacted: if has_token {
				"********".into()
			} else {
				String::new()
			},
			updated_by: model.updated_by,
			updated_at: model.updated_at.into(),
		}
	}
}
