use async_graphql::{Context, Object, Result, ID};
use models::entity::{
	book_request, book_request_gateway_setting, book_request_grab, book_request_handoff,
	book_request_release,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use stump_auth::AuthContext;

use crate::{
	data::CoreContext,
	object::book_request::{
		BookRequest, BookRequestGatewaySettings, BookRequestGrab, BookRequestHandoff,
		BookRequestRelease, BookRequestStatus,
	},
};

#[derive(Default)]
pub struct BookRequestQuery;

fn can_manage(auth: &AuthContext) -> bool {
	auth.user.is_server_owner
		|| auth
			.user
			.has_permission(models::shared::enums::UserPermission::ManageServer)
		|| auth
			.user
			.has_permission(models::shared::enums::UserPermission::ManageLibrary)
}

fn can_view(auth: &AuthContext, request: &book_request::Model) -> bool {
	can_manage(auth) || request.requester_id == auth.user.id
}

async fn load_request(core: &CoreContext, id: &str) -> Result<book_request::Model> {
	book_request::Entity::find_by_id(id)
		.one(core.conn.as_ref())
		.await?
		.ok_or_else(|| async_graphql::Error::new("book request not found"))
}

#[Object]
impl BookRequestQuery {
	async fn book_request(
		&self,
		ctx: &Context<'_>,
		id: ID,
	) -> Result<Option<BookRequest>> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let Some(request) = book_request::Entity::find_by_id(id.as_ref())
			.one(core.conn.as_ref())
			.await?
		else {
			return Ok(None);
		};
		if !can_view(auth, &request) {
			return Err(async_graphql::Error::new(
				"not authorized to view this request",
			));
		}
		Ok(Some(request.into()))
	}

	async fn book_requests(
		&self,
		ctx: &Context<'_>,
		status: Option<BookRequestStatus>,
		#[graphql(default = false)] mine_only: bool,
		#[graphql(default = 50)] limit: i32,
		#[graphql(default = 0)] offset: i32,
	) -> Result<Vec<BookRequest>> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let manager = can_manage(auth);
		if !manager && !mine_only {
			// Ordinary users may only enumerate their own ledger rows.
		}
		let mut query = book_request::Entity::find();
		if !manager || mine_only {
			query = query.filter(book_request::Column::RequesterId.eq(&auth.user.id));
		}
		if let Some(status) = status {
			query = query.filter(book_request::Column::Status.eq(status.as_str()));
		}
		let limit = limit.clamp(1, 200) as u64;
		let offset = offset.max(0) as u64;
		Ok(query
			.order_by_desc(book_request::Column::CreatedAt)
			.offset(offset)
			.limit(limit)
			.all(core.conn.as_ref())
			.await?
			.into_iter()
			.map(Into::into)
			.collect())
	}

	async fn book_request_releases(
		&self,
		ctx: &Context<'_>,
		request_id: ID,
	) -> Result<Vec<BookRequestRelease>> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let request = load_request(core, request_id.as_ref()).await?;
		if !can_view(auth, &request) {
			return Err(async_graphql::Error::new(
				"not authorized to view this request",
			));
		}
		Ok(book_request_release::Entity::find()
			.filter(book_request_release::Column::RequestId.eq(request_id.as_ref()))
			.order_by_asc(book_request_release::Column::Rank)
			.all(core.conn.as_ref())
			.await?
			.into_iter()
			.map(Into::into)
			.collect())
	}

	async fn book_request_grabs(
		&self,
		ctx: &Context<'_>,
		request_id: ID,
	) -> Result<Vec<BookRequestGrab>> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let request = load_request(core, request_id.as_ref()).await?;
		if !can_view(auth, &request) {
			return Err(async_graphql::Error::new(
				"not authorized to view this request",
			));
		}
		Ok(book_request_grab::Entity::find()
			.filter(book_request_grab::Column::RequestId.eq(request_id.as_ref()))
			.order_by_desc(book_request_grab::Column::CreatedAt)
			.all(core.conn.as_ref())
			.await?
			.into_iter()
			.map(Into::into)
			.collect())
	}

	async fn book_request_handoffs(
		&self,
		ctx: &Context<'_>,
		request_id: ID,
	) -> Result<Vec<BookRequestHandoff>> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let request = load_request(core, request_id.as_ref()).await?;
		if !can_view(auth, &request) {
			return Err(async_graphql::Error::new(
				"not authorized to view this request",
			));
		}
		Ok(book_request_handoff::Entity::find()
			.filter(book_request_handoff::Column::RequestId.eq(request_id.as_ref()))
			.order_by_desc(book_request_handoff::Column::CreatedAt)
			.all(core.conn.as_ref())
			.await?
			.into_iter()
			.map(Into::into)
			.collect())
	}

	async fn book_request_gateway(
		&self,
		ctx: &Context<'_>,
	) -> Result<Option<BookRequestGatewaySettings>> {
		let auth = ctx.data::<AuthContext>()?;
		if !can_manage(auth) {
			return Err(async_graphql::Error::new(
				"gateway settings require administrator access",
			));
		}
		let core = ctx.data::<CoreContext>()?;
		Ok(book_request_gateway_setting::Entity::find_by_id("default")
			.one(core.conn.as_ref())
			.await?
			.map(Into::into))
	}
}
