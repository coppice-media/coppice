//! Explicit metadata owned by a confirmed liseur work.
//!
//! Work metadata is deliberately separate from edition metadata. Applying a
//! work field therefore never mirrors an audiobook value onto an ebook (or the
//! reverse); a caller must select a destination scope explicitly.

use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "book_work_metadata")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub work_id: String,
	#[sea_orm(column_type = "Text")]
	pub user_id: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub title: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub author: Option<String>,
	/// Additional fields are kept as a JSON object so work metadata can grow
	/// without adding columns or pretending an edition field is shared.
	#[sea_orm(column_type = "Json", nullable)]
	pub metadata: Option<Json>,
	#[sea_orm(column_type = "Json", nullable)]
	pub locked_fields: Option<Json>,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::liseur_sync_work::Entity",
		from = "Column::WorkId",
		to = "super::liseur_sync_work::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Work,
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::UserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	User,
}

impl Related<super::liseur_sync_work::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Work.def()
	}
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
		if insert && self.created_at.is_not_set() {
			self.created_at = Set(now);
		}
		self.updated_at = Set(now);
		Ok(self)
	}
}

impl Entity {
	pub fn find_for_user(user_id: &str) -> Select<Entity> {
		Self::find().filter(Column::UserId.eq(user_id))
	}
}
