//! The audio facts of one audiobook: `media_audio`, keyed by `media_id`.
//!
//! An audiobook is a `media` row like any other book, so its audio-only facts
//! live here rather than as permanently-null columns on `media`. The primary
//! key is `media_id` itself because the relation is strictly 1:1 — one
//! publication, one duration, one codec.

use sea_orm::entity::prelude::*;

use crate::domain::audio::AudioChapterSource;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "MediaAudioModel"))]
#[sea_orm(table_name = "media_audio")]
pub struct Model {
	/// The media this audio belongs to; also the primary key.
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub media_id: String,
	/// Duration of the whole publication in milliseconds — the sum of the
	/// track durations for a multi-file book. Reading positions are expressed
	/// against this value.
	pub duration_ms: i64,
	/// The codec of the audio, as a short lowercase name (`aac`, `mp3`,
	/// `opus`, `flac`). A folder book with mixed codecs reports `mixed`.
	#[sea_orm(column_type = "Text")]
	pub codec: String,
	pub sample_rate: Option<i32>,
	pub channels: Option<i32>,
	/// Bits per second, average where the container states one and derived
	/// from size over duration otherwise.
	pub bitrate: Option<i32>,
	/// How the rows in [`super::media_audio_chapter`] were obtained. This is
	/// provenance, never a preference.
	#[sea_orm(column_type = "Text")]
	pub chapter_source: AudioChapterSource,
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
