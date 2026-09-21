use async_graphql::{ComplexObject, Context, Result, SimpleObject};
use models::entity::{
	book_club_book_suggestion, book_club_book_suggestion_like, book_club_member,
};
use sea_orm::{prelude::*, ColumnTrait, EntityTrait, QueryFilter};

use crate::data::CoreContext;
use crate::object::book_club_member::BookClubMember;

#[derive(Debug, SimpleObject)]
#[graphql(complex)]
pub struct BookClubBookSuggestion {
	#[graphql(flatten)]
	model: book_club_book_suggestion::Model,
}

impl From<book_club_book_suggestion::Model> for BookClubBookSuggestion {
	fn from(model: book_club_book_suggestion::Model) -> Self {
		Self { model }
	}
}

#[ComplexObject]
impl BookClubBookSuggestion {
	/// Get the member who suggested this book
	async fn suggested_by(&self, ctx: &Context<'_>) -> Result<BookClubMember> {
		ensure_membership(ctx, &self.model.book_club_id).await?;
		let core = ctx.data::<CoreContext>()?;

		let member = book_club_member::Entity::find_by_id(&self.model.suggested_by_id)
			.filter(book_club_member::Column::BookClubId.eq(&self.model.book_club_id))
			.one(core.conn.as_ref())
			.await?
			.ok_or("Member not found")?;

		Ok(BookClubMember::from(member))
	}
	async fn resolved_by(&self, ctx: &Context<'_>) -> Result<Option<BookClubMember>> {
		ensure_membership(ctx, &self.model.book_club_id).await?;
		let core = ctx.data::<CoreContext>()?;

		if let Some(ref resolved_by_id) = self.model.resolved_by_id {
			let member = book_club_member::Entity::find_by_id(resolved_by_id)
				.filter(book_club_member::Column::BookClubId.eq(&self.model.book_club_id))
				.one(core.conn.as_ref())
				.await?;

			Ok(member.map(BookClubMember::from))
		} else {
			Ok(None)
		}
	}

	/// Get the count of likes (votes) on this suggestion
	/// TODO(dataloader): Create dataloader
	async fn like_count(&self, ctx: &Context<'_>) -> Result<i64> {
		ensure_membership(ctx, &self.model.book_club_id).await?;
		let core = ctx.data::<CoreContext>()?;

		let count = book_club_book_suggestion_like::Entity::find()
			.filter(
				book_club_book_suggestion_like::Column::SuggestionId.eq(&self.model.id),
			)
			.count(core.conn.as_ref())
			.await?;

		Ok(count as i64)
	}

	async fn is_liked_by_me(&self, ctx: &Context<'_>) -> Result<bool> {
		ensure_membership(ctx, &self.model.book_club_id).await?;
		let core = ctx.data::<CoreContext>()?;
		let auth_ctx = ctx.data::<stump_auth::AuthContext>()?;

		let member = book_club_member::Entity::find_by_club_for_user(
			&auth_ctx.user,
			&self.model.book_club_id,
		)
		.one(core.conn.as_ref())
		.await?;

		if let Some(member) = member {
			let like = book_club_book_suggestion_like::Entity::find()
				.filter(
					book_club_book_suggestion_like::Column::SuggestionId
						.eq(&self.model.id),
				)
				.filter(book_club_book_suggestion_like::Column::LikedById.eq(&member.id))
				.one(core.conn.as_ref())
				.await?;

			Ok(like.is_some())
		} else {
			Ok(false)
		}
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
