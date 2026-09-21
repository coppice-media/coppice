use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "book_request_grabs")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub request_id: String,
	#[sea_orm(column_type = "Text")]
	pub release_id: String,
	#[sea_orm(column_type = "Text", unique)]
	pub opaque_id: String,
	#[sea_orm(column_type = "Text")]
	pub status: String,
	pub attempts: i32,
	pub max_attempts: i32,
	#[sea_orm(column_type = "Text", nullable)]
	pub failure_code: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub failure_message: Option<String>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub started_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub finished_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub last_polled_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub next_poll_at: Option<DateTimeWithTimeZone>,
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
		belongs_to = "super::book_request_release::Entity",
		from = "Column::ReleaseId",
		to = "super::book_request_release::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Release,
}
impl Related<super::book_request::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Request.def()
	}
}
impl Related<super::book_request_release::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Release.def()
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
