//! One privacy-minimized observation from a [`remote_source`](super::remote_source)
//! root. Worker identity and lineage are retained for reconciliation, but no
//! server filesystem path is authoritative.

use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "remote_source_items")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub source_id: String,
	#[sea_orm(column_type = "Text")]
	pub worker_item_id: String,
	#[sea_orm(column_type = "Text")]
	pub worker_content_version: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub relative_path: Option<String>,
	pub size: i64,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub modified_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "Text", nullable)]
	pub media_type: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub quick_fingerprint: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub sha256: Option<String>,
	#[sea_orm(column_type = "Json", nullable)]
	pub metadata: Option<Json>,
	#[sea_orm(column_type = "Json", nullable)]
	pub retention: Option<Json>,
	#[sea_orm(column_type = "Text")]
	pub observation_state: String,
	pub last_seen_revision: i64,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub last_seen_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub updated_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "Text", nullable)]
	pub imported_media_id: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::remote_source::Entity",
		from = "Column::SourceId",
		to = "super::remote_source::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Source,
	#[sea_orm(
		belongs_to = "super::media::Entity",
		from = "Column::ImportedMediaId",
		to = "super::media::Column::Id",
		on_update = "Cascade",
		on_delete = "SetNull"
	)]
	ImportedMedia,
	#[sea_orm(has_many = "super::media_location::Entity")]
	Locations,
	#[sea_orm(has_many = "super::remote_source_import::Entity")]
	Imports,
}

impl Related<super::remote_source::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Source.def()
	}
}

impl Related<super::media::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::ImportedMedia.def()
	}
}

impl Related<super::media_location::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Locations.def()
	}
}
impl Related<super::remote_source_import::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Imports.def()
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
		assert_eq!(Entity.table_name(), "remote_source_items");
	}
}
