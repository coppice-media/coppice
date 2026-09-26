//! The native liseur-sync wire contract (`/v1/*`).
//!
//! The crate deliberately owns the protocol DTOs, bearer middleware, validation
//! and HTTP handlers, while the application supplies persistence through
//! [`LiseurSyncBackend`].  This keeps the protocol usable by a headless server
//! without coupling it to Stump's database implementation.
//!
//! See `crates/liseur-sync/README.md` for pinned client/server contracts,
//! compatibility notes, decisions, and verification.

use std::{
	collections::{BTreeMap, HashMap, HashSet},
	fmt,
};

use async_trait::async_trait;

use axum::{
	body::{Body, Bytes},
	extract::{rejection::JsonRejection, DefaultBodyLimit, Json, Path, Query, Request},
	http::{header, HeaderMap, HeaderValue, StatusCode, Uri},
	middleware::Next,
	response::{IntoResponse, Response},
	routing::{any, delete, get, post, put},
	Extension, Router,
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use stump_auth::AuthContext;
use thiserror::Error;
use tower_http::services::ServeFile;

const MAX_OP_BATCH: usize = 500;
const MAX_SESSION_BATCH: usize = 1_000;
const MAX_ANNOTATION_BATCH: usize = 100;
const MAX_ID_BYTES: usize = 64;
const MAX_REFERENCE_BYTES: usize = 128;
const MAX_CLIENT_TS_BYTES: usize = 64;
const MAX_LOCATOR_BYTES: usize = 16 * 1024;
const MAX_EXCERPT_BYTES: usize = 1024;
const MAX_BODY_BYTES: usize = 16 * 1024;
const MAX_TOKEN_NAME_BYTES: usize = 256;

pub const MAX_SETTINGS_PER_ACCOUNT: usize = 256;
pub const MAX_SETTING_KEY_BYTES: usize = 128;
pub const MAX_SETTING_VALUE_BYTES: usize = 4 * 1024;
const MAX_SYNC_BODY_BYTES: usize = 1 << 20;
pub const MAX_SESSION_ACTIVE_MS: i64 = 9_007_199_254_740_991;

/// Upper bound on one attachment when the host does not override it through
/// [`LiseurSyncBackend::attachment_max_bytes`].
pub const DEFAULT_ATTACHMENT_MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_MEDIA_TYPE_BYTES: usize = 128;
const ATTACHMENT_SHA256_HEADER: &str = "x-attachment-sha256";

const KNOWN_SCOPES: [&str; 7] = [
	"sync",
	"read-insights",
	"library-read",
	"library-manage",
	"library-upload",
	"library-delete",
	"admin",
];
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum LiseurSyncError {
	#[error("authentication required")]
	Unauthorized,
	#[error("{0}")]
	Forbidden(String),
	#[error("{0}")]
	BadRequest(String),
	#[error("{0}")]
	NotFound(String),
	#[error("{0}")]
	Gone(String),
	#[error("{0}")]
	Conflict(String),
	#[error("{0}")]
	PayloadTooLarge(String),
	#[error("{0}")]
	TimeInFuture(String),
	#[error("{message}")]
	ItemRefusal {
		status: StatusCode,
		code: &'static str,
		message: String,
		item_index: Option<usize>,
		session_id: Option<String>,
		op_id: Option<String>,
		work_id: Option<String>,
		limit: Option<usize>,
	},
	#[error("identifiers resolve to multiple works")]
	IdentityConflict(Vec<String>),
	#[error("{0}")]
	Internal(String),
}

/// A bearer token accepted by the native protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiseurTokenKind {
	/// The short-lived credential returned by `POST /v1/login`.
	LoginSession,
	/// A long-lived per-device credential returned by `POST /v1/tokens`.
	Device,
}

/// A bearer token accepted by the native protocol.
#[derive(Clone, Debug)]
pub struct LiseurToken {
	/// The regular Stump authenticated user context.
	pub context: AuthContext,
	/// Server-generated device identity stamped onto pushed records.
	pub device_id: String,
	/// Display name recorded when the token was created.
	pub name: String,
	/// Protocol scopes granted to the token.
	pub scopes: Vec<String>,
	/// Whether this credential is the short-lived login session or a device token.
	pub kind: LiseurTokenKind,
	/// An optional backend token identifier for diagnostics.
	pub token_id: Option<String>,
}

impl LiseurToken {
	/// Returns whether this token has a named scope.
	pub fn has_scope(&self, scope: &str) -> bool {
		self.scopes.iter().any(|candidate| candidate == scope)
	}

	/// Returns whether this token has a named scope, including `admin`'s
	/// protocol-wide implication.
	pub fn allows_scope(&self, scope: &str) -> bool {
		self.has_scope(scope) || self.has_scope("admin")
	}

	/// Returns whether this is the short-lived credential from `/v1/login`.
	pub fn is_login_session(&self) -> bool {
		self.kind == LiseurTokenKind::LoginSession
	}
}

/// Result of authenticating Stump credentials at the protocol boundary.
#[derive(Clone, Debug, Serialize)]
pub struct LoginResult {
	/// Bearer secret for the short-lived token-management session.
	pub auth_token: String,
	/// Number of seconds for which the bearer secret is valid.
	pub expires_in: i64,
}

/// Credentials accepted by `POST /v1/login`.
#[derive(Clone, Debug, Deserialize)]
pub struct LoginRequest {
	pub username: String,
	pub password: String,
}

/// Credentials accepted by `POST /v1/tokens`.
///
/// `scope` is retained as the upstream legacy singleton form. New clients
/// send `scopes`; when both are present they must describe the same set.
#[derive(Clone, Debug, Deserialize)]
pub struct TokenCreateRequest {
	pub name: String,
	#[serde(default)]
	pub scope: Option<String>,
	#[serde(default)]
	pub scopes: Option<Vec<String>>,
	#[serde(default)]
	pub expires_in_seconds: Option<i64>,
}

/// The one-time response from `POST /v1/tokens`.
#[derive(Clone, Debug, Serialize)]
pub struct TokenCreateResult {
	pub token_id: String,
	pub device_id: String,
	pub name: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub scope: Option<String>,
	pub scopes: Vec<String>,
	pub secret: String,
	pub expires_at: Option<String>,
}

/// The public description returned by `GET /v1/token`.
#[derive(Clone, Debug, Serialize)]
pub struct TokenIntrospection {
	pub id: String,
	pub device_id: String,
	pub name: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub scope: Option<String>,
	pub scopes: Vec<String>,
	pub account_id: String,
	pub session_active_ms: bool,
}

fn normalize_scopes(
	scopes: impl IntoIterator<Item = String>,
) -> Result<Vec<String>, String> {
	let mut seen = HashSet::new();
	let mut normalized = Vec::new();
	for scope in scopes {
		if !KNOWN_SCOPES.contains(&scope.as_str()) {
			return Err(format!("invalid scope {scope:?}"));
		}
		if seen.insert(scope.clone()) {
			normalized.push(scope);
		}
	}
	if normalized.is_empty() {
		return Err("at least one scope is required".into());
	}
	normalized.sort_by_key(|scope| {
		KNOWN_SCOPES
			.iter()
			.position(|known| *known == scope.as_str())
			.expect("validated scope")
	});
	Ok(normalized)
}

fn requested_scopes(request: &TokenCreateRequest) -> Result<Vec<String>, String> {
	let scalar = request
		.scope
		.clone()
		.map(|scope| normalize_scopes(std::iter::once(scope)))
		.transpose()?;
	let array = request.scopes.clone().map(normalize_scopes).transpose()?;
	match (scalar, array) {
		(None, None) => Err("scope or scopes required".into()),
		(Some(scopes), None) | (None, Some(scopes)) => Ok(scopes),
		(Some(scalar), Some(array)) if scalar == array => Ok(scalar),
		(Some(_), Some(_)) => Err("scope and scopes must describe the same set".into()),
	}
}

/// One work identity alias.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Identifier {
	pub kind: String,
	pub value: String,
}

/// Input for side-loaded work resolution.
#[derive(Clone, Debug, Deserialize)]
pub struct ResolveRequest {
	#[serde(default)]
	pub identifiers: Vec<Identifier>,
	pub title: Option<String>,
	pub author: Option<String>,
	#[serde(default)]
	pub confirmed: bool,
}

/// Result of work resolution.
#[derive(Clone, Debug, Serialize)]
pub struct ResolveResult {
	pub work_id: String,
	pub confidence: String,
	pub created: bool,
}

/// A position operation sent by a device.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpInput {
	pub op_id: String,
	pub work_id: String,
	#[serde(default)]
	pub edition_sha: Option<String>,
	pub client_ts: String,
	pub progression: Option<f64>,
	#[serde(default)]
	pub locator: Option<Value>,
	#[serde(default)]
	pub foreign_pos: Option<String>,
}

/// An operation as returned by the server.
#[derive(Clone, Debug, Serialize)]
pub struct OpRecord {
	pub op_id: String,
	pub work_id: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub edition_sha: Option<String>,
	pub client_ts: String,
	pub progression: f64,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub locator: Option<Value>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub foreign_pos: Option<String>,
	pub seq: i64,
	pub device_id: String,
	pub origin: String,
	pub received_at: String,
}

/// Per-item operation push result.
#[derive(Clone, Debug, Serialize)]
pub struct OpResult {
	pub op_id: String,
	pub status: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub seq: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reason: Option<String>,
}

/// Delta page for `/v1/changes`.
#[derive(Clone, Debug, Serialize)]
pub struct ChangesPage {
	pub ops: Vec<OpRecord>,
	pub high_water: i64,
	pub has_more: bool,
}

/// One immutable reading session.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SessionInput {
	pub session_id: String,
	pub work_id: String,
	#[serde(default)]
	pub edition_sha: Option<String>,
	pub started_at: String,
	pub ended_at: String,
	pub start_progression: Option<f64>,
	pub end_progression: Option<f64>,
	#[serde(default)]
	pub idle_ms: i64,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub active_ms: Option<i64>,
}

/// A setting stored for an account and returned by `GET /v1/me/settings`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SettingValue {
	pub value: String,
	pub updated_at: String,
}

/// One client timestamped setting update. The backend applies strict LWW.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingUpdate {
	pub key: String,
	pub value: String,
	pub updated_at: String,
}

/// A work's position snapshot used by the recovery endpoint.
#[derive(Clone, Debug, Serialize)]
pub struct HeadsPage {
	pub ops: Vec<OpRecord>,
	pub snapshot_seq: i64,
}

/// Batched annotation input.  `base_rev == 0` creates a record.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AnnotationInput {
	pub id: String,
	pub base_rev: i64,
	pub work_id: String,
	#[serde(default)]
	pub edition_sha: Option<String>,
	pub kind: String,
	#[serde(default)]
	pub locator: Option<Value>,
	#[serde(default)]
	pub progression: Option<f64>,
	#[serde(default)]
	pub excerpt: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub color: Option<String>,
	/// KOReader highlight decoration; distinct from semantic `kind`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub drawer: Option<String>,
	#[serde(default)]
	pub body: String,
	pub client_ts: String,
}

/// Current annotation state, or a deletion tombstone.
#[derive(Clone, Debug, Serialize)]
pub struct AnnotationRecord {
	pub id: String,
	pub rev: i64,
	#[serde(skip_serializing_if = "is_zero")]
	pub seq: i64,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub work_id: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub edition_sha: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub kind: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub locator: Option<Value>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub progression: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub excerpt: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub color: Option<String>,
	/// KOReader highlight decoration; distinct from semantic `kind`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub drawer: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub body: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub device_id: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub client_ts: Option<String>,
	pub updated_at: String,
	#[serde(skip_serializing_if = "is_false")]
	pub deleted: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub deleted_at: Option<String>,
}

/// Per-item annotation push result.
#[derive(Clone, Debug, Serialize)]
pub struct AnnotationResult {
	pub id: String,
	pub status: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub rev: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub seq: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reason: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub server: Option<AnnotationRecord>,
}

/// Result of deleting an annotation.
#[derive(Clone, Debug)]
pub struct DeleteAnnotationResult {
	pub id: String,
	pub status: String,
	pub rev: i64,
	pub seq: i64,
	pub server: Option<AnnotationRecord>,
}

/// Every attachment kind the wire accepts, in `PUT` path order.
///
/// `markup-svg` is the handwritten stroke layer of a Kobo on-page markup,
/// `markup-page` the page snapshot it was drawn on, and `notebook` a device
/// notebook export. The kinds are closed: a client cannot invent one.
pub const ATTACHMENT_KINDS: [&str; 3] = ["markup-svg", "markup-page", "notebook"];

/// Verified bytes accepted by `PUT /v1/annotations/{id}/attachments/{kind}`.
///
/// The digest is the client's `X-Attachment-Sha256`, already compared against
/// the received bytes, so a backend stores it without re-hashing.
#[derive(Clone, Debug)]
pub struct AttachmentUpload {
	pub kind: String,
	pub media_type: String,
	pub sha256: String,
	pub bytes: Bytes,
}

/// The stored identity of an attachment, returned by an upload.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttachmentUploadResult {
	pub id: String,
	pub sha256: String,
	pub byte_size: i64,
}

/// One attachment as listed by `GET /v1/annotations/{id}/attachments`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttachmentRecord {
	pub id: String,
	pub annotation_id: String,
	pub kind: String,
	pub media_type: String,
	pub byte_size: i64,
	pub sha256: String,
	pub created_at: String,
}

/// One folder exposed by the catalog.
#[derive(Clone, Debug, Serialize)]
pub struct CatalogFolder {
	pub folder_id: String,
	pub name: String,
	pub accepts_uploads: bool,
}

/// A page from `GET /v1/folders`.
#[derive(Clone, Debug, Serialize)]
pub struct CatalogFoldersPage {
	pub folders: Vec<CatalogFolder>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub next_after: Option<String>,
}

/// One contributor on a catalog book.
#[derive(Clone, Debug, Serialize)]
pub struct CatalogContributor {
	pub name: String,
	pub role: String,
}

/// One series membership on a catalog book.
#[derive(Clone, Debug, Serialize)]
pub struct CatalogSeriesMembership {
	pub id: String,
	pub name: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub position: Option<f64>,
	pub source: String,
}

/// The single book shape returned by catalog routes.
#[derive(Clone, Debug, Serialize)]
pub struct CatalogBook {
	pub book_id: String,
	pub folder_id: String,
	pub title: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub author: Option<String>,
	pub contributors: Vec<CatalogContributor>,
	pub series: Vec<CatalogSeriesMembership>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub series_claim_updated_at: Option<String>,
	pub series_source: String,
	pub size_bytes: i64,
	pub status: String,
	pub missing: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cover_url: Option<String>,
	pub updated_at: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub sha256: Option<String>,
}

/// A page from `GET /v1/folders/{id}/books`.
#[derive(Clone, Debug, Serialize)]
pub struct CatalogBooksPage {
	pub books: Vec<CatalogBook>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub next_cursor: Option<String>,
}

/// The file returned by a catalog download.
#[derive(Clone, Debug)]
pub struct CatalogDownload {
	pub path: String,
	pub filename: String,
	pub media_type: String,
}

/// An image returned by a catalog cover request.
#[derive(Clone, Debug)]
pub struct CatalogCover {
	pub content_type: String,
	pub bytes: Vec<u8>,
}

/// The layers returned by `GET /v1/books/{id}/series`.
#[derive(Clone, Debug, Serialize)]
pub struct CatalogBookSeries {
	pub book_id: String,
	pub source: String,
	pub series: Vec<CatalogSeriesMembership>,
	pub folder: Vec<CatalogSeriesMembership>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub shared: Option<Vec<CatalogSeriesMembership>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub personal: Option<Vec<CatalogSeriesMembership>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub shared_updated_at: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub personal_updated_at: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub outcome: Option<String>,
}

/// The response to `PUT` or `DELETE /v1/entities/series/{id}/name`.
///
/// A name is a display overlay; `scanned_name` remains the catalog value.
#[derive(Clone, Debug, Serialize)]
pub struct CatalogSeriesName {
	pub id: String,
	pub name: String,
	pub scanned_name: String,
	pub name_source: String,
	pub book_count: i64,
}

/// Largest accepted series display name, in UTF-8 bytes.
pub const MAX_SERIES_NAME_BYTES: usize = 512;

/// Liseur and KOReader highlight colors are stored without lossy mapping.
/// KOReader styles use the distinct top-level `drawer` field.
pub const ANNOTATION_COLORS: [&str; 10] = [
	"yellow", "green", "blue", "pink", "purple", "orange", "red", "olive", "cyan", "gray",
];
/// KOReader's highlight drawer styles; the locator remains opaque.
pub const ANNOTATION_DRAWERS: [&str; 4] = ["lighten", "underline", "strikeout", "invert"];
/// Header and capability token required to receive KOReader-only fields.
pub const EXTENDED_ANNOTATION_CAPABILITIES_HEADER: &str =
	"x-liseur-annotation-capabilities";
pub const EXTENDED_ANNOTATION_COLOR_DRAWER_CAPABILITY: &str =
	"annotation-color-drawer-v1";
/// Palette understood by pinned Liseur Android v0.13 and v0.18 clients.
pub const LISEUR_ANNOTATION_COLORS: [&str; 6] =
	["yellow", "green", "blue", "pink", "purple", "orange"];

fn supports_extended_annotation_fields(headers: &HeaderMap) -> bool {
	headers
		.get(EXTENDED_ANNOTATION_CAPABILITIES_HEADER)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|capabilities| {
			capabilities.split(',').any(|capability| {
				capability.trim() == EXTENDED_ANNOTATION_COLOR_DRAWER_CAPABILITY
			})
		})
}

fn annotation_record_for_client(
	mut record: AnnotationRecord,
	supports_extended: bool,
) -> AnnotationRecord {
	if !supports_extended {
		if record
			.color
			.as_deref()
			.is_some_and(|color| !LISEUR_ANNOTATION_COLORS.contains(&color))
		{
			record.color = None;
		}
		record.drawer = None;
	}
	record
}

fn annotation_results_for_client(
	results: Vec<AnnotationResult>,
	supports_extended: bool,
) -> Vec<AnnotationResult> {
	results
		.into_iter()
		.map(|mut result| {
			result.server = result
				.server
				.map(|record| annotation_record_for_client(record, supports_extended));
			result
		})
		.collect()
}

fn delete_annotation_for_client(
	mut result: DeleteAnnotationResult,
	supports_extended: bool,
) -> DeleteAnnotationResult {
	result.server = result
		.server
		.map(|record| annotation_record_for_client(record, supports_extended));
	result
}
/// The result of joining a catalog book to the caller's work.
#[derive(Clone, Debug, Serialize)]
pub struct CatalogResolveResult {
	pub book_id: String,
	pub work_id: String,
	pub confidence: String,
	pub created: bool,
	pub identifiers: Vec<Identifier>,
}

/// Input accepted by `POST /v1/books/{id}/resolve`.
#[derive(Clone, Debug, Default, Deserialize)]
struct CatalogResolveRequest {
	#[serde(default)]
	confirmed: bool,
}

/// Backend contract implemented by the host application.
///
/// Existing sync callbacks receive a user id after the generic bearer
/// middleware has authenticated a liseur token. Catalog callbacks receive the
/// authenticated context so the host can apply the same visibility filters as
/// its other library routes.
#[async_trait]
pub trait LiseurSyncBackend: Clone + Send + Sync + 'static {
	async fn login(
		&self,
		username: &str,
		password: &str,
	) -> Result<LoginResult, LiseurSyncError>;

	async fn authenticate(&self, token: &str) -> Result<LiseurToken, LiseurSyncError>;

	async fn mint_token(
		&self,
		user_id: &str,
		name: &str,
		scopes: Vec<String>,
		expires_in_seconds: Option<i64>,
	) -> Result<TokenCreateResult, LiseurSyncError>;

	async fn revoke_token(
		&self,
		user_id: &str,
		token_id: &str,
	) -> Result<(), LiseurSyncError>;
	async fn settings(
		&self,
		user_id: &str,
	) -> Result<BTreeMap<String, SettingValue>, LiseurSyncError>;

	async fn put_settings(
		&self,
		user_id: &str,
		settings: Vec<SettingUpdate>,
	) -> Result<(), LiseurSyncError>;

	async fn folders(
		&self,
		_auth: &AuthContext,
		_after: Option<String>,
		_limit: usize,
	) -> Result<CatalogFoldersPage, LiseurSyncError> {
		Err(LiseurSyncError::NotFound("catalog unavailable".into()))
	}

	async fn folder_books(
		&self,
		_auth: &AuthContext,
		_folder_id: &str,
		_cursor: Option<String>,
		_limit: usize,
	) -> Result<CatalogBooksPage, LiseurSyncError> {
		Err(LiseurSyncError::NotFound("catalog unavailable".into()))
	}

	async fn folder_search(
		&self,
		_auth: &AuthContext,
		_folder_id: &str,
		_query: &str,
	) -> Result<Vec<CatalogBook>, LiseurSyncError> {
		Err(LiseurSyncError::NotFound("catalog unavailable".into()))
	}

	async fn book(
		&self,
		_auth: &AuthContext,
		_book_id: &str,
	) -> Result<CatalogBook, LiseurSyncError> {
		Err(LiseurSyncError::NotFound("catalog unavailable".into()))
	}

	async fn book_download(
		&self,
		_auth: &AuthContext,
		_book_id: &str,
	) -> Result<CatalogDownload, LiseurSyncError> {
		Err(LiseurSyncError::NotFound("catalog unavailable".into()))
	}

	async fn book_cover(
		&self,
		_auth: &AuthContext,
		_book_id: &str,
	) -> Result<CatalogCover, LiseurSyncError> {
		Err(LiseurSyncError::NotFound("catalog unavailable".into()))
	}

	async fn book_series(
		&self,
		_auth: &AuthContext,
		_book_id: &str,
		_scope: Option<String>,
	) -> Result<CatalogBookSeries, LiseurSyncError> {
		Err(LiseurSyncError::NotFound("catalog unavailable".into()))
	}
	async fn set_series_name(
		&self,
		_auth: &AuthContext,
		_series_id: &str,
		_scope: &str,
		_name: &str,
	) -> Result<CatalogSeriesName, LiseurSyncError> {
		Err(LiseurSyncError::NotFound("catalog unavailable".into()))
	}

	async fn clear_series_name(
		&self,
		_auth: &AuthContext,
		_series_id: &str,
		_scope: &str,
	) -> Result<CatalogSeriesName, LiseurSyncError> {
		Err(LiseurSyncError::NotFound("catalog unavailable".into()))
	}

	async fn resolve_catalog_book(
		&self,
		_auth: &AuthContext,
		_book_id: &str,
		_confirmed: bool,
	) -> Result<CatalogResolveResult, LiseurSyncError> {
		Err(LiseurSyncError::NotFound("catalog unavailable".into()))
	}

	async fn resolve_work(
		&self,
		user_id: &str,
		request: ResolveRequest,
	) -> Result<ResolveResult, LiseurSyncError>;

	async fn append_ops(
		&self,
		user_id: &str,
		device_id: &str,
		ops: Vec<OpInput>,
	) -> Result<Vec<OpResult>, LiseurSyncError>;

	async fn changes(
		&self,
		user_id: &str,
		since: i64,
		limit: usize,
	) -> Result<ChangesPage, LiseurSyncError>;

	async fn heads(&self, user_id: &str) -> Result<HeadsPage, LiseurSyncError>;

	async fn positions(
		&self,
		user_id: &str,
		work_id: &str,
		limit: usize,
	) -> Result<Vec<OpRecord>, LiseurSyncError>;

	async fn append_sessions(
		&self,
		user_id: &str,
		device_id: &str,
		sessions: Vec<SessionInput>,
	) -> Result<usize, LiseurSyncError>;

	async fn append_annotations(
		&self,
		user_id: &str,
		device_id: &str,
		annotations: Vec<AnnotationInput>,
	) -> Result<Vec<AnnotationResult>, LiseurSyncError>;

	async fn annotation_changes(
		&self,
		user_id: &str,
		since: i64,
		limit: usize,
	) -> Result<(Vec<AnnotationRecord>, i64, bool), LiseurSyncError>;

	async fn work_annotations(
		&self,
		user_id: &str,
		work_id: &str,
	) -> Result<Vec<AnnotationRecord>, LiseurSyncError>;
	async fn work_annotations_with_deleted(
		&self,
		user_id: &str,
		work_id: &str,
		_include_deleted: bool,
	) -> Result<Vec<AnnotationRecord>, LiseurSyncError> {
		self.work_annotations(user_id, work_id).await
	}

	async fn delete_annotation(
		&self,
		user_id: &str,
		id: &str,
		rev: i64,
	) -> Result<DeleteAnnotationResult, LiseurSyncError>;

	/// Upper bound on one accepted attachment. A host that exposes a
	/// configuration key overrides this.
	fn attachment_max_bytes(&self) -> usize {
		DEFAULT_ATTACHMENT_MAX_BYTES
	}

	/// Store `upload` in the annotation's `kind` slot. A repeat of the same
	/// digest is idempotent and keeps the original attachment id; a different
	/// digest replaces the slot. Neither touches the annotation's revision.
	///
	/// An unknown or tombstoned annotation is
	/// [`LiseurSyncError::NotFound`].
	async fn put_attachment(
		&self,
		user_id: &str,
		annotation_id: &str,
		upload: AttachmentUpload,
	) -> Result<AttachmentUploadResult, LiseurSyncError>;

	async fn attachments(
		&self,
		user_id: &str,
		annotation_id: &str,
	) -> Result<Vec<AttachmentRecord>, LiseurSyncError>;
}

/// Build the native liseur-sync routes for any Axum application state.
///
/// The host adds `Extension<B>` to the returned router.  The route state is
/// intentionally generic and unused: all protocol state arrives through the
/// backend extension and the authenticated [`AuthContext`] extension.
pub fn routes<S, B>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	B: LiseurSyncBackend,
{
	let settings = Router::<S>::new()
		.route(
			"/v1/me/settings",
			get(get_settings::<B>).put(put_settings::<B>),
		)
		.layer(DefaultBodyLimit::max(MAX_SYNC_BODY_BYTES));
	let sessions = Router::<S>::new()
		.route("/v1/sessions", post(push_sessions::<B>))
		.layer(DefaultBodyLimit::max(MAX_SYNC_BODY_BYTES));
	let ops = Router::<S>::new()
		.route("/v1/ops", post(push_ops::<B>))
		.layer(DefaultBodyLimit::max(MAX_SYNC_BODY_BYTES));

	let protected = Router::<S>::new()
		.merge(settings)
		.merge(sessions)
		.route("/v1/token", get(token_introspection))
		.route("/v1/tokens", post(create_token::<B>))
		.route("/v1/tokens/{id}", delete(revoke_token::<B>))
		.route("/v1/works/resolve", post(resolve_work::<B>))
		.merge(ops)
		.route("/v1/changes", get(changes::<B>))
		.route("/v1/heads", get(heads::<B>))
		.route("/v1/works/{id}/positions", get(positions::<B>))
		.route("/v1/annotations", post(push_annotations::<B>))
		.route("/v1/annotations/changes", get(annotation_changes::<B>))
		.route("/v1/annotations/{id}", delete(delete_annotation::<B>))
		.route(
			"/v1/annotations/{id}/attachments",
			get(list_attachments::<B>),
		)
		.route(
			"/v1/annotations/{id}/attachments/{kind}",
			put(put_attachment::<B>),
		)
		.route("/v1/works/{id}/annotations", get(work_annotations::<B>))
		.route("/v1/folders", get(folders::<B>))
		.route("/v1/folders/{folder}/books", get(folder_books::<B>))
		.route("/v1/folders/{folder}/search", get(folder_search::<B>))
		.route("/v1/books/{id}", get(book::<B>))
		.route("/v1/books/{id}/cover", get(book_cover::<B>))
		.route("/v1/books/{id}/download", get(book_download::<B>))
		.route("/v1/books/{id}/series", get(book_series::<B>))
		.route("/v1/books/{id}/resolve", post(resolve_catalog_book::<B>))
		.route("/v1/entities/series/{id}/order", any(deferred_catalog))
		.route("/v1/events", get(deferred_events))
		.route(
			"/v1/entities/series/{id}/name",
			put(set_series_name::<B>).delete(clear_series_name::<B>),
		)
		.layer(axum::middleware::from_fn(token_auth::<B>));

	Router::<S>::new()
		.route("/v1/login", post(login::<B>))
		.merge(protected)
}

async fn token_auth<B>(mut request: Request, next: Next) -> Result<Response, Response>
where
	B: LiseurSyncBackend,
{
	let token = request
		.headers()
		.get(header::AUTHORIZATION)
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.strip_prefix("Bearer "))
		.filter(|value| !value.is_empty())
		.ok_or_else(|| error_response(LiseurSyncError::Unauthorized))?;

	let backend = request.extensions().get::<B>().cloned().ok_or_else(|| {
		error_response(LiseurSyncError::Internal("liseur backend missing".into()))
	})?;
	let authenticated = backend.authenticate(token).await.map_err(error_response)?;

	request
		.extensions_mut()
		.insert(authenticated.context.clone());
	request.extensions_mut().insert(authenticated);
	Ok(next.run(request).await)
}
#[derive(Debug, Deserialize, Default)]
struct CatalogFoldersQuery {
	after: Option<String>,
	limit: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
struct CatalogBooksQuery {
	cursor: Option<String>,
	limit: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
struct CatalogSearchQuery {
	q: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct CatalogSeriesQuery {
	scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SetSeriesNameRequest {
	#[serde(default = "default_personal_series_name_scope")]
	scope: String,
	name: String,
}

#[derive(Debug, Deserialize, Default)]
struct ClearSeriesNameQuery {
	scope: Option<String>,
}

fn default_personal_series_name_scope() -> String {
	"personal".into()
}

fn catalog_limit(limit: Option<usize>) -> Result<usize, LiseurSyncError> {
	let limit = limit.unwrap_or(50);
	if (1..=200).contains(&limit) {
		Ok(limit)
	} else {
		Err(LiseurSyncError::BadRequest(
			"limit must be between 1 and 200".into(),
		))
	}
}

fn request_origin(uri: &Uri, headers: &HeaderMap) -> String {
	let scheme = uri
		.scheme_str()
		.or_else(|| {
			headers
				.get("x-forwarded-proto")
				.and_then(|value| value.to_str().ok())
				.and_then(|value| value.split(',').next())
				.map(str::trim)
		})
		.unwrap_or("http");
	let host = headers
		.get(header::HOST)
		.and_then(|value| value.to_str().ok())
		.filter(|value| !value.is_empty())
		.unwrap_or("localhost");
	format!("{scheme}://{host}")
}

fn with_cover_url(mut book: CatalogBook, origin: &str) -> CatalogBook {
	book.cover_url = Some(format!(
		"{}/v1/books/{}/cover",
		origin.trim_end_matches('/'),
		book.book_id
	));
	book
}

async fn folders<B>(
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	Query(query): Query<CatalogFoldersQuery>,
) -> Result<Json<CatalogFoldersPage>, Response>
where
	B: LiseurSyncBackend,
{
	require_library_read(&token).map_err(error_response)?;
	let limit = catalog_limit(query.limit).map_err(error_response)?;
	let page = backend
		.folders(&auth, query.after, limit)
		.await
		.map_err(error_response)?;
	Ok(Json(page))
}

async fn folder_books<B>(
	Path(folder_id): Path<String>,
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	Query(query): Query<CatalogBooksQuery>,
	headers: HeaderMap,
	uri: Uri,
) -> Result<Json<CatalogBooksPage>, Response>
where
	B: LiseurSyncBackend,
{
	require_library_read(&token).map_err(error_response)?;
	let limit = catalog_limit(query.limit).map_err(error_response)?;
	let origin = request_origin(&uri, &headers);
	let mut page = backend
		.folder_books(&auth, &folder_id, query.cursor, limit)
		.await
		.map_err(error_response)?;
	page.books = page
		.books
		.into_iter()
		.map(|book| with_cover_url(book, &origin))
		.collect();
	Ok(Json(page))
}

async fn folder_search<B>(
	Path(folder_id): Path<String>,
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	Query(query): Query<CatalogSearchQuery>,
	headers: HeaderMap,
	uri: Uri,
) -> Result<Json<Value>, Response>
where
	B: LiseurSyncBackend,
{
	require_library_read(&token).map_err(error_response)?;
	let search = query.q.unwrap_or_default();
	let books = backend
		.folder_search(&auth, &folder_id, &search)
		.await
		.map_err(error_response)?;
	let origin = request_origin(&uri, &headers);
	let books = books
		.into_iter()
		.map(|book| with_cover_url(book, &origin))
		.collect::<Vec<_>>();
	Ok(Json(json!({ "books": books })))
}

async fn book<B>(
	Path(book_id): Path<String>,
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	headers: HeaderMap,
	uri: Uri,
) -> Result<Json<CatalogBook>, Response>
where
	B: LiseurSyncBackend,
{
	require_library_read(&token).map_err(error_response)?;
	let book = backend
		.book(&auth, &book_id)
		.await
		.map_err(error_response)?;
	Ok(Json(with_cover_url(book, &request_origin(&uri, &headers))))
}

async fn book_series<B>(
	Path(book_id): Path<String>,
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	Query(query): Query<CatalogSeriesQuery>,
) -> Result<Json<CatalogBookSeries>, Response>
where
	B: LiseurSyncBackend,
{
	require_library_read(&token).map_err(error_response)?;
	let series = backend
		.book_series(&auth, &book_id, query.scope)
		.await
		.map_err(error_response)?;
	Ok(Json(series))
}

async fn set_series_name<B>(
	Path(series_id): Path<String>,
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	Json(request): Json<SetSeriesNameRequest>,
) -> Result<Json<CatalogSeriesName>, Response>
where
	B: LiseurSyncBackend,
{
	require_library_manage(&token).map_err(error_response)?;
	validate_personal_series_name_scope(&request.scope).map_err(error_response)?;
	validate_series_name(&request.name).map_err(error_response)?;
	let result = backend
		.set_series_name(&auth, &series_id, &request.scope, &request.name)
		.await
		.map_err(error_response)?;
	Ok(Json(result))
}

async fn clear_series_name<B>(
	Path(series_id): Path<String>,
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	Query(query): Query<ClearSeriesNameQuery>,
) -> Result<Json<CatalogSeriesName>, Response>
where
	B: LiseurSyncBackend,
{
	require_library_manage(&token).map_err(error_response)?;
	let scope = query
		.scope
		.as_deref()
		.filter(|scope| !scope.is_empty())
		.unwrap_or("personal");
	validate_personal_series_name_scope(scope).map_err(error_response)?;
	let result = backend
		.clear_series_name(&auth, &series_id, scope)
		.await
		.map_err(error_response)?;
	Ok(Json(result))
}

async fn book_cover<B>(
	Path(book_id): Path<String>,
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
) -> Result<Response, Response>
where
	B: LiseurSyncBackend,
{
	require_library_read(&token).map_err(error_response)?;
	let cover = backend
		.book_cover(&auth, &book_id)
		.await
		.map_err(error_response)?;
	let mut response = cover.bytes.into_response();
	response.headers_mut().insert(
		header::CONTENT_TYPE,
		HeaderValue::from_str(&cover.content_type)
			.unwrap_or_else(|_| HeaderValue::from_static("image/jpeg")),
	);
	response.headers_mut().insert(
		"x-content-type-options",
		HeaderValue::from_static("nosniff"),
	);
	Ok(response)
}

async fn book_download<B>(
	Path(book_id): Path<String>,
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	headers: HeaderMap,
) -> Result<Response, Response>
where
	B: LiseurSyncBackend,
{
	require_library_read(&token).map_err(error_response)?;
	let file = backend
		.book_download(&auth, &book_id)
		.await
		.map_err(error_response)?;
	let mut serve_request = Request::new(Body::empty());
	*serve_request.headers_mut() = headers;
	let mut response = ServeFile::new(&file.path)
		.try_call(serve_request)
		.await
		.map_err(|error| error_response(LiseurSyncError::Internal(error.to_string())))?
		.into_response();
	response.headers_mut().insert(
		header::CONTENT_TYPE,
		HeaderValue::from_str(&file.media_type)
			.unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
	);
	let filename = file.filename.replace(['\\', '"', '\r', '\n'], "_");
	response.headers_mut().insert(
		header::CONTENT_DISPOSITION,
		HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
			.unwrap_or_else(|_| HeaderValue::from_static("attachment")),
	);
	Ok(response)
}

async fn resolve_catalog_book<B>(
	Path(book_id): Path<String>,
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	json: Option<Json<CatalogResolveRequest>>,
) -> Result<(StatusCode, Json<CatalogResolveResult>), Response>
where
	B: LiseurSyncBackend,
{
	require_library_read(&token).map_err(error_response)?;
	require_sync(&token).map_err(error_response)?;
	let confirmed = json.map(|Json(body)| body.confirmed).unwrap_or(false);
	let result = backend
		.resolve_catalog_book(&auth, &book_id, confirmed)
		.await
		.map_err(error_response)?;
	let status = if result.created {
		StatusCode::CREATED
	} else {
		StatusCode::OK
	};
	Ok((status, Json(result)))
}

async fn deferred_catalog() -> Response {
	error_response(LiseurSyncError::NotFound(
		"catalog entity route is not implemented".into(),
	))
}

/// Liseur ≥ v0.15.0 opens `GET /v1/events` as an SSE topic feed once per
/// foreground session and stops for good on 404 (`LiveRetry.delayMillis`),
/// but retries with capped backoff on any other outcome — including the
/// host's SPA fallback, which answers a redirect to an HTML page.  Answering
/// a real 404 keeps the client on its request/response sync.
async fn deferred_events() -> Response {
	error_response(LiseurSyncError::NotFound(
		"live event stream is not implemented".into(),
	))
}

fn parse_json<T>(result: Result<Json<T>, JsonRejection>) -> Result<T, LiseurSyncError> {
	result
		.map(|Json(value)| value)
		.map_err(|_| LiseurSyncError::BadRequest("invalid JSON body".into()))
}

async fn login<B>(
	Extension(backend): Extension<B>,
	json: Result<Json<LoginRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<LoginResult>), Response>
where
	B: LiseurSyncBackend,
{
	let request = parse_json(json).map_err(error_response)?;
	let result = backend
		.login(&request.username, &request.password)
		.await
		.map_err(error_response)?;
	Ok((StatusCode::OK, Json(result)))
}

async fn create_token<B>(
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	json: Result<Json<TokenCreateRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<TokenCreateResult>), Response>
where
	B: LiseurSyncBackend,
{
	if !token.is_login_session() {
		return Err(error_response(LiseurSyncError::Unauthorized));
	}
	let request = parse_json(json).map_err(error_response)?;
	if request.name.is_empty() || request.name.len() > MAX_TOKEN_NAME_BYTES {
		return Err(error_response(LiseurSyncError::BadRequest(
			"name required (at most 256 bytes)".into(),
		)));
	}
	if request
		.expires_in_seconds
		.is_some_and(|seconds| seconds < 0)
	{
		return Err(error_response(LiseurSyncError::BadRequest(
			"expires_in_seconds must be >= 0".into(),
		)));
	}
	let scopes = requested_scopes(&request)
		.map_err(|error| error_response(LiseurSyncError::BadRequest(error)))?;
	if scopes.iter().any(|scope| scope == "admin") && !auth.user.is_server_owner {
		return Err(error_response(LiseurSyncError::Forbidden(
			"admin scope requires an administrator account".into(),
		)));
	}
	let result = backend
		.mint_token(
			&auth.id(),
			&request.name,
			scopes,
			request.expires_in_seconds,
		)
		.await
		.map_err(error_response)?;
	Ok((StatusCode::CREATED, Json(result)))
}

async fn token_introspection(
	Extension(token): Extension<LiseurToken>,
) -> Result<Json<TokenIntrospection>, Response> {
	if token.is_login_session() {
		return Err(error_response(LiseurSyncError::Unauthorized));
	}
	let scope = (token.scopes.len() == 1).then(|| token.scopes[0].clone());
	Ok(Json(TokenIntrospection {
		id: token.token_id.unwrap_or_default(),
		device_id: token.device_id,
		name: token.name,
		scope,
		scopes: token.scopes,
		account_id: token.context.id(),
		session_active_ms: true,
	}))
}
#[derive(Debug, Deserialize)]
struct PutSettingsRequest {
	#[serde(default)]
	settings: HashMap<String, SettingRequest>,
}

#[derive(Debug, Deserialize)]
struct SettingRequest {
	#[serde(default)]
	value: SettingValueField,
	updated_at: String,
}

#[derive(Debug, Default)]
enum SettingValueField {
	#[default]
	Missing,
	Null,
	String(String),
	Other,
}

impl<'de> Deserialize<'de> for SettingValueField {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		Ok(match Value::deserialize(deserializer)? {
			Value::Null => Self::Null,
			Value::String(value) => Self::String(value),
			_ => Self::Other,
		})
	}
}

#[derive(Debug, Serialize)]
struct SettingsResponse {
	settings: BTreeMap<String, SettingValue>,
}

async fn get_settings<B>(
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
) -> Result<Json<SettingsResponse>, Response>
where
	B: LiseurSyncBackend,
{
	require_sync(&token).map_err(error_response)?;
	let settings = backend.settings(&auth.id()).await.map_err(error_response)?;
	Ok(Json(SettingsResponse { settings }))
}

async fn put_settings<B>(
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	json: Result<Json<PutSettingsRequest>, JsonRejection>,
) -> Result<Json<SettingsResponse>, Response>
where
	B: LiseurSyncBackend,
{
	require_sync(&token).map_err(error_response)?;
	let request = json.map(|Json(value)| value).map_err(|rejection| {
		if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
			error_response(LiseurSyncError::PayloadTooLarge(
				"request body too large".into(),
			))
		} else {
			error_response(LiseurSyncError::BadRequest("invalid JSON body".into()))
		}
	})?;
	let settings = validated_settings(request.settings).map_err(error_response)?;
	backend
		.put_settings(&auth.id(), settings)
		.await
		.map_err(error_response)?;
	let settings = backend.settings(&auth.id()).await.map_err(error_response)?;
	Ok(Json(SettingsResponse { settings }))
}

fn validated_settings(
	request: HashMap<String, SettingRequest>,
) -> Result<Vec<SettingUpdate>, LiseurSyncError> {
	if request.is_empty() {
		return Err(LiseurSyncError::BadRequest("no settings provided".into()));
	}
	if request.len() > MAX_SETTINGS_PER_ACCOUNT {
		return Err(LiseurSyncError::BadRequest(
			"too many settings in one request".into(),
		));
	}

	let mut updates = Vec::with_capacity(request.len());
	for (key, setting) in request {
		if key.is_empty() {
			return Err(LiseurSyncError::BadRequest("empty settings key".into()));
		}
		if key.len() > MAX_SETTING_KEY_BYTES {
			return Err(LiseurSyncError::BadRequest(format!(
				"settings key too long: {key}"
			)));
		}
		let value = match setting.value {
			SettingValueField::Missing => String::new(),
			SettingValueField::String(value) => value,
			SettingValueField::Null => {
				return Err(LiseurSyncError::BadRequest(format!(
					"settings value may not be null for key {key}"
				)));
			},
			SettingValueField::Other => {
				return Err(LiseurSyncError::BadRequest(format!(
					"settings value must be a string for key {key}"
				)));
			},
		};
		if value.len() > MAX_SETTING_VALUE_BYTES {
			return Err(LiseurSyncError::BadRequest(format!(
				"settings value too long for key {key}"
			)));
		}
		if key.contains('\0') || value.contains('\0') {
			return Err(LiseurSyncError::BadRequest(
				"settings key or value contains a character that cannot be stored".into(),
			));
		}
		let updated_at = DateTime::parse_from_rfc3339(&setting.updated_at)
			.map_err(|_| {
				LiseurSyncError::BadRequest(format!("invalid updated_at for key {key}"))
			})?
			.with_timezone(&Utc);
		if updated_at > Utc::now() + chrono::Duration::hours(24) {
			return Err(LiseurSyncError::TimeInFuture(format!(
				"updated_at in the future for key {key}"
			)));
		}
		let updated_at = updated_at.to_rfc3339_opts(SecondsFormat::Micros, true);
		updates.push(SettingUpdate {
			key,
			value,
			updated_at,
		});
	}
	updates.sort_by(|left, right| left.key.cmp(&right.key));
	Ok(updates)
}

#[derive(Debug, Serialize)]
struct TokenStatus {
	status: &'static str,
}

async fn revoke_token<B>(
	Path(token_id): Path<String>,
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
) -> Result<Json<TokenStatus>, Response>
where
	B: LiseurSyncBackend,
{
	if !token.is_login_session() {
		return Err(error_response(LiseurSyncError::Unauthorized));
	}
	backend
		.revoke_token(&auth.id(), &token_id)
		.await
		.map_err(error_response)?;
	Ok(Json(TokenStatus { status: "revoked" }))
}

async fn resolve_work<B>(
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	json: Result<Json<ResolveRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ResolveResult>), Response>
where
	B: LiseurSyncBackend,
{
	let request = parse_json(json).map_err(error_response)?;
	require_sync(&token).map_err(error_response)?;
	if request.identifiers.is_empty() {
		return Err(error_response(LiseurSyncError::BadRequest(
			"identifiers required".into(),
		)));
	}
	validate_identifiers(&request.identifiers)
		.map_err(|e| error_response(LiseurSyncError::BadRequest(e)))?;

	let result = backend
		.resolve_work(&auth.id(), request)
		.await
		.map_err(error_response)?;
	let status = if result.created {
		StatusCode::CREATED
	} else {
		StatusCode::OK
	};
	Ok((status, Json(result)))
}

#[derive(Debug, Deserialize)]
struct OpsRequest {
	ops: Vec<OpInput>,
}

async fn push_ops<B>(
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	json: Result<Json<OpsRequest>, JsonRejection>,
) -> Result<Json<OpsResponse>, Response>
where
	B: LiseurSyncBackend,
{
	let request = json.map(|Json(value)| value).map_err(|rejection| {
		if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
			error_response(LiseurSyncError::PayloadTooLarge(
				"request body too large".into(),
			))
		} else {
			error_response(LiseurSyncError::BadRequest("invalid JSON body".into()))
		}
	})?;
	require_sync(&token).map_err(error_response)?;
	if request.ops.is_empty() {
		return Err(error_response(LiseurSyncError::BadRequest(
			"ops required".into(),
		)));
	}
	if request.ops.len() > MAX_OP_BATCH {
		return Err(error_response(LiseurSyncError::ItemRefusal {
			status: StatusCode::BAD_REQUEST,
			code: "batch_too_large",
			message: "batch too large".into(),
			item_index: None,
			session_id: None,
			op_id: None,
			work_id: None,
			limit: Some(MAX_OP_BATCH),
		}));
	}
	for (index, op) in request.ops.iter().enumerate() {
		if let Err((code, message, limit)) = validate_op(op) {
			let op_id = (!op.op_id.is_empty() && op.op_id.len() <= MAX_ID_BYTES)
				.then(|| op.op_id.clone());
			return Err(error_response(LiseurSyncError::ItemRefusal {
				status: StatusCode::BAD_REQUEST,
				code,
				message,
				item_index: Some(index),
				session_id: None,
				op_id,
				work_id: None,
				limit,
			}));
		}
	}
	let results = backend
		.append_ops(&auth.id(), &token.device_id, request.ops)
		.await
		.map_err(error_response)?;
	Ok(Json(OpsResponse { results }))
}

#[derive(Debug, Serialize)]
struct OpsResponse {
	results: Vec<OpResult>,
}

#[derive(Debug, Deserialize, Default)]
struct CursorQuery {
	since: Option<i64>,
	limit: Option<i64>,
}

fn cursor_params(query: &CursorQuery) -> Result<(i64, usize), LiseurSyncError> {
	let since = query.since.unwrap_or(0);
	if since < 0 {
		return Err(LiseurSyncError::BadRequest("since must be >= 0".into()));
	}
	let limit = match query.limit {
		Some(limit) if (1..=500).contains(&limit) => limit as usize,
		_ => 500,
	};
	Ok((since, limit))
}

async fn changes<B>(
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	Query(query): Query<CursorQuery>,
) -> Result<Json<ChangesPage>, Response>
where
	B: LiseurSyncBackend,
{
	require_sync(&token).map_err(error_response)?;
	let (since, limit) = cursor_params(&query).map_err(error_response)?;
	let page = backend
		.changes(&auth.id(), since, limit)
		.await
		.map_err(error_response)?;
	Ok(Json(page))
}

async fn heads<B>(
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
) -> Result<Json<HeadsPage>, Response>
where
	B: LiseurSyncBackend,
{
	require_sync(&token).map_err(error_response)?;
	Ok(Json(
		backend.heads(&auth.id()).await.map_err(error_response)?,
	))
}

#[derive(Debug, Deserialize, Default)]
struct PositionQuery {
	limit: Option<i64>,
}

fn position_limit(limit: Option<i64>) -> usize {
	match limit {
		Some(limit) if (1..=200).contains(&limit) => limit as usize,
		_ => 50,
	}
}

async fn positions<B>(
	Path(work_id): Path<String>,
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	Query(query): Query<PositionQuery>,
) -> Result<Json<PositionsResponse>, Response>
where
	B: LiseurSyncBackend,
{
	require_sync(&token).map_err(error_response)?;
	let limit = position_limit(query.limit);
	let ops = backend
		.positions(&auth.id(), &work_id, limit)
		.await
		.map_err(error_response)?;
	Ok(Json(PositionsResponse { ops }))
}

#[derive(Debug, Serialize)]
struct PositionsResponse {
	ops: Vec<OpRecord>,
}

#[derive(Debug, Deserialize)]
struct SessionsRequest {
	sessions: Vec<SessionInput>,
}

async fn push_sessions<B>(
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	json: Result<Json<SessionsRequest>, JsonRejection>,
) -> Result<Json<SessionsResponse>, Response>
where
	B: LiseurSyncBackend,
{
	let request = json.map(|Json(value)| value).map_err(|rejection| {
		if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
			error_response(LiseurSyncError::PayloadTooLarge(
				"request body too large".into(),
			))
		} else {
			error_response(LiseurSyncError::BadRequest("invalid JSON body".into()))
		}
	})?;
	require_sync(&token).map_err(error_response)?;
	if request.sessions.is_empty() {
		return Err(error_response(LiseurSyncError::BadRequest(
			"sessions required".into(),
		)));
	}
	if request.sessions.len() > MAX_SESSION_BATCH {
		return Err(error_response(LiseurSyncError::ItemRefusal {
			status: StatusCode::BAD_REQUEST,
			code: "batch_too_large",
			message: "batch too large".into(),
			item_index: None,
			session_id: None,
			op_id: None,
			work_id: None,
			limit: Some(MAX_SESSION_BATCH),
		}));
	}
	for (index, session) in request.sessions.iter().enumerate() {
		if let Err((code, message)) = validate_session(session) {
			let session_id = (!session.session_id.is_empty()
				&& session.session_id.len() <= MAX_ID_BYTES)
				.then(|| session.session_id.clone());
			return Err(error_response(LiseurSyncError::ItemRefusal {
				status: StatusCode::BAD_REQUEST,
				code,
				message,
				item_index: Some(index),
				session_id,
				op_id: None,
				work_id: None,
				limit: None,
			}));
		}
	}
	let accepted = backend
		.append_sessions(&auth.id(), &token.device_id, request.sessions)
		.await
		.map_err(error_response)?;
	Ok(Json(SessionsResponse { accepted }))
}

#[derive(Debug, Serialize)]
struct SessionsResponse {
	accepted: usize,
}

#[derive(Debug, Deserialize)]
struct RawAnnotationsRequest {
	annotations: Vec<Value>,
}

async fn push_annotations<B>(
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	headers: HeaderMap,
	json: Result<Json<RawAnnotationsRequest>, JsonRejection>,
) -> Result<Json<AnnotationsResponse>, Response>
where
	B: LiseurSyncBackend,
{
	let request = parse_json(json).map_err(error_response)?;
	require_sync(&token).map_err(error_response)?;
	let supports_extended = supports_extended_annotation_fields(&headers);
	if request.annotations.is_empty() {
		return Err(error_response(LiseurSyncError::BadRequest(
			"annotations required".into(),
		)));
	}
	if request.annotations.len() > MAX_ANNOTATION_BATCH {
		return Err(error_response(LiseurSyncError::BadRequest(
			"batch too large".into(),
		)));
	}

	let mut results: Vec<Option<AnnotationResult>> =
		vec![None; request.annotations.len()];
	let mut valid = Vec::new();
	let mut valid_positions = Vec::new();
	for (index, raw) in request.annotations.into_iter().enumerate() {
		let id = raw
			.get("id")
			.and_then(Value::as_str)
			.unwrap_or_default()
			.to_owned();
		let parsed = serde_json::from_value::<AnnotationInput>(raw);
		let annotation = match parsed {
			Ok(annotation) => annotation,
			Err(_) => {
				results[index] = Some(AnnotationResult {
					id,
					status: "invalid".into(),
					rev: None,
					seq: None,
					reason: Some("malformed annotation".into()),
					server: None,
				});
				continue;
			},
		};
		if let Err(reason) = validate_annotation(&annotation) {
			results[index] = Some(AnnotationResult {
				id: annotation.id.clone(),
				status: "invalid".into(),
				rev: None,
				seq: None,
				reason: Some(reason),
				server: None,
			});
			continue;
		}
		valid_positions.push(index);
		valid.push(annotation);
	}

	if !valid.is_empty() {
		let backend_results = backend
			.append_annotations(&auth.id(), &token.device_id, valid)
			.await
			.map_err(error_response)?;
		if backend_results.len() != valid_positions.len() {
			return Err(error_response(LiseurSyncError::Internal(
				"annotation backend returned an unexpected result count".into(),
			)));
		}
		for (position, result) in valid_positions.into_iter().zip(backend_results) {
			results[position] = Some(result);
		}
	}

	Ok(Json(AnnotationsResponse {
		results: annotation_results_for_client(
			results
				.into_iter()
				.map(|result| result.expect("every annotation has a result"))
				.collect(),
			supports_extended,
		),
	}))
}

#[derive(Debug, Serialize)]
struct AnnotationsResponse {
	results: Vec<AnnotationResult>,
}

async fn annotation_changes<B>(
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	headers: HeaderMap,
	Query(query): Query<CursorQuery>,
) -> Result<Json<AnnotationChangesResponse>, Response>
where
	B: LiseurSyncBackend,
{
	require_sync(&token).map_err(error_response)?;
	let supports_extended = supports_extended_annotation_fields(&headers);
	let (since, limit) = cursor_params(&query).map_err(error_response)?;
	let (annotations, high_water, has_more) = backend
		.annotation_changes(&auth.id(), since, limit)
		.await
		.map_err(error_response)?;
	Ok(Json(AnnotationChangesResponse {
		annotations: annotations
			.into_iter()
			.map(|annotation| annotation_record_for_client(annotation, supports_extended))
			.collect(),
		high_water,
		has_more,
	}))
}

#[derive(Debug, Serialize)]
struct AnnotationChangesResponse {
	annotations: Vec<AnnotationRecord>,
	high_water: i64,
	has_more: bool,
}

async fn work_annotations<B>(
	Path(work_id): Path<String>,
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	headers: HeaderMap,
	Query(query): Query<WorkAnnotationsQuery>,
) -> Result<Json<WorkAnnotationsResponse>, Response>
where
	B: LiseurSyncBackend,
{
	require_sync(&token).map_err(error_response)?;
	let supports_extended = supports_extended_annotation_fields(&headers);
	let annotations = backend
		.work_annotations_with_deleted(&auth.id(), &work_id, query.include_deleted)
		.await
		.map_err(error_response)?;
	let annotations = annotations
		.into_iter()
		.map(|annotation| annotation_record_for_client(annotation, supports_extended))
		.collect();
	Ok(Json(WorkAnnotationsResponse { annotations }))
}

#[derive(Debug, Serialize)]
struct WorkAnnotationsResponse {
	annotations: Vec<AnnotationRecord>,
}
#[derive(Debug, Default, Deserialize)]
struct WorkAnnotationsQuery {
	#[serde(default)]
	include_deleted: bool,
}

#[derive(Debug, Deserialize)]
struct DeleteQuery {
	rev: Option<i64>,
}

async fn delete_annotation<B>(
	Path(id): Path<String>,
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	headers: HeaderMap,
	Query(query): Query<DeleteQuery>,
) -> Result<Response, Response>
where
	B: LiseurSyncBackend,
{
	require_sync(&token).map_err(error_response)?;
	let supports_extended = supports_extended_annotation_fields(&headers);
	let rev = query.rev.filter(|rev| *rev >= 1).ok_or_else(|| {
		error_response(LiseurSyncError::BadRequest(
			"rev query parameter required".into(),
		))
	})?;
	let result = backend
		.delete_annotation(&auth.id(), &id, rev)
		.await
		.map_err(error_response)?;
	let result = delete_annotation_for_client(result, supports_extended);
	if result.status == "conflict" {
		return Ok((
			StatusCode::CONFLICT,
			axum::Json(json!({
				"error": "rev conflict",
				"server": result.server,
			})),
		)
			.into_response());
	}
	Ok((
		StatusCode::OK,
		axum::Json(json!({
			"id": result.id,
			"status": result.status,
			"rev": result.rev,
			"seq": result.seq,
		})),
	)
		.into_response())
}

#[derive(Debug, Serialize)]
struct AttachmentsResponse {
	attachments: Vec<AttachmentRecord>,
}

async fn list_attachments<B>(
	Path(annotation_id): Path<String>,
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
) -> Result<Json<AttachmentsResponse>, Response>
where
	B: LiseurSyncBackend,
{
	require_sync(&token).map_err(error_response)?;
	let attachments = backend
		.attachments(&auth.id(), &annotation_id)
		.await
		.map_err(error_response)?;
	Ok(Json(AttachmentsResponse { attachments }))
}

async fn put_attachment<B>(
	Path((annotation_id, kind)): Path<(String, String)>,
	Extension(auth): Extension<AuthContext>,
	Extension(token): Extension<LiseurToken>,
	Extension(backend): Extension<B>,
	headers: HeaderMap,
	body: Body,
) -> Result<Json<AttachmentUploadResult>, Response>
where
	B: LiseurSyncBackend,
{
	require_sync(&token).map_err(error_response)?;
	let upload = read_attachment(
		&annotation_id,
		&kind,
		&headers,
		body,
		backend.attachment_max_bytes(),
	)
	.await
	.map_err(error_response)?;
	backend
		.put_attachment(&auth.id(), &annotation_id, upload)
		.await
		.map(Json)
		.map_err(error_response)
}

/// Validate the upload envelope and read at most `max_bytes` of body.
///
/// `Content-Length` is honoured first so an oversized upload is refused
/// before its bytes are streamed; a chunked body without the header is caught
/// by the read limit instead. The digest is compared here so every backend
/// reports the same `409` for a corrupted transfer.
async fn read_attachment(
	annotation_id: &str,
	kind: &str,
	headers: &HeaderMap,
	body: Body,
	max_bytes: usize,
) -> Result<AttachmentUpload, LiseurSyncError> {
	validate_attachment_annotation_id(annotation_id)?;
	if !ATTACHMENT_KINDS.contains(&kind) {
		return Err(LiseurSyncError::BadRequest(format!(
			"kind must be one of {}",
			ATTACHMENT_KINDS.join(", ")
		)));
	}
	let media_type = attachment_media_type(kind, headers)?;
	let expected = attachment_digest(headers)?;

	if let Some(declared) = headers
		.get(header::CONTENT_LENGTH)
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.parse::<usize>().ok())
	{
		if declared > max_bytes {
			return Err(LiseurSyncError::PayloadTooLarge(format!(
				"attachment exceeds {max_bytes} bytes"
			)));
		}
	}

	let bytes = axum::body::to_bytes(body, max_bytes).await.map_err(|_| {
		LiseurSyncError::PayloadTooLarge(format!("attachment exceeds {max_bytes} bytes"))
	})?;
	if bytes.is_empty() {
		return Err(LiseurSyncError::BadRequest(
			"attachment body is empty".into(),
		));
	}
	let actual = format!("{:x}", Sha256::digest(&bytes));
	if actual != expected {
		return Err(LiseurSyncError::Conflict(
			"X-Attachment-Sha256 does not match the uploaded bytes".into(),
		));
	}

	Ok(AttachmentUpload {
		kind: kind.to_owned(),
		media_type,
		sha256: actual,
		bytes,
	})
}

/// An annotation id names the directory its attachments are stored in, so it
/// must be a safe single path segment. Ids that carry no attachments are
/// unaffected: the annotation lane itself keeps accepting any bounded id.
fn validate_attachment_annotation_id(id: &str) -> Result<(), LiseurSyncError> {
	let safe = !id.is_empty()
		&& id.len() <= MAX_ID_BYTES
		&& !id.starts_with('.')
		&& id.bytes().all(|byte| {
			byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
		});
	if safe {
		Ok(())
	} else {
		Err(LiseurSyncError::BadRequest(
			"an annotation carrying attachments needs an id of [A-Za-z0-9._-] not starting with a dot".into(),
		))
	}
}

/// The declared `Content-Type`, without parameters, checked against the kind.
fn attachment_media_type(
	kind: &str,
	headers: &HeaderMap,
) -> Result<String, LiseurSyncError> {
	let raw = headers
		.get(header::CONTENT_TYPE)
		.and_then(|value| value.to_str().ok())
		.filter(|value| value.len() <= MAX_MEDIA_TYPE_BYTES)
		.ok_or_else(|| LiseurSyncError::BadRequest("Content-Type required".into()))?;
	let media_type = raw
		.split(';')
		.next()
		.unwrap_or_default()
		.trim()
		.to_ascii_lowercase();
	let accepted: &[&str] = match kind {
		"markup-svg" => &["image/svg+xml"],
		"markup-page" => &["image/jpeg", "image/png"],
		_ => &[
			"application/pdf",
			"application/epub+zip",
			"application/zip",
			"application/vnd.openxmlformats-officedocument.wordprocessingml.document",
			"text/html",
			"text/plain",
			"image/jpeg",
			"image/png",
			"image/svg+xml",
		],
	};
	if accepted.contains(&media_type.as_str()) {
		Ok(media_type)
	} else {
		Err(LiseurSyncError::BadRequest(format!(
			"{kind} accepts {}",
			accepted.join(", ")
		)))
	}
}

/// The client's `X-Attachment-Sha256`, normalized to lowercase hex.
fn attachment_digest(headers: &HeaderMap) -> Result<String, LiseurSyncError> {
	let digest = headers
		.get(ATTACHMENT_SHA256_HEADER)
		.and_then(|value| value.to_str().ok())
		.map(|value| value.trim().to_ascii_lowercase())
		.filter(|value| {
			value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
		});
	digest.ok_or_else(|| {
		LiseurSyncError::BadRequest(
			"X-Attachment-Sha256 must be 64 hex characters".into(),
		)
	})
}
fn require_library_read(token: &LiseurToken) -> Result<(), LiseurSyncError> {
	if token.is_login_session() || !token.allows_scope("library-read") {
		Err(LiseurSyncError::Forbidden(
			"library-read scope required".into(),
		))
	} else {
		Ok(())
	}
}

fn require_library_manage(token: &LiseurToken) -> Result<(), LiseurSyncError> {
	if token.is_login_session() || !token.allows_scope("library-manage") {
		Err(LiseurSyncError::Forbidden(
			"library-manage scope required".into(),
		))
	} else {
		Ok(())
	}
}

fn validate_personal_series_name_scope(scope: &str) -> Result<(), LiseurSyncError> {
	if scope == "personal" {
		Ok(())
	} else {
		Err(LiseurSyncError::BadRequest(
			"Stump supports only personal series-name overlays".into(),
		))
	}
}

fn validate_series_name(name: &str) -> Result<(), LiseurSyncError> {
	if name.len() > MAX_SERIES_NAME_BYTES {
		return Err(LiseurSyncError::BadRequest(
			"series name is too long".into(),
		));
	}
	if name.trim().is_empty() {
		return Err(LiseurSyncError::BadRequest(
			"a series name cannot be empty".into(),
		));
	}
	Ok(())
}

fn require_sync(token: &LiseurToken) -> Result<(), LiseurSyncError> {
	if token.is_login_session() {
		return Err(LiseurSyncError::Forbidden("sync scope required".into()));
	}
	if token.allows_scope("sync") {
		Ok(())
	} else {
		Err(LiseurSyncError::Forbidden("sync scope required".into()))
	}
}
fn validate_identifiers(identifiers: &[Identifier]) -> Result<(), String> {
	for identifier in identifiers {
		let kind = identifier.kind.trim();
		let value = identifier.value.trim();
		if value.is_empty() || value.len() > 512 {
			return Err("identifier value required (at most 512 bytes)".into());
		}
		match kind {
			"sha256" | "partial-md5" | "source" | "dc" | "ta" => {},
			_ => return Err("identifier kind is not supported".into()),
		}
	}
	Ok(())
}

fn validate_op(op: &OpInput) -> Result<(), (&'static str, String, Option<usize>)> {
	if op.op_id.is_empty() || op.op_id.len() > MAX_ID_BYTES {
		return Err((
			"missing_field",
			"op_id required (at most 64 bytes)".into(),
			None,
		));
	}
	if op.work_id.is_empty() || op.work_id.len() > MAX_REFERENCE_BYTES {
		return Err((
			"missing_field",
			"work_id required (at most 128 bytes)".into(),
			None,
		));
	}
	if let Some(edition_sha) = &op.edition_sha {
		if edition_sha.len() > MAX_REFERENCE_BYTES {
			return Err(("missing_field", "edition_sha too large".into(), None));
		}
	}
	let progression = op
		.progression
		.ok_or_else(|| ("missing_field", "progression required".to_owned(), None))?;
	if !(0.0..=1.0).contains(&progression) || !progression.is_finite() {
		return Err((
			"progression_out_of_range",
			"progression out of range [0,1]".into(),
			None,
		));
	}
	if let Some(locator) = &op.locator {
		let bytes = serde_json::to_vec(locator)
			.map_err(|_| ("missing_field", "invalid locator".to_owned(), None))?;
		if bytes.len() > MAX_LOCATOR_BYTES {
			return Err((
				"locator_too_large",
				"locator too large".into(),
				Some(MAX_LOCATOR_BYTES),
			));
		}
	}
	if op.client_ts.len() > MAX_CLIENT_TS_BYTES {
		return Err(("bad_time", "bad client_ts".into(), None));
	}
	let timestamp = DateTime::parse_from_rfc3339(&op.client_ts)
		.map_err(|_| ("bad_time", "bad client_ts".to_owned(), None))?
		.with_timezone(&Utc);
	if timestamp > Utc::now() + chrono::Duration::hours(24) {
		return Err((
			"time_in_future",
			"client_ts is too far in the future".into(),
			None,
		));
	}
	Ok(())
}

fn validate_session(session: &SessionInput) -> Result<(), (&'static str, String)> {
	if session.session_id.is_empty() || session.session_id.len() > MAX_ID_BYTES {
		return Err((
			"missing_field",
			"session_id required (at most 64 bytes)".into(),
		));
	}
	if session.work_id.is_empty() || session.work_id.len() > MAX_REFERENCE_BYTES {
		return Err((
			"missing_field",
			"work_id required (at most 128 bytes)".into(),
		));
	}
	if let Some(edition_sha) = &session.edition_sha {
		if edition_sha.len() > MAX_REFERENCE_BYTES {
			return Err(("missing_field", "edition_sha too large".into()));
		}
	}
	let started = DateTime::parse_from_rfc3339(&session.started_at)
		.map_err(|_| ("bad_time", "bad started_at".to_owned()))?;
	let ended = DateTime::parse_from_rfc3339(&session.ended_at)
		.map_err(|_| ("bad_time", "bad ended_at".to_owned()))?;
	if ended < started {
		return Err(("bad_time", "ended_at before started_at".into()));
	}
	let start = session
		.start_progression
		.ok_or_else(|| ("missing_field", "start_progression required".to_owned()))?;
	let end = session
		.end_progression
		.ok_or_else(|| ("missing_field", "end_progression required".to_owned()))?;
	if !(0.0..=1.0).contains(&start)
		|| !start.is_finite()
		|| !(0.0..=1.0).contains(&end)
		|| !end.is_finite()
	{
		return Err((
			"progression_out_of_range",
			"progression out of range [0,1]".into(),
		));
	}
	let duration = (ended - started).num_milliseconds();
	if session.idle_ms < 0 || session.idle_ms > duration {
		return Err(("idle_out_of_range", "idle_ms out of range".into()));
	}
	if session
		.active_ms
		.is_some_and(|active_ms| !(0..=MAX_SESSION_ACTIVE_MS).contains(&active_ms))
	{
		return Err(("active_out_of_range", "active_ms out of range".into()));
	}
	Ok(())
}

fn validate_annotation(annotation: &AnnotationInput) -> Result<(), String> {
	if annotation.id.is_empty() || annotation.id.len() > MAX_ID_BYTES {
		return Err("id required (at most 64 bytes)".into());
	}
	if annotation.base_rev < 0 {
		return Err("base_rev must be >= 0".into());
	}
	if annotation.work_id.is_empty() || annotation.work_id.len() > MAX_REFERENCE_BYTES {
		return Err("work_id required (at most 128 bytes)".into());
	}
	if let Some(edition_sha) = &annotation.edition_sha {
		if edition_sha.len() > MAX_REFERENCE_BYTES {
			return Err("edition_sha too large".into());
		}
	}
	let locator_present = annotation
		.locator
		.as_ref()
		.is_some_and(|value| !value.is_null());
	match annotation.kind.as_str() {
		"note" => {
			if annotation.body.is_empty() {
				return Err("a note requires a body".into());
			}
		},
		"highlight" | "bookmark" => {
			if !locator_present {
				return Err(format!("{} requires a locator", annotation.kind));
			}
		},
		_ => return Err("kind must be highlight, note or bookmark".into()),
	}
	if annotation.kind == "bookmark" && !annotation.body.is_empty() {
		return Err("a bookmark carries no body".into());
	}
	if let Some(locator) = &annotation.locator {
		let bytes =
			serde_json::to_vec(locator).map_err(|_| "invalid locator".to_owned())?;
		if bytes.len() > MAX_LOCATOR_BYTES {
			return Err("locator too large".into());
		}
	}
	if annotation.progression.is_some_and(|progression| {
		!(0.0..=1.0).contains(&progression) || !progression.is_finite()
	}) {
		return Err("progression out of range [0,1]".into());
	}
	if annotation.excerpt.len() > MAX_EXCERPT_BYTES {
		return Err("excerpt too large".into());
	}
	if annotation.body.len() > MAX_BODY_BYTES {
		return Err("body too large".into());
	}
	let color = annotation.color.as_deref().unwrap_or_default();
	if !color.is_empty() && !ANNOTATION_COLORS.contains(&color) {
		return Err("color must be one of the palette tokens".into());
	}
	if !color.is_empty() && annotation.kind != "highlight" {
		return Err("color belongs to a highlight".into());
	}
	if let Some(drawer) = annotation
		.drawer
		.as_deref()
		.filter(|drawer| !drawer.is_empty())
	{
		if !ANNOTATION_DRAWERS.contains(&drawer) {
			return Err("drawer must be one of the KOReader highlight styles".into());
		}
		if annotation.kind != "highlight" {
			return Err("drawer belongs to a highlight".into());
		}
	}
	validate_timestamp(&annotation.client_ts, false)
		.map(|_| ())
		.map_err(|_| "bad client_ts".to_owned())
}

fn validate_timestamp(value: &str, allow_future: bool) -> Result<DateTime<Utc>, ()> {
	if value.len() > MAX_CLIENT_TS_BYTES {
		return Err(());
	}
	let timestamp = DateTime::parse_from_rfc3339(value)
		.map_err(|_| ())?
		.with_timezone(&Utc);
	if !allow_future && timestamp > Utc::now() + chrono::Duration::hours(24) {
		return Err(());
	}
	Ok(timestamp)
}

fn error_response(error: LiseurSyncError) -> Response {
	let status = match &error {
		LiseurSyncError::Unauthorized => StatusCode::UNAUTHORIZED,
		LiseurSyncError::Forbidden(_) => StatusCode::FORBIDDEN,
		LiseurSyncError::BadRequest(_) | LiseurSyncError::TimeInFuture(_) => {
			StatusCode::BAD_REQUEST
		},
		LiseurSyncError::NotFound(_) => StatusCode::NOT_FOUND,
		LiseurSyncError::Gone(_) => StatusCode::GONE,
		LiseurSyncError::Conflict(_) | LiseurSyncError::IdentityConflict(_) => {
			StatusCode::CONFLICT
		},
		LiseurSyncError::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
		LiseurSyncError::ItemRefusal { status, .. } => *status,
		LiseurSyncError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
	};
	let body = match error {
		LiseurSyncError::IdentityConflict(work_ids) => {
			json!({
				"error": "identifiers resolve to multiple works",
				"works": work_ids,
			})
		},
		LiseurSyncError::TimeInFuture(message) => {
			json!({"error": message, "code": "time_in_future"})
		},
		LiseurSyncError::ItemRefusal {
			code,
			message,
			item_index,
			session_id,
			op_id,
			work_id,
			limit,
			..
		} => {
			let mut body = json!({"error": message, "code": code});
			if let Some(item_index) = item_index {
				body["item_index"] = json!(item_index);
			}
			if let Some(session_id) = session_id {
				body["session_id"] = json!(session_id);
			}
			if let Some(op_id) = op_id {
				body["op_id"] = json!(op_id);
			}
			if let Some(work_id) = work_id {
				body["work_id"] = json!(work_id);
			}
			if let Some(limit) = limit {
				body["limit"] = json!(limit);
			}
			body
		},
		other => json!({ "error": other.to_string() }),
	};
	(status, axum::Json(body)).into_response()
}

fn is_zero(value: &i64) -> bool {
	*value == 0
}

fn is_false(value: &bool) -> bool {
	!*value
}

impl fmt::Display for LiseurToken {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(&self.device_id)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use axum::body::Body;
	use axum::http::Request;
	use tower::ServiceExt;

	#[test]
	fn annotation_tombstones_omit_live_fields() {
		let record = AnnotationRecord {
			id: "a".into(),
			rev: 2,
			seq: 4,
			work_id: Some("work".into()),
			edition_sha: None,
			kind: None,
			locator: None,
			progression: None,
			excerpt: None,
			color: None,
			drawer: None,
			body: None,
			device_id: None,
			client_ts: None,
			updated_at: "2026-01-01T00:00:00Z".into(),
			deleted: true,
			deleted_at: Some("2026-01-01T00:00:00Z".into()),
		};
		let value = serde_json::to_value(record).unwrap();
		assert_eq!(value["id"], "a");
		assert_eq!(value["rev"], 2);
		assert_eq!(value["seq"], 4);
		assert_eq!(value["deleted"], true);
		assert_eq!(value["work_id"], "work");
		assert!(value.get("kind").is_none());
	}

	#[test]
	fn invalid_query_limits_use_reference_client_defaults() {
		assert_eq!(
			cursor_params(&CursorQuery {
				since: None,
				limit: None,
			})
			.unwrap(),
			(0, 500)
		);
		for limit in [-1, 0, 501] {
			assert_eq!(
				cursor_params(&CursorQuery {
					since: Some(7),
					limit: Some(limit),
				})
				.unwrap(),
				(7, 500)
			);
		}
		assert_eq!(
			cursor_params(&CursorQuery {
				since: Some(7),
				limit: Some(200),
			})
			.unwrap(),
			(7, 200)
		);
		assert_eq!(position_limit(None), 50);
		for limit in [-1, 0, 201] {
			assert_eq!(position_limit(Some(limit)), 50);
		}
		assert_eq!(position_limit(Some(200)), 200);
	}

	#[test]
	fn validates_protocol_boundaries() {
		let op = OpInput {
			op_id: "op".into(),
			work_id: "work".into(),
			edition_sha: None,
			client_ts: "2026-01-01T00:00:00Z".into(),
			progression: Some(0.5),
			locator: None,
			foreign_pos: None,
		};
		assert!(validate_op(&op).is_ok());
		assert!(validate_op(&OpInput {
			progression: Some(f64::NAN),
			..op.clone()
		})
		.is_err());
		let locator_too_large = validate_op(&OpInput {
			locator: Some(json!({"text": "x".repeat(MAX_LOCATOR_BYTES)})),
			..op.clone()
		})
		.unwrap_err();
		assert_eq!(locator_too_large.0, "locator_too_large");
		assert_eq!(locator_too_large.2, Some(MAX_LOCATOR_BYTES));
		assert_eq!(
			validate_op(&OpInput {
				client_ts: (Utc::now() + chrono::Duration::hours(25)).to_rfc3339(),
				..op
			})
			.unwrap_err()
			.0,
			"time_in_future"
		);

		let annotation = AnnotationInput {
			id: "a".into(),
			base_rev: 0,
			work_id: "w".into(),
			edition_sha: None,
			kind: "note".into(),
			locator: None,
			progression: None,
			excerpt: String::new(),
			color: None,
			drawer: None,
			body: "body".into(),
			client_ts: "2026-01-01T00:00:00Z".into(),
		};
		assert!(validate_annotation(&annotation).is_ok());
		let anchored_note = AnnotationInput {
			locator: Some(json!({"page": 4})),
			..annotation.clone()
		};
		assert!(validate_annotation(&anchored_note).is_ok());
		let highlight = AnnotationInput {
			kind: "highlight".into(),
			locator: Some(json!({"page": 1})),
			color: Some("red".into()),
			drawer: Some("underline".into()),
			..annotation.clone()
		};
		assert!(validate_annotation(&highlight).is_ok());
		for color in ANNOTATION_COLORS {
			assert!(validate_annotation(&AnnotationInput {
				color: Some(color.into()),
				..highlight.clone()
			})
			.is_ok());
		}
		assert!(validate_annotation(&AnnotationInput {
			color: Some("teal".into()),
			..highlight.clone()
		})
		.is_err());
		for drawer in ANNOTATION_DRAWERS {
			assert!(validate_annotation(&AnnotationInput {
				drawer: Some(drawer.into()),
				..highlight.clone()
			})
			.is_ok());
		}
		assert!(validate_annotation(&AnnotationInput {
			drawer: Some("wavy".into()),
			..highlight.clone()
		})
		.is_err());
		assert!(validate_annotation(&AnnotationInput {
			drawer: Some("underline".into()),
			..annotation
		})
		.is_err());
		assert_eq!(
			serde_json::to_value(&highlight).unwrap()["drawer"],
			"underline"
		);
		let record = AnnotationRecord {
			id: "red-highlight".into(),
			rev: 1,
			seq: 1,
			work_id: Some("work".into()),
			edition_sha: None,
			kind: Some("highlight".into()),
			locator: Some(json!({"href": "chapter.xhtml"})),
			progression: None,
			excerpt: Some("a passage".into()),
			color: Some("red".into()),
			drawer: Some("underline".into()),
			body: None,
			device_id: Some("device".into()),
			client_ts: Some("2026-01-01T00:00:00Z".into()),
			updated_at: "2026-01-01T00:00:00Z".into(),
			deleted: false,
			deleted_at: None,
		};
		let red_for_liseur =
			serde_json::to_value(annotation_record_for_client(record.clone(), false))
				.unwrap();
		assert_eq!(red_for_liseur["id"], "red-highlight");
		assert_eq!(red_for_liseur["kind"], "highlight");
		assert_eq!(red_for_liseur["locator"]["href"], "chapter.xhtml");
		assert!(!red_for_liseur.as_object().unwrap().contains_key("color"));
		assert!(!red_for_liseur.as_object().unwrap().contains_key("drawer"));
		let red_for_koreader =
			serde_json::to_value(annotation_record_for_client(record, true)).unwrap();
		assert_eq!(red_for_koreader["color"], "red");
		assert_eq!(red_for_koreader["drawer"], "underline");
		let mut headers = HeaderMap::new();
		assert!(!supports_extended_annotation_fields(&headers));
		headers.insert(
			EXTENDED_ANNOTATION_CAPABILITIES_HEADER,
			HeaderValue::from_static(EXTENDED_ANNOTATION_COLOR_DRAWER_CAPABILITY),
		);
		assert!(supports_extended_annotation_fields(&headers));
	}

	#[test]
	fn settings_validation_matches_timestamp_and_value_contract() {
		let missing_value: PutSettingsRequest = serde_json::from_value(json!({
			"settings": {
				"reader.font": {"updated_at": "2026-01-01T00:00:00.123456789Z"}
			}
		}))
		.unwrap();
		let update = validated_settings(missing_value.settings)
			.unwrap()
			.remove(0);
		assert_eq!(update.value, "");
		assert_eq!(update.updated_at, "2026-01-01T00:00:00.123456Z");

		let null_value: PutSettingsRequest = serde_json::from_value(json!({
			"settings": {
				"reader.font": {"value": null, "updated_at": "2026-01-01T00:00:00Z"}
			}
		}))
		.unwrap();
		assert!(matches!(
			validated_settings(null_value.settings),
			Err(LiseurSyncError::BadRequest(_))
		));

		let future = (Utc::now() + chrono::Duration::hours(25)).to_rfc3339();
		let future_value: PutSettingsRequest = serde_json::from_value(json!({
			"settings": {
				"reader.font": {"value": "serif", "updated_at": future}
			}
		}))
		.unwrap();
		assert!(matches!(
			validated_settings(future_value.settings),
			Err(LiseurSyncError::TimeInFuture(_))
		));
	}

	#[test]
	fn measured_session_time_is_authoritative_but_idle_stays_bounded() {
		let session = SessionInput {
			session_id: "session".into(),
			work_id: "work".into(),
			edition_sha: None,
			started_at: "2026-01-01T00:00:00Z".into(),
			ended_at: "2026-01-01T00:01:00Z".into(),
			start_progression: Some(0.1),
			end_progression: Some(0.2),
			idle_ms: 0,
			active_ms: None,
		};
		assert!(validate_session(&session).is_ok());
		assert!(validate_session(&SessionInput {
			active_ms: Some(0),
			..session.clone()
		})
		.is_ok());
		assert!(validate_session(&SessionInput {
			active_ms: Some(MAX_SESSION_ACTIVE_MS),
			..session.clone()
		})
		.is_ok());
		assert_eq!(
			validate_session(&SessionInput {
				active_ms: Some(-1),
				..session.clone()
			})
			.unwrap_err()
			.0,
			"active_out_of_range"
		);
		assert_eq!(
			validate_session(&SessionInput {
				active_ms: Some(MAX_SESSION_ACTIVE_MS + 1),
				..session.clone()
			})
			.unwrap_err()
			.0,
			"active_out_of_range"
		);
		assert_eq!(
			validate_session(&SessionInput {
				idle_ms: 60_001,
				active_ms: Some(0),
				..session.clone()
			})
			.unwrap_err()
			.0,
			"idle_out_of_range"
		);
	}

	#[tokio::test]
	async fn session_refusal_serializes_recovery_identity() {
		let response = error_response(LiseurSyncError::ItemRefusal {
			status: StatusCode::BAD_REQUEST,
			code: "unknown_work",
			message: "unknown work".into(),
			item_index: Some(2),
			session_id: Some("session-3".into()),
			op_id: None,
			work_id: Some("work-3".into()),
			limit: None,
		});
		assert_eq!(response.status(), StatusCode::BAD_REQUEST);
		let body = axum::body::to_bytes(response.into_body(), 1024)
			.await
			.unwrap();
		let body: Value = serde_json::from_slice(&body).unwrap();
		assert_eq!(body["code"], "unknown_work");
		assert_eq!(body["item_index"], 2);
		assert_eq!(body["session_id"], "session-3");
		assert_eq!(body["work_id"], "work-3");
	}

	#[test]
	fn serialized_wire_names_match_openapi() {
		let input = serde_json::to_value(SessionInput {
			session_id: "s".into(),
			work_id: "w".into(),
			edition_sha: Some("sha".into()),
			started_at: "2026-01-01T00:00:00Z".into(),
			ended_at: "2026-01-01T00:01:00Z".into(),
			start_progression: Some(0.1),
			end_progression: Some(0.2),
			idle_ms: 0,
			active_ms: Some(30_000),
		})
		.unwrap();
		assert_eq!(input["session_id"], "s");
		assert_eq!(input["start_progression"], 0.1);
		assert_eq!(input["active_ms"], 30_000);
		assert!(input.get("sessionId").is_none());
	}
	#[test]
	fn token_scope_requests_are_validated_and_canonicalized() {
		let request = TokenCreateRequest {
			name: "Boox Palma".into(),
			scope: None,
			scopes: Some(vec![
				"library-read".into(),
				"sync".into(),
				"library-read".into(),
			]),
			expires_in_seconds: None,
		};
		assert_eq!(
			requested_scopes(&request).unwrap(),
			vec!["sync".to_owned(), "library-read".to_owned()]
		);
		assert!(requested_scopes(&TokenCreateRequest {
			name: "empty".into(),
			scope: None,
			scopes: Some(Vec::new()),
			expires_in_seconds: None,
		})
		.is_err());
		assert!(requested_scopes(&TokenCreateRequest {
			name: "unknown".into(),
			scope: Some("not-a-scope".into()),
			scopes: None,
			expires_in_seconds: None,
		})
		.is_err());
		assert_eq!(
			requested_scopes(&TokenCreateRequest {
				name: "same".into(),
				scope: Some("sync".into()),
				scopes: Some(vec!["sync".into(), "sync".into()]),
				expires_in_seconds: None,
			})
			.unwrap(),
			vec!["sync".to_owned()]
		);
		assert!(requested_scopes(&TokenCreateRequest {
			name: "conflict".into(),
			scope: Some("sync".into()),
			scopes: Some(vec!["library-read".into()]),
			expires_in_seconds: None,
		})
		.is_err());
	}

	#[test]
	fn token_introspection_serializes_client_wire_shape() {
		let value = serde_json::to_value(TokenIntrospection {
			id: "token-id".into(),
			device_id: "device-id".into(),
			name: "Boox Palma".into(),
			scope: None,
			scopes: vec!["sync".into(), "library-read".into()],
			account_id: "account-id".into(),
			session_active_ms: true,
		})
		.unwrap();
		assert_eq!(value["id"], "token-id");

		assert_eq!(value["device_id"], "device-id");
		assert_eq!(value["name"], "Boox Palma");
		assert_eq!(value["scopes"], serde_json::json!(["sync", "library-read"]));
		assert_eq!(value["account_id"], "account-id");
		assert_eq!(value["session_active_ms"], true);
		assert!(value.get("scope").is_none());
		assert!(value.get("secret").is_none());
	}
	#[tokio::test]
	async fn session_tokens_are_management_only() {
		let session = LiseurToken {
			context: AuthContext {
				user: Default::default(),
				api_key: None,
				device_id: None,
			},
			device_id: "session-device".into(),
			name: "login".into(),
			scopes: Vec::new(),
			kind: LiseurTokenKind::LoginSession,
			token_id: Some("session-id".into()),
		};
		assert!(matches!(
			require_sync(&session),
			Err(LiseurSyncError::Forbidden(_))
		));
		let response = token_introspection(Extension(session))
			.await
			.expect_err("login sessions must not self-introspect");
		assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
	}

	#[tokio::test]
	async fn device_tokens_can_sync_and_self_introspect() {
		let device = LiseurToken {
			context: AuthContext {
				user: Default::default(),
				api_key: None,
				device_id: None,
			},
			device_id: "device-id".into(),
			name: "Boox Palma".into(),
			scopes: vec!["sync".into(), "library-read".into()],
			kind: LiseurTokenKind::Device,
			token_id: Some("token-id".into()),
		};
		assert!(require_sync(&device).is_ok());
		let response = token_introspection(Extension(device))
			.await
			.expect("device tokens can self-introspect");
		let value = serde_json::to_value(response.0).unwrap();
		assert_eq!(value["device_id"], "device-id");
		assert_eq!(value["name"], "Boox Palma");
		assert_eq!(value["account_id"], "");
	}

	#[derive(Clone)]
	struct FakeBackend;

	#[async_trait]
	impl LiseurSyncBackend for FakeBackend {
		async fn login(&self, _: &str, _: &str) -> Result<LoginResult, LiseurSyncError> {
			Ok(LoginResult {
				auth_token: "token".into(),
				expires_in: 3600,
			})
		}

		async fn authenticate(&self, _: &str) -> Result<LiseurToken, LiseurSyncError> {
			Err(LiseurSyncError::Unauthorized)
		}

		async fn mint_token(
			&self,
			_: &str,
			_: &str,
			_: Vec<String>,
			_: Option<i64>,
		) -> Result<TokenCreateResult, LiseurSyncError> {
			unreachable!()
		}

		async fn revoke_token(&self, _: &str, _: &str) -> Result<(), LiseurSyncError> {
			unreachable!()
		}
		async fn settings(
			&self,
			_: &str,
		) -> Result<BTreeMap<String, SettingValue>, LiseurSyncError> {
			Ok(BTreeMap::new())
		}

		async fn put_settings(
			&self,
			_: &str,
			_: Vec<SettingUpdate>,
		) -> Result<(), LiseurSyncError> {
			Ok(())
		}
		async fn resolve_work(
			&self,
			_: &str,
			_: ResolveRequest,
		) -> Result<ResolveResult, LiseurSyncError> {
			unreachable!()
		}

		async fn append_ops(
			&self,
			_: &str,
			_: &str,
			_: Vec<OpInput>,
		) -> Result<Vec<OpResult>, LiseurSyncError> {
			unreachable!()
		}
		async fn changes(
			&self,
			_: &str,
			_: i64,
			_: usize,
		) -> Result<ChangesPage, LiseurSyncError> {
			unreachable!()
		}
		async fn heads(&self, _: &str) -> Result<HeadsPage, LiseurSyncError> {
			unreachable!()
		}
		async fn positions(
			&self,
			_: &str,
			_: &str,
			_: usize,
		) -> Result<Vec<OpRecord>, LiseurSyncError> {
			unreachable!()
		}
		async fn append_sessions(
			&self,
			_: &str,
			_: &str,
			_: Vec<SessionInput>,
		) -> Result<usize, LiseurSyncError> {
			unreachable!()
		}
		async fn append_annotations(
			&self,
			_: &str,
			_: &str,
			_: Vec<AnnotationInput>,
		) -> Result<Vec<AnnotationResult>, LiseurSyncError> {
			unreachable!()
		}
		async fn annotation_changes(
			&self,
			_: &str,
			_: i64,
			_: usize,
		) -> Result<(Vec<AnnotationRecord>, i64, bool), LiseurSyncError> {
			unreachable!()
		}
		async fn work_annotations(
			&self,
			_: &str,
			_: &str,
		) -> Result<Vec<AnnotationRecord>, LiseurSyncError> {
			unreachable!()
		}
		async fn delete_annotation(
			&self,
			_: &str,
			_: &str,
			_: i64,
		) -> Result<DeleteAnnotationResult, LiseurSyncError> {
			unreachable!()
		}
		async fn put_attachment(
			&self,
			_: &str,
			_: &str,
			_: AttachmentUpload,
		) -> Result<AttachmentUploadResult, LiseurSyncError> {
			unreachable!()
		}
		async fn attachments(
			&self,
			_: &str,
			_: &str,
		) -> Result<Vec<AttachmentRecord>, LiseurSyncError> {
			unreachable!()
		}
	}

	#[tokio::test]
	async fn login_route_is_public_but_protected_routes_require_bearer() {
		let app = routes::<(), FakeBackend>().layer(Extension(FakeBackend));
		let login = app
			.clone()
			.oneshot(
				Request::builder()
					.method("POST")
					.uri("/v1/login")
					.header("content-type", "application/json")
					.body(Body::from(r#"{"username":"u","password":"p"}"#))
					.unwrap(),
			)
			.await
			.unwrap();
		assert_eq!(login.status(), StatusCode::OK);

		let protected = app
			.oneshot(
				Request::builder()
					.method("GET")
					.uri("/v1/changes")
					.body(Body::empty())
					.unwrap(),
			)
			.await
			.unwrap();
		assert_eq!(protected.status(), StatusCode::UNAUTHORIZED);
	}

	/// Liseur's live connector stops for the session on 404 and retries
	/// forever on anything else, so the refusal must be a real 404 behind the
	/// same bearer boundary as every other `/v1` route.
	#[tokio::test]
	async fn live_event_stream_is_refused_with_not_found_behind_bearer_auth() {
		let app = routes::<(), FakeBackend>().layer(Extension(FakeBackend));
		let anonymous = app
			.clone()
			.oneshot(
				Request::builder()
					.method("GET")
					.uri("/v1/events")
					.header("accept", "text/event-stream")
					.body(Body::empty())
					.unwrap(),
			)
			.await
			.unwrap();
		assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

		let refused = deferred_events().await;
		assert_eq!(refused.status(), StatusCode::NOT_FOUND);
		let body = axum::body::to_bytes(refused.into_body(), 1024)
			.await
			.unwrap();
		let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
		assert_eq!(json["error"], "live event stream is not implemented");
	}
}
