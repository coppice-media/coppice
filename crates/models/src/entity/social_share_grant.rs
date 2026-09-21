//! Explicit, revocable cross-user sharing grants.
//!
//! A recommendation acceptance never implies a grant. The recipient must
//! accept this row independently and each read re-checks its state and expiry.

use chrono::Utc;
use sea_orm::{
	entity::prelude::{async_trait::async_trait, *},
	ActiveValue, Condition,
};

pub const PENDING: &str = "PENDING";
pub const ACTIVE: &str = "ACTIVE";
pub const DECLINED: &str = "DECLINED";
pub const REVOKED: &str = "REVOKED";
pub const EXPIRED: &str = "EXPIRED";

pub const SCOPE_METADATA: i32 = 1;
pub const SCOPE_RATING: i32 = 1 << 1;
pub const SCOPE_REVIEW_TEXT: i32 = 1 << 2;
pub const SCOPE_COMPLETION: i32 = 1 << 3;
pub const SCOPE_PERCENTAGE: i32 = 1 << 4;
pub const SCOPE_ANNOTATIONS: i32 = 1 << 5;
pub const VALID_SCOPES: i32 = SCOPE_METADATA
	| SCOPE_RATING
	| SCOPE_REVIEW_TEXT
	| SCOPE_COMPLETION
	| SCOPE_PERCENTAGE
	| SCOPE_ANNOTATIONS;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "SocialShareGrantModel"))]
#[sea_orm(table_name = "social_share_grants")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub target_key: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub target_media_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub target_work_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub source_provider: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub remote_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub external_key: Option<String>,
	#[sea_orm(column_type = "Text")]
	pub title: String,
	#[sea_orm(column_type = "Text")]
	pub authors: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub cover_url: Option<String>,
	#[sea_orm(column_type = "Text")]
	pub source_user_id: String,
	#[sea_orm(column_type = "Text")]
	pub recipient_user_id: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub recommendation_id: Option<String>,
	pub scope_mask: i32,
	#[sea_orm(column_type = "Text")]
	pub state: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub idempotency_key: Option<String>,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub expires_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub accepted_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub declined_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub revoked_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "Text", nullable)]
	pub revoked_by_user_id: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::SourceUserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Source,
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::RecipientUserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Recipient,
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
			if self.state.is_not_set() {
				self.state = ActiveValue::Set(PENDING.to_owned());
			}
			if self.created_at.is_not_set() {
				self.created_at = ActiveValue::Set(Utc::now().into());
			}
		}
		Ok(self)
	}
}

impl Entity {
	pub fn find_for_recipient(user_id: &str) -> Select<Entity> {
		Self::find().filter(Column::RecipientUserId.eq(user_id))
	}

	pub fn find_for_source(user_id: &str) -> Select<Entity> {
		Self::find().filter(Column::SourceUserId.eq(user_id))
	}

	pub fn find_for_participant(user_id: &str) -> Select<Entity> {
		Self::find().filter(
			Condition::any()
				.add(Column::SourceUserId.eq(user_id))
				.add(Column::RecipientUserId.eq(user_id)),
		)
	}
}
