use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "book_request_approvals")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub request_id: String,
	#[sea_orm(column_type = "Text")]
	pub approver_id: String,
	#[sea_orm(column_type = "Text")]
	pub decision: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub reason: Option<String>,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::book_request::Entity",
		from = "Column::RequestId",
		to = "super::book_request::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Request,
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::ApproverId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Approver,
}
impl Related<super::book_request::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Request.def()
	}
}
impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Approver.def()
	}
}
impl ActiveModelBehavior for ActiveModel {}
