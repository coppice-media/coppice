use async_graphql::{InputObject, ID};

/// A catalog/work identity outside the Coppice library, stored as an immutable
/// metadata snapshot on the user's request.
#[derive(Debug, Clone, InputObject)]
pub struct ExternalWorkReferenceInput {
	pub source_provider: String,
	pub remote_id: String,
	pub external_key: Option<String>,
	pub title: String,
	pub authors: Option<String>,
	pub cover_url: Option<String>,
}

#[derive(Debug, Clone, InputObject)]
pub struct CreateBookRequestInput {
	pub media_id: Option<ID>,
	pub work_id: Option<ID>,
	pub external: Option<ExternalWorkReferenceInput>,
	pub title: Option<String>,
	pub authors: Option<String>,
	pub cover_url: Option<String>,
	pub destination_shelf_id: Option<ID>,
	pub destination_device_id: Option<ID>,
}
