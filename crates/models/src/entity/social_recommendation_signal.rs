//! Durable, privacy-safe signals used by local adaptive recommendations.
//!
//! `target_key` is an opaque normalized work identity, never a user id. Signal
//! explanations use `reason_code` and a public snapshot only.

use chrono::Utc;
use sea_orm::{
	entity::prelude::{async_trait::async_trait, *},
	ActiveValue,
};

pub const READ: &str = "READ";
pub const RATING: &str = "RATING";
pub const ACCEPT: &str = "ACCEPT";
pub const DECLINE: &str = "DECLINE";
pub const DISMISS: &str = "DISMISS";
pub const REQUEST: &str = "REQUEST";

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "SocialRecommendationSignalModel"))]
#[sea_orm(table_name = "social_recommendation_signals")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub user_id: String,
	#[sea_orm(column_type = "Text")]
	pub target_key: String,
	#[sea_orm(column_type = "Text")]
	pub signal_kind: String,
	pub weight: i32,
	#[sea_orm(column_type = "Text", nullable)]
	pub recommendation_id: Option<String>,
	#[sea_orm(column_type = "Text")]
	pub reason_code: String,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
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
	#[sea_orm(
		belongs_to = "super::social_recommendation::Entity",
		from = "Column::RecommendationId",
		to = "super::social_recommendation::Column::Id",
		on_update = "Cascade",
		on_delete = "SetNull"
	)]
	Recommendation,
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
		if insert {
			if self.id.is_not_set() {
				self.id = ActiveValue::Set(uuid::Uuid::new_v4().to_string());
			}
			if self.created_at.is_not_set() {
				self.created_at = ActiveValue::Set(Utc::now().into());
			}
		}
		Ok(self)
	}
}

impl Entity {
	pub fn find_for_user(user_id: &str) -> Select<Entity> {
		Self::find().filter(Column::UserId.eq(user_id))
	}
}
