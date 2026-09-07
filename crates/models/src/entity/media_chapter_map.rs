//! The tier-1 chapter map of one ebook↔audiobook pair: `media_chapter_map`.
//!
//! One row says "spine item *n* of the ebook is chapter *m* of the audiobook",
//! which is everything read-time position conversion needs: an audio offset
//! becomes a fraction of its chapter, and that fraction lands at the same
//! fraction of the mapped spine item's characters (and back). The map is the
//! only storage tier 1 has — no cues, no transcript.
//!
//! It is keyed by the two media rows rather than by a pair id: a pair is two
//! `liseur_sync_media_links` rows sharing a `work_id` and has no row of its
//! own, so there is nothing else to point at. `confidence` is per entry
//! because the two ways of matching are not equally trustworthy (an exact
//! normalised-title hit versus an ordinal fallback), and the console shows the
//! weak ones for review instead of hiding them.

use sea_orm::{entity::prelude::*, ActiveValue};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "MediaChapterMapModel"))]
#[sea_orm(table_name = "media_chapter_map")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	/// The ebook edition. The ebook is always the canonical *text* side of a
	/// pair, so it owns the spine index half of the mapping.
	#[sea_orm(column_type = "Text")]
	pub ebook_media_id: String,
	/// The audio edition, the canonical *time* side.
	#[sea_orm(column_type = "Text")]
	pub audio_media_id: String,
	/// 0-based index into the ebook's linear spine, as
	/// `stump_media::media::readium` enumerates it for `positions.json`.
	pub ebook_spine_index: i32,
	/// 0-based `media_audio_chapters.index` of the mapped chapter.
	pub audio_chapter_index: i32,
	/// `0..=1`. `1.0` is an exact normalised-title match, lower values come
	/// from the ordinal fallback; front and back matter with no counterpart
	/// has no row at all rather than a low-confidence one.
	pub confidence: f64,
	pub created_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::media::Entity",
		from = "Column::EbookMediaId",
		to = "super::media::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	EbookMedia,
	#[sea_orm(
		belongs_to = "super::media::Entity",
		from = "Column::AudioMediaId",
		to = "super::media::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	AudioMedia,
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
