use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use models::entity::{
	book_club_reader_annotation, book_club_reader_message, book_club_reader_participant,
	book_club_reader_progress, book_club_reader_session,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateSessionInput {
	pub name: String,
	pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PublishBookInput {
	pub book_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateParticipantInput {
	pub display_name: String,
	pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JoinSessionInput {
	pub display_name: String,
	pub share_progress: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CredentialRotationInput {
	pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RedeemInput {
	pub session_id: String,
	pub token: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateViewerInput {
	pub display_name: Option<String>,
	pub share_progress: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProgressInput {
	pub progression: f64,
	pub locator: Option<Value>,
	pub page: Option<i32>,
	pub position_ms: Option<i64>,
	pub is_complete: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateAnnotationInput {
	pub kind: String,
	pub locator: Option<Value>,
	pub page: Option<i32>,
	pub position_ms: Option<i64>,
	pub excerpt: Option<String>,
	pub body: Option<String>,
	pub color: Option<String>,
	pub shared: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateAnnotationInput {
	pub locator: Option<Value>,
	pub page: Option<i32>,
	pub position_ms: Option<i64>,
	pub excerpt: Option<String>,
	pub body: Option<String>,
	pub color: Option<String>,
	pub shared: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MessagePageQuery {
	pub before: Option<String>,
	pub limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateMessageInput {
	pub body: String,
	pub book_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateMessageInput {
	pub body: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CredentialResponse {
	pub participant: ReaderParticipantAdmin,
	pub reader_path: String,
	pub token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReaderParticipantAdmin {
	pub id: String,
	pub display_name: String,
	pub expires_at: Option<DateTime<Utc>>,
	pub revoked_at: Option<DateTime<Utc>>,
	pub linked_account: bool,
}

impl From<book_club_reader_participant::Model> for ReaderParticipantAdmin {
	fn from(participant: book_club_reader_participant::Model) -> Self {
		Self {
			id: participant.id,
			display_name: participant.display_name,
			expires_at: participant
				.credential_expires_at
				.map(|date| date.with_timezone(&Utc)),
			revoked_at: participant.revoked_at.map(|date| date.with_timezone(&Utc)),
			linked_account: participant.linked_user_id.is_some(),
		}
	}
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReaderSessionAdmin {
	pub id: String,
	pub name: String,
	pub club_id: String,
	pub expires_at: Option<DateTime<Utc>>,
	pub closed_at: Option<DateTime<Utc>>,
	pub published_book_id: Option<String>,
	pub published_book_title: Option<String>,
	pub participants: Vec<ReaderParticipantAdmin>,
	pub can_manage: bool,
}

impl ReaderSessionAdmin {
	pub fn from_model(
		session: book_club_reader_session::Model,
		published_book_title: Option<String>,
		participants: Vec<ReaderParticipantAdmin>,
		can_manage: bool,
	) -> Self {
		Self {
			id: session.id,
			name: session.name,
			club_id: session.book_club_id,
			expires_at: session.expires_at.map(|date| date.with_timezone(&Utc)),
			closed_at: session.closed_at.map(|date| date.with_timezone(&Utc)),
			published_book_id: session.published_book_id,
			published_book_title,
			participants,
			can_manage,
		}
	}
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReaderSessionCollection {
	pub sessions: Vec<ReaderSessionAdmin>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReaderSessionInfo {
	pub id: String,
	pub name: String,
	pub club_id: String,
	pub expires_at: Option<DateTime<Utc>>,
}

impl From<book_club_reader_session::Model> for ReaderSessionInfo {
	fn from(session: book_club_reader_session::Model) -> Self {
		Self {
			id: session.id,
			name: session.name,
			club_id: session.book_club_id,
			expires_at: session.expires_at.map(|date| date.with_timezone(&Utc)),
		}
	}
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum ReaderKind {
	Epub,
	Paged,
	Audio,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReaderBook {
	pub id: String,
	pub title: String,
	pub extension: String,
	pub reader_kind: ReaderKind,
	pub page_count: i32,
	pub visible_pages: Vec<i32>,
	pub audio: Option<ReaderAudio>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReaderAudio {
	pub duration_ms: i64,
	pub codec: String,
	pub chapter_source: String,
	pub tracks: Vec<ReaderAudioTrack>,
	pub chapters: Vec<ReaderAudioChapter>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReaderAudioTrack {
	pub index: i32,
	pub mime: String,
	pub duration_ms: i64,
	pub start_offset_ms: i64,
	pub byte_size: i64,
	pub url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReaderAudioChapter {
	pub index: i32,
	pub title: Option<String>,
	pub start_ms: i64,
	pub end_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReaderViewer {
	pub id: String,
	pub display_name: String,
	pub share_progress: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReaderProgress {
	pub progression: f64,
	pub locator: Option<Value>,
	pub page: Option<i32>,
	pub position_ms: Option<i64>,
	pub is_complete: bool,
}

impl From<book_club_reader_progress::Model> for ReaderProgress {
	fn from(progress: book_club_reader_progress::Model) -> Self {
		Self {
			progression: progress.progression,
			locator: progress.locator,
			page: progress.page,
			position_ms: progress.position_ms,
			is_complete: progress.is_complete,
		}
	}
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReaderParticipantSnapshot {
	pub id: String,
	pub display_name: String,
	pub percentage: Option<i32>,
	pub is_complete: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReaderAnnotation {
	pub id: String,
	pub author_id: String,
	pub author_name: String,
	pub book_id: String,
	pub kind: String,
	pub locator: Option<Value>,
	pub page: Option<i32>,
	pub position_ms: Option<i64>,
	pub excerpt: Option<String>,
	pub body: Option<String>,
	pub color: Option<String>,
	pub shared: bool,
	pub editable: bool,
}

impl ReaderAnnotation {
	pub fn from_model(
		annotation: book_club_reader_annotation::Model,
		author_name: String,
		editable: bool,
	) -> Self {
		Self {
			id: annotation.id,
			author_id: annotation.participant_id,
			author_name,
			book_id: annotation.book_id,
			kind: annotation.kind,
			locator: annotation.locator,
			page: annotation.page,
			position_ms: annotation.position_ms,
			excerpt: annotation.excerpt,
			body: annotation.body,
			color: annotation.color,
			shared: annotation.shared,
			editable,
		}
	}
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReaderSnapshot {
	pub session: ReaderSessionInfo,
	pub book: Option<ReaderBook>,
	pub viewer: ReaderViewer,
	pub progress: Option<ReaderProgress>,
	pub participants: Vec<ReaderParticipantSnapshot>,
	pub annotations: Vec<ReaderAnnotation>,
}

/// One session-discussion message. `author_id` is `None` for a participant
/// reader once the author is no longer active, so a revoked or expired
/// capability cannot be linked back to its messages.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReaderMessage {
	pub id: String,
	pub author_id: Option<String>,
	pub author_name: String,
	pub book_id: Option<String>,
	pub body: String,
	pub created_at: DateTime<Utc>,
	pub edited_at: Option<DateTime<Utc>>,
	pub mine: bool,
}

impl ReaderMessage {
	pub fn from_model(
		message: book_club_reader_message::Model,
		author_id: Option<String>,
		author_name: String,
		mine: bool,
	) -> Self {
		Self {
			id: message.id,
			author_id,
			author_name,
			book_id: message.book_id,
			body: message.body,
			created_at: message.created_at.with_timezone(&Utc),
			edited_at: message.edited_at.map(|date| date.with_timezone(&Utc)),
			mine,
		}
	}
}

/// A newest-first page of a session discussion.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReaderMessagePage {
	pub messages: Vec<ReaderMessage>,
	pub has_more: bool,
}
