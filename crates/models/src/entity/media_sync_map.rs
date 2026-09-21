//! One validated tier-2 ebook↔audiobook timing artifact: `media_sync_maps`.
//!
//! Rows are immutable cache identities over the two media rows, granularity,
//! exact input digests, and generator version. A repeated import updates the
//! artifact payload for that identity rather than creating another row. The
//! worker job id is intentionally nullable: operator imports have no job.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "media_sync_maps")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub ebook_media_id: String,
	#[sea_orm(column_type = "Text")]
	pub audio_media_id: String,
	/// Serialized `stump_worker::AlignGranularity` (`sentence`/`word`).
	#[sea_orm(column_type = "Text")]
	pub granularity: String,
	/// `SyncMapProvenance::implementation`.
	#[sea_orm(column_type = "Text")]
	pub generator: String,
	/// `SyncMapProvenance::version`.
	#[sea_orm(column_type = "Text")]
	pub generator_version: String,
	pub algorithm: Option<String>,
	pub model: Option<String>,
	pub text_digest: String,
	pub audio_manifest_digest: String,
	pub cue_count: i32,
	pub coverage: Option<f64>,
	/// The complete validated `SyncMapV1`, retained for future readers.
	#[sea_orm(column_type = "Json")]
	pub map: Json,
	/// `import` or `job`.
	#[sea_orm(column_type = "Text")]
	pub source: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub job_id: Option<String>,
	pub created_at: DateTimeWithTimeZone,
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

impl ActiveModelBehavior for ActiveModel {}
