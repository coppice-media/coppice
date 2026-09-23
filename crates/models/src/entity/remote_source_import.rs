//! A durable, explicit approval proposal for importing one verified source item.
//!
//! Matching is advisory only.  The immutable worker/content/digest/size fields
//! are checked again before an operator decision can link or stage the item.

use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};
use serde_json::Value as JsonValue;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "remote_source_imports")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub source_id: String,
	#[sea_orm(column_type = "Text")]
	pub source_item_id: String,
	#[sea_orm(column_type = "Text")]
	pub target_media_id: String,
	#[sea_orm(column_type = "Text")]
	pub worker_item_id: String,
	#[sea_orm(column_type = "Text")]
	pub worker_content_version: String,
	#[sea_orm(column_type = "Text")]
	pub source_sha256: String,
	pub source_size: i64,
	#[sea_orm(column_type = "Text")]
	pub match_kind: String,
	pub match_score: i32,
	#[sea_orm(column_type = "Json")]
	pub match_evidence: JsonValue,
	#[sea_orm(column_type = "Text")]
	pub status: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub decision_actor_id: Option<String>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub decision_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "Text", nullable)]
	pub decision_reason: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub applied_location_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub staged_item_id: Option<String>,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub updated_at: DateTimeWithTimeZone,
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
		belongs_to = "super::remote_source_item::Entity",
		from = "Column::SourceItemId",
		to = "super::remote_source_item::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	SourceItem,
	#[sea_orm(
		belongs_to = "super::media::Entity",
		from = "Column::TargetMediaId",
		to = "super::media::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	TargetMedia,
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::DecisionActorId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "SetNull"
	)]
	DecisionActor,
}

impl Related<super::remote_source::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Source.def()
	}
}

impl Related<super::remote_source_item::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::SourceItem.def()
	}
}

impl Related<super::media::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::TargetMedia.def()
	}
}

impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::DecisionActor.def()
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
			if self.status.is_not_set() {
				self.status = Set("proposed".to_owned());
			}
			if self.created_at.is_not_set() {
				self.created_at = Set(DateTimeWithTimeZone::from(Utc::now()));
			}
			if self.updated_at.is_not_set() {
				self.updated_at = Set(DateTimeWithTimeZone::from(Utc::now()));
			}
		} else {
			self.updated_at = Set(DateTimeWithTimeZone::from(Utc::now()));
		}
		Ok(self)
	}
}
