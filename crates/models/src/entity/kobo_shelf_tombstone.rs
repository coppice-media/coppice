use chrono::Utc;
use sea_orm::entity::prelude::*;
use sea_orm::ActiveValue;
/// A shelf (container) that was deleted while it was projected to Kobo
/// devices. A device's next incremental sync emits a `DeletedTag` for every
/// tombstone in its window, so shelves removed natively disappear from the
/// device without a full resync. Rows are pruned opportunistically once they
/// are older than any reasonable sync window.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "kobo_shelf_tombstones")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub user_id: String,
	/// The shelf id (== the container id) that the device saw.
	#[sea_orm(column_type = "Text")]
	pub shelf_id: String,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub deleted_at: DateTimeWithTimeZone,
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

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
	async fn before_save<C>(mut self, _db: &C, insert: bool) -> Result<Self, DbErr>
	where
		C: ConnectionTrait,
	{
		if insert {
			self.deleted_at = ActiveValue::Set(DateTimeWithTimeZone::from(Utc::now()));
			if self.id.is_not_set() {
				self.id = ActiveValue::Set(Uuid::new_v4().to_string());
			}
		}

		Ok(self)
	}
}
