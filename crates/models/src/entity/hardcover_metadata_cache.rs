use sea_orm::entity::prelude::*;
use serde_json::Value;

/// Public metadata cache only. Personal journal, quote, progress, and account
/// responses are deliberately not represented by this entity.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "hardcover_metadata_cache")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub provider: String,
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub query_key: String,
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub schema_version: String,
	#[sea_orm(column_type = "Json")]
	pub payload: Value,
	pub expires_at: DateTimeWithTimeZone,
	pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
