use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "collections")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub name: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub description: Option<String>,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub updated_at: DateTimeWithTimeZone,
	pub ordered: bool,
	/// Whether this collection is projected to Kobo devices as a shelf `Tag`.
	#[sea_orm(default_value = true)]
	pub kobo_shelf: bool,
	/// Device that last wrote this collection through the Kobo `tags`
	/// write-back endpoints; `None` for native/Komga writes.
	#[sea_orm(column_type = "Text", nullable)]
	pub source_device: Option<String>,
	#[sea_orm(column_type = "Text")]
	pub creating_user_id: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(has_many = "super::collection_series::Entity")]
	CollectionSeries,
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::CreatingUserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	User,
}

impl Related<super::collection_series::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::CollectionSeries.def()
	}
}

impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::User.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
