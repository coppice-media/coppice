use async_graphql::{Context, Object, Result, ID};
use models::services::annotation_attachment as rows;
use stump_auth::AuthContext;

use crate::{data::CoreContext, object::annotation_attachment::AnnotationAttachment};

#[derive(Default)]
pub struct AnnotationAttachmentQuery;

#[Object]
impl AnnotationAttachmentQuery {
	/// The attachments of one liseur-sync annotation, oldest first.
	///
	/// Always the caller's own: a liseur annotation id is chosen by the client
	/// and is therefore unique per user, so there is no server-wide annotation
	/// to target. An unknown annotation has no attachments and returns an
	/// empty list rather than an error.
	async fn annotation_attachments(
		&self,
		ctx: &Context<'_>,
		annotation_id: ID,
	) -> Result<Vec<AnnotationAttachment>> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		Ok(
			rows::list(core.conn.as_ref(), &auth.user.id, annotation_id.as_str())
				.await?
				.into_iter()
				.map(AnnotationAttachment::from)
				.collect(),
		)
	}
}
