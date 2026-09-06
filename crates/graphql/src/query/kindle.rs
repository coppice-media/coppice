//! Reading the send-to-Kindle history.
//!
//! A delivery is visible to whoever can see the device it went to, so the
//! query resolves the caller's devices first and never filters on rows the
//! registry would have hidden. That also gives the device name each row
//! reports without a join.

use std::collections::HashMap;

use async_graphql::{Context, Object, Result, ID};

use crate::{data::CoreContext, mutation::kindle::KindleDelivery};

/// The newest deliveries returned when the caller names no limit, and the
/// ceiling on what it may ask for: the surfaces are a device card and a book
/// row, neither of which pages.
const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 100;

#[derive(Default)]
pub struct KindleQuery;

#[Object]
impl KindleQuery {
	/// Books mailed to a Kindle, newest first.
	///
	/// Scoped to the devices the caller can see: their own, or every device
	/// for the server owner. `deviceId` narrows it to one device and
	/// `mediaId` to one book; a device the caller cannot see resolves to no
	/// rows rather than to an error, exactly like the registry's own lookups.
	async fn kindle_deliveries(
		&self,
		ctx: &Context<'_>,
		device_id: Option<ID>,
		media_id: Option<ID>,
		limit: Option<u64>,
	) -> Result<Vec<KindleDelivery>> {
		let core = ctx.data::<CoreContext>()?;
		let user = &ctx.data::<stump_auth::AuthContext>()?.user;
		let devices = core.devices();

		let visible = match &device_id {
			// One device: the registry decides whether the caller may see it.
			Some(id) => match devices.get(user, id.as_str()).await {
				Ok(device) => vec![device],
				Err(stump_devices::DeviceError::NotFound) => return Ok(Vec::new()),
				Err(error) => return Err(error.into()),
			},
			None => devices.list(user).await?,
		};
		let names = visible
			.iter()
			.map(|device| (device.id.clone(), device.name.clone()))
			.collect::<HashMap<_, _>>();
		let ids = visible
			.into_iter()
			.map(|device| device.id)
			.collect::<Vec<_>>();

		let rows = stump_kindle::deliveries(
			core.conn.as_ref(),
			&ids,
			media_id.as_ref().map(|id| id.as_str()),
			limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
		)
		.await?;

		Ok(rows
			.into_iter()
			.map(|row| {
				let name = names.get(&row.device_id).cloned().unwrap_or_default();
				KindleDelivery::new(row, name)
			})
			.collect())
	}
}
