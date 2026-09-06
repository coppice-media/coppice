use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};
use serde_json::Value as JsonValue;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "ingest_drop_items")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub library_id: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub created_by: Option<String>,
	#[sea_orm(column_type = "Text")]
	pub source_filename: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub relative_path: Option<String>,
	pub byte_size: i64,
	#[sea_orm(column_type = "Text")]
	pub source_sha256: String,
	#[sea_orm(column_type = "Text")]
	pub media_kind: String,
	#[sea_orm(column_type = "Text")]
	pub staging_path: String,
	#[sea_orm(column_type = "Text")]
	pub status: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub analysis_job_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub quality_report_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub media_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub series_id: Option<String>,
	#[sea_orm(column_type = "Json", nullable)]
	pub pending_fields: Option<JsonValue>,
	#[sea_orm(column_type = "Text", nullable)]
	pub error: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub idempotency_key: Option<String>,
	/// When the optional ingest preprocess hook last succeeded for this item.
	/// `NULL` = never (including after a failed hook), which is what keeps the
	/// hook to one successful run per item across retries and re-analysis.
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub preprocessed_at: Option<DateTimeWithTimeZone>,
	pub revision: i32,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub updated_at: DateTimeWithTimeZone,
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
		belongs_to = "super::user::Entity",
		from = "Column::CreatedBy",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "SetNull"
	)]
	CreatedBy,
	#[sea_orm(
		belongs_to = "super::media::Entity",
		from = "Column::MediaId",
		to = "super::media::Column::Id",
		on_update = "Cascade",
		on_delete = "SetNull"
	)]
	Media,
	#[sea_orm(
		belongs_to = "super::series::Entity",
		from = "Column::SeriesId",
		to = "super::series::Column::Id",
		on_update = "Cascade",
		on_delete = "SetNull"
	)]
	Series,
	/// The latest quality report (`quality_report_id`); older reports stay as
	/// audit history on the report table.
	#[sea_orm(
		belongs_to = "super::ingest_quality_report::Entity",
		from = "Column::QualityReportId",
		to = "super::ingest_quality_report::Column::Id"
	)]
	QualityReport,
}

impl Related<super::library::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Library.def()
	}
}

impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::CreatedBy.def()
	}
}

impl Related<super::media::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Media.def()
	}
}

impl Related<super::series::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Series.def()
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
			if self.revision.is_not_set() {
				self.revision = Set(1);
			}
			if self.status.is_not_set() {
				self.status = Set("RECEIVED".to_string());
			}
		} else {
			self.updated_at = Set(DateTimeWithTimeZone::from(Utc::now()));
		}
		Ok(self)
	}
}
