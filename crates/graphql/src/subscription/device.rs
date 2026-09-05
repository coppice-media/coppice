use async_graphql::{Context, Result, Subscription};
use stump_core::CoreEvent;
use stump_devices::DeviceSeen;
use tokio::sync::broadcast::error::RecvError;

use crate::data::CoreContext;

#[derive(Default)]
pub struct DeviceSubscription;

#[Subscription]
impl DeviceSubscription {
	/// Sightings of the current user's devices (every device for the server
	/// owner), optionally narrowed to one device.
	async fn device_seen(
		&self,
		ctx: &Context<'_>,
		device_id: Option<String>,
	) -> Result<impl futures_util::Stream<Item = Result<DeviceSeen>>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let mut rx = ctx.data::<CoreContext>()?.get_client_receiver();
		let user_id = user.id.clone();
		let sees_all = user.is_server_owner;

		Ok(async_stream::stream! {
			loop {
				match rx.recv().await {
					Ok(CoreEvent::DeviceSeen(seen)) => {
						let mine = sees_all || seen.user_id == user_id;
						let wanted = device_id
							.as_deref()
							.is_none_or(|wanted| wanted == seen.device_id);
						if mine && wanted {
							yield Ok(seen);
						}
					},
					Ok(_) => {},
					// A slow consumer skips sightings it could not keep up with
					// instead of losing the subscription.
					Err(RecvError::Lagged(_)) => {},
					Err(RecvError::Closed) => break,
				}
			}
		})
	}
}
