use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "crosspoint_delivery_attempts")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub queue_id: String,
	pub attempt_no: i32,
	#[sea_orm(column_type = "Text")]
	pub status: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub error: Option<String>,
	pub bytes: i64,
	pub started_at: DateTimeWithTimeZone,
	pub finished_at: Option<DateTimeWithTimeZone>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::crosspoint_delivery_queue::Entity",
		from = "Column::QueueId",
		to = "super::crosspoint_delivery_queue::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Queue,
}

impl Related<super::crosspoint_delivery_queue::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Queue.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
