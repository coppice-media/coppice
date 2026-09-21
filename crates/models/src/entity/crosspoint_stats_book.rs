use sea_orm::entity::prelude::*;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "crosspoint_stats_book")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub user_id: String,
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub device_id: String,
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub document: String,
	pub version: i32,
	pub sessions: i64,
	pub seconds: i64,
	pub pages: i64,
	pub completed: bool,
	pub avg_fwd: i64,
	pub pace_n: i64,
	pub eta: i64,
	pub start_manual: bool,
	pub finish_manual: bool,
	pub start_date: i64,
	pub finished_date: i64,
	#[sea_orm(column_type = "Json")]
	pub tod_json: Value,
	#[sea_orm(column_type = "Json")]
	pub dow_json: Value,
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
	#[sea_orm(
		belongs_to = "super::device::Entity",
		from = "Column::DeviceId",
		to = "super::device::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Device,
}

impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::User.def()
	}
}

impl Related<super::device::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Device.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
