use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[sea_orm(table_name = "notifiers")]
#[cfg_attr(feature = "graphql", graphql(name = "NotifierModel"))]
pub struct Model {
	#[sea_orm(primary_key)]
	pub id: i32,
	#[sea_orm(column_type = "Text")]
	pub r#type: String,

	#[cfg_attr(feature = "graphql", graphql(skip))] // unmarshalled later on graphql side
	#[sea_orm(column_type = "Blob")]
	pub config: Vec<u8>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

#[derive(
	Eq,
	Copy,
	Hash,
	Debug,
	Clone,
	EnumIter,
	PartialEq,
	Serialize,
	Deserialize,
	DeriveActiveEnum,
	EnumString,
	Display,
)]
#[cfg_attr(feature = "graphql", derive(async_graphql::Enum))]
#[sea_orm(
	rs_type = "String",
	rename_all = "SCREAMING_SNAKE_CASE",
	db_type = "String(StringLen::None)"
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum NotifierType {
	Discord,
	Telegram,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
pub struct DiscordConfig {
	pub webhook_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
pub struct TelegramConfig {
	pub encrypted_token: String,
	pub chat_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "graphql", derive(async_graphql::Union))]
#[serde(untagged)]
pub enum NotifierConfig {
	Discord(DiscordConfig),
	Telegram(TelegramConfig),
}

impl NotifierConfig {
	#[cfg(feature = "graphql")]
	pub fn into_bytes(self) -> async_graphql::Result<Vec<u8>> {
		Ok(serde_json::to_vec(&self)?)
	}

	#[cfg(not(feature = "graphql"))]
	pub fn into_bytes(self) -> std::result::Result<Vec<u8>, serde_json::Error> {
		serde_json::to_vec(&self)
	}

	#[cfg(feature = "graphql")]
	pub fn from_bytes(bytes: &[u8]) -> async_graphql::Result<Self> {
		let config: NotifierConfig = serde_json::from_slice(bytes)?;
		Ok(config)
	}

	#[cfg(not(feature = "graphql"))]
	pub fn from_bytes(bytes: &[u8]) -> std::result::Result<Self, serde_json::Error> {
		serde_json::from_slice(bytes)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use pretty_assertions::assert_eq;

	#[test]
	fn test_deserialize_notifier_config() {
		let bytes = r#"{"webhook_url":"https://discord.com/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890"}"#.as_bytes().to_vec();
		let deserialized: NotifierConfig = serde_json::from_slice(&bytes).unwrap();
		let expected = NotifierConfig::Discord(DiscordConfig {
            webhook_url: "https://discord.com/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890".to_string(),
        });
		assert_eq!(deserialized, expected);
	}

	#[test]
	fn test_serialize_notifier_config() {
		let config = NotifierConfig::Telegram(TelegramConfig {
			encrypted_token: "encrypted_token".to_string(),
			chat_id: "123456789".to_string(),
		});
		let serialized = config.into_bytes().unwrap();
		assert_eq!(
			serialized,
			r#"{"encrypted_token":"encrypted_token","chat_id":"123456789"}"#
				.as_bytes()
				.to_vec()
		);
	}
}
