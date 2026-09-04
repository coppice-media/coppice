use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};
use serde_json::Value as JsonValue;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "ingest_quality_reports")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub drop_item_id: String,
	#[sea_orm(column_type = "Text")]
	pub source_sha256: String,
	#[sea_orm(column_type = "Text")]
	pub algorithm_version: String,
	pub score: i32,
	#[sea_orm(column_type = "Json")]
	pub checks: JsonValue,
	#[sea_orm(column_type = "Json")]
	pub settings_snapshot: JsonValue,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::ingest_drop_item::Entity",
		from = "Column::DropItemId",
		to = "super::ingest_drop_item::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	DropItem,
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
		if insert {
			if self.id.is_not_set() {
				self.id = Set(uuid::Uuid::new_v4().to_string());
			}
			if self.created_at.is_not_set() {
				self.created_at = Set(DateTimeWithTimeZone::from(Utc::now()));
			}
		}
		Ok(self)
	}
}
