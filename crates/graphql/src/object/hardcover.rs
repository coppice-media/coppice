use async_graphql::{SimpleObject, ID};
use chrono::{DateTime, FixedOffset};
use models::entity::{hardcover_connection, hardcover_media_link};

/// Redacted personal Hardcover status. No PAT or provider response payload is
/// ever part of this object.
#[derive(Debug, Clone, SimpleObject)]
pub struct HardcoverConnection {
	pub connected: bool,
	pub remote_user_id: Option<String>,
	pub remote_username: Option<String>,
	pub scopes: Vec<String>,
	pub capabilities: Vec<String>,
	pub use_for_metadata: bool,
	pub import_journals: bool,
	pub sync_progress: bool,
	pub connected_at: Option<DateTime<FixedOffset>>,
	pub verified_at: Option<DateTime<FixedOffset>>,
	pub last_sync_at: Option<DateTime<FixedOffset>>,
	pub last_error: Option<String>,
}

impl From<hardcover_connection::Model> for HardcoverConnection {
	fn from(model: hardcover_connection::Model) -> Self {
		Self {
			connected: true,
			remote_user_id: model.remote_user_id,
			remote_username: model.remote_username,
			scopes: strings_from_json(model.scopes),
			capabilities: strings_from_json(model.capabilities),
			use_for_metadata: model.use_for_metadata,
			import_journals: model.import_journals,
			sync_progress: model.sync_progress,
			connected_at: Some(model.connected_at),
			verified_at: model.verified_at,
			last_sync_at: model.last_sync_at,
			last_error: model.last_error,
		}
	}
}

impl Default for HardcoverConnection {
	fn default() -> Self {
		Self {
			connected: false,
			remote_user_id: None,
			remote_username: None,
			scopes: Vec::new(),
			capabilities: Vec::new(),
			use_for_metadata: true,
			import_journals: false,
			sync_progress: false,
			connected_at: None,
			verified_at: None,
			last_sync_at: None,
			last_error: None,
		}
	}
}

#[derive(Debug, Clone, SimpleObject)]
pub struct HardcoverSyncResult {
	pub status: String,
	pub imported: u32,
	pub unresolved: u32,
	pub projected: u32,
	pub skipped: u32,
	pub last_sync_at: Option<DateTime<FixedOffset>>,
	pub error: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct HardcoverMediaLink {
	pub id: ID,
	pub media_id: ID,
	pub remote_id: String,
	pub remote_title: Option<String>,
	pub linked_at: DateTime<FixedOffset>,
	pub updated_at: DateTime<FixedOffset>,
}

impl From<hardcover_media_link::Model> for HardcoverMediaLink {
	fn from(model: hardcover_media_link::Model) -> Self {
		Self {
			id: ID::from(model.id),
			media_id: ID::from(model.media_id),
			remote_id: model.remote_id,
			remote_title: model.remote_title,
			linked_at: model.linked_at,
			updated_at: model.updated_at,
		}
	}
}

fn strings_from_json(value: Option<serde_json::Value>) -> Vec<String> {
	match value {
		Some(serde_json::Value::Array(values)) => values
			.into_iter()
			.filter_map(|value| value.as_str().map(ToOwned::to_owned))
			.collect(),
		Some(serde_json::Value::Object(values)) => values
			.into_iter()
			.filter_map(|(key, value)| {
				value.as_bool().filter(|enabled| *enabled).map(|_| key)
			})
			.collect(),
		_ => Vec::new(),
	}
}
#[derive(Debug, Clone, SimpleObject)]
pub struct HardcoverMetadataLookup {
	pub provider: String,
	pub remote_id: String,
	pub title: Option<String>,
	pub summary: Option<String>,
	pub writers: Vec<String>,
	pub year: Option<i32>,
	pub page_count: Option<i32>,
	pub cover_url: Option<String>,
	pub provider_url: Option<String>,
}
