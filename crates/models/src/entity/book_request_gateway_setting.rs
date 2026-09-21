use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "book_request_gateway_settings")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub endpoint: String,
	/// Encrypted with the server's configured encryption key; never exposed via GraphQL.
	#[sea_orm(column_type = "Text")]
	pub encrypted_token: String,
	pub enabled: bool,
	pub require_approval: bool,
	pub automation_enabled: bool,
	pub scoring_floor: i32,
	pub verification_threshold: i32,
	pub max_retries: i32,
	#[sea_orm(column_type = "Text", nullable)]
	pub handoff_root: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub updated_by: Option<String>,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

#[async_trait]
impl ActiveModelBehavior for ActiveModel {
	async fn before_save<C>(mut self, _db: &C, _insert: bool) -> Result<Self, DbErr>
	where
		C: ConnectionTrait,
	{
		self.updated_at = Set(DateTimeWithTimeZone::from(Utc::now()));
		Ok(self)
	}
}
