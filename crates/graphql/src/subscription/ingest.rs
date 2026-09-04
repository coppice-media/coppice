use async_graphql::{Context, Error, ErrorExtensions, Subscription, ID};
use futures_util::{stream, Stream, StreamExt};
use models::shared::enums::UserPermission;

use crate::{
	data::CoreContext, guard::PermissionGuard, object::ingest::IngestProgressEvent,
};

#[derive(Default)]
pub struct IngestSubscription;

#[Subscription]
impl IngestSubscription {
	#[graphql(
		guard = "PermissionGuard::new(&[UserPermission::ReadJobs, UserPermission::MetadataFetchRecordRead])"
	)]
	async fn ingest_progress(
		&self,
		ctx: &Context<'_>,
		library_id: Option<ID>,
		drop_item_id: Option<ID>,
		analysis_job_id: Option<ID>,
		after_event_id: Option<ID>,
	) -> impl Stream<Item = async_graphql::Result<IngestProgressEvent>> {
		let core = match ctx.data::<CoreContext>() {
			Ok(core) => core.clone(),
			Err(error) => {
				let message = error.message.clone();
				return stream::once(async move { Err(Error::new(message)) }).boxed();
			},
		};
		let library_id = library_id.map(|id| id.to_string());
		let drop_item_id = drop_item_id.map(|id| id.to_string());
		let analysis_job_id = analysis_job_id.map(|id| id.to_string());
		let after_event_id = after_event_id.map(|id| id.to_string());
		let result = core
			.ingest()
			.coordinator
			.subscribe_stored(
				library_id.as_deref(),
				drop_item_id.as_deref(),
				analysis_job_id.as_deref(),
				after_event_id.as_deref(),
			)
			.await;
		match result {
			Ok(events) => events
				.map(|result| match result {
					Ok(stored) => Ok(IngestProgressEvent::from_stored(
						stored.event,
						stored.emitted_at,
					)),
					Err(error) => {
						Err(Error::new(error.to_string()).extend_with(|_, extensions| {
							extensions.set("code", "CURSOR_EXPIRED");
						}))
					},
				})
				.boxed(),
			Err(error) => stream::once(async move {
				Err(Error::new(error.to_string()).extend_with(|_, extensions| {
					extensions.set("code", "CURSOR_EXPIRED");
				}))
			})
			.boxed(),
		}
	}
}
