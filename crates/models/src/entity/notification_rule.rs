use sea_orm::entity::prelude::*;

/// One per-user routing entry: which channel receives which notification kind.
/// `event_kind` is a [`stump_notify::NotificationKind`](strum) name or `*`
/// for every kind; precedence between exact and wildcard rows is resolved by
/// `stump_notify::routing`.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "notification_rules")]
pub struct Model {
	#[sea_orm(primary_key)]
	pub id: i32,
	#[sea_orm(column_type = "Text")]
	pub user_id: String,
	#[sea_orm(column_type = "Text")]
	pub event_kind: String,
	#[sea_orm(column_type = "Text")]
	pub channel_id: String,
	pub enabled: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
