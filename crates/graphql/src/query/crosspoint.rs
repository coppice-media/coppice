//! GraphQL queries for owned CrossPoint targets and durable deliveries.

use async_graphql::{Context, Error, Object, Result, ID};
use models::{
	entity::{crosspoint_delivery_queue, crosspoint_device_target, media},
	shared::{enums::DeviceKind, visibility::VisibilityScope},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::{
	data::CoreContext,
	object::crosspoint::{CrosspointDelivery, CrosspointTarget},
};

#[derive(Default)]
pub struct CrosspointQuery;

#[Object]
impl CrosspointQuery {
	/// Return the verified CrossPoint target owned by this authenticated user.
	async fn crosspoint_target(
		&self,
		ctx: &Context<'_>,
		device_id: ID,
	) -> Result<Option<CrosspointTarget>> {
		let (core, user, _) = owned_crosspoint_device(ctx, &device_id).await?;
		let target = crosspoint_device_target::Entity::find_by_id(device_id.to_string())
			.one(core.conn.as_ref())
			.await?;
		let Some(target) = target else {
			return Ok(None);
		};
		if target.user_id != user.id {
			return Err(Error::new(
				"CrossPoint target is not owned by the authenticated user",
			));
		}
		CrosspointTarget::from_model(target)
			.map(Some)
			.map_err(|error| Error::new(error))
	}

	/// Return durable delivery history for one owned CrossPoint device.
	async fn crosspoint_deliveries(
		&self,
		ctx: &Context<'_>,
		device_id: ID,
		limit: Option<i32>,
	) -> Result<Vec<CrosspointDelivery>> {
		let (core, user, _) = owned_crosspoint_device(ctx, &device_id).await?;
		let limit = match limit {
			Some(limit) if limit > 0 => limit.min(1_000) as u64,
			Some(_) => {
				return Err(Error::new("CrossPoint delivery limit must be positive"))
			},
			None => 100,
		};
		let rows = crosspoint_delivery_queue::Entity::find()
			.filter(crosspoint_delivery_queue::Column::UserId.eq(&user.id))
			.filter(crosspoint_delivery_queue::Column::DeviceId.eq(device_id.to_string()))
			.order_by_desc(crosspoint_delivery_queue::Column::QueuedAt)
			.limit(limit)
			.all(core.conn.as_ref())
			.await?;
		Ok(rows.into_iter().map(CrosspointDelivery::from).collect())
	}
}

/// Resolve an authenticated, non-revoked CrossPoint device. Cross-user access
/// is deliberately rejected even for the server owner: these mutations and
/// snapshots are user-owned protocol state, not an administrative surface.
pub(crate) async fn owned_crosspoint_device<'a>(
	ctx: &'a Context<'_>,
	device_id: &ID,
) -> Result<(
	CoreContext,
	&'a models::entity::user::AuthUser,
	stump_devices::Device,
)> {
	let stump_auth::AuthContext { user, .. } = ctx.data::<stump_auth::AuthContext>()?;
	let core = ctx.data::<CoreContext>()?;
	let device = core.devices().get(user, &device_id.to_string()).await?;
	if device.kind != DeviceKind::Crosspoint {
		return Err(Error::new("device is not a CrossPoint device"));
	}
	if device.is_revoked() {
		return Err(Error::new("CrossPoint device has been revoked"));
	}
	if device.user_id != user.id {
		return Err(Error::new(
			"CrossPoint device is not owned by the authenticated user",
		));
	}
	Ok((core.clone(), user, device))
}

/// Query media through the selected device's effective scope. This keeps a
/// session request from queueing a book that the target device is not allowed
/// to see, while retaining the user's own exclusion and age filters.
pub(crate) async fn visible_media_ids_for_device(
	core: &CoreContext,
	user: &models::entity::user::AuthUser,
	device: &stump_devices::Device,
	media_ids: &[String],
) -> Result<Vec<String>> {
	let scope_ids = device.library_scope_ids();
	let scope = match scope_ids.as_deref() {
		Some(ids) => VisibilityScope::scoped(user, ids),
		None => VisibilityScope::inherit(user),
	};
	Ok(media::Entity::find_for_user(scope)
		.select_only()
		.column(media::Column::Id)
		.filter(media::Column::Id.is_in(media_ids.iter().map(String::as_str)))
		.into_tuple::<String>()
		.all(core.conn.as_ref())
		.await?)
}
