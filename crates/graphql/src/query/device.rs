use async_graphql::{Context, Object, Result};
use stump_devices::Endpoint;

use crate::{data::CoreContext, object::device::Device};

#[derive(Default)]
pub struct DeviceQuery;

#[Object]
impl DeviceQuery {
	/// The devices registered to the current user; every device for the server owner.
	async fn devices(&self, ctx: &Context<'_>) -> Result<Vec<Device>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let devices = core.devices().list(user).await?;
		Ok(devices.into_iter().map(Device::from).collect())
	}

	async fn device(&self, ctx: &Context<'_>, id: String) -> Result<Device> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		Ok(Device::from(core.devices().get(user, &id).await?))
	}

	/// What to configure on the device for the server the request arrived at,
	/// with the secret redacted; the full secret is only returned when a
	/// credential is minted.
	async fn device_endpoints(
		&self,
		ctx: &Context<'_>,
		id: String,
	) -> Result<Vec<Endpoint>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let origin = ctx.data::<stump_api_types::RequestOrigin>()?;

		Ok(core.devices().endpoints(user, &id, origin).await?)
	}
}
