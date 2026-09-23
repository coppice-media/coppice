//! A configured home-library root advertised by a source worker.
//!
//! The server stores only opaque root identity and bounded display metadata;
//! absolute filesystem paths remain in the worker's local catalog.

use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "remote_sources")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub device_id: String,
	#[sea_orm(column_type = "Text")]
	pub root_id: String,
	#[sea_orm(column_type = "Text")]
	pub label: String,
	#[sea_orm(column_type = "Text")]
	pub kind: String,
	#[sea_orm(column_type = "Text")]
	pub privacy_mode: String,
	#[sea_orm(column_type = "Text")]
	pub transport: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub direct_base_url: Option<String>,
	pub current_revision: i64,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub last_seen_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "Text")]
	pub health: String,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::device::Entity",
		from = "Column::DeviceId",
		to = "super::device::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Device,
	#[sea_orm(has_many = "super::remote_source_item::Entity")]
	Items,
	#[sea_orm(has_many = "super::remote_source_import::Entity")]
	Imports,
}

impl Related<super::device::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Device.def()
	}
}

impl Related<super::remote_source_item::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Items.def()
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
	use super::{Entity, Relation};
	use sea_orm::{EntityName, RelationTrait};

	#[test]
	fn table_and_relation_names_are_stable() {
		assert_eq!(Entity.table_name(), "remote_sources");
		assert_eq!(
			Relation::Device.def().rel_type,
			sea_orm::RelationType::HasOne
		);
		assert_eq!(
			Relation::Items.def().rel_type,
			sea_orm::RelationType::HasMany
		);
	}
}
