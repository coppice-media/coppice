use async_graphql::{Context, Object, Result, ID};
use models::entity::book_request;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use stump_auth::AuthContext;

use crate::{
	data::CoreContext,
	object::book_request::{BookRequest, BookRequestStatus},
};

#[derive(Default)]
pub struct BookRequestQuery;

pub(super) fn can_manage(auth: &AuthContext) -> bool {
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
}
