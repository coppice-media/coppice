//! Which work a media row is an edition of: `liseur_sync_media_links`.
//!
//! The table is written by the native liseur-sync lane
//! (`POST /v1/works/resolve`, raw SQL in
//! `apps/server/src/routers/liseur_sync/storage.rs`) and read by edition
//! pairing, which is why it has an entity at all: pairing needs to join it
//! against `media` and `media_metadata`, and a work with two links is the only
//! representation of "this audiobook is that ebook" the tree needs.
//!
//! `uq-liseur-sync-media-links-user-media` makes a media row belong to at most
//! one work per user, so a pair is exactly two rows sharing `(user_id,
//! work_id)`.
//!
//! Two columns are pairing's, not liseur's: [`Column::PairStatus`] and
//! [`Column::PairEvidence`]. `resolution_status` cannot carry them — it means
//! "was the edition hash verified against the file" and the liseur lane
//! rewrites it on every re-resolve. See
//! [`crate::domain::edition_pair`] for the state machine.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "liseur_sync_media_links")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub user_id: String,
	#[sea_orm(column_type = "Text")]
	pub media_id: String,
	#[sea_orm(column_type = "Text")]
	pub work_id: String,
	/// The liseur edition digest of the file this link was resolved from;
	/// empty when the link was created by pairing, which identifies editions
	/// by metadata rather than by content hash.
	#[sea_orm(column_type = "Text")]
	pub edition_sha: String,
	/// `verified` / `unverified`: whether `edition_sha` was recomputed from
	/// the file. Owned by the liseur lane.
	#[sea_orm(column_type = "Text")]
	pub resolution_status: String,
	/// RFC 3339 string, as the liseur storage writes it.
	#[sea_orm(column_type = "Text")]
	pub created_at: String,
	/// `confirmed` / `suggested` / `rejected`; see
	/// [`crate::domain::edition_pair::PairStatus`]. Defaults to `confirmed`
	/// in the schema so a liseur `INSERT` that does not name it keeps
	/// meaning what it always did.
	#[sea_orm(column_type = "Text")]
	pub pair_status: String,
	/// Why pairing believes this link, as a
	/// [`crate::domain::edition_pair::PairEvidence`] discriminant. Null on
	/// links the liseur lane created.
	#[sea_orm(column_type = "Text", nullable)]
	pub pair_evidence: Option<String>,
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
		belongs_to = "super::media::Entity",
		from = "Column::MediaId",
		to = "super::media::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Media,
	#[sea_orm(
		belongs_to = "super::liseur_sync_work::Entity",
		from = "Column::WorkId",
		to = "super::liseur_sync_work::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Work,
}

impl Related<super::media::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Media.def()
	}
}

impl Related<super::liseur_sync_work::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Work.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
