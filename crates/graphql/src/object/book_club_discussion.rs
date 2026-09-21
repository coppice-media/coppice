use crate::{data::CoreContext, object::book_club_book::BookClubBook};
use async_graphql::{ComplexObject, Context, Result, SimpleObject};
use models::entity::{
	book_club_discussion, book_club_discussion_message, book_club_member,
};
use sea_orm::{prelude::*, ColumnTrait, EntityTrait, QueryFilter};

#[derive(Debug, SimpleObject)]
#[graphql(complex)]
pub struct BookClubDiscussion {
	#[graphql(flatten)]
	model: book_club_discussion::Model,
}

impl From<book_club_discussion::Model> for BookClubDiscussion {
	fn from(model: book_club_discussion::Model) -> Self {
		Self { model }
	}
}

#[ComplexObject]
impl BookClubDiscussion {
	/// Get the book this discussion is for
	async fn book(&self, ctx: &Context<'_>) -> Result<Option<BookClubBook>> {
		ensure_membership(ctx, &self.model.book_club_id).await?;
		let book_club_book_id = match &self.model.book_club_book_id {
			Some(id) => id,
			None => return Ok(None),
		};

		let core = ctx.data::<CoreContext>()?;
		let book = models::entity::book_club_book::Entity::find_by_id(book_club_book_id)
			.filter(
				models::entity::book_club_book::Column::BookClubId
					.eq(&self.model.book_club_id),
			)
			.one(core.conn.as_ref())
			.await?;

		Ok(book.map(BookClubBook::from))
	}

	/// A display name for the discussion
	async fn display_name(&self, ctx: &Context<'_>) -> Result<String> {
		ensure_membership(ctx, &self.model.book_club_id).await?;
		if let Some(title) = &self.model.title {
			return Ok(title.clone());
		}

		if let Some(book_id) = &self.model.book_club_book_id {
			let core = ctx.data::<CoreContext>()?;
			if let Some(book) =
				models::entity::book_club_book::Entity::find_by_id(book_id)
					.filter(
						models::entity::book_club_book::Column::BookClubId
							.eq(&self.model.book_club_id),
					)
					.one(core.conn.as_ref())
					.await?
			{
				if let Some(title) = &book.title {
					return Ok(title.clone());
				}

				if let Some(book_entity_id) = &book.book_entity_id {
					let stump_auth::AuthContext { user, .. } =
						ctx.data::<stump_auth::AuthContext>()?;
					if let Ok(visible) = models::services::social::visible_media(
						core.conn.as_ref(),
						user,
						book_entity_id,
					)
					.await
					{
						if let Some(metadata) = visible.metadata {
							if let Some(title) = metadata.title {
								return Ok(title);
							}
						}
						return Ok(visible.media.name);
					}
				}
			}
		}

		Ok("General".to_string())
	}

	/// Get the count of messages in this discussion (excluding deleted messages)
	async fn message_count(&self, ctx: &Context<'_>) -> Result<i64> {
		ensure_membership(ctx, &self.model.book_club_id).await?;
		let core = ctx.data::<CoreContext>()?;

		let count = book_club_discussion_message::Entity::find()
			.filter(book_club_discussion_message::Column::DiscussionId.eq(&self.model.id))
			.filter(
				book_club_discussion_message::Column::BookClubId
					.eq(&self.model.book_club_id),
			)
			.filter(book_club_discussion_message::Column::DeletedAt.is_null())
			.count(core.conn.as_ref())
			.await?;

		Ok(count as i64)
	}
}

async fn ensure_membership(ctx: &Context<'_>, book_club_id: &str) -> Result<()> {
	let stump_auth::AuthContext { user, .. } = ctx.data::<stump_auth::AuthContext>()?;
	if user.is_server_owner {
		return Ok(());
	}
	let conn = ctx.data::<CoreContext>()?.conn.as_ref();
	if book_club_member::Entity::find_by_club_for_user(user, book_club_id)
		.one(conn)
		.await?
		.is_some()
	{
		Ok(())
	} else {
		Err("You must be a member of the book club to access this field".into())
	}
}
