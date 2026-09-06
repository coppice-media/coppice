use async_graphql::{Context, Error, ErrorExtensions, Result, Subscription, ID};
use futures_util::{stream, Stream, StreamExt};
use models::shared::enums::UserPermission;
use stump_core::event::{CoreEvent, IngestItemChanged};
use tokio::sync::broadcast::error::RecvError;

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

	/// Drop-item row changes, optionally narrowed to one library. One event
	/// per persisted `revision` bump, which is what lets a client hold a drop
	/// queue live instead of refetching it on a timer. Ungated, like
	/// `readEvents`, which carries the same events unfiltered.
	async fn ingest_events(
		&self,
		ctx: &Context<'_>,
		library_id: Option<ID>,
	) -> Result<impl Stream<Item = Result<IngestItemChanged>>> {
		let mut rx = ctx.data::<CoreContext>()?.get_client_receiver();
		let library_id = library_id.map(|id| id.to_string());

		Ok(async_stream::stream! {
			loop {
				match rx.recv().await {
					Ok(CoreEvent::IngestItemChanged(changed)) => {
						let wanted = library_id
							.as_deref()
							.is_none_or(|wanted| wanted == changed.library_id);
						if wanted {
							yield Ok(changed);
						}
					},
					Ok(_) => {},
					// A slow consumer skips what it could not keep up with
					// instead of losing the subscription.
					Err(RecvError::Lagged(_)) => {},
					Err(RecvError::Closed) => break,
				}
			}
		})
	}
}
