use async_graphql::{Context, Object, Result};
use chrono::Utc;
use models::{
	entity::device_pairing::{self, DevicePairingStatus},
	shared::enums::UserPermission,
};
use sea_orm::{prelude::*, QueryOrder};

use crate::{data::CoreContext, guard::PermissionGuard, object::device_pairing::DevicePairing};

#[derive(Default)]
pub struct DevicePairingQuery;

#[Object]
impl DevicePairingQuery {
	/// Pairings waiting for approval, oldest first. Pending pairings are not yet
	/// owned by anyone, so every user allowed to create device credentials sees
	/// them; approving binds the pairing to the approver.
	#[graphql(guard = "PermissionGuard::one(UserPermission::AccessApiKeys)")]
	async fn pending_device_pairings(
		&self,
		ctx: &Context<'_>,
	) -> Result<Vec<DevicePairing>> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let now = DateTimeWithTimeZone::from(Utc::now());

		let pairings = device_pairing::Entity::find()
			.filter(
				device_pairing::Column::Status
					.eq(DevicePairingStatus::Pending)
					.and(device_pairing::Column::ExpiresAt.gt(now)),
			)
			.order_by_asc(device_pairing::Column::CreatedAt)
			.all(conn)
			.await?;

		Ok(pairings
			.into_iter()
			.map(|pairing| DevicePairing::from_model(pairing, now))
			.collect())
	}
}
