use async_graphql::{Context, Object, Result, ID};
use models::entity::{hardcover_connection, hardcover_media_link};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::{
	data::CoreContext,
	guard::ServerOwnerGuard,
	object::hardcover::{HardcoverConnection, HardcoverMediaLink},
};

#[derive(Default)]
pub struct HardcoverQuery;

#[Object]
impl HardcoverQuery {
	/// Redacted status for the current user's own Hardcover connection.
	async fn hardcover_connection(
		&self,
		ctx: &Context<'_>,
	) -> Result<Option<HardcoverConnection>> {
		let core = ctx.data::<CoreContext>()?;
		let user = &ctx.data::<stump_auth::AuthContext>()?.user;
		Ok(hardcover_connection::Entity::find_by_id(user.id.clone())
			.one(core.conn.as_ref())
			.await?
			.map(Into::into))
	}

	/// Explicit media links are personal and therefore always scoped to the
	/// authenticated owner, even when a server owner is viewing the UI.
	async fn hardcover_media_links(
		&self,
		ctx: &Context<'_>,
		media_id: Option<ID>,
	) -> Result<Vec<HardcoverMediaLink>> {
		let core = ctx.data::<CoreContext>()?;
		let user = &ctx.data::<stump_auth::AuthContext>()?.user;
		let mut query = hardcover_media_link::Entity::find()
			.filter(hardcover_media_link::Column::UserId.eq(&user.id))
			.order_by_asc(hardcover_media_link::Column::LinkedAt);
		if let Some(media_id) = media_id {
			query = query
				.filter(hardcover_media_link::Column::MediaId.eq(media_id.to_string()));
		}
		Ok(query
			.all(core.conn.as_ref())
			.await?
			.into_iter()
			.map(Into::into)
			.collect())
	}
	#[graphql(guard = "ServerOwnerGuard")]
	async fn annotation_sync_root(&self, ctx: &Context<'_>) -> Result<Option<String>> {
		let core = ctx.data::<CoreContext>()?;
		Ok(core
			.config
			.annotation_sync
			.annotation_sync_root
			.clone()
			.or_else(|| {
				Some(
					core.config
						.get_annotation_sync_root()
						.to_string_lossy()
						.to_string(),
				)
			}))
	}
}
