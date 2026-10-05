//! The per-session discussion: plain-text messages that only the session's
//! active participants (guests and joined members, under their aliases) and its
//! organizers can read. Club member discussions are never exposed here.

use std::{
	collections::{HashMap, HashSet},
	sync::Arc,
};

use axum::{
	extract::{Path as AxumPath, Query, State},
	http::{HeaderMap, StatusCode},
	Extension, Json,
};
use chrono::Duration;
use models::entity::{
	book_club_reader_message as message, book_club_reader_participant as participant,
};
use sea_orm::{
	prelude::*, ActiveModelTrait, Condition, IntoActiveModel, QueryOrder, QuerySelect,
	Set,
};
use stump_auth::AuthContext;

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	utils::current_utc_time,
};

use super::{
	admin,
	events::{Kinds, ReaderEvents},
	media as reader_media, reader, security,
	types::*,
};

const MAX_BODY_CHARS: usize = 4_000;
const DEFAULT_PAGE_SIZE: u64 = 50;
const MAX_PAGE_SIZE: u64 = 100;
/// Posting budget per participant inside a sliding [`RATE_WINDOW_SECS`]
/// window. Deleted messages still count, so delete-and-repost cannot bypass it.
const MAX_MESSAGES_PER_WINDOW: u64 = 20;
const RATE_WINDOW_SECS: i64 = 60;

/// Who reads a page: a participant, for whom inactive authors lose their id,
/// or an organizer, who already sees every participant of the session.
#[derive(Clone, Copy)]
enum Audience<'a> {
	Participant(&'a str),
	Organizer,
}

fn project(
	message: message::Model,
	author: &participant::Model,
	audience: Audience<'_>,
) -> ReaderMessage {
	let (author_id, mine) = match audience {
		Audience::Participant(viewer) => (
			reader::is_active_participant(author).then(|| author.id.clone()),
			viewer == author.id,
		),
		Audience::Organizer => (Some(author.id.clone()), false),
	};
	ReaderMessage::from_model(message, author_id, author.display_name.clone(), mine)
}

fn validate_body(body: &str) -> APIResult<String> {
	let body = body.trim();
	if body.is_empty()
		|| body.chars().count() > MAX_BODY_CHARS
		|| body.chars().any(|character| {
			character.is_control() && !matches!(character, '\n' | '\r' | '\t')
		}) {
		return Err(APIError::BadRequest(
			"Message must be 1 to 4000 characters of text".to_string(),
		));
	}
	Ok(body.to_string())
}

fn message_not_found() -> APIError {
	APIError::NotFound("Reader message not found".to_string())
}

/// A newest-first page of the session's live (undeleted) messages.
async fn page(
	conn: &DatabaseConnection,
	session_id: &str,
	query: &MessagePageQuery,
	audience: Audience<'_>,
) -> APIResult<ReaderMessagePage> {
	let limit = query.limit.unwrap_or(DEFAULT_PAGE_SIZE);
	if !(1..=MAX_PAGE_SIZE).contains(&limit) {
		return Err(APIError::BadRequest(
			"Limit must be between 1 and 100".to_string(),
		));
	}
	let mut select = message::Entity::find()
		.filter(message::Column::SessionId.eq(session_id))
		.filter(message::Column::DeletedAt.is_null());
	if let Some(before) = query.before.as_deref() {
		// A tombstone stays a valid cursor, so a page boundary survives deletes.
		let cursor = message::Entity::find_by_id(before)
			.filter(message::Column::SessionId.eq(session_id))
			.one(conn)
			.await?
			.ok_or_else(|| APIError::BadRequest("Unknown message cursor".to_string()))?;
		select = select.filter(
			Condition::any()
				.add(message::Column::CreatedAt.lt(cursor.created_at))
				.add(
					Condition::all()
						.add(message::Column::CreatedAt.eq(cursor.created_at))
						.add(message::Column::Id.lt(cursor.id)),
				),
		);
	}
	let mut rows = select
		.order_by_desc(message::Column::CreatedAt)
		.order_by_desc(message::Column::Id)
		.limit(limit + 1)
		.all(conn)
		.await?;
	let has_more = rows.len() as u64 > limit;
	rows.truncate(limit as usize);

	let author_ids = rows
		.iter()
		.map(|row| row.participant_id.as_str())
		.collect::<HashSet<_>>();
	let authors = participant::Entity::find()
		.filter(participant::Column::Id.is_in(author_ids))
		.all(conn)
		.await?
		.into_iter()
		.map(|author| (author.id.clone(), author))
		.collect::<HashMap<_, _>>();
	// Authors cannot be missing: deleting a participant cascades to its messages.
	let messages = rows
		.into_iter()
		.filter_map(|row| {
			let author = authors.get(&row.participant_id)?;
			Some(project(row, author, audience))
		})
		.collect();
	Ok(ReaderMessagePage { messages, has_more })
}

/// Tombstone one live message: the row keeps its id, author, and timestamps
/// (cursor and rate window) but loses its text.
async fn tombstone<C: ConnectionTrait>(
	conn: &C,
	session_id: &str,
	message_id: &str,
	author_id: Option<&str>,
) -> APIResult<()> {
	let mut update = message::Entity::update_many()
		.col_expr(
			message::Column::DeletedAt,
			Expr::value(DateTimeWithTimeZone::from(current_utc_time())),
		)
		.col_expr(message::Column::Body, Expr::value(String::new()))
		.filter(message::Column::Id.eq(message_id))
		.filter(message::Column::SessionId.eq(session_id))
		.filter(message::Column::DeletedAt.is_null());
	if let Some(author_id) = author_id {
		update = update.filter(message::Column::ParticipantId.eq(author_id));
	}
	if update.exec(conn).await?.rows_affected == 0 {
		return Err(message_not_found());
	}
	Ok(())
}

/// `GET /sessions/{sessionId}/messages?before={messageId}&limit=50`
pub(super) async fn list(
	AxumPath(session_id): AxumPath<String>,
	State(ctx): State<AppState>,
	Query(query): Query<MessagePageQuery>,
	headers: HeaderMap,
) -> APIResult<Json<ReaderMessagePage>> {
	let access = reader::load_access(&ctx, &session_id, &headers).await?;
	let audience = Audience::Participant(&access.participant.id);
	Ok(Json(
		page(ctx.conn.as_ref(), &access.session.id, &query, audience).await?,
	))
}

/// `POST /sessions/{sessionId}/messages` `{body, bookId?}`
pub(super) async fn create(
	AxumPath(session_id): AxumPath<String>,
	State(ctx): State<AppState>,
	Extension(events): Extension<Arc<ReaderEvents>>,
	headers: HeaderMap,
	Json(input): Json<CreateMessageInput>,
) -> APIResult<(StatusCode, Json<ReaderMessage>)> {
	let access = reader::load_access(&ctx, &session_id, &headers).await?;
	security::require_expected_participant(&headers, &access.participant.id)?;
	let body = validate_body(&input.body)?;
	let txn = models::txn::begin_write(ctx.conn.as_ref()).await?;
	let session = reader::verify_open_in_transaction(&txn, &access).await?;
	if let Some(book_id) = input.book_id.as_deref() {
		let live = reader_media::live_book_id(
			&txn,
			&session.book_club_id,
			session.published_book_id.as_deref(),
		)
		.await?;
		if live.as_deref() != Some(book_id) {
			return Err(APIError::Conflict(
				"A message can only reference the current published book".to_string(),
			));
		}
	}
	// Counted under the write lock, so concurrent posts cannot overrun it.
	let now = current_utc_time();
	let recent = message::Entity::find()
		.filter(message::Column::ParticipantId.eq(&access.participant.id))
		.filter(message::Column::CreatedAt.gt(DateTimeWithTimeZone::from(
			now - Duration::seconds(RATE_WINDOW_SECS),
		)))
		.count(&txn)
		.await?;
	if recent >= MAX_MESSAGES_PER_WINDOW {
		return Err(APIError::TooManyRequests);
	}
	let created = message::ActiveModel {
		id: Set(uuid::Uuid::new_v4().to_string()),
		session_id: Set(session.id.clone()),
		participant_id: Set(access.participant.id.clone()),
		book_id: Set(input.book_id),
		body: Set(body),
		created_at: Set(DateTimeWithTimeZone::from(now)),
		edited_at: Set(None),
		deleted_at: Set(None),
	}
	.insert(&txn)
	.await?;
	txn.commit().await?;
	events.publish(&session.id, Kinds::MESSAGES);
	Ok((
		StatusCode::CREATED,
		Json(project(
			created,
			&access.participant,
			Audience::Participant(&access.participant.id),
		)),
	))
}

/// `PATCH /sessions/{sessionId}/messages/{messageId}` `{body}`: own messages only.
pub(super) async fn update(
	AxumPath((session_id, message_id)): AxumPath<(String, String)>,
	State(ctx): State<AppState>,
	Extension(events): Extension<Arc<ReaderEvents>>,
	headers: HeaderMap,
	Json(input): Json<UpdateMessageInput>,
) -> APIResult<Json<ReaderMessage>> {
	let access = reader::load_access(&ctx, &session_id, &headers).await?;
	security::require_expected_participant(&headers, &access.participant.id)?;
	let body = validate_body(&input.body)?;
	let txn = models::txn::begin_write(ctx.conn.as_ref()).await?;
	reader::verify_open_in_transaction(&txn, &access).await?;
	let existing = message::Entity::find_by_id(&message_id)
		.filter(message::Column::SessionId.eq(&access.session.id))
		.filter(message::Column::ParticipantId.eq(&access.participant.id))
		.filter(message::Column::DeletedAt.is_null())
		.one(&txn)
		.await?
		.ok_or_else(message_not_found)?;
	let audience = Audience::Participant(&access.participant.id);
	if existing.body == body {
		// Saving unchanged text is not an edit.
		return Ok(Json(project(existing, &access.participant, audience)));
	}
	let mut active = existing.into_active_model();
	active.body = Set(body);
	active.edited_at = Set(Some(DateTimeWithTimeZone::from(current_utc_time())));
	let updated = active.update(&txn).await?;
	txn.commit().await?;
	events.publish(&access.session.id, Kinds::MESSAGES);
	Ok(Json(project(updated, &access.participant, audience)))
}

/// `DELETE /sessions/{sessionId}/messages/{messageId}`: own messages only.
pub(super) async fn delete(
	AxumPath((session_id, message_id)): AxumPath<(String, String)>,
	State(ctx): State<AppState>,
	Extension(events): Extension<Arc<ReaderEvents>>,
	headers: HeaderMap,
) -> APIResult<StatusCode> {
	let access = reader::load_access(&ctx, &session_id, &headers).await?;
	security::require_expected_participant(&headers, &access.participant.id)?;
	let txn = models::txn::begin_write(ctx.conn.as_ref()).await?;
	reader::verify_open_in_transaction(&txn, &access).await?;
	tombstone(
		&txn,
		&access.session.id,
		&message_id,
		Some(&access.participant.id),
	)
	.await?;
	txn.commit().await?;
	events.publish(&access.session.id, Kinds::MESSAGES);
	Ok(StatusCode::NO_CONTENT)
}

/// `GET /book-clubs/{clubId}/reader-sessions/{sessionId}/messages`: the
/// organizer's read view, open and closed sessions alike.
pub(super) async fn organizer_list(
	AxumPath((club_id, session_id)): AxumPath<(String, String)>,
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<MessagePageQuery>,
) -> APIResult<Json<ReaderMessagePage>> {
	admin::require_manager(&ctx, &auth, &club_id).await?;
	let session = admin::load_session(&ctx, &club_id, &session_id).await?;
	Ok(Json(
		page(ctx.conn.as_ref(), &session.id, &query, Audience::Organizer).await?,
	))
}

/// `DELETE /book-clubs/{clubId}/reader-sessions/{sessionId}/messages/{messageId}`:
/// organizer moderation of any participant's message.
pub(super) async fn moderate(
	AxumPath((club_id, session_id, message_id)): AxumPath<(String, String, String)>,
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<Arc<ReaderEvents>>,
) -> APIResult<StatusCode> {
	admin::require_manager(&ctx, &auth, &club_id).await?;
	let session = admin::load_session(&ctx, &club_id, &session_id).await?;
	tombstone(ctx.conn.as_ref(), &session.id, &message_id, None).await?;
	events.publish(&session.id, Kinds::MESSAGES);
	Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn bodies_are_trimmed_bounded_plain_text() {
		assert_eq!(validate_body("  hi\n there\t ").unwrap(), "hi\n there");
		assert!(validate_body(" \n ").is_err());
		assert!(validate_body("bell\u{7}").is_err());
		assert!(validate_body(&"x".repeat(MAX_BODY_CHARS)).is_ok());
		assert!(validate_body(&"é".repeat(MAX_BODY_CHARS + 1)).is_err());
	}
}
