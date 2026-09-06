use async_graphql::{Context, Object, Result, ID};
use stump_auth::AuthContext;

use crate::{
	data::CoreContext,
	object::annotation::{AnnotationSink, AnnotationSyncStatus},
};

#[derive(Default)]
pub struct AnnotationQuery;

#[Object]
impl AnnotationQuery {
	/// The compiled-in annotation export sinks and their setting schemas.
	async fn annotation_sinks(&self) -> Vec<AnnotationSink> {
		stump_annotation_sync::registry::catalog()
			.into_iter()
			.map(AnnotationSink::from)
			.collect()
	}

	/// The annotation sync state for a user: configured sinks, last runs,
	/// errors, and whether a debounced export is pending. Defaults to the
	/// calling user; other users require the server owner.
	async fn annotation_sync_status(
		&self,
		ctx: &Context<'_>,
		user_id: Option<ID>,
	) -> Result<AnnotationSyncStatus> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let user_id = resolve_target_user(auth, user_id)?;
		build_status(core, &user_id).await
	}
}

/// The user an annotation sync operation targets: the caller unless a server
/// owner names someone else.
pub(crate) fn resolve_target_user(
	auth: &AuthContext,
	user_id: Option<ID>,
) -> Result<String> {
	match user_id {
		Some(id) if id.as_str() != auth.user.id => {
			if auth.user.is_server_owner {
				Ok(id.to_string())
			} else {
				Err(
					"Only the server owner may target another user's annotation sync"
						.into(),
				)
			}
		},
		_ => Ok(auth.user.id.clone()),
	}
}

pub(crate) async fn build_status(
	core: &CoreContext,
	user_id: &str,
) -> Result<AnnotationSyncStatus> {
	let rows = stump_core::annotation_sync::sink_status_rows(core.conn.as_ref(), user_id)
		.await
		.map_err(|error| async_graphql::Error::new(error.to_string()))?;

	Ok(AnnotationSyncStatus {
		user_id: user_id.to_owned(),
		pending: core.annotation_sync_pending(user_id),
		sinks: rows.into_iter().map(Into::into).collect(),
	})
}
