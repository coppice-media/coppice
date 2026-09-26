use crate::input::book_request::RequestFormat;
use async_graphql::Object;
use models::entity::book_request;

#[derive(Debug, Clone, Copy, Eq, PartialEq, async_graphql::Enum)]
pub enum BookRequestStatus {
	Pending,
	/// Historical acquisition state retained so existing rows remain readable.
	Searching,
	/// Historical request state retained so existing rows remain readable.
	AwaitingApproval,
	/// Historical acquisition state retained so existing rows remain readable.
	NeedsSelection,
	Approved,
	/// Historical acquisition state retained so existing rows remain readable.
	Grabbed,
	/// Historical acquisition state retained so existing rows remain readable.
	Importing,
	/// Historical acquisition state retained so existing rows remain readable.
	Queued,
	/// Historical acquisition state retained so existing rows remain readable.
	Completed,
	Rejected,
	/// Historical acquisition state retained so existing rows remain readable.
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
			"GRABBED" => Self::Grabbed,
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
	async fn format(&self) -> RequestFormat {
		self.model.format.as_str().into()
	}
	async fn isbn(&self) -> Option<&str> {
		self.model.isbn.as_deref()
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
	async fn preferred_narrator(&self) -> Option<&str> {
		self.model.preferred_narrator.as_deref()
	}
	async fn status(&self) -> BookRequestStatus {
		self.model.status.as_str().into()
	}
	async fn approval_policy(&self) -> &str {
		&self.model.approval_policy
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
