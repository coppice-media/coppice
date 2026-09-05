use sea_orm::entity::prelude::*;
use serde_json::Value;

/// Per-user configuration for one annotation export sink
/// (`markdown`, `git`, ...). `settings` holds the sink's configured values
/// (secret values encrypted by the host); `state` holds the host-owned
/// [`stump_annotation_sync::sink::SinkState`] (export cursors plus opaque
/// sink data) round-tripped between runs.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "annotation_sink_configs")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub user_id: String,
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub sink_id: String,
	#[sea_orm(column_type = "Json", nullable)]
	pub settings: Option<Value>,
	pub enabled: bool,
	#[sea_orm(nullable)]
	pub last_run_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "Text", nullable)]
	pub last_error: Option<String>,
	#[sea_orm(column_type = "Json", nullable)]
	pub state: Option<Value>,
	pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::UserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	User,
}

impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::User.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
