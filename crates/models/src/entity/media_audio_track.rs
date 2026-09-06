//! The ordered files that make up one audiobook: `media_audio_tracks`.
//!
//! A folder audiobook is many files that together form a single publication,
//! so the files cannot be `media` rows and cannot be a JSON blob: a range
//! request has to resolve a publication-relative offset to exactly one file,
//! which is a lookup.

use sea_orm::{entity::prelude::*, ActiveValue};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "MediaAudioTrackModel"))]
#[sea_orm(table_name = "media_audio_tracks")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub media_id: String,
	/// 0-based position of the file within the publication. Unique per media.
	pub index: i32,
	/// Absolute path of the file on disk.
	#[sea_orm(column_type = "Text")]
	pub path: String,
	pub duration_ms: i64,
	/// Running sum of the preceding `duration_ms` values, stored so a
	/// publication-relative position maps to a file with one indexed read.
	pub start_offset_ms: i64,
	/// Size of the file in bytes, kept so a range request needs no `stat`.
	pub byte_size: i64,
	#[sea_orm(column_type = "Text")]
	pub mime: String,
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
