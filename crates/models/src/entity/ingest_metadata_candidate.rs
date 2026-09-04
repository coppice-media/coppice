use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};
use serde_json::Value as JsonValue;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "ingest_metadata_candidates")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub drop_item_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub media_id: Option<String>,
	#[sea_orm(column_type = "Text")]
	pub provider_id: String,
	#[sea_orm(column_type = "Text")]
	pub provider_version: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub external_id: Option<String>,
	#[sea_orm(column_type = "Text")]
	pub source_sha256: String,
	pub confidence: f64,
	#[sea_orm(column_type = "Json")]
	pub fields: JsonValue,
	#[sea_orm(column_type = "Json")]
	pub field_confidence: JsonValue,
	#[sea_orm(column_type = "Json")]
	pub provenance: JsonValue,
	#[sea_orm(column_type = "Text")]
	pub status: String,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub updated_at: DateTimeWithTimeZone,
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
	#[sea_orm(
		belongs_to = "super::media::Entity",
		from = "Column::MediaId",
		to = "super::media::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Media,
}

impl Related<super::ingest_drop_item::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::DropItem.def()
	}
}

impl Related<super::media::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Media.def()
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
			if self.updated_at.is_not_set() {
				self.updated_at = Set(DateTimeWithTimeZone::from(Utc::now()));
			}
			if self.status.is_not_set() {
				self.status = Set("PENDING".to_string());
			}
		} else {
			self.updated_at = Set(DateTimeWithTimeZone::from(Utc::now()));
		}
		Ok(self)
	}
}
