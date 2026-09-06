use sea_orm::entity::prelude::*;

/// A binary side object of a liseur-sync annotation: a handwritten markup SVG,
/// its page snapshot, or an exported notebook.
///
/// The row is metadata only. `storage_path` is relative to the server's
/// attachment root (`StumpConfig::get_attachments_dir`), so moving the
/// configuration directory does not invalidate stored rows.
///
/// `user_id` is part of the identity because a liseur annotation id is chosen
/// by the client and is therefore unique per user, not globally; the table's
/// foreign key targets the annotation's `(user_id, annotation_id)` unique
/// index and cascades on delete. Timestamps are RFC 3339 strings with
/// nanosecond precision, matching the liseur-sync tables this hangs off.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "annotation_attachments")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub user_id: String,
	/// The liseur-sync annotation id, not a Stump `media_annotations` id.
	#[sea_orm(column_type = "Text")]
	pub annotation_id: String,
	/// `markup-svg`, `markup-page`, or `notebook`.
	#[sea_orm(column_type = "Text")]
	pub kind: String,
	#[sea_orm(column_type = "Text")]
	pub media_type: String,
	pub byte_size: i64,
	/// Lowercase hex SHA-256 of the stored bytes; the idempotency key.
	#[sea_orm(column_type = "Text")]
	pub sha256: String,
	#[sea_orm(column_type = "Text")]
	pub storage_path: String,
	#[sea_orm(column_type = "Text")]
	pub created_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
