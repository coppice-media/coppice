//! The chapter marks of one audiobook: `media_audio_chapters`.
//!
//! Chapters are publication-relative time marks, not files: an embedded `chpl`
//! atom can put four chapters inside a single `.m4b`, and a folder book can
//! have one chapter per file. Keeping them in their own table is what lets
//! both shapes coexist.

use sea_orm::{entity::prelude::*, ActiveValue};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "MediaAudioChapterModel"))]
#[sea_orm(table_name = "media_audio_chapters")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub media_id: String,
	/// 0-based ordinal of the chapter. Unique per media.
	pub index: i32,
	#[sea_orm(column_type = "Text", nullable)]
	pub title: Option<String>,
	/// Offset from the start of the *publication*, the same unit as
	/// `reading_heads.position_ms`.
	pub start_ms: i64,
	/// Nullable: most containers only carry start marks and the last chapter
	/// runs to `media_audio.duration_ms`.
	pub end_ms: Option<i64>,
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

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
	async fn before_save<C>(mut self, _db: &C, insert: bool) -> Result<Self, DbErr>
	where
		C: ConnectionTrait,
	{
		if insert && self.id.is_not_set() {
			self.id = ActiveValue::Set(uuid::Uuid::new_v4().to_string());
		}
		Ok(self)
	}
}
