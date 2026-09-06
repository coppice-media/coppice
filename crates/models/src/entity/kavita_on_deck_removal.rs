use sea_orm::entity::prelude::*;

/// A Kavita series a user removed from their Kavita on-deck dashboard.
///
/// The Kavita equivalent is `AppUserOnDeckRemoval` (`SeriesRepository.
/// RemoveFromOnDeckAsync`): the entry hides the series from on-deck until the
/// next read event on that series clears it (`ClearOnDeckRemovalAsync`).
///
/// A Kavita series is backed by a Stump series in Manga/Comic libraries and by
/// a single media item in Book/LightNovel libraries, so the target is stored as
/// `(target_kind, target_id)`: [`Model::TARGET_SERIES`] with a series id or
/// [`Model::TARGET_MEDIA`] with a media id. Targets carry no foreign key; the
/// Kavita read-event path deletes rows, and a row for a deleted target hides
/// nothing because the target no longer lists.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "kavita_on_deck_removals")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub user_id: String,
	/// [`Model::TARGET_SERIES`] or [`Model::TARGET_MEDIA`].
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub target_kind: String,
	/// The Stump series or media id, according to `target_kind`.
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub target_id: String,
	pub created_at: DateTimeWithTimeZone,
}

impl Model {
	/// `target_id` is a Stump series id (Manga/Comic libraries).
	pub const TARGET_SERIES: &'static str = "series";
	/// `target_id` is a Stump media id presented as its own Kavita series
	/// (Book/LightNovel libraries).
	pub const TARGET_MEDIA: &'static str = "media";
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

impl Entity {
	pub fn find_for_user(user_id: &str) -> Select<Entity> {
		Entity::find().filter(Column::UserId.eq(user_id))
	}
}

impl ActiveModelBehavior for ActiveModel {}
