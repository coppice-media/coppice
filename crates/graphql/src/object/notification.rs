//! GraphQL objects for notification channels, rules, and settings.

use async_graphql::{Json, SimpleObject};
use serde_json::Value;
use stump_notify::Channel;

use crate::object::ingest::IngestSettingDefinition;

/// A channel the current user can route notifications to, with its settings
/// schema.
#[derive(SimpleObject, Clone, Debug)]
pub struct NotificationChannel {
	pub id: String,
	pub label: String,
	pub settings: Vec<IngestSettingDefinition>,
}

impl NotificationChannel {
	pub fn from_channel(channel: &dyn Channel) -> Self {
		Self {
			id: channel.id().to_string(),
			label: channel.label().to_string(),
			settings: channel
				.settings()
				.iter()
				.cloned()
				.map(IngestSettingDefinition::from)
				.collect(),
		}
	}
}

/// The current user's effective settings for one channel, with secret values
/// redacted.
#[derive(SimpleObject, Clone, Debug)]
pub struct NotificationChannelSettingsValue {
	pub channel_id: String,
	/// Merged defaults and stored values keyed by setting key; secrets are
	/// omitted entirely.
	pub values: Json<Value>,
}

/// One routing rule: which channel receives which notification kind.
#[derive(SimpleObject, Clone, Debug)]
#[graphql(name = "NotificationRuleModel")]
pub struct NotificationRule {
	pub id: i32,
	/// A `NotificationKind` name, or `*` for every routable kind.
	pub event_kind: String,
	pub channel_id: String,
	pub enabled: bool,
}

impl From<models::entity::notification_rule::Model> for NotificationRule {
	fn from(model: models::entity::notification_rule::Model) -> Self {
		Self {
			id: model.id,
			event_kind: model.event_kind,
			channel_id: model.channel_id,
			enabled: model.enabled,
		}
	}
}
