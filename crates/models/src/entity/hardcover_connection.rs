use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue};
use serde_json::Value;

/// One encrypted, user-owned Hardcover credential and its redacted sync state.
/// Raw PATs never derive into GraphQL and are only decrypted at the request
/// boundary when a provider call is explicitly requested by that same user.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "hardcover_connections")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub user_id: String,
	#[sea_orm(column_type = "Text")]
	pub encrypted_api_token: String,
	pub credential_version: i64,
	#[sea_orm(column_type = "Text", nullable)]
	pub remote_user_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub remote_username: Option<String>,
	#[sea_orm(column_type = "Json", nullable)]
	pub scopes: Option<Value>,
	#[sea_orm(column_type = "Json", nullable)]
	pub capabilities: Option<Value>,
	pub use_for_metadata: bool,
	pub import_journals: bool,
	pub sync_progress: bool,
	pub connected_at: DateTimeWithTimeZone,
	pub verified_at: Option<DateTimeWithTimeZone>,
	pub last_sync_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "Text", nullable)]
	pub last_error: Option<String>,
	pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::UserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	User,
}

impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::User.def()
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
			if self.credential_version.is_not_set() {
				self.credential_version = ActiveValue::Set(1);
			}
			if self.connected_at.is_not_set() {
				self.connected_at = ActiveValue::Set(now.clone());
			}
		}
		self.updated_at = ActiveValue::Set(now);
		Ok(self)
	}
}
