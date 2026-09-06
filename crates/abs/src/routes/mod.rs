//! Router composition and the server-facing backend contract.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use axum::{
	body::Body,
	http::{HeaderMap, Response},
	response::IntoResponse,
	routing::{delete, get, post},
	Extension, Router,
};
use models::entity::user::AuthUser;
use sea_orm::DatabaseConnection;

use crate::{
	errors::AbsResult,
	model::{AbsAudio, AbsBookmark, AbsImage, AbsPositionUpdate, AbsProgress},
};

mod identity;
mod items;
mod libraries;
mod me;
mod query;
mod session;
#[cfg(test)]
mod tests;

/// A play session, as the profile stores it in `abs_sessions`.
///
/// Audiobookshelf keeps playback sessions server-side and a client syncs
/// against the *session* id, not the item id
/// (`POST /api/session/{id}/sync`, Lissen
/// `common/api/library/AudioBookshelfLibrarySyncService.kt:17-28`), so the row
/// has to outlive the play request. `time_listening_ms` accumulates across
/// syncs; `current_time_ms` is the last position the client reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbsSession {
	pub id: String,
	pub user_id: String,
	pub media_id: String,
	pub library_id: String,
	pub device_id: Option<String>,
	pub client_name: Option<String>,
	pub client_version: Option<String>,
	pub media_player: Option<String>,
	pub current_time_ms: i64,
	pub time_listening_ms: i64,
	pub started_at: chrono::DateTime<chrono::Utc>,
	pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// The persistence and platform operations the Audiobookshelf surface needs
/// from the server.
///
/// Library, series, book and metadata rows are read directly through
/// [`AbsBackend::conn`] with the shared visibility helpers, exactly as the
/// other profiles do. Everything that depends on tables outside the profile's
/// own contract — the audio probe results, the unified reading state,
/// bookmarks, files on disk — is behind an async method, so the route surface
/// can be exercised against an in-memory stub and the profile never encodes a
/// second opinion about how a position is stored.
#[async_trait]
pub trait AbsBackend: Send + Sync {
	fn conn(&self) -> &DatabaseConnection;

	/// The secret the profile's JWTs are signed with (Stump's access-token
	/// secret).
	async fn token_secret(&self) -> AbsResult<Vec<u8>>;

	/// Resolve username/password credentials (`POST /login`) and register the
	/// client as an ABS device, so the device's library scope applies to
	/// every later request on the minted token. Returns the authenticated
	/// user and the `devices.id` row, if one was registered.
	async fn authenticate_password(
		&self,
		username: &str,
		password: &str,
	) -> AbsResult<(AuthUser, Option<String>)>;

	/// Whether this installation runs in a container, for `Source`.
	fn is_docker(&self) -> bool;

	/// The user behind a token subject, and when their row was created
	/// (`user.createdAt`). `POST /auth/refresh` arrives with nothing but a
	/// user id in the token, so this is also how that route resolves its
	/// caller.
	async fn user(
		&self,
		user_id: &str,
	) -> AbsResult<(AuthUser, chrono::DateTime<chrono::Utc>)>;

	/// The audio facts of one book, or `None` when the row has no audio (an
	/// ebook, or a row the probe has not reached yet).
	async fn audio(&self, media_id: &str) -> AbsResult<Option<AbsAudio>>;

	/// The audio facts of a whole page of books in one query. A list route
	/// must never fan out per row.
	async fn audio_batch(
		&self,
		media_ids: &[String],
	) -> AbsResult<HashMap<String, AbsAudio>>;

	/// The user's listening position for one book.
	async fn progress(
		&self,
		user: &AuthUser,
		media_id: &str,
	) -> AbsResult<Option<AbsProgress>>;

	/// Every book the user has a position for, newest first — the shape
	/// `GET /api/me` and the "continue listening" shelf both need.
	async fn progress_all(
		&self,
		user: &AuthUser,
	) -> AbsResult<Vec<(String, AbsProgress)>>;

	/// Apply a position from a client onto the unified reading state.
	async fn apply_position(
		&self,
		user: &AuthUser,
		media_id: &str,
		update: AbsPositionUpdate,
	) -> AbsResult<()>;

	async fn bookmarks(
		&self,
		user: &AuthUser,
		media_id: &str,
	) -> AbsResult<Vec<AbsBookmark>>;

	/// Every bookmark of the user, for the `bookmarks[]` of `GET /api/me`.
	async fn bookmarks_all(
		&self,
		user: &AuthUser,
	) -> AbsResult<Vec<(String, AbsBookmark)>>;

	/// Create or rename the bookmark at `position_ms`; Audiobookshelf keys a
	/// bookmark by `(item, whole second)`, so a second create at the same
	/// second is a rename.
	async fn upsert_bookmark(
		&self,
		user: &AuthUser,
		media_id: &str,
		position_ms: i64,
		title: &str,
	) -> AbsResult<AbsBookmark>;

	async fn delete_bookmark(
		&self,
		user: &AuthUser,
		media_id: &str,
		position_ms: i64,
	) -> AbsResult<()>;

	/// The book's cover image.
	async fn cover(&self, user: &AuthUser, media_id: &str) -> AbsResult<AbsImage>;

	/// Stream one audio track, honouring `Range` from `headers`.
	///
	/// `device_id` is the device whose credential authenticated the request,
	/// when one did. The audio transform preset that decides whether the
	/// bytes are transcoded lives on that device, so a backend that ignored
	/// it would serve every client the stored encoding.
	async fn serve_track(
		&self,
		headers: HeaderMap,
		media_id: &str,
		track_index: i32,
		device_id: Option<&str>,
	) -> AbsResult<Response<Body>>;

	/// Persist a new play session.
	async fn create_session(&self, session: AbsSession) -> AbsResult<()>;

	/// Load a play session owned by `user_id`.
	async fn session(
		&self,
		user_id: &str,
		session_id: &str,
	) -> AbsResult<Option<AbsSession>>;

	/// Record the position and accumulated listening time of a session.
	async fn update_session(
		&self,
		session_id: &str,
		current_time_ms: i64,
		time_listening_ms: i64,
	) -> AbsResult<()>;

	async fn close_session(&self, session_id: &str) -> AbsResult<()>;

	/// Resolve, allocating on first use, the ABS ids that have no Stump uuid
	/// to borrow: `media.id` per book, `libraryFolder.id` per library and one
	/// id per author name.
	async fn book_ids(&self, media_ids: &[String]) -> AbsResult<HashMap<String, String>>;

	async fn folder_id(&self, library_id: &str) -> AbsResult<String>;

	async fn author_ids(&self, names: &[String]) -> AbsResult<HashMap<String, String>>;

	/// The author name an allocated author id belongs to.
	async fn author_name(&self, author_id: &str) -> AbsResult<Option<String>>;
}

/// The type every handler takes: the backend, plus the user the server's
/// middleware already resolved.
pub(crate) type Backend = Extension<Arc<dyn AbsBackend>>;
pub(crate) type User = Extension<AuthUser>;

/// abs-ref answers `res.sendStatus(200)` with the reason phrase as
/// `text/plain`, which is the literal `OK` captured in
/// `capture/{session_sync,session_close,progress_patch}.txt`. Clients that
/// parse the body — Lissen decodes `Response<Unit>` — accept it either way,
/// but a byte-diff against abs-ref should not light up over this.
pub(crate) fn ok_text() -> Response<Body> {
	(
		[(
			axum::http::header::CONTENT_TYPE,
			"text/plain; charset=utf-8",
		)],
		"OK",
	)
		.into_response()
}

/// The routes that need no credentials: the two probes a client hits before
/// login, plus login and refresh themselves.
pub fn public_router<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::new()
		.route("/ping", get(identity::ping))
		.route("/healthcheck", get(identity::healthcheck))
		.route("/status", get(identity::status))
		.route("/login", post(identity::login))
		.route("/auth/refresh", post(identity::refresh))
}

/// Everything behind authentication. The server mounts this under the same
/// prefix abs-ref uses (`/api`), with its auth middleware in front, so a
/// handler here always has an [`AuthUser`].
pub fn authenticated_router<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::new()
		.route("/authorize", post(identity::authorize))
		.route("/me", get(me::me))
		.route(
			"/me/progress/{item_id}",
			get(me::progress).patch(me::patch_progress),
		)
		.route(
			"/me/item/{item_id}/bookmark",
			post(me::create_bookmark).patch(me::update_bookmark),
		)
		.route(
			"/me/item/{item_id}/bookmark/{time}",
			delete(me::delete_bookmark),
		)
		.route("/libraries", get(libraries::list))
		.route("/libraries/{library_id}", get(libraries::detail))
		.route("/libraries/{library_id}/items", get(libraries::items))
		.route(
			"/libraries/{library_id}/personalized",
			get(libraries::personalized),
		)
		.route("/libraries/{library_id}/authors", get(libraries::authors))
		.route("/libraries/{library_id}/search", get(libraries::search))
		.route("/items/batch/get", post(items::batch_get))
		.route("/items/{item_id}", get(items::detail))
		.route("/items/{item_id}/cover", get(items::cover))
		.route("/items/{item_id}/file/{ino}", get(items::file))
		.route("/items/{item_id}/play", post(items::play))
		.route("/session/{session_id}", get(session::detail))
		.route("/session/{session_id}/sync", post(session::sync))
		.route("/session/{session_id}/close", post(session::close))
		.route("/authors/{author_id}", get(items::author))
		.route("/authors/{author_id}/image", get(items::author_image))
}
