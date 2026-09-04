use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};
use serde_json::Value as JsonValue;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "ingest_plugin_settings")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub plugin_id: String,
	#[sea_orm(column_type = "Text")]
	pub kind: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub library_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub user_id: Option<String>,
	pub enabled: bool,
	pub opted_in: bool,
	#[sea_orm(column_type = "Json", nullable)]
	pub values: Option<JsonValue>,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

#[async_trait]
impl ActiveModelBehavior for ActiveModel {
	async fn before_save<C>(mut self, _db: &C, insert: bool) -> Result<Self, DbErr>
	where
		C: ConnectionTrait,
	{
		if insert && self.id.is_not_set() {
			self.id = Set(uuid::Uuid::new_v4().to_string());
		}
		self.updated_at = Set(DateTimeWithTimeZone::from(Utc::now()));
		Ok(self)
	}
}
