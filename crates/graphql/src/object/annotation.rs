use async_graphql::SimpleObject;

use stump_core::annotation_sync::SinkStatusRow;

use crate::object::ingest::IngestSettingDefinition;

/// A compiled-in annotation export sink and its setting schema.
#[derive(Debug, Clone, SimpleObject)]
pub struct AnnotationSink {
	pub id: String,
	pub name: String,
	pub description: String,
	pub settings: Vec<IngestSettingDefinition>,
}

impl From<stump_annotation_sync::sink::SinkDescriptor> for AnnotationSink {
	fn from(descriptor: stump_annotation_sync::sink::SinkDescriptor) -> Self {
		Self {
			id: descriptor.id.to_owned(),
			name: descriptor.name.to_owned(),
			description: descriptor.description.to_owned(),
			settings: descriptor.settings.into_iter().map(Into::into).collect(),
		}
	}
}

/// Per-sink configuration state for one user.
#[derive(Debug, Clone, SimpleObject)]
pub struct AnnotationSinkStatus {
	pub sink_id: String,
	pub enabled: bool,
	pub last_run_at: Option<sea_orm::prelude::DateTimeWithTimeZone>,
	pub last_error: Option<String>,
}

impl From<SinkStatusRow> for AnnotationSinkStatus {
	fn from(row: SinkStatusRow) -> Self {
		Self {
			sink_id: row.sink_id,
			enabled: row.enabled,
			last_run_at: row.last_run_at,
			last_error: row.last_error,
		}
	}
}

/// The annotation sync state for one user.
#[derive(Debug, Clone, SimpleObject)]
pub struct AnnotationSyncStatus {
	pub user_id: String,
	/// A debounced export is scheduled for this user.
	pub pending: bool,
	pub sinks: Vec<AnnotationSinkStatus>,
}
