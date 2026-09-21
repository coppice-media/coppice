//! User-scoped recommendation records over immutable work snapshots.
//!
//! A recommendation deliberately stores enough metadata to render after the
//! sender's media/work disappears. `media_id`/`work_id` are hints for the
//! sender's own write path only; readers must render the snapshot fields and
//! re-authorize any current media lookup.

use chrono::Utc;
use sea_orm::{
	entity::prelude::{async_trait::async_trait, *},
	ActiveValue, Condition,
};

pub const PENDING: &str = "PENDING";
pub const ACCEPTED: &str = "ACCEPTED";
pub const DECLINED: &str = "DECLINED";
pub const REVOKED: &str = "REVOKED";
pub const EXPIRED: &str = "EXPIRED";
pub const DISMISSED: &str = "DISMISSED";

pub const TARGET_INTERNAL_MEDIA: &str = "INTERNAL_MEDIA";
pub const TARGET_INTERNAL_WORK: &str = "INTERNAL_WORK";
pub const TARGET_EXTERNAL_WORK: &str = "EXTERNAL_WORK";

pub const HANDOFF_NONE: &str = "NONE";
pub const HANDOFF_REQUESTED: &str = "REQUESTED";
pub const HANDOFF_LINKED: &str = "LINKED";
pub const HANDOFF_FAILED: &str = "FAILED";

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "SocialRecommendationModel"))]
#[sea_orm(table_name = "social_recommendations")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub sender_user_id: String,
	#[sea_orm(column_type = "Text")]
	pub recipient_user_id: String,
	#[sea_orm(column_type = "Text")]
	pub target_kind: String,
	/// Sender-local hints. They are never dereferenced for the recipient.
	#[sea_orm(column_type = "Text", nullable)]
	pub media_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub work_id: Option<String>,
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
	#[sea_orm(column_type = "Text", nullable)]
	pub message: Option<String>,
	#[sea_orm(column_type = "Text")]
	pub state: String,
	#[sea_orm(column_type = "Text")]
	pub handoff_state: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub request_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub destination_shelf_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub destination_device_id: Option<String>,
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
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub dismissed_at: Option<DateTimeWithTimeZone>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::SenderUserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Sender,
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
			if self.handoff_state.is_not_set() {
				self.handoff_state = ActiveValue::Set(HANDOFF_NONE.to_owned());
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

	pub fn find_for_sender(user_id: &str) -> Select<Entity> {
		Self::find().filter(Column::SenderUserId.eq(user_id))
	}

	pub fn find_for_participant(user_id: &str) -> Select<Entity> {
		Self::find().filter(
			Condition::any()
				.add(Column::SenderUserId.eq(user_id))
				.add(Column::RecipientUserId.eq(user_id)),
		)
	}
}
