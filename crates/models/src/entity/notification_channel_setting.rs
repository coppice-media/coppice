use sea_orm::entity::prelude::*;
use serde_json::Value;

/// A user's stored settings for one notification channel, keyed by the
/// channel's [`stump_notify::Channel::settings`] definition keys. Values for
/// definitions marked `secret` are stored encrypted and decrypted only when
/// a dispatch or test delivery builds the recipient.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "notification_channel_settings")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false)]
	#[sea_orm(column_type = "Text")]
	pub user_id: String,
	#[sea_orm(primary_key, auto_increment = false)]
	#[sea_orm(column_type = "Text")]
	pub channel_id: String,
	#[sea_orm(column_type = "Json")]
	pub settings: Value,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
