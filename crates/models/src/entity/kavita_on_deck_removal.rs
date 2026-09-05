use sea_orm::entity::prelude::*;


/// A series a user removed from their Kavita on-deck dashboard.
///
/// The Kavita equivalent is `AppUserOnDeckRemoval` (`SeriesRepository.
/// RemoveFromOnDeckAsync`): the entry hides the series from on-deck until the
/// next read event on that series clears it (`ClearOnDeckRemovalAsync`).
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "kavita_on_deck_removals")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub user_id: String,
	/// The Stump series id.
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub series_id: String,
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
		belongs_to = "super::series::Entity",
		from = "Column::SeriesId",
		to = "super::series::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Series,
}

impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::User.def()
	}
}

impl Related<super::series::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Series.def()
	}
}

impl Entity {
	pub fn find_for_user(user_id: &str) -> Select<Entity> {
		Entity::find().filter(Column::UserId.eq(user_id))
	}
}

impl ActiveModelBehavior for ActiveModel {}
