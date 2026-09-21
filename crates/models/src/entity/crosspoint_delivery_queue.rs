use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "crosspoint_delivery_queue")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub user_id: String,
	#[sea_orm(column_type = "Text")]
	pub device_id: String,
	#[sea_orm(column_type = "Text")]
	pub media_id: String,
	#[sea_orm(column_type = "Text")]
	pub source_revision: String,
	#[sea_orm(column_type = "Text")]
	pub profile_digest: String,
	#[sea_orm(column_type = "Text")]
	pub profile_json: String,
	#[sea_orm(column_type = "Text")]
	pub destination_path: String,
	#[sea_orm(column_type = "Text")]
	pub idempotency_key: String,
	#[sea_orm(column_type = "Text")]
	pub status: String,
	pub attempts: i32,
	pub max_attempts: i32,
	pub next_attempt_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "Text", nullable)]
	pub last_error: Option<String>,
	pub queued_at: DateTimeWithTimeZone,
	pub started_at: Option<DateTimeWithTimeZone>,
	pub completed_at: Option<DateTimeWithTimeZone>,
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
	#[sea_orm(
		belongs_to = "super::media::Entity",
		from = "Column::MediaId",
		to = "super::media::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Media,
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

impl Related<super::media::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Media.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
