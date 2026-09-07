//! A work: `liseur_sync_works`.
//!
//! The identity an edition pair hangs from. Neither ISBN nor ASIN is a safe
//! key on its own — print, ebook and audio editions carry different ISBNs, and
//! an audiobook usually carries an Audible ASIN and no ISBN at all — so the
//! work is what "the same book" means, and `liseur_sync_media_links` is how a
//! media row joins one.
//!
//! Rows are per user: the table is written by the native liseur-sync lane
//! (`POST /v1/works/resolve`) from whatever the device asserted, and by
//! `stump_library::editions` when a suggestion needs a work to point at.
//! `pending` marks a work created from a client assertion that has not been
//! reconciled with real metadata yet.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "liseur_sync_works")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub user_id: String,
	#[sea_orm(column_type = "Text")]
	pub title: String,
	#[sea_orm(column_type = "Text")]
	pub author: String,
	pub pending: bool,
	/// RFC 3339 string, as the liseur storage writes it.
	#[sea_orm(column_type = "Text")]
	pub created_at: String,
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
	#[sea_orm(has_many = "super::liseur_sync_media_link::Entity")]
	MediaLinks,
}

impl Related<super::liseur_sync_media_link::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::MediaLinks.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
