use async_graphql::{Context, Object, Result, ID};
use chrono::{Duration, Utc};
use models::{
	entity::{book_club_invitation, book_club_member, user, user::AuthUser},
	shared::book_club::BookClubMemberRole,
	txn::begin_write,
};
use sea_orm::{prelude::*, ConnectionTrait, IntoActiveModel, Set};
use uuid::Uuid;

use crate::{
	data::CoreContext,
	input::book_club::{
		BookClubInvitationInput, BookClubInvitationResponseInput,
		BookClubInvitationResponseValidator, BookClubMemberInput,
	},
	mutation::book_club::get_book_club_for_admin,
	object::book_club_invitation::BookClubInvitation,
};

#[derive(Default)]
pub struct BookClubInvitationMutation;

#[Object]
impl BookClubInvitationMutation {
	async fn create_book_club_invitation(
		&self,
		ctx: &Context<'_>,
		id: ID,
		input: BookClubInvitationInput,
	) -> Result<BookClubInvitation> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		validate_book_club_invitation_input(user, &id, &input, conn).await?;

		let mut active_model = create_invitation_active_model(&id, &input);
		active_model.created_by_user_id = Set(Some(user.id.clone()));
		let created_invitation = active_model.insert(conn).await?;

		Ok(created_invitation.into())
	}

	async fn respond_to_book_club_invitation(
		&self,
		ctx: &Context<'_>,
		id: ID,
		#[graphql(validator(custom = "BookClubInvitationResponseValidator"))]
		input: BookClubInvitationResponseInput,
	) -> Result<BookClubInvitation> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		Ok(handle_book_club_invitation(user, &id, input, conn)
			.await?
			.into())
	}
	async fn revoke_book_club_invitation(
		&self,
		ctx: &Context<'_>,
		id: ID,
	) -> Result<BookClubInvitation> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let invitation = book_club_invitation::Entity::find_by_id(id.as_ref())
			.one(conn)
			.await?
			.ok_or("Invitation not found")?;
		let is_admin = get_book_club_for_admin(
			user,
			&ID::from(invitation.book_club_id.clone()),
			conn,
		)
		.await?
		.is_some();
		if invitation.created_by_user_id.as_deref() != Some(&user.id)
			&& !is_admin
			&& !user.is_server_owner
		{
			return Err(
				"Only the sender or a club administrator can revoke this invitation"
					.into(),
			);
		}
		if invitation.status != book_club_invitation::PENDING
			|| invitation.expires_at.is_some_and(|deadline| {
				deadline <= DateTimeWithTimeZone::from(Utc::now())
			}) {
			return Err("Invitation is no longer pending".into());
		}
		let mut active = invitation.into_active_model();
		active.status = Set(book_club_invitation::REVOKED.to_owned());
		active.revoked_at = Set(Some(Utc::now().into()));
		active.revoked_by_user_id = Set(Some(user.id.clone()));
		Ok(active.update(conn).await?.into())
	}
}

async fn validate_book_club_invitation_input(
	user: &AuthUser,
	id: &ID,
	input: &BookClubInvitationInput,
	conn: &DatabaseConnection,
) -> Result<()> {
	let _book_club = get_book_club_for_admin(user, id, conn)
		.await?
		.ok_or("Book club not found or you lack permission to update")?;

	let invalid_role = input
		.role
		.as_ref()
		.is_some_and(|role| *role == BookClubMemberRole::Creator);

	if invalid_role {
		return Err("Cannot invite a user as a creator".into());
	}

	if input.user_id == user.id {
		return Err("Cannot create an invitation for yourself".into());
	}

	if user::Entity::find_by_id(&input.user_id)
		.filter(user::Column::DeletedAt.is_null())
		.filter(user::Column::IsLocked.eq(false))
		.one(conn)
		.await?
		.is_none()
	{
		return Err("Cannot invite a locked, deleted, or unknown user".into());
	}

	Ok(())
}
async fn handle_book_club_invitation(
	user: &AuthUser,
	id: &ID,
	input: BookClubInvitationResponseInput,
	conn: &DatabaseConnection,
) -> Result<book_club_invitation::Model> {
	let txn = begin_write(conn).await?;
	let invitation = get_book_club_invitation(user, id, &txn).await?;

	let updated = if !input.accept {
		decline_invitation(invitation, &txn).await?
	} else {
		accept_invitation(user, invitation, input.member, &txn).await?
	};
	txn.commit().await?;
	Ok(updated)
}

async fn get_book_club_invitation<C>(
	user: &AuthUser,
	id: &ID,
	conn: &C,
) -> Result<book_club_invitation::Model>
where
	C: ConnectionTrait,
{
	book_club_invitation::Entity::find_live_for_user_and_id(user, id.as_ref())
		.one(conn)
		.await?
		.ok_or("Invitation not found, expired, or already resolved".into())
}

async fn decline_invitation<C>(
	invitation: book_club_invitation::Model,
	conn: &C,
) -> Result<book_club_invitation::Model>
where
	C: ConnectionTrait,
{
	if invitation.status != book_club_invitation::PENDING {
		return Err("Invitation is no longer pending".into());
	}
	let mut active = invitation.into_active_model();
	active.status = Set(book_club_invitation::DECLINED.to_owned());
	active.declined_at = Set(Some(Utc::now().into()));
	Ok(active.update(conn).await?)
}

async fn accept_invitation<C>(
	user: &AuthUser,
	invitation: book_club_invitation::Model,
	input: Option<BookClubMemberInput>,
	conn: &C,
) -> Result<book_club_invitation::Model>
where
	C: ConnectionTrait,
{
	if invitation.status != book_club_invitation::PENDING {
		return Err("Invitation is no longer pending".into());
	}
	let member_input = input.ok_or("Accepting an invitation requires a member object")?;
	let already_member = book_club_member::Entity::find()
		.filter(book_club_member::Column::BookClubId.eq(&invitation.book_club_id))
		.filter(book_club_member::Column::UserId.eq(&user.id))
		.one(conn)
		.await?
		.is_some();
	if already_member {
		return Err("You are already a member of this book club".into());
	}

	create_member_active_model(user, &invitation, member_input)
		.insert(conn)
		.await?;

	let mut active = invitation.into_active_model();
	active.status = Set(book_club_invitation::ACCEPTED.to_owned());
	active.accepted_at = Set(Some(Utc::now().into()));
	Ok(active.update(conn).await?)
}

fn create_invitation_active_model(
	id: &ID,
	input: &BookClubInvitationInput,
) -> book_club_invitation::ActiveModel {
	book_club_invitation::ActiveModel {
		role: Set(input.role.unwrap_or(BookClubMemberRole::Member)),
		user_id: Set(input.user_id.clone()),
		book_club_id: Set(id.to_string()),
		status: Set(book_club_invitation::PENDING.to_owned()),
		created_at: Set(Utc::now().into()),
		expires_at: Set(Some((Utc::now() + Duration::days(7)).into())),
		..Default::default()
	}
}

fn create_member_active_model(
	user: &AuthUser,
	invitation: &book_club_invitation::Model,
	input: BookClubMemberInput,
) -> book_club_member::ActiveModel {
	book_club_member::ActiveModel {
		id: Set(Uuid::new_v4().to_string()),
		display_name: Set(input.display_name),
		user_id: Set(user.id.clone()),
		book_club_id: Set(invitation.book_club_id.clone()),
		role: Set(invitation.role),
		hide_progress: Set(false),
		bio: Set(None),
		joined_at: Set(Utc::now().into()),
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	use crate::tests::common::*;
	use models::entity::{book_club, user};
	use pretty_assertions::assert_eq;

	fn get_default_book_club() -> book_club::Model {
		book_club::Model {
			id: "123".to_string(),
			name: "Test".to_string(),
			slug: "test".to_string(),
			description: None,
			is_private: false,
			member_role_spec: None,
			created_at: chrono::Utc::now().into(),
			emoji: None,
		}
	}
	fn get_default_invitee() -> user::Model {
		user::Model {
			id: "456".to_owned(),
			username: "invitee".to_owned(),
			hashed_password: "hash".to_owned(),
			is_server_owner: false,
			avatar_path: None,
			avatar_meta: None,
			avatar_updated_at: None,
			created_at: chrono::Utc::now().into(),
			deleted_at: None,
			is_locked: false,
			max_sessions_allowed: None,
			permissions: None,
			user_preferences_id: None,
			oidc_issuer_id: None,
			oidc_email: None,
		}
	}

	fn get_default_book_club_invitation() -> book_club_invitation::Model {
		book_club_invitation::Model {
			id: "314".to_string(),
			role: BookClubMemberRole::Admin,
			user_id: "42".to_string(),
			book_club_id: "123".to_string(),
			status: book_club_invitation::PENDING.to_string(),
			created_at: chrono::Utc::now().into(),
			expires_at: Some((chrono::Utc::now() + chrono::Duration::days(7)).into()),
			created_by_user_id: Some("7".to_string()),
			accepted_at: None,
			declined_at: None,
			revoked_at: None,
			revoked_by_user_id: None,
			audit_note: None,
		}
	}

	#[tokio::test]
	async fn decline_book_club_invitation_no_member() {
		let user = get_default_user();
		let id: ID = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into();
		let member = BookClubMemberInput {
			user_id: "42".to_string(),
			display_name: None,
		};
		let input = BookClubInvitationResponseInput {
			accept: false,
			member: Some(member),
		};

		let invitation = get_default_book_club_invitation();
		let mut declined = invitation.clone();
		declined.status = book_club_invitation::DECLINED.to_owned();
		declined.declined_at = Some(Utc::now().into());
		let mock_db = get_mock_db_for_model(vec![invitation])
			.append_query_results(vec![vec![declined.clone()]])
			.into_connection();
		let result = handle_book_club_invitation(&user, &id, input, &mock_db)
			.await
			.unwrap();
		assert_eq!(result.status, book_club_invitation::DECLINED);
		assert!(result.declined_at.is_some());
	}

	#[tokio::test]
	async fn accept_book_club_invitation_no_member() {
		let user = get_default_user();
		let id: ID = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into();
		let member = BookClubMemberInput {
			user_id: "42".to_string(),
			display_name: None,
		};

		let input = BookClubInvitationResponseInput {
			accept: true,
			member: Some(member.clone()),
		};

		let invitation = get_default_book_club_invitation();
		let inserted_member = book_club_member::Model {
			id: Uuid::new_v4().to_string(),
			display_name: member.display_name.clone(),
			bio: None,
			hide_progress: false,
			role: invitation.role,
			joined_at: Utc::now().into(),
			user_id: user.id.clone(),
			book_club_id: invitation.book_club_id.clone(),
		};
		let mut accepted = invitation.clone();
		accepted.status = book_club_invitation::ACCEPTED.to_owned();
		accepted.accepted_at = Some(Utc::now().into());
		let mock_db = get_mock_db_for_model(vec![invitation])
			.append_query_results(vec![Vec::<book_club_member::Model>::new()])
			.append_query_results(vec![vec![inserted_member]])
			.append_query_results(vec![vec![accepted.clone()]])
			.into_connection();
		let result = handle_book_club_invitation(&user, &id, input, &mock_db)
			.await
			.unwrap();
		assert_eq!(result.status, book_club_invitation::ACCEPTED);
		assert!(result.accepted_at.is_some());
	}

	#[tokio::test]
	async fn test_get_book_club_invitation() {
		let book_club_invitation = get_default_book_club_invitation();
		let mock_db =
			get_mock_db_for_model(vec![book_club_invitation.clone()]).into_connection();
		let id: ID = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into();

		// check for successful result
		let result = get_book_club_invitation(&get_default_user(), &id, &mock_db)
			.await
			.unwrap();
		assert_eq!(result, book_club_invitation);

		// check for empty result
		let mock_db = get_mock_db_for_model::<book_club_invitation::Model>(vec![])
			.into_connection();
		let result = get_book_club_invitation(&get_default_user(), &id, &mock_db).await;
		assert!(result.is_err());
	}

	#[tokio::test]
	async fn test_validate_book_club_invitation_input_valid() {
		let mock_db = get_mock_db_for_model(vec![get_default_book_club()])
			.append_query_results(vec![vec![get_default_invitee()]])
			.into_connection();

		let input = BookClubInvitationInput {
			role: Some(BookClubMemberRole::Admin),
			user_id: "456".to_string(),
		};

		let result = validate_book_club_invitation_input(
			&get_default_user(),
			&"123".into(),
			&input,
			&mock_db,
		)
		.await;

		assert!(result.is_ok());
	}

	#[tokio::test]
	async fn test_validate_book_club_invitation_input_same_user() {
		let mock_db: DatabaseConnection =
			get_mock_db_for_model(vec![get_default_book_club()]).into_connection();
		let input: BookClubInvitationInput = BookClubInvitationInput {
			role: Some(BookClubMemberRole::Admin),
			user_id: get_default_user().id,
		};
		let result = validate_book_club_invitation_input(
			&get_default_user(),
			&"123".into(),
			&input,
			&mock_db,
		)
		.await;

		assert!(result.is_err());
	}

	#[tokio::test]
	async fn test_validate_book_club_invitation_input_missing_role() {
		let mock_db = get_mock_db_for_model(vec![get_default_book_club()])
			.append_query_results(vec![vec![get_default_invitee()]])
			.into_connection();
		let input: BookClubInvitationInput = BookClubInvitationInput {
			role: None,
			user_id: "456".to_string(),
		};
		let result = validate_book_club_invitation_input(
			&get_default_user(),
			&"123".into(),
			&input,
			&mock_db,
		)
		.await;
		assert!(result.is_ok());
	}

	#[tokio::test]
	async fn test_validate_book_club_invitation_input_invalid_role() {
		let mock_db: DatabaseConnection =
			get_mock_db_for_model(vec![get_default_book_club()]).into_connection();
		let input: BookClubInvitationInput = BookClubInvitationInput {
			role: Some(BookClubMemberRole::Creator),
			user_id: "456".to_string(),
		};
		let result = validate_book_club_invitation_input(
			&get_default_user(),
			&"123".into(),
			&input,
			&mock_db,
		)
		.await;
		assert!(result.is_err());
	}

	#[tokio::test]
	async fn test_validate_book_club_invitation_input_invalid_user() {
		let user = get_default_user();
		let mock_db: DatabaseConnection =
			get_mock_db_for_model(vec![get_default_book_club()]).into_connection();
		let input: BookClubInvitationInput = BookClubInvitationInput {
			role: Some(BookClubMemberRole::Creator),
			user_id: user.id.clone(),
		};
		let result =
			validate_book_club_invitation_input(&user, &"123".into(), &input, &mock_db)
				.await;
		assert!(result.is_err());
	}

	#[test]
	fn test_create_invitation_active_model() {
		let id: ID = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into();
		let input = BookClubInvitationInput {
			role: Some(BookClubMemberRole::Admin),
			user_id: "456".to_string(),
		};

		let active_model = create_invitation_active_model(&id, &input);

		assert_eq!(active_model.role.unwrap(), BookClubMemberRole::Admin);
		assert_eq!(active_model.user_id.unwrap(), "456".to_string());
		assert_eq!(active_model.book_club_id.unwrap(), id.to_string());
	}
}
