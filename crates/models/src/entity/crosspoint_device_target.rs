use sea_orm::entity::prelude::*;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "crosspoint_device_targets")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub device_id: String,
	#[sea_orm(column_type = "Text")]
	pub user_id: String,
	#[sea_orm(column_type = "Text")]
	pub host_or_ip: String,
	pub http_port: i32,
	pub ws_port: i32,
	#[sea_orm(column_type = "Text")]
	pub root_path: String,
	#[sea_orm(column_type = "Text")]
	pub discovery_method: String,
	pub verified_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "Json", nullable)]
	pub fingerprint: Option<Value>,
	#[sea_orm(column_type = "Json")]
	pub profile_json: Value,
	#[sea_orm(column_type = "Text")]
	pub profile_digest: String,
	pub created_at: DateTimeWithTimeZone,
	pub updated_at: DateTimeWithTimeZone,
	pub revoked_at: Option<DateTimeWithTimeZone>,
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
