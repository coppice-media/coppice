//! Reading the send-to-Kindle history.
//!
//! A delivery is visible to whoever can see the device it went to, so the
//! query resolves the caller's devices first and never filters on rows the
//! registry would have hidden. That also gives the device name each row
//! reports without a join.

use std::collections::HashMap;

use async_graphql::{Context, Object, Result, ID};
use models::entity::kindle_destination;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::{
	data::CoreContext,
	mutation::kindle::{KindleDelivery, KindleDestination, KindleDestinationDelivery},
};
/// The newest deliveries returned when the caller names no limit, and the
/// ceiling on what it may ask for: the surfaces are a device card and a book
/// row, neither of which pages.
const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 100;

#[derive(Default)]
pub struct KindleQuery;

#[Object]
impl KindleQuery {
	/// User-owned destinations. Email is shown because it is not a secret and
	/// is the address Amazon uses; server SMTP credentials are never returned.
	async fn kindle_destinations(
		&self,
		ctx: &Context<'_>,
	) -> Result<Vec<KindleDestination>> {
		let core = ctx.data::<CoreContext>()?;
		let user = &ctx.data::<stump_auth::AuthContext>()?.user;
		Ok(kindle_destination::Entity::find()
			.filter(kindle_destination::Column::UserId.eq(&user.id))
			.order_by_desc(kindle_destination::Column::IsDefault)
			.order_by_asc(kindle_destination::Column::Name)
			.all(core.conn.as_ref())
			.await?
			.into_iter()
			.map(Into::into)
			.collect())
	}

	/// Legacy/device delivery history. This intentionally excludes
	/// destination-only rows so existing clients keep their device-scoped
	/// non-null semantics; use `kindleDestinationDeliveries` for the new lane.
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
			.filter_map(|row| {
				let name = row
					.device_id
					.as_ref()
					.and_then(|id| names.get(id))
					.cloned()
					.unwrap_or_default();
				KindleDelivery::from_legacy(row, name)
			})
			.collect())
	}

	/// Destination delivery history, scoped by the durable owner snapshot so
	/// deleting a destination retains and continues to expose its history.
	async fn kindle_destination_deliveries(
		&self,
		ctx: &Context<'_>,
		media_id: Option<ID>,
		limit: Option<u64>,
	) -> Result<Vec<KindleDestinationDelivery>> {
		let core = ctx.data::<CoreContext>()?;
		let user = &ctx.data::<stump_auth::AuthContext>()?.user;
		let rows = stump_kindle::deliveries_for_user(
			core.conn.as_ref(),
			&user.id,
			&[],
			media_id.as_ref().map(|id| id.as_str()),
			limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
		)
		.await?;
		Ok(rows
			.into_iter()
			.filter_map(KindleDestinationDelivery::from_row)
			.collect())
	}
}
