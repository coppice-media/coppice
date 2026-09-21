use async_graphql::{InputObject, ID};

/// A catalog/work identity outside the Coppice library. The gateway only sees
/// normalized metadata and an opaque candidate id later returned by search.
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
	#[graphql(default = false)]
	pub automation_enabled: bool,
}

#[derive(Debug, Clone, InputObject)]
pub struct BookRequestGatewayInput {
	pub endpoint: String,
	pub token: String,
	#[graphql(default = false)]
	pub enabled: bool,
	#[graphql(default = true)]
	pub require_approval: bool,
	#[graphql(default = false)]
	pub automation_enabled: bool,
	#[graphql(default = 80)]
	pub scoring_floor: i32,
	#[graphql(default = 70)]
	pub verification_threshold: i32,
	#[graphql(default = 3)]
	pub max_retries: i32,
	pub handoff_root: Option<String>,
}
