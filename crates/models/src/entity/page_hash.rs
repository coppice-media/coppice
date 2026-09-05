use sea_orm::entity::prelude::*;

/// One perceptual difference hash (dHash, 8×8) per analyzed image page.
///
/// Rows are written by the media analysis job and read by the
/// `duplicate_pages_across_books` quality check, the duplicate-page candidate
/// aggregation, and the serve-time `visible_pages` filter. `dhash` stores the
/// `u64` hash bit-cast to `i64`; `page` is 1-based like every page route.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "page_hashes")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub media_id: String,
	#[sea_orm(primary_key, auto_increment = false)]
	pub page: i32,
	pub dhash: i64,
	pub created_at: DateTimeWithTimeZone,
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
}

impl Related<super::media::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Media.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
