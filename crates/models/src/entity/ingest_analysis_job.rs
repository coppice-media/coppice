use crate::shared::enums::JobStatus;
use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};
use serde_json::Value as JsonValue;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "ingest_analysis_jobs")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub drop_item_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub media_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub job_id: Option<String>,
	pub status: JobStatus,
	#[sea_orm(column_type = "Text")]
	pub phase: String,
	pub priority: i32,
	pub attempts: i32,
	#[sea_orm(column_type = "Json")]
	pub plan: JsonValue,
	#[sea_orm(column_type = "Text", nullable)]
	pub error: Option<String>,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub started_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub finished_at: Option<DateTimeWithTimeZone>,
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
			if self.status.is_not_set() {
				self.status = Set(JobStatus::Queued);
			}
			if self.phase.is_not_set() {
				self.phase = Set("STAGING".to_string());
			}
			if self.priority.is_not_set() {
				self.priority = Set(0);
			}
			if self.attempts.is_not_set() {
				self.attempts = Set(0);
			}
			if self.plan.is_not_set() {
				self.plan = Set(JsonValue::Object(Default::default()));
			}
		}
		Ok(self)
	}
}
