use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "book_request_handoffs")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub request_id: String,
	#[sea_orm(column_type = "Text")]
	pub grab_id: String,
	#[sea_orm(column_type = "Text")]
	pub library_id: String,
	#[sea_orm(column_type = "Text")]
	pub relative_path: String,
	#[sea_orm(column_type = "Text")]
	pub sha256: String,
	pub byte_size: i64,
	#[sea_orm(column_type = "Text")]
	pub status: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub drop_item_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub shelf_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub device_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub error_code: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub error_message: Option<String>,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub updated_at: DateTimeWithTimeZone,
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
		belongs_to = "super::book_request_grab::Entity",
		from = "Column::GrabId",
		to = "super::book_request_grab::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Grab,
}
impl Related<super::book_request::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Request.def()
	}
}
impl Related<super::book_request_grab::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Grab.def()
	}
}
#[async_trait]
impl ActiveModelBehavior for ActiveModel {
	async fn before_save<C>(mut self, _db: &C, insert: bool) -> Result<Self, DbErr>
	where
		C: ConnectionTrait,
	{
		let now = DateTimeWithTimeZone::from(Utc::now());
		if insert && self.created_at.is_not_set() {
			self.created_at = Set(now);
		}
		self.updated_at = Set(now);
		Ok(self)
	}
}
