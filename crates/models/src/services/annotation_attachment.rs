//! Persistence for the binary side objects of a liseur-sync annotation.
//!
//! The rows are owned by this service; the bytes are written and served by the
//! application, which is the only layer that knows the configured attachment
//! root. [`relative_path`] is therefore the single definition of the on-disk
//! layout: `<annotation_id>/<sha256>.<ext>` below that root.
//!
//! Attachments are side objects keyed by annotation id. Storing one never
//! touches the annotation's revision or feed sequence; the compare-and-set
//! contract of `liseur_sync_annotations` is unaffected.

use chrono::{SecondsFormat, Utc};
use sea_orm::{
	ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter,
	QueryOrder,
};
use uuid::Uuid;

use crate::entity::annotation_attachment::{
	ActiveModel, Column, Entity, Model as Attachment,
};

/// The handwritten stroke layer of a Kobo on-page markup.
pub const KIND_MARKUP_SVG: &str = "markup-svg";
/// The rendered page snapshot a markup was drawn on.
pub const KIND_MARKUP_PAGE: &str = "markup-page";
/// A device notebook export.
pub const KIND_NOTEBOOK: &str = "notebook";

/// Every accepted attachment kind, in wire order.
pub const KINDS: [&str; 3] = [KIND_MARKUP_SVG, KIND_MARKUP_PAGE, KIND_NOTEBOOK];

/// The bytes an upload wants to store, already digested by the caller.
#[derive(Clone, Debug)]
pub struct NewAttachment {
	pub user_id: String,
	pub annotation_id: String,
	pub kind: String,
	pub media_type: String,
	pub byte_size: i64,
	pub sha256: String,
}

/// What an [`upsert`] did to the `(annotation, kind)` slot.
#[derive(Clone, Debug)]
pub enum Upserted {
	/// The slot was empty.
	Created(Attachment),
	/// The slot held a different digest; `previous` is no longer referenced.
	Replaced {
		attachment: Attachment,
		previous: Attachment,
	},
	/// The same digest was already stored: the upload was a retry.
	Unchanged(Attachment),
}

impl Upserted {
	pub fn attachment(&self) -> &Attachment {
		match self {
			Self::Created(attachment)
			| Self::Replaced { attachment, .. }
			| Self::Unchanged(attachment) => attachment,
		}
	}
}

/// The stored path of an attachment, relative to the attachment root.
pub fn relative_path(annotation_id: &str, sha256: &str, media_type: &str) -> String {
	format!("{annotation_id}/{sha256}.{}", extension_for(media_type))
}

/// The filename extension bytes of `media_type` are stored under. Unknown
/// types are stored as `bin` rather than rejected here: what the wire accepts
/// is the protocol's decision, not the storage layout's.
pub fn extension_for(media_type: &str) -> &'static str {
	match media_type {
		"image/svg+xml" => "svg",
		"image/jpeg" => "jpg",
		"image/png" => "png",
		"application/pdf" => "pdf",
		"application/epub+zip" => "epub",
		"application/zip" => "zip",
		"application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
			"docx"
		},
		"text/html" => "html",
		"text/plain" => "txt",
		_ => "bin",
	}
}

/// Every attachment of one annotation, oldest first.
pub async fn list<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	annotation_id: &str,
) -> Result<Vec<Attachment>, DbErr> {
	Entity::find()
		.filter(Column::UserId.eq(user_id))
		.filter(Column::AnnotationId.eq(annotation_id))
		.order_by_asc(Column::CreatedAt)
		.order_by_asc(Column::Id)
		.all(conn)
		.await
}

/// One attachment by id, scoped to its owner and annotation.
pub async fn find<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	annotation_id: &str,
	attachment_id: &str,
) -> Result<Option<Attachment>, DbErr> {
	Entity::find_by_id(attachment_id)
		.filter(Column::UserId.eq(user_id))
		.filter(Column::AnnotationId.eq(annotation_id))
		.one(conn)
		.await
}

/// The attachment currently occupying an annotation's `kind` slot.
pub async fn find_by_kind<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	annotation_id: &str,
	kind: &str,
) -> Result<Option<Attachment>, DbErr> {
	Entity::find()
		.filter(Column::UserId.eq(user_id))
		.filter(Column::AnnotationId.eq(annotation_id))
		.filter(Column::Kind.eq(kind))
		.one(conn)
		.await
}

/// Store `new` in its `(annotation, kind)` slot.
///
/// A retry of the same digest is idempotent and keeps the original id, so a
/// client replaying a batch never accumulates rows. A different digest
/// replaces the slot and the caller is handed the row whose bytes it must
/// unlink.
pub async fn upsert<C: ConnectionTrait>(
	conn: &C,
	new: NewAttachment,
) -> Result<Upserted, DbErr> {
	let existing =
		find_by_kind(conn, &new.user_id, &new.annotation_id, &new.kind).await?;
	let storage_path = relative_path(&new.annotation_id, &new.sha256, &new.media_type);

	if let Some(previous) = existing {
		if previous.sha256 == new.sha256 && previous.media_type == new.media_type {
			return Ok(Upserted::Unchanged(previous));
		}
		let attachment = ActiveModel {
			id: Set(previous.id.clone()),
			user_id: Set(new.user_id),
			annotation_id: Set(new.annotation_id),
			kind: Set(new.kind),
			media_type: Set(new.media_type),
			byte_size: Set(new.byte_size),
			sha256: Set(new.sha256),
			storage_path: Set(storage_path),
			created_at: Set(now_string()),
		};
		let attachment = Entity::update(attachment).exec(conn).await?;
		return Ok(Upserted::Replaced {
			attachment,
			previous,
		});
	}

	let attachment = ActiveModel {
		id: Set(Uuid::new_v4().to_string()),
		user_id: Set(new.user_id),
		annotation_id: Set(new.annotation_id),
		kind: Set(new.kind),
		media_type: Set(new.media_type),
		byte_size: Set(new.byte_size),
		sha256: Set(new.sha256),
		storage_path: Set(storage_path),
		created_at: Set(now_string()),
	};
	Ok(Upserted::Created(
		Entity::insert(attachment).exec_with_returning(conn).await?,
	))
}

/// Drop every attachment row of one annotation and hand the caller the rows
/// whose bytes it must unlink. Used when an annotation is tombstoned: the
/// record's content is gone, so its side objects go with it.
pub async fn delete_for_annotation<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	annotation_id: &str,
) -> Result<Vec<Attachment>, DbErr> {
	let attachments = list(conn, user_id, annotation_id).await?;
	if attachments.is_empty() {
		return Ok(attachments);
	}
	Entity::delete_many()
		.filter(Column::UserId.eq(user_id))
		.filter(Column::AnnotationId.eq(annotation_id))
		.exec(conn)
		.await?;
	Ok(attachments)
}

fn now_string() -> String {
	Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn path_is_content_addressed_below_the_annotation() {
		assert_eq!(
			relative_path("ks1-abc", "a".repeat(64).as_str(), "image/svg+xml"),
			format!("ks1-abc/{}.svg", "a".repeat(64))
		);
		assert_eq!(
			relative_path("ks1-abc", "b".repeat(64).as_str(), "image/jpeg"),
			format!("ks1-abc/{}.jpg", "b".repeat(64))
		);
	}

	#[test]
	fn unknown_media_types_still_have_a_stable_extension() {
		assert_eq!(extension_for("application/x-nebo"), "bin");
		assert_eq!(extension_for("application/pdf"), "pdf");
	}
}
