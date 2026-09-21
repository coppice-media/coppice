//! User-owned reviews for a work or one unpaired media edition.
//!
//! The legacy `reviews` table remains available to older account-import code. New
//! book-detail writes use this table so a review has exactly one explicit target:
//! either a confirmed liseur work or a standalone media id.

use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "book_reviews")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	/// Exactly one of `work_id` and `media_id` is set.
	#[sea_orm(column_type = "Text", nullable)]
	pub work_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub media_id: Option<String>,
	#[sea_orm(column_type = "Text")]
	pub user_id: String,
	/// A star rating from 0 (unrated) through 5.
	pub rating: i32,
	#[sea_orm(column_type = "Text", nullable)]
	pub content: Option<String>,
	pub is_private: bool,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::media::Entity",
		from = "Column::MediaId",
		to = "super::media::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Media,
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::UserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	User,
	#[sea_orm(
		belongs_to = "super::liseur_sync_work::Entity",
		from = "Column::WorkId",
		to = "super::liseur_sync_work::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Work,
}

impl Related<super::media::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Media.def()
	}
}

impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::User.def()
	}
}

impl Related<super::liseur_sync_work::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Work.def()
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

impl Entity {
	pub fn find_for_user(user_id: &str) -> Select<Entity> {
		Self::find().filter(Column::UserId.eq(user_id))
	}

	pub fn find_for_media(user_id: &str, media_id: &str) -> Select<Entity> {
		Self::find_for_user(user_id)
			.filter(Column::MediaId.eq(media_id))
			.filter(Column::WorkId.is_null())
	}

	pub fn find_for_work(user_id: &str, work_id: &str) -> Select<Entity> {
		Self::find_for_user(user_id)
			.filter(Column::WorkId.eq(work_id))
			.filter(Column::MediaId.is_null())
	}
}
