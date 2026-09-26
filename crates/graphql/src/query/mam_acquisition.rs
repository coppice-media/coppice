use async_graphql::{Context, Object, Result, ID};
use models::{entity::book_request, shared::enums::UserPermission};
use sea_orm::EntityTrait;
use stump_auth::AuthContext;

use crate::{
	data::CoreContext,
	input::book_request::RequestFormat,
	object::mam_acquisition::{AcquisitionGrab, AcquisitionStatus, ReleaseSearch},
};

#[derive(Default)]
pub struct MamAcquisitionQuery;

fn can_acquire(auth: &AuthContext) -> bool {
	auth.user.is_server_owner || auth.user.has_permission(UserPermission::AcquireReleases)
}

fn can_view(auth: &AuthContext, request: &book_request::Model) -> bool {
	can_acquire(auth) || request.requester_id == auth.user.id
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

fn require_acquire(auth: &AuthContext) -> Result<()> {
	if can_acquire(auth) {
		Ok(())
	} else {
		Err(async_graphql::Error::new(
			"release search requires the AcquireReleases permission",
		))
	}
}

#[Object]
impl MamAcquisitionQuery {
	async fn acquisition_status(&self, ctx: &Context<'_>) -> Result<AcquisitionStatus> {
		let auth = ctx.data::<AuthContext>()?;
		require_acquire(auth)?;
		let core = ctx.data::<CoreContext>()?;
		Ok(
			stump_core::mam_acquisition::acquisition_status(&core.config)
				.await
				.into(),
		)
	}
	async fn search_releases(
		&self,
		ctx: &Context<'_>,
		request_id: ID,
		text: Option<String>,
		format: Option<RequestFormat>,
		#[graphql(default = 25)] limit: Option<i32>,
	) -> Result<ReleaseSearch> {
		let auth = ctx.data::<AuthContext>()?;
		require_acquire(auth)?;
		let core = ctx.data::<CoreContext>()?;
		let request = load_request(core, request_id.as_ref()).await?;
		if request.status != "APPROVED" {
			return Err(async_graphql::Error::new(
				"release search requires an approved request",
			));
		}
		Ok(stump_core::mam_acquisition::search_releases(
			core,
			&request,
			text.as_deref(),
			format.map(RequestFormat::as_str),
			limit.unwrap_or(25),
		)
		.await
		.map_err(|error| async_graphql::Error::new(error.to_string()))?
		.into())
	}

	async fn cached_release_search(
		&self,
		ctx: &Context<'_>,
		request_id: ID,
	) -> Result<Option<ReleaseSearch>> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let request = load_request(core, request_id.as_ref()).await?;
		if !can_view(auth, &request) {
			return Err(async_graphql::Error::new(
				"not authorized to view this request",
			));
		}
		Ok(
			stump_core::mam_acquisition::cached_release_search(core, request_id.as_ref())
				.await
				.map_err(|error| async_graphql::Error::new(error.to_string()))?
				.map(Into::into),
		)
	}

	async fn acquisition_grabs(
		&self,
		ctx: &Context<'_>,
		request_id: ID,
	) -> Result<Vec<AcquisitionGrab>> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let request = load_request(core, request_id.as_ref()).await?;
		if !can_view(auth, &request) {
			return Err(async_graphql::Error::new(
				"not authorized to view this request",
			));
		}
		Ok(
			stump_core::mam_acquisition::list_grabs(core, request_id.as_ref())
				.await
				.map_err(|error| async_graphql::Error::new(error.to_string()))?
				.into_iter()
				.map(Into::into)
				.collect(),
		)
	}
}
