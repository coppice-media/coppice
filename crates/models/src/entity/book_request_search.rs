use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "book_request_searches")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub request_id: String,
	#[sea_orm(column_type = "Text")]
	pub query: String,
	#[sea_orm(column_type = "Text")]
	pub provider: String,
	#[sea_orm(column_type = "Text")]
	pub status: String,
	pub result_count: i32,
	#[sea_orm(column_type = "Text", nullable)]
	pub error_code: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub error_message: Option<String>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub started_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub finished_at: Option<DateTimeWithTimeZone>,
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
}

impl Related<super::book_request::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Request.def()
	}
}

#[async_trait]
impl ActiveModelBehavior for ActiveModel {
	async fn before_save<C>(mut self, _db: &C, insert: bool) -> Result<Self, DbErr>
	where
		C: ConnectionTrait,
	{
		if insert && self.created_at.is_not_set() {
			self.created_at = Set(DateTimeWithTimeZone::from(Utc::now()));
		}
		Ok(self)
	}
}
