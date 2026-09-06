use async_graphql::{Context, Result, Subscription, Union};
use stump_core::event::{
	CoreEvent, ProviderCatalogRefreshed, ProviderSeriesMaterialized,
	ProviderSourceHealthChanged,
};
use tokio::sync::broadcast::error::RecvError;

use crate::data::CoreContext;

/// The provider lane of [`CoreEvent`], as one union a client can exhaust.
///
/// The members are the same object types `readEvents` yields, so the schema
/// registers each exactly once and a client's `... on` selections are portable
/// between the two subscriptions.
#[derive(Union)]
pub enum ProviderEvent {
	SourceHealthChanged(ProviderSourceHealthChanged),
	SeriesMaterialized(ProviderSeriesMaterialized),
	CatalogRefreshed(ProviderCatalogRefreshed),
}

#[derive(Default)]
pub struct ProviderSubscription;

#[Subscription]
impl ProviderSubscription {
	/// Provider host activity: source health transitions, materialised
	/// series, and catalog refreshes. Ungated, like `readEvents`, which
	/// carries the same events unfiltered.
	async fn provider_events(
		&self,
		ctx: &Context<'_>,
	) -> Result<impl futures_util::Stream<Item = Result<ProviderEvent>>> {
		let mut rx = ctx.data::<CoreContext>()?.get_client_receiver();

		Ok(async_stream::stream! {
			loop {
				match rx.recv().await {
					Ok(CoreEvent::ProviderSourceHealthChanged(changed)) => {
						yield Ok(ProviderEvent::SourceHealthChanged(changed));
					},
					Ok(CoreEvent::ProviderSeriesMaterialized(materialized)) => {
						yield Ok(ProviderEvent::SeriesMaterialized(materialized));
					},
					Ok(CoreEvent::ProviderCatalogRefreshed(refreshed)) => {
						yield Ok(ProviderEvent::CatalogRefreshed(refreshed));
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
