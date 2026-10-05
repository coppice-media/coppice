use std::{collections::HashMap, sync::Arc};

use axum::{
	extract::{Path as AxumPath, State},
	http::{header, HeaderMap, StatusCode},
	response::{IntoResponse, Response},
	Extension, Json,
};
use chrono::Utc;
use models::entity::{
	book_club_book, book_club_reader_annotation as annotation,
	book_club_reader_participant as participant, book_club_reader_progress as progress,
	book_club_reader_session as session, media, user,
};
use sea_orm::{
	prelude::*, ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait,
	IntoActiveModel, QueryFilter, QueryOrder, Set,
};
use serde_json::Value;

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	utils::current_utc_time,
};

use super::{
	events::{Kinds, ReaderEvents},
	media as reader_media, security,
	types::*,
};

const MAX_DISPLAY_NAME_CHARS: usize = 80;
const MAX_ANNOTATION_TEXT_CHARS: usize = 10_000;
const MAX_LOCATOR_BYTES: usize = 32 * 1024;

pub(super) struct GuestAccess {
	pub session: session::Model,
	pub participant: participant::Model,
	pub publisher: user::AuthUser,
	credential_digest: String,
}

pub(super) async fn load_access(
	ctx: &AppState,
	session_id: &str,
	headers: &HeaderMap,
) -> APIResult<GuestAccess> {
	let secret = security::cookie_token(headers)?;
	load_access_from_secret(ctx, session_id, &secret).await
}

/// The full capability check behind [`load_access`], also re-run by every live
/// event stream on each notification and timer tick.
pub(super) async fn load_access_from_secret(
	ctx: &AppState,
	session_id: &str,
	secret: &str,
) -> APIResult<GuestAccess> {
	if session_id.len() > 64 || secret.len() != 43 {
		return Err(APIError::Unauthorized);
	}
	let session = session::Entity::find_by_id(session_id)
		.one(ctx.conn.as_ref())
		.await?
		.ok_or(APIError::Unauthorized)?;
	if session.closed_at.is_some() || security::is_expired(session.expires_at.clone()) {
		return Err(APIError::Unauthorized);
	}

	let digest = security::secret_digest(secret);
	let participant = participant::Entity::find()
		.filter(participant::Column::SessionId.eq(session_id))
		.filter(participant::Column::TokenDigest.eq(digest.clone()))
		.filter(participant::Column::RevokedAt.is_null())
		.one(ctx.conn.as_ref())
		.await?
		.ok_or(APIError::Unauthorized)?;
	let stored_digest = participant
		.token_digest
		.as_deref()
		.ok_or(APIError::Unauthorized)?;
	if !security::digest_matches(secret, stored_digest)
		|| security::is_expired(participant.credential_expires_at.clone())
	{
		return Err(APIError::Unauthorized);
	}

	let issuer = active_account(ctx, &session.created_by_user_id).await?;
	security::authorize_club_member(
		ctx,
		&issuer,
		&session.book_club_id,
		Some(models::shared::book_club::BookClubMemberRole::Admin),
		true,
	)
	.await
	.map_err(|_| APIError::Unauthorized)?;

	let published = if session.published_book_id.is_some() {
		let publisher_id = session
			.published_by_user_id
			.as_deref()
			.ok_or(APIError::Unauthorized)?;
		let publisher = active_account(ctx, publisher_id).await?;
		security::authorize_club_member(
			ctx,
			&publisher,
			&session.book_club_id,
			Some(models::shared::book_club::BookClubMemberRole::Admin),
			true,
		)
		.await
		.map_err(|_| APIError::Unauthorized)?;
		Some(publisher)
	} else {
		None
	};

	let publisher = if let Some(publisher) = published {
		if let Some(book_id) = session.published_book_id.as_deref() {
			if let Some(book) = book_club_book::Entity::find_by_id(book_id)
				.filter(book_club_book::Column::BookClubId.eq(&session.book_club_id))
				.one(ctx.conn.as_ref())
				.await?
			{
				if let Some(media_id) = book.book_entity_id.as_deref() {
					let exists = media::Entity::find_by_id(media_id)
						.filter(media::Column::DeletedAt.is_null())
						.one(ctx.conn.as_ref())
						.await?
						.is_some();
					if exists
						&& media::Entity::find_for_user(&publisher)
							.filter(media::Column::Id.eq(media_id))
							.filter(media::Column::DeletedAt.is_null())
							.one(ctx.conn.as_ref())
							.await?
							.is_none()
					{
						return Err(APIError::Unauthorized);
					}
				}
			}
		}
		publisher
	} else {
		issuer
	};

	Ok(GuestAccess {
		session,
		participant,
		publisher,
		credential_digest: digest,
	})
}

/// The live account behind a guest capability. Deleted or locked accounts
/// stop delegating immediately; permissions decode through `LoginUser`.
async fn active_account(ctx: &AppState, user_id: &str) -> APIResult<user::AuthUser> {
	let account = user::LoginUser::find_by_id(user_id.to_owned())
		.filter(user::Column::DeletedAt.is_null())
		.into_model::<user::LoginUser>()
		.one(ctx.conn.as_ref())
		.await?
		.ok_or(APIError::Unauthorized)?;
	if account.is_locked {
		return Err(APIError::Unauthorized);
	}
	Ok(user::AuthUser::from(account))
}

/// A participant other readers still see: neither revoked nor expired.
pub(super) fn is_active_participant(participant: &participant::Model) -> bool {
	participant.revoked_at.is_none()
		&& !security::is_expired(participant.credential_expires_at)
}

pub(super) async fn redeem(
	State(ctx): State<AppState>,
	crate::middleware::HostExtractor(host): crate::middleware::HostExtractor,
	Json(input): Json<RedeemInput>,
) -> APIResult<Response> {
	let access = load_access_from_secret(&ctx, &input.session_id, &input.token).await?;
	let expires_at = access
		.participant
		.credential_expires_at
		.as_ref()
		.map(|expires_at| expires_at.with_timezone(&Utc));
	let secure = !is_loopback_http(&host);
	let cookie = security::credential_cookie(
		&access.session.id,
		&input.token,
		expires_at,
		secure,
	)?;
	let body = Json(serde_json::json!({ "sessionId": access.session.id }));
	let mut response = body.into_response();
	response.headers_mut().insert(header::SET_COOKIE, cookie);
	Ok(response)
}

fn is_loopback_http(host: &crate::middleware::host::HostDetails) -> bool {
	if host.scheme != "http" {
		return false;
	}
	let Ok(authority) = host.host.parse::<axum::http::uri::Authority>() else {
		return false;
	};
	matches!(authority.host(), "localhost" | "127.0.0.1" | "::1")
}

pub(super) async fn snapshot(
	AxumPath(session_id): AxumPath<String>,
	State(ctx): State<AppState>,
	headers: HeaderMap,
) -> APIResult<Json<ReaderSnapshot>> {
	let access = load_access(&ctx, &session_id, &headers).await?;
	Ok(Json(build_snapshot(&ctx, &access).await?))
}

async fn build_snapshot(
	ctx: &AppState,
	access: &GuestAccess,
) -> APIResult<ReaderSnapshot> {
	let book = reader_media::snapshot_book(ctx, access).await?;
	let book_id = book.as_ref().map(|book| book.id.as_str());
	let participants = participant::Entity::find()
		.filter(participant::Column::SessionId.eq(&access.session.id))
		.order_by_asc(participant::Column::CreatedAt)
		.all(ctx.conn.as_ref())
		.await?;
	let active_participants = participants
		.into_iter()
		.filter(is_active_participant)
		.collect::<Vec<_>>();
	let active_by_id = active_participants
		.iter()
		.map(|participant| (participant.id.as_str(), participant))
		.collect::<HashMap<_, _>>();
	let current_progress = if let Some(book_id) = book_id {
		progress::Entity::find()
			.filter(progress::Column::SessionId.eq(&access.session.id))
			.filter(progress::Column::BookId.eq(book_id))
			.all(ctx.conn.as_ref())
			.await?
	} else {
		Vec::new()
	};
	let progress_by_participant = current_progress
		.iter()
		.map(|entry| (entry.participant_id.as_str(), entry))
		.collect::<HashMap<_, _>>();
	let participants = active_participants
		.iter()
		.filter(|participant| participant.id != access.participant.id)
		.map(|participant| {
			let visible = participant.share_progress.then(|| {
				progress_by_participant
					.get(participant.id.as_str())
					.map(|progress| {
						(
							models::services::social::coarse_percentage(
								progress.progression,
							),
							progress.is_complete,
						)
					})
			});
			let projection = visible.flatten();
			ReaderParticipantSnapshot {
				id: participant.id.clone(),
				display_name: participant.display_name.clone(),
				percentage: projection.map(|projection| projection.0),
				is_complete: projection.map(|projection| projection.1),
			}
		})
		.collect();
	let own_progress = book_id.and_then(|_| {
		progress_by_participant
			.get(access.participant.id.as_str())
			.map(|progress| ReaderProgress::from((*progress).clone()))
	});
	let annotations = if let Some(book_id) = book_id {
		annotation::Entity::find()
			.filter(annotation::Column::SessionId.eq(&access.session.id))
			.filter(annotation::Column::BookId.eq(book_id))
			.order_by_asc(annotation::Column::CreatedAt)
			.all(ctx.conn.as_ref())
			.await?
			.into_iter()
			.filter_map(|annotation| {
				let own = annotation.participant_id == access.participant.id;
				let author = active_by_id.get(annotation.participant_id.as_str())?;
				(own || annotation.shared).then(|| {
					ReaderAnnotation::from_model(
						annotation,
						author.display_name.clone(),
						own,
					)
				})
			})
			.collect()
	} else {
		Vec::new()
	};

	Ok(ReaderSnapshot {
		session: ReaderSessionInfo::from(access.session.clone()),
		book,
		viewer: ReaderViewer {
			id: access.participant.id.clone(),
			display_name: access.participant.display_name.clone(),
			share_progress: access.participant.share_progress,
		},
		progress: own_progress,
		participants,
		annotations,
	})
}

pub(super) async fn update_viewer(
	AxumPath(session_id): AxumPath<String>,
	State(ctx): State<AppState>,
	Extension(events): Extension<Arc<ReaderEvents>>,
	headers: HeaderMap,
	Json(input): Json<UpdateViewerInput>,
) -> APIResult<Json<ReaderSnapshot>> {
	let access = load_access(&ctx, &session_id, &headers).await?;
	security::require_expected_participant(&headers, &access.participant.id)?;
	let mut changed = Kinds::NONE;
	let mut active = access.participant.clone().into_active_model();
	if let Some(display_name) = input.display_name {
		let display_name = validate_display_name(&display_name)?;
		if display_name != access.participant.display_name {
			// Annotation and message authors resolve to the current alias.
			changed |= Kinds::PARTICIPANTS | Kinds::ANNOTATIONS | Kinds::MESSAGES;
		}
		active.display_name = Set(display_name);
	}
	if let Some(share_progress) = input.share_progress {
		if share_progress != access.participant.share_progress {
			changed |= Kinds::PARTICIPANTS;
		}
		active.share_progress = Set(share_progress);
	}
	active.updated_at = Set(DateTimeWithTimeZone::from(current_utc_time()));
	active.update(ctx.conn.as_ref()).await?;
	if !changed.is_empty() {
		events.publish(&access.session.id, changed);
	}
	let access = load_access(&ctx, &session_id, &headers).await?;
	Ok(Json(build_snapshot(&ctx, &access).await?))
}

pub(super) async fn update_progress(
	AxumPath((session_id, book_id)): AxumPath<(String, String)>,
	State(ctx): State<AppState>,
	Extension(events): Extension<Arc<ReaderEvents>>,
	headers: HeaderMap,
	Json(input): Json<ProgressInput>,
) -> APIResult<Json<ReaderSnapshot>> {
	let access = load_access(&ctx, &session_id, &headers).await?;
	security::require_expected_participant(&headers, &access.participant.id)?;
	let (book_row, _, book) = reader_media::active_book(&ctx, &access, &book_id).await?;
	validate_progress(&input, &book)?;
	let txn = models::txn::begin_write(ctx.conn.as_ref()).await?;
	verify_live_access_in_transaction(&txn, &access, &book_id).await?;
	let current = progress::Entity::find()
		.filter(progress::Column::SessionId.eq(&session_id))
		.filter(progress::Column::ParticipantId.eq(&access.participant.id))
		.filter(progress::Column::BookId.eq(&book_row.id))
		.one(&txn)
		.await?;
	let now = DateTimeWithTimeZone::from(current_utc_time());
	if let Some(current) = current {
		let mut active = current.into_active_model();
		active.progression = Set(input.progression);
		active.locator = Set(input.locator);
		active.page = Set(input.page);
		active.position_ms = Set(input.position_ms);
		active.is_complete = Set(input.is_complete.unwrap_or(false));
		active.updated_at = Set(now);
		active.update(&txn).await?;
	} else {
		progress::ActiveModel {
			id: Set(uuid::Uuid::new_v4().to_string()),
			session_id: Set(session_id),
			participant_id: Set(access.participant.id.clone()),
			book_id: Set(book_row.id),
			progression: Set(input.progression),
			locator: Set(input.locator),
			page: Set(input.page),
			position_ms: Set(input.position_ms),
			is_complete: Set(input.is_complete.unwrap_or(false)),
			created_at: Set(now),
			updated_at: Set(now),
		}
		.insert(&txn)
		.await?;
	}
	txn.commit().await?;
	events.publish(&access.session.id, Kinds::PROGRESS);
	Ok(Json(build_snapshot(&ctx, &access).await?))
}

async fn verify_live_access_in_transaction(
	txn: &DatabaseTransaction,
	access: &GuestAccess,
	book_id: &str,
) -> APIResult<()> {
	let session = session::Entity::find_by_id(&access.session.id)
		.one(txn)
		.await?
		.ok_or(APIError::Unauthorized)?;
	if session.closed_at.is_some()
		|| security::is_expired(session.expires_at.clone())
		|| session.published_book_id.as_deref() != Some(book_id)
	{
		return Err(APIError::NotFound("Reader book not found".to_string()));
	}
	verify_capability_in_transaction(txn, access).await?;
	let head = reader_media::queue_head(txn, &session.book_club_id).await?;
	if head.as_ref().map(|head| head.id.as_str()) != Some(book_id) {
		return Err(reader_media::paused_error());
	}
	Ok(())
}

/// Re-read the session and this request's capability under the write lock, so
/// a concurrent close, revocation, or rotation wins over a discussion write.
/// Unlike book writes this holds while the session is paused or unpublished.
pub(super) async fn verify_open_in_transaction(
	txn: &DatabaseTransaction,
	access: &GuestAccess,
) -> APIResult<session::Model> {
	let session = session::Entity::find_by_id(&access.session.id)
		.one(txn)
		.await?
		.ok_or(APIError::Unauthorized)?;
	if session.closed_at.is_some() || security::is_expired(session.expires_at) {
		return Err(APIError::Unauthorized);
	}
	verify_capability_in_transaction(txn, access).await?;
	Ok(session)
}

/// The participant row still carries exactly this request's live capability.
async fn verify_capability_in_transaction(
	txn: &DatabaseTransaction,
	access: &GuestAccess,
) -> APIResult<()> {
	let participant = participant::Entity::find_by_id(&access.participant.id)
		.one(txn)
		.await?
		.ok_or(APIError::Unauthorized)?;
	if participant.session_id != access.session.id
		|| participant.revoked_at.is_some()
		|| participant.token_digest.as_deref() != Some(&access.credential_digest)
		|| security::is_expired(participant.credential_expires_at)
	{
		return Err(APIError::Unauthorized);
	}
	Ok(())
}

fn validate_progress(input: &ProgressInput, book: &ReaderBook) -> APIResult<()> {
	if !input.progression.is_finite() || !(0.0..=1.0).contains(&input.progression) {
		return Err(APIError::BadRequest(
			"Progression must be between 0 and 1".to_string(),
		));
	}
	match book.reader_kind {
		ReaderKind::Epub => {
			if input.page.is_some() || input.position_ms.is_some() {
				return Err(APIError::BadRequest("Invalid EPUB position".to_string()));
			}
		},
		ReaderKind::Paged => {
			if input.locator.is_some() || input.position_ms.is_some() {
				return Err(APIError::BadRequest("Invalid page position".to_string()));
			}
			if input
				.page
				.is_some_and(|page| page <= 0 || page > book.page_count)
			{
				return Err(APIError::BadRequest(
					"Page is outside the published book".to_string(),
				));
			}
		},
		ReaderKind::Audio => {
			if input.locator.is_some() || input.page.is_some() {
				return Err(APIError::BadRequest("Invalid audio position".to_string()));
			}
			if input.position_ms.is_some_and(|position| {
				position < 0
					|| book
						.audio
						.as_ref()
						.is_none_or(|audio| position > audio.duration_ms)
			}) {
				return Err(APIError::BadRequest(
					"Position is outside the published audio".to_string(),
				));
			}
		},
	}
	if let Some(locator) = input.locator.as_ref() {
		validate_locator(locator)?;
	}
	Ok(())
}

pub(super) async fn create_annotation(
	AxumPath((session_id, book_id)): AxumPath<(String, String)>,
	State(ctx): State<AppState>,
	Extension(events): Extension<Arc<ReaderEvents>>,
	headers: HeaderMap,
	Json(input): Json<CreateAnnotationInput>,
) -> APIResult<(StatusCode, Json<ReaderAnnotation>)> {
	let access = load_access(&ctx, &session_id, &headers).await?;
	security::require_expected_participant(&headers, &access.participant.id)?;
	let (book_row, _, book) = reader_media::active_book(&ctx, &access, &book_id).await?;
	let fields = AnnotationFields {
		kind: input.kind,
		locator: input.locator,
		page: input.page,
		position_ms: input.position_ms,
		excerpt: input.excerpt,
		body: input.body,
		color: input.color,
		shared: input.shared.unwrap_or(false),
	};
	let fields = validate_annotation_fields(fields, &book)?;
	let txn = models::txn::begin_write(ctx.conn.as_ref()).await?;
	verify_live_access_in_transaction(&txn, &access, &book_id).await?;
	let now = DateTimeWithTimeZone::from(current_utc_time());
	let annotation = annotation::ActiveModel {
		id: Set(uuid::Uuid::new_v4().to_string()),
		session_id: Set(session_id),
		participant_id: Set(access.participant.id.clone()),
		book_id: Set(book_row.id),
		kind: Set(fields.kind),
		locator: Set(fields.locator),
		page: Set(fields.page),
		position_ms: Set(fields.position_ms),
		excerpt: Set(fields.excerpt),
		body: Set(fields.body),
		color: Set(fields.color),
		shared: Set(fields.shared),
		created_at: Set(now),
		updated_at: Set(now),
	}
	.insert(&txn)
	.await?;
	txn.commit().await?;
	events.publish(&access.session.id, Kinds::ANNOTATIONS);
	Ok((
		StatusCode::CREATED,
		Json(ReaderAnnotation::from_model(
			annotation,
			access.participant.display_name,
			true,
		)),
	))
}

struct AnnotationFields {
	kind: String,
	locator: Option<Value>,
	page: Option<i32>,
	position_ms: Option<i64>,
	excerpt: Option<String>,
	body: Option<String>,
	color: Option<String>,
	shared: bool,
}

fn validate_annotation_fields(
	mut fields: AnnotationFields,
	book: &ReaderBook,
) -> APIResult<AnnotationFields> {
	if !matches!(fields.kind.as_str(), "highlight" | "note") {
		return Err(APIError::BadRequest(
			"Annotation kind must be highlight or note".to_string(),
		));
	}
	if fields.color.as_deref().is_some_and(|color| {
		!models::entity::social_share_overlay::COLORS.contains(&color)
	}) {
		return Err(APIError::BadRequest(
			"Unsupported annotation color".to_string(),
		));
	}
	if fields
		.body
		.as_ref()
		.is_some_and(|body| body.chars().count() > MAX_ANNOTATION_TEXT_CHARS)
		|| fields
			.excerpt
			.as_ref()
			.is_some_and(|excerpt| excerpt.chars().count() > MAX_ANNOTATION_TEXT_CHARS)
	{
		return Err(APIError::BadRequest(
			"Annotation text is too long".to_string(),
		));
	}
	match book.reader_kind {
		ReaderKind::Epub => {
			if fields.page.is_some() || fields.position_ms.is_some() {
				return Err(APIError::BadRequest(
					"Invalid EPUB annotation anchor".to_string(),
				));
			}
		},
		ReaderKind::Paged => {
			if fields.locator.is_some() || fields.position_ms.is_some() {
				return Err(APIError::BadRequest(
					"Invalid page annotation anchor".to_string(),
				));
			}
			validate_page(fields.page, book)?;
		},
		ReaderKind::Audio => {
			if fields.locator.is_some() || fields.page.is_some() {
				return Err(APIError::BadRequest(
					"Invalid audio annotation anchor".to_string(),
				));
			}
			validate_audio_position(fields.position_ms, book)?;
		},
	}
	if let Some(locator) = fields.locator.as_ref() {
		validate_locator(locator)?;
	}
	if fields.kind == "highlight" {
		if fields
			.excerpt
			.as_deref()
			.is_none_or(|excerpt| excerpt.trim().is_empty())
		{
			return Err(APIError::BadRequest(
				"A highlight requires selected text".to_string(),
			));
		}
		let has_anchor = match book.reader_kind {
			ReaderKind::Epub => fields
				.locator
				.as_ref()
				.and_then(|locator| locator.get("href"))
				.and_then(Value::as_str)
				.is_some(),
			ReaderKind::Paged => fields.page.is_some(),
			ReaderKind::Audio => fields.position_ms.is_some(),
		};
		if !has_anchor {
			return Err(APIError::BadRequest(
				"A highlight requires a book-relative anchor".to_string(),
			));
		}
	} else if fields
		.body
		.as_deref()
		.is_none_or(|body| body.trim().is_empty())
	{
	}
	if fields
		.body
		.as_deref()
		.is_some_and(|body| body.trim().is_empty())
	{
		fields.body = None;
	}
	Ok(fields)
}

fn validate_locator(locator: &Value) -> APIResult<()> {
	let encoded = serde_json::to_vec(locator)
		.map_err(|_| APIError::BadRequest("Invalid annotation locator".to_string()))?;
	if encoded.len() > MAX_LOCATOR_BYTES || !locator.is_object() {
		return Err(APIError::BadRequest(
			"Invalid annotation locator".to_string(),
		));
	}
	let Some(href) = locator.get("href").and_then(Value::as_str) else {
		return Err(APIError::BadRequest(
			"Invalid annotation locator".to_string(),
		));
	};
	if href.is_empty()
		|| href.len() > 2_048
		|| href.contains('\\')
		|| href.starts_with('/')
	{
		return Err(APIError::BadRequest(
			"Invalid annotation locator".to_string(),
		));
	}
	let path = href.split(['#', '?']).next().unwrap_or_default();
	if path.is_empty()
		|| path.contains("://")
		|| path
			.split('/')
			.next()
			.is_some_and(|segment| segment.contains(':'))
		|| std::path::Path::new(path)
			.components()
			.any(|component| !matches!(component, std::path::Component::Normal(_)))
	{
		return Err(APIError::BadRequest(
			"Invalid annotation locator".to_string(),
		));
	}
	Ok(())
}

fn validate_page(page: Option<i32>, book: &ReaderBook) -> APIResult<()> {
	if page.is_some_and(|page| page <= 0 || page > book.page_count) {
		return Err(APIError::BadRequest(
			"Page is outside the published book".to_string(),
		));
	}
	Ok(())
}

fn validate_audio_position(position_ms: Option<i64>, book: &ReaderBook) -> APIResult<()> {
	if position_ms.is_some_and(|position| {
		position < 0
			|| book
				.audio
				.as_ref()
				.is_none_or(|audio| position > audio.duration_ms)
	}) {
		return Err(APIError::BadRequest(
			"Position is outside the published audio".to_string(),
		));
	}
	Ok(())
}

pub(super) async fn update_annotation(
	AxumPath((session_id, book_id, annotation_id)): AxumPath<(String, String, String)>,
	State(ctx): State<AppState>,
	Extension(events): Extension<Arc<ReaderEvents>>,
	headers: HeaderMap,
	Json(input): Json<UpdateAnnotationInput>,
) -> APIResult<Json<ReaderAnnotation>> {
	let access = load_access(&ctx, &session_id, &headers).await?;
	security::require_expected_participant(&headers, &access.participant.id)?;
	let (book_row, _, book) = reader_media::active_book(&ctx, &access, &book_id).await?;
	let existing = annotation::Entity::find_by_id(&annotation_id)
		.filter(annotation::Column::SessionId.eq(&session_id))
		.filter(annotation::Column::BookId.eq(&book_row.id))
		.filter(annotation::Column::ParticipantId.eq(&access.participant.id))
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| APIError::NotFound("Reader annotation not found".to_string()))?;
	let fields = AnnotationFields {
		kind: existing.kind.clone(),
		locator: input.locator.or(existing.locator),
		page: input.page.or(existing.page),
		position_ms: input.position_ms.or(existing.position_ms),
		excerpt: input.excerpt.or(existing.excerpt),
		body: input.body.or(existing.body),
		color: input.color.or(existing.color),
		shared: input.shared.unwrap_or(existing.shared),
	};
	let fields = validate_annotation_fields(fields, &book)?;
	let txn = models::txn::begin_write(ctx.conn.as_ref()).await?;
	verify_live_access_in_transaction(&txn, &access, &book_id).await?;
	let existing = annotation::Entity::find_by_id(&annotation_id)
		.filter(annotation::Column::SessionId.eq(&session_id))
		.filter(annotation::Column::BookId.eq(&book_row.id))
		.filter(annotation::Column::ParticipantId.eq(&access.participant.id))
		.one(&txn)
		.await?
		.ok_or_else(|| APIError::NotFound("Reader annotation not found".to_string()))?;
	let mut active = existing.into_active_model();
	active.locator = Set(fields.locator);
	active.page = Set(fields.page);
	active.position_ms = Set(fields.position_ms);
	active.excerpt = Set(fields.excerpt);
	active.body = Set(fields.body);
	active.color = Set(fields.color);
	active.shared = Set(fields.shared);
	active.updated_at = Set(DateTimeWithTimeZone::from(current_utc_time()));
	let updated = active.update(&txn).await?;
	txn.commit().await?;
	events.publish(&access.session.id, Kinds::ANNOTATIONS);
	Ok(Json(ReaderAnnotation::from_model(
		updated,
		access.participant.display_name,
		true,
	)))
}

pub(super) async fn delete_annotation(
	AxumPath((session_id, book_id, annotation_id)): AxumPath<(String, String, String)>,
	State(ctx): State<AppState>,
	Extension(events): Extension<Arc<ReaderEvents>>,
	headers: HeaderMap,
) -> APIResult<StatusCode> {
	let access = load_access(&ctx, &session_id, &headers).await?;
	security::require_expected_participant(&headers, &access.participant.id)?;
	let (book_row, _, _) = reader_media::active_book(&ctx, &access, &book_id).await?;
	let txn = models::txn::begin_write(ctx.conn.as_ref()).await?;
	verify_live_access_in_transaction(&txn, &access, &book_id).await?;
	let result = annotation::Entity::delete_many()
		.filter(annotation::Column::Id.eq(annotation_id))
		.filter(annotation::Column::SessionId.eq(session_id))
		.filter(annotation::Column::BookId.eq(book_row.id))
		.filter(annotation::Column::ParticipantId.eq(access.participant.id))
		.exec(&txn)
		.await?;
	if result.rows_affected == 0 {
		return Err(APIError::NotFound(
			"Reader annotation not found".to_string(),
		));
	}
	txn.commit().await?;
	events.publish(&access.session.id, Kinds::ANNOTATIONS);
	Ok(StatusCode::NO_CONTENT)
}

pub(super) fn validate_display_name(display_name: &str) -> APIResult<String> {
	let display_name = display_name.trim();
	if display_name.is_empty()
		|| display_name.chars().count() > MAX_DISPLAY_NAME_CHARS
		|| display_name.chars().any(char::is_control)
	{
		return Err(APIError::BadRequest(
			"Display name must be 1 to 80 characters".to_string(),
		));
	}
	Ok(display_name.to_string())
}
