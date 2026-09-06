use async_graphql::{SimpleObject, ID};
use models::{
	entity::annotation_attachment::Model, services::annotation_attachment as rows,
};

/// A binary side object of a liseur-sync annotation.
///
/// Attachments are keyed by annotation id and never take part in the
/// annotation's compare-and-set revision: uploading one does not change the
/// record a device replicates. Bytes are fetched from
/// `GET /api/v2/annotations/{annotationId}/attachments/{id}` with the caller's
/// session or its liseur device credential.
#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct AnnotationAttachment {
	pub id: ID,
	/// The liseur-sync annotation this hangs off, not a native
	/// `media_annotations` id.
	pub annotation_id: ID,
	/// `markup-svg` (handwritten strokes), `markup-page` (the page snapshot
	/// they were drawn on), or `notebook` (a device notebook export).
	pub kind: String,
	pub media_type: String,
	pub byte_size: i64,
	/// Lowercase hex SHA-256 of the bytes; also the upload idempotency key.
	pub sha256: String,
	/// The filename extension the bytes are stored under, so a client can
	/// name a download without parsing the media type.
	pub extension: String,
	pub created_at: String,
}

impl From<Model> for AnnotationAttachment {
	fn from(model: Model) -> Self {
		let extension = rows::extension_for(&model.media_type).to_owned();
		Self {
			id: ID(model.id),
			annotation_id: ID(model.annotation_id),
			kind: model.kind,
			media_type: model.media_type,
			byte_size: model.byte_size,
			sha256: model.sha256,
			extension,
			created_at: model.created_at,
		}
	}
}
