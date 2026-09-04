use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};
use serde_json::Value as JsonValue;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "ingest_progress_events")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = true)]
	pub cursor: i64,
	#[sea_orm(column_type = "Text")]
	pub library_id: String,
	#[sea_orm(column_type = "Text")]
	pub drop_item_id: String,
	#[sea_orm(column_type = "Json")]
	pub payload: JsonValue,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::library::Entity",
		from = "Column::LibraryId",
		to = "super::library::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Library,
	#[sea_orm(
		belongs_to = "super::ingest_drop_item::Entity",
		from = "Column::DropItemId",
		to = "super::ingest_drop_item::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	DropItem,
}

impl Related<super::library::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Library.def()
	}
}

impl Related<super::ingest_drop_item::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::DropItem.def()
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
