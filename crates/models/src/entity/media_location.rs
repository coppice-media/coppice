//! A verified physical or remote location for a media row.
//!
//! Remote locations carry server-verified content identity. The optional source
//! item is provenance and may be cleared when an observation is deleted.

use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "media_locations")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub media_id: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub source_item_id: Option<String>,
	#[sea_orm(column_type = "Text")]
	pub kind: String,
	#[sea_orm(column_type = "Text")]
	pub sha256: String,
	#[sea_orm(column_type = "Text")]
	pub content_version: String,
	#[sea_orm(column_type = "Text")]
	pub health: String,
	#[sea_orm(column_type = "Text")]
	pub durability_role: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub cache_path: Option<String>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub verified_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub last_seen_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::media::Entity",
		from = "Column::MediaId",
		to = "super::media::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Media,
	#[sea_orm(
		belongs_to = "super::remote_source_item::Entity",
		from = "Column::SourceItemId",
		to = "super::remote_source_item::Column::Id",
		on_update = "Cascade",
		on_delete = "SetNull"
	)]
	SourceItem,
}

impl Related<super::media::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Media.def()
	}
}

impl Related<super::remote_source_item::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::SourceItem.def()
	}
}

#[async_trait]
impl ActiveModelBehavior for ActiveModel {
	async fn before_save<C>(mut self, _db: &C, insert: bool) -> Result<Self, DbErr>
	where
		C: ConnectionTrait,
	{
		let now = DateTimeWithTimeZone::from(Utc::now());
		if insert {
			if self.id.is_not_set() {
				self.id = Set(uuid::Uuid::new_v4().to_string());
			}
			if self.created_at.is_not_set() {
				self.created_at = Set(now);
			}
		}
		self.updated_at = Set(now);
		Ok(self)
	}
}

#[cfg(test)]
mod tests {
	use super::Entity;
	use sea_orm::EntityName;

	#[test]
	fn table_name_is_stable() {
		assert_eq!(Entity.table_name(), "media_locations");
	}
}
