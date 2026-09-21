use crate::guard::PermissionGuard;
use crate::{
	data::CoreContext,
	object::emailer::{Emailer, SmtpSettings},
};
use async_graphql::{Context, Object, Result};
use models::{entity::emailer, shared::enums::UserPermission};
use sea_orm::prelude::*;

#[derive(Default)]
pub struct EmailerQuery;

#[Object]
impl EmailerQuery {
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageServer)")]
	async fn emailers(&self, ctx: &Context<'_>) -> Result<Vec<Emailer>> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		Ok(emailer::Entity::find()
			.all(conn)
			.await?
			.into_iter()
			.map(Emailer::from)
			.collect())
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageServer)")]
	async fn emailer_by_id(&self, ctx: &Context<'_>, id: i32) -> Result<Option<Emailer>> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let emailer = emailer::Entity::find_by_id(id).one(conn).await?;
		Ok(emailer.map(Emailer::from))
	}
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageServer)")]
	async fn smtp_settings(&self, ctx: &Context<'_>) -> Result<SmtpSettings> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let primary = emailer::Entity::find()
			.filter(emailer::Column::IsPrimary.eq(true))
			.one(conn)
			.await?;
		Ok(primary.into())
	}
}
