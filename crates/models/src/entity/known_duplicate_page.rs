use sea_orm::entity::prelude::*;

use crate::shared::enums::DuplicatePageAction;

/// A librarian decision about one recurring page hash inside a library.
///
/// `SKIP` hides every page whose hash is within the duplicate tolerance from
/// all page-serving routes; `KEEP` records that the page was reviewed and is
/// legitimate so it stops being reported as a candidate. Files are never
/// modified.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "known_duplicate_pages")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub library_id: String,
	#[sea_orm(primary_key, auto_increment = false)]
	pub dhash: i64,
	#[sea_orm(column_type = "Text")]
	pub action: DuplicatePageAction,
	#[sea_orm(column_type = "Text", nullable)]
	pub created_by: Option<String>,
	pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::library::Entity",
		from = "Column::LibraryId",
		to = "super::library::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Library,
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::CreatedBy",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "SetNull"
	)]
	User,
}

impl Related<super::library::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Library.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
