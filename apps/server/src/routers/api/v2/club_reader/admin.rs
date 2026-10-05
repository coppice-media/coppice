use std::sync::Arc;

use axum::{
	extract::{Path as AxumPath, State},
	http::StatusCode,
	Extension, Json,
};
use chrono::{DateTime, Duration, Utc};
use models::{
	entity::{
		book_club_book, book_club_discussion,
		book_club_reader_participant as participant, book_club_reader_session as session,
		media,
	},
	shared::book_club::BookClubMemberRole,
};
use sea_orm::{
	prelude::*, ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait,
	IntoActiveModel, QueryFilter, QueryOrder, Set,
};
use stump_auth::AuthContext;

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	utils::current_utc_time,
};

use super::{
	events::{Kinds, ReaderEvents},
	media as reader_media, reader, security,
	types::*,
};

const MAX_SESSION_NAME_CHARS: usize = 120;

fn require_interactive_account(
	auth: &AuthContext,
) -> APIResult<models::entity::user::AuthUser> {
	if auth.api_key.is_some() || auth.device_id.is_some() {
		return Err(APIError::Forbidden(
			"Book-club reader management requires an interactive account credential"
				.to_string(),
		));
	}
	Ok(auth.user())
}

async fn require_member(
	ctx: &AppState,
	auth: &AuthContext,
	club_id: &str,
) -> APIResult<models::entity::user::AuthUser> {
	let user = require_interactive_account(auth)?;
	security::authorize_club_member(ctx, &user, club_id, None, false).await?;
	Ok(user)
}

pub(super) async fn require_manager(
	ctx: &AppState,
	auth: &AuthContext,
	club_id: &str,
) -> APIResult<models::entity::user::AuthUser> {
	let user = require_interactive_account(auth)?;
	security::authorize_club_member(
		ctx,
		&user,
		club_id,
		Some(BookClubMemberRole::Admin),
		true,
	)
	.await?;
	Ok(user)
}

async fn can_manage(
	ctx: &AppState,
	user: &models::entity::user::AuthUser,
	club_id: &str,
) -> APIResult<bool> {
	match security::authorize_club_member(
		ctx,
		user,
		club_id,
		Some(BookClubMemberRole::Admin),
		true,
	)
	.await
	{
		Ok(can_manage) => Ok(can_manage),
		Err(APIError::Forbidden(_)) => Ok(false),
		Err(error) => Err(error),
	}
}

pub(super) async fn load_session(
	ctx: &AppState,
	club_id: &str,
	session_id: &str,
) -> APIResult<session::Model> {
	session::Entity::find_by_id(session_id)
		.filter(session::Column::BookClubId.eq(club_id))
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| APIError::NotFound("Reader session not found".to_string()))
}

fn ensure_open(session: &session::Model) -> APIResult<()> {
	if session.closed_at.is_some() || security::is_expired(session.expires_at.clone()) {
		return Err(APIError::Conflict(
			"Reader session is closed or expired".to_string(),
		));
	}
	Ok(())
}

fn validate_session_name(name: &str) -> APIResult<String> {
	let name = name.trim();
	if name.is_empty()
		|| name.chars().count() > MAX_SESSION_NAME_CHARS
		|| name.chars().any(char::is_control)
	{
		return Err(APIError::BadRequest(
			"Session name must be 1 to 120 characters".to_string(),
		));
	}
	Ok(name.to_string())
}

fn validate_expiry(
	expires_at: Option<DateTime<Utc>>,
) -> APIResult<Option<DateTimeWithTimeZone>> {
	if expires_at.is_some_and(|expires_at| expires_at <= Utc::now()) {
		return Err(APIError::BadRequest(
			"Expiry must be in the future".to_string(),
		));
	}
	Ok(expires_at.map(DateTimeWithTimeZone::from))
}

fn participant_expiry(
	requested: Option<DateTime<Utc>>,
	session: &session::Model,
) -> APIResult<Option<DateTimeWithTimeZone>> {
	let requested = validate_expiry(requested)?;
	let session_expiry = session.expires_at.clone();
	Ok(match (requested, session_expiry) {
		(Some(requested), Some(session_expiry)) => Some(requested.min(session_expiry)),
		(Some(requested), None) => Some(requested),
		(None, Some(session_expiry)) => Some(session_expiry),
		(None, None) => None,
	})
}

async fn session_admin(
	ctx: &AppState,
	session: session::Model,
	manager: bool,
) -> APIResult<ReaderSessionAdmin> {
	// Only manual/external queue rows carry a title; library rows resolve like
	// the guest snapshot and Home (`resolvedName`).
	let published_book_title = match session.published_book_id.as_deref() {
		Some(book_id) => match book_club_book::Entity::find_by_id(book_id)
			.filter(book_club_book::Column::BookClubId.eq(&session.book_club_id))
			.one(ctx.conn.as_ref())
			.await?
		{
			Some(book) => match (book.title, book.book_entity_id) {
				(Some(title), _) => Some(title),
				(None, Some(media_id)) => match media::Entity::find_by_id(media_id)
					.filter(media::Column::DeletedAt.is_null())
					.one(ctx.conn.as_ref())
					.await?
				{
					Some(media) => Some(
						reader_media::media_display_title(ctx.conn.as_ref(), &media)
							.await?,
					),
					None => None,
				},
				(None, None) => None,
			},
			None => None,
		},
		None => None,
	};
	let participants = if manager {
		participant::Entity::find()
			.filter(participant::Column::SessionId.eq(&session.id))
			.order_by_asc(participant::Column::CreatedAt)
			.all(ctx.conn.as_ref())
			.await?
			.into_iter()
			.map(ReaderParticipantAdmin::from)
			.collect()
	} else {
		Vec::new()
	};
	Ok(ReaderSessionAdmin::from_model(
		session,
		published_book_title,
		participants,
		manager,
	))
}

pub(super) async fn list(
	AxumPath(club_id): AxumPath<String>,
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<Json<ReaderSessionCollection>> {
	let user = require_member(&ctx, &auth, &club_id).await?;
	let manager = can_manage(&ctx, &user, &club_id).await?;
	let sessions = session::Entity::find()
		.filter(session::Column::BookClubId.eq(&club_id))
		.order_by_desc(session::Column::CreatedAt)
		.all(ctx.conn.as_ref())
		.await?;
	let mut response = Vec::with_capacity(sessions.len());
	for session in sessions {
		response.push(session_admin(&ctx, session, manager).await?);
	}
	Ok(Json(ReaderSessionCollection { sessions: response }))
}

pub(super) async fn create(
	AxumPath(club_id): AxumPath<String>,
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Json(input): Json<CreateSessionInput>,
) -> APIResult<(StatusCode, Json<ReaderSessionAdmin>)> {
	let user = require_manager(&ctx, &auth, &club_id).await?;
	let name = validate_session_name(&input.name)?;
	let expires_at = validate_expiry(input.expires_at)?.or_else(|| {
		Some(DateTimeWithTimeZone::from(
			current_utc_time() + Duration::days(30),
		))
	});
	let now = DateTimeWithTimeZone::from(current_utc_time());
	let created = session::ActiveModel {
		id: Set(uuid::Uuid::new_v4().to_string()),
		book_club_id: Set(club_id),
		created_by_user_id: Set(user.id),
		published_by_user_id: Set(None),
		name: Set(name),
		published_book_id: Set(None),
		expires_at: Set(expires_at),
		closed_at: Set(None),
		created_at: Set(now.clone()),
		updated_at: Set(now),
	}
	.insert(ctx.conn.as_ref())
	.await?;
	let admin = session_admin(&ctx, created, true).await?;
	Ok((StatusCode::CREATED, Json(admin)))
}

pub(super) async fn publish(
	AxumPath((club_id, session_id)): AxumPath<(String, String)>,
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<Arc<ReaderEvents>>,
	Json(input): Json<PublishBookInput>,
) -> APIResult<Json<ReaderSessionAdmin>> {
	let user = require_manager(&ctx, &auth, &club_id).await?;
	let current_session = load_session(&ctx, &club_id, &session_id).await?;
	ensure_open(&current_session)?;
	let current = reader_media::queue_head(ctx.conn.as_ref(), &club_id)
		.await?
		.filter(|head| head.id == input.book_id)
		.ok_or_else(|| {
			APIError::Conflict(
				"Only the current queued book can be published".to_string(),
			)
		})?;
	reader_media::media_for_club_book(
		&ctx,
		&user,
		&current,
		&session_id,
		reader_media::ValidationMode::Publish,
	)
	.await?;

	let txn = models::txn::begin_write(ctx.conn.as_ref()).await?;
	let session_model = load_session_txn(&txn, &club_id, &session_id).await?;
	ensure_open(&session_model)?;
	let mut session = session_model.into_active_model();
	let current_after_lock = reader_media::queue_head(&txn, &club_id).await?;
	if current_after_lock.map(|head| head.id).as_deref() != Some(current.id.as_str()) {
		return Err(APIError::Conflict(
			"The book-club queue changed".to_string(),
		));
	}
	session.published_book_id = Set(Some(input.book_id));
	session.published_by_user_id = Set(Some(user.id));
	session.updated_at = Set(DateTimeWithTimeZone::from(current_utc_time()));
	let updated = session.update(&txn).await?;
	txn.commit().await?;
	events.publish(&session_id, Kinds::PUBLICATION);
	Ok(Json(session_admin(&ctx, updated, true).await?))
}

pub(super) async fn advance(
	AxumPath((club_id, session_id)): AxumPath<(String, String)>,
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<Arc<ReaderEvents>>,
) -> APIResult<Json<ReaderSessionAdmin>> {
	let user = require_manager(&ctx, &auth, &club_id).await?;
	let current_session = load_session(&ctx, &club_id, &session_id).await?;
	ensure_open(&current_session)?;
	let queue = unfinished_queue(ctx.conn.as_ref(), &club_id).await?;
	let current = queue.first().ok_or_else(|| {
		APIError::Conflict("There is no queued book to advance".to_string())
	})?;
	if current_session.published_book_id.as_deref() != Some(&current.id) {
		return Err(APIError::Conflict(
			"The current queued book is not the published reader book".to_string(),
		));
	}
	let next = queue.get(1);
	if let Some(next) = next {
		reader_media::media_for_club_book(
			&ctx,
			&user,
			next,
			&session_id,
			reader_media::ValidationMode::Publish,
		)
		.await?;
	}

	let txn = models::txn::begin_write(ctx.conn.as_ref()).await?;
	let session_model = load_session_txn(&txn, &club_id, &session_id).await?;
	ensure_open(&session_model)?;
	if session_model.published_book_id.as_deref() != Some(&current.id) {
		return Err(APIError::Conflict(
			"The current queued book is not the published reader book".to_string(),
		));
	}
	let mut session = session_model.into_active_model();
	let locked_queue = unfinished_queue(&txn, &club_id).await?;
	let locked_current = locked_queue.first().ok_or_else(|| {
		APIError::Conflict("There is no queued book to advance".to_string())
	})?;
	if locked_current.id != current.id
		|| locked_queue.get(1).map(|book| book.id.as_str())
			!= next.map(|book| book.id.as_str())
	{
		return Err(APIError::Conflict(
			"The book-club queue changed".to_string(),
		));
	}
	let current_model = book_club_book::Entity::find_by_id(&current.id)
		.one(&txn)
		.await?
		.ok_or_else(|| {
			APIError::Conflict("The current queued book disappeared".to_string())
		})?;
	let mut current_active = current_model.into_active_model();
	current_active.completed_at =
		Set(Some(DateTimeWithTimeZone::from(current_utc_time())));
	current_active.update(&txn).await?;
	book_club_discussion::Entity::update_many()
		.col_expr(book_club_discussion::Column::IsArchived, Expr::value(true))
		.filter(book_club_discussion::Column::BookClubBookId.eq(&current.id))
		.exec(&txn)
		.await?;
	session.published_book_id = Set(next.map(|book| book.id.clone()));
	session.published_by_user_id = Set(next.map(|_| user.id));
	session.updated_at = Set(DateTimeWithTimeZone::from(current_utc_time()));
	let updated = session.update(&txn).await?;
	txn.commit().await?;
	events.publish(&session_id, Kinds::PUBLICATION);
	Ok(Json(session_admin(&ctx, updated, true).await?))
}

async fn unfinished_queue<C: ConnectionTrait>(
	conn: &C,
	club_id: &str,
) -> Result<Vec<book_club_book::Model>, sea_orm::DbErr> {
	book_club_book::Entity::find_for_book_club_id(club_id)
		.filter(book_club_book::Column::CompletedAt.is_null())
		.order_by_asc(book_club_book::Column::Id)
		.all(conn)
		.await
}

async fn load_session_txn(
	txn: &DatabaseTransaction,
	club_id: &str,
	session_id: &str,
) -> APIResult<session::Model> {
	session::Entity::find_by_id(session_id)
		.filter(session::Column::BookClubId.eq(club_id))
		.one(txn)
		.await?
		.ok_or_else(|| APIError::NotFound("Reader session not found".to_string()))
}

pub(super) async fn create_participant(
	AxumPath((club_id, session_id)): AxumPath<(String, String)>,
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<Arc<ReaderEvents>>,
	Json(input): Json<CreateParticipantInput>,
) -> APIResult<(StatusCode, Json<CredentialResponse>)> {
	let user = require_manager(&ctx, &auth, &club_id).await?;
	let session = load_session(&ctx, &club_id, &session_id).await?;
	ensure_open(&session)?;
	let participant = issue_participant(
		&ctx,
		&session,
		&user,
		None,
		reader::validate_display_name(&input.display_name)?,
		input.expires_at,
		false,
	)
	.await?;
	events.publish(&session.id, Kinds::PARTICIPANTS);
	Ok((StatusCode::CREATED, Json(participant)))
}

pub(super) async fn join(
	AxumPath((club_id, session_id)): AxumPath<(String, String)>,
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<Arc<ReaderEvents>>,
	Json(input): Json<JoinSessionInput>,
) -> APIResult<(StatusCode, Json<CredentialResponse>)> {
	let user = require_member(&ctx, &auth, &club_id).await?;
	let session = load_session(&ctx, &club_id, &session_id).await?;
	ensure_open(&session)?;
	let display_name = reader::validate_display_name(&input.display_name)?;
	let existing = participant::Entity::find()
		.filter(participant::Column::SessionId.eq(&session_id))
		.filter(participant::Column::LinkedUserId.eq(user.id.clone()))
		.one(ctx.conn.as_ref())
		.await?;
	let share_progress = input
		.share_progress
		.or_else(|| {
			existing
				.as_ref()
				.map(|participant| participant.share_progress)
		})
		.unwrap_or(false);
	let response = issue_participant(
		&ctx,
		&session,
		&user,
		Some(user.id.clone()),
		display_name,
		None,
		share_progress,
	)
	.await?;
	// A re-join may rename the member, and the reissued credential ends the
	// streams of the one it replaces.
	events.publish(
		&session.id,
		Kinds::PARTICIPANTS | Kinds::ANNOTATIONS | Kinds::MESSAGES,
	);
	Ok((StatusCode::CREATED, Json(response)))
}

async fn issue_participant(
	ctx: &AppState,
	session: &session::Model,
	actor: &models::entity::user::AuthUser,
	linked_user_id: Option<String>,
	display_name: String,
	requested_expiry: Option<DateTime<Utc>>,
	share_progress: bool,
) -> APIResult<CredentialResponse> {
	let token = security::generate_secret()?;
	let token_digest = security::secret_digest(&token);
	let expires_at = participant_expiry(requested_expiry, session)?;
	let now = DateTimeWithTimeZone::from(current_utc_time());
	let existing = if let Some(linked_user_id) = linked_user_id.as_deref() {
		participant::Entity::find()
			.filter(participant::Column::SessionId.eq(&session.id))
			.filter(participant::Column::LinkedUserId.eq(linked_user_id))
			.one(ctx.conn.as_ref())
			.await?
	} else {
		None
	};
	let participant = if let Some(existing) = existing {
		if existing.revoked_at.is_some() {
			return Err(revoked_conflict());
		}
		let mut active = existing.into_active_model();
		active.display_name = Set(display_name);
		active.share_progress = Set(share_progress);
		active.token_digest = Set(Some(token_digest));
		active.credential_expires_at = Set(expires_at);
		active.updated_at = Set(now);
		// A concurrent revoke must win: never re-issue onto a revoked row.
		update_unrevoked(ctx, active).await?
	} else {
		participant::ActiveModel {
			id: Set(uuid::Uuid::new_v4().to_string()),
			session_id: Set(session.id.clone()),
			linked_user_id: Set(linked_user_id),
			created_by_user_id: Set(Some(actor.id.clone())),
			display_name: Set(display_name),
			share_progress: Set(share_progress),
			token_digest: Set(Some(token_digest)),
			credential_expires_at: Set(expires_at),
			revoked_at: Set(None),
			created_at: Set(now),
			updated_at: Set(now),
		}
		.insert(ctx.conn.as_ref())
		.await?
	};
	Ok(CredentialResponse {
		participant: ReaderParticipantAdmin::from(participant),
		reader_path: format!("/app/club-reader/{}", session.id),
		token,
	})
}

pub(super) async fn rotate(
	AxumPath((club_id, session_id, participant_id)): AxumPath<(String, String, String)>,
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<Arc<ReaderEvents>>,
	Json(input): Json<CredentialRotationInput>,
) -> APIResult<Json<CredentialResponse>> {
	let _actor = require_manager(&ctx, &auth, &club_id).await?;
	let session = load_session(&ctx, &club_id, &session_id).await?;
	ensure_open(&session)?;
	let existing = participant::Entity::find_by_id(&participant_id)
		.filter(participant::Column::SessionId.eq(&session_id))
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| APIError::NotFound("Reader participant not found".to_string()))?;
	if existing.revoked_at.is_some() {
		return Err(revoked_conflict());
	}
	let expiry = participant_expiry(input.expires_at, &session)?;
	let token = security::generate_secret()?;
	let mut active = existing.into_active_model();
	active.token_digest = Set(Some(security::secret_digest(&token)));
	active.credential_expires_at = Set(expiry);
	active.updated_at = Set(DateTimeWithTimeZone::from(current_utc_time()));
	let updated = update_unrevoked(&ctx, active).await?;
	// Nothing visible changes for other readers; the old credential's streams
	// re-check and end.
	events.publish(&session_id, Kinds::NONE);
	Ok(Json(CredentialResponse {
		participant: ReaderParticipantAdmin::from(updated),
		reader_path: format!("/app/club-reader/{session_id}"),
		token,
	}))
}

fn revoked_conflict() -> APIError {
	APIError::Conflict("Reader participant is revoked".to_string())
}

/// Update a participant only while it is still unrevoked, so neither self-service
/// join nor manager rotation can resurrect a revoked capability.
async fn update_unrevoked(
	ctx: &AppState,
	active: participant::ActiveModel,
) -> APIResult<participant::Model> {
	match participant::Entity::update(active)
		.filter(participant::Column::RevokedAt.is_null())
		.exec(ctx.conn.as_ref())
		.await
	{
		Ok(updated) => Ok(updated),
		Err(sea_orm::DbErr::RecordNotUpdated) => Err(revoked_conflict()),
		Err(error) => Err(error.into()),
	}
}

pub(super) async fn revoke_participant(
	AxumPath((club_id, session_id, participant_id)): AxumPath<(String, String, String)>,
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<Arc<ReaderEvents>>,
) -> APIResult<StatusCode> {
	let _actor = require_manager(&ctx, &auth, &club_id).await?;
	let _session = load_session(&ctx, &club_id, &session_id).await?;
	let existing = participant::Entity::find_by_id(&participant_id)
		.filter(participant::Column::SessionId.eq(&session_id))
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| APIError::NotFound("Reader participant not found".to_string()))?;
	if existing.revoked_at.is_none() {
		let mut active = existing.into_active_model();
		let now = DateTimeWithTimeZone::from(current_utc_time());
		active.token_digest = Set(None);
		active.credential_expires_at = Set(Some(now.clone()));
		active.revoked_at = Set(Some(now.clone()));
		active.updated_at = Set(now);
		active.update(ctx.conn.as_ref()).await?;
		// The revoked reader's annotations leave every projection and its
		// messages lose their author link.
		events.publish(
			&session_id,
			Kinds::PARTICIPANTS | Kinds::ANNOTATIONS | Kinds::MESSAGES,
		);
	}
	Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn close(
	AxumPath((club_id, session_id)): AxumPath<(String, String)>,
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<Arc<ReaderEvents>>,
) -> APIResult<StatusCode> {
	let _actor = require_manager(&ctx, &auth, &club_id).await?;
	let txn = models::txn::begin_write(ctx.conn.as_ref()).await?;
	let session = load_session_txn(&txn, &club_id, &session_id).await?;
	let mut active = session.into_active_model();
	let now = DateTimeWithTimeZone::from(current_utc_time());
	active.closed_at = Set(Some(now.clone()));
	active.updated_at = Set(now.clone());
	active.update(&txn).await?;
	let participants = participant::Entity::find()
		.filter(participant::Column::SessionId.eq(&session_id))
		.filter(participant::Column::RevokedAt.is_null())
		.all(&txn)
		.await?;
	for participant in participants {
		let mut active = participant.into_active_model();
		active.token_digest = Set(None);
		active.credential_expires_at = Set(Some(now.clone()));
		active.revoked_at = Set(Some(now.clone()));
		active.updated_at = Set(now.clone());
		active.update(&txn).await?;
	}
	txn.commit().await?;
	events.publish(&session_id, Kinds::PUBLICATION);
	Ok(StatusCode::NO_CONTENT)
}
