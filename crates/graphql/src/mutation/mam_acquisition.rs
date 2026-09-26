use async_graphql::{Context, Object, Result, ID};
use models::{entity::book_request, shared::enums::UserPermission};
use sea_orm::EntityTrait;
use stump_auth::AuthContext;

use crate::{data::CoreContext, object::mam_acquisition::AcquisitionGrab};

#[derive(Default)]
pub struct MamAcquisitionMutation;

fn can_acquire(auth: &AuthContext) -> bool {
	auth.user.is_server_owner || auth.user.has_permission(UserPermission::AcquireReleases)
}

async fn load_request(
	core: &CoreContext,
	request_id: &str,
) -> Result<book_request::Model> {
	book_request::Entity::find_by_id(request_id)
		.one(core.conn.as_ref())
		.await?
		.ok_or_else(|| async_graphql::Error::new("book request not found"))
}

#[Object]
impl MamAcquisitionMutation {
	async fn grab_release(
		&self,
		ctx: &Context<'_>,
		request_id: ID,
		torrent_id: i32,
		confirm: bool,
	) -> Result<AcquisitionGrab> {
		let auth = ctx.data::<AuthContext>()?;
		if !can_acquire(auth) {
			return Err(async_graphql::Error::new(
				"grabbing releases requires the AcquireReleases permission",
			));
		}
		if !confirm {
			return Err(async_graphql::Error::new(
				"confirm must be true before a release can be grabbed",
			));
		}
		let core = ctx.data::<CoreContext>()?;
		let request = load_request(core, request_id.as_ref()).await?;
		if request.status != "APPROVED" {
			return Err(async_graphql::Error::new(
				"grabbing releases requires an approved request",
			));
		}
		Ok(
			stump_core::mam_acquisition::create_grab(core, &request, torrent_id)
				.await
				.map_err(|error| async_graphql::Error::new(error.to_string()))?
				.into(),
		)
	}
}
