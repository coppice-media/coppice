//! GraphQL objects for notification channels, rules, and settings.

use async_graphql::SimpleObject;
use serde_json::Value;
use stump_notify::{Channel, NotificationKind};

/// One configurable setting of a notification channel, mirroring
/// [`stump_api_types::settings::SettingDefinition`] (which is transport-only
/// and carries no GraphQL derives).
#[derive(SimpleObject, Clone, Debug)]
pub struct NotificationSettingDefinition {
	pub key: String,
	pub label: String,
	pub description: String,
	pub kind: String,
	pub default: Value,
	pub required: bool,
	/// Secret values are never returned to clients.
	pub secret: bool,
	pub help_url: Option<String>,
}

impl NotificationSettingDefinition {
	pub fn from_definition(definition: &stump_notify::SettingDefinition) -> Self {
		Self {
			key: definition.key.to_string(),
			label: definition.label.to_string(),
			description: definition.description.to_string(),
			kind: serde_json::to_value(definition.kind)
				.and_then(|value| {
					value
						.as_str()
						.map(str::to_string)
						.ok_or_else(|| serde_json::Error::custom("expected string"))
				})
				.unwrap_or_else(|_| "STRING".to_string()),
			default: definition.default.clone(),
			required: definition.required,
			secret: definition.secret,
			help_url: definition.help_url.map(str::to_string),
		}
	}
}

/// A channel the current user can route notifications to, with its settings
/// schema.
#[derive(SimpleObject, Clone, Debug)]
pub struct NotificationChannel {
	pub id: String,
	pub label: String,
	pub settings: Vec<NotificationSettingDefinition>,
}

impl NotificationChannel {
	pub fn from_channel(channel: &dyn Channel) -> Self {
		Self {
			id: channel.id().to_string(),
			label: channel.label().to_string(),
			settings: channel
				.settings()
				.iter()
				.map(NotificationSettingDefinition::from_definition)
				.collect(),
		}
	}
}

/// The current user's effective settings for one channel, with secret values
/// redacted.
#[derive(SimpleObject, Clone, Debug)]
pub struct NotificationChannelSettingsValue {
	pub channel_id: String,
	/// Merged defaults and stored values; secrets are omitted entirely.
	pub values: Value,
}

/// One routing rule: which channel receives which notification kind.
#[derive(SimpleObject, Clone, Debug)]
#[graphql(name = "NotificationRuleModel")]
pub struct NotificationRule {
	pub id: i32,
	/// A [`NotificationKind`] name or `*` for every kind.
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
