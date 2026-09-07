//! Router composition and the server-facing backend contract.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use axum::{
	body::Body,
	http::{HeaderMap, Response},
	response::IntoResponse,
	routing::{delete, get, patch, post},
	Extension, Router,
};
use models::entity::user::AuthUser;
use sea_orm::DatabaseConnection;

use crate::{
	errors::AbsResult,
	model::{
		AbsAudio, AbsBookmark, AbsEbookFile, AbsImage, AbsPlaylist, AbsPositionUpdate,
		AbsProgress,
	},
};

mod identity;
mod items;
mod libraries;
pub(crate) mod me;
mod playlists;
pub(crate) mod query;
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
	/// Audiobookshelf's `playMethod`: `0` DirectPlay for a session opened by
	/// a play request, `3` Local for one the official app recorded offline
	/// and uploaded through `POST /api/session/local`.
	pub play_method: i64,
}

/// Audiobookshelf's `playMethod` for a session served from the stored
/// tracks. Stump never transcodes on this lane.
pub const PLAY_METHOD_DIRECT: i64 = 0;
/// Audiobookshelf's `playMethod` for listening the client did offline.
pub const PLAY_METHOD_LOCAL: i64 = 3;

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
	///
	/// Answers whether the update **moved the head**. It is not always
	/// accepted: the head keeps the newest position, and an update dated
	/// more than the tolerance behind it that also reports less progress is
	/// recorded as provenance only. `POST /api/session/local-all` reports
	/// exactly that distinction back to the client as `progressSynced`.
	async fn apply_position(
		&self,
		user: &AuthUser,
		media_id: &str,
		update: AbsPositionUpdate,
	) -> AbsResult<bool>;

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

	/// The book's cover image. `None` for the user is the anonymous cover
	/// lane: from 2.17.0 the official app sends no credential with an image
	/// request, and the route has already established that the row is an
	/// audible, undeleted book.
	async fn cover(&self, user: Option<&AuthUser>, media_id: &str)
		-> AbsResult<AbsImage>;

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

	/// Load a play session by id alone, with no user to scope it.
	///
	/// `GET /public/session/{id}/track/{index}` is unauthenticated by
	/// Audiobookshelf's own design — the official app streams every
	/// direct-play track from it once the server reports >= 2.22.0
	/// (`android/.../data/PlaybackSession.kt:198-201`,
	/// `plugins/capacitor/AbsAudioPlayer.js:258`) — so the session id is the
	/// capability. It is a v4 uuid, minted per play request and never
	/// enumerable.
	async fn session_by_id(&self, session_id: &str) -> AbsResult<Option<AbsSession>>;

	/// Every play session of the user, newest first, optionally for one
	/// book: the listening history behind `GET /api/me/listening-sessions`,
	/// `/listening-stats` and `/api/me/item/listening-sessions/{id}`.
	async fn sessions(
		&self,
		user_id: &str,
		media_id: Option<&str>,
	) -> AbsResult<Vec<AbsSession>>;

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

	/// The confirmed EPUB edition paired with each of `media_ids`, for the
	/// requesting user.
	///
	/// Pairs are per user (`liseur_sync_media_links.user_id`), so this takes
	/// the caller: two users of the same server can disagree about whether
	/// an audiobook and an ebook are the same work.
	async fn ebook_editions(
		&self,
		user: &AuthUser,
		media_ids: &[String],
	) -> AbsResult<HashMap<String, AbsEbookFile>>;

	/// Every audiobook of the user that has a confirmed EPUB edition, for
	/// the `filter=ebooks.<base64>` list filter. Bounded by the user's
	/// confirmed pairs, not by the library.
	async fn paired_ebook_media_ids(&self, user: &AuthUser) -> AbsResult<Vec<String>>;

	/// Stream the paired EPUB, honouring `Range`.
	async fn serve_ebook(
		&self,
		headers: HeaderMap,
		user: &AuthUser,
		ebook: &AbsEbookFile,
	) -> AbsResult<Response<Body>>;

	/// Zip every file of one library item — its audio tracks, plus the
	/// paired EPUB when there is one — as `GET /api/items/{id}/download`.
	async fn download_item(
		&self,
		headers: HeaderMap,
		user: &AuthUser,
		media_id: &str,
		ebook: Option<&AbsEbookFile>,
	) -> AbsResult<Response<Body>>;

	/// The user's playlists, newest first, or one of them.
	async fn playlists(&self, user: &AuthUser) -> AbsResult<Vec<AbsPlaylist>>;

	async fn playlist(
		&self,
		user: &AuthUser,
		playlist_id: &str,
	) -> AbsResult<Option<AbsPlaylist>>;

	async fn create_playlist(
		&self,
		user: &AuthUser,
		name: &str,
		description: Option<&str>,
		media_ids: &[String],
	) -> AbsResult<AbsPlaylist>;

	/// Rename, re-describe or re-populate a playlist. `None` leaves a field
	/// untouched.
	async fn update_playlist(
		&self,
		user: &AuthUser,
		playlist_id: &str,
		name: Option<&str>,
		description: Option<Option<&str>>,
		media_ids: Option<&[String]>,
	) -> AbsResult<AbsPlaylist>;

	async fn delete_playlist(&self, user: &AuthUser, playlist_id: &str) -> AbsResult<()>;
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
/// login, login and refresh themselves, the direct-play track lane, and the
/// item covers.
///
/// The last two are unauthenticated because Audiobookshelf made them so and
/// the official app depends on it:
///
/// - `GET /public/session/{id}/track/{index}` is where every direct-play
///   track is streamed from once the server reports >= 2.22.0
///   (`android/.../data/PlaybackSession.kt:196-204`,
///   `plugins/capacitor/AbsAudioPlayer.js:254-261`). The v4 session uuid is
///   the capability.
/// - `GET /api/items/{id}/cover` carries no token once the server reports
///   >= 2.17.0: the webview builds the `<img>` src without one
///   (`store/index.js:89-102`, `store/globals.js:54-56,68-70`) and so does
///   the notification art loader
///   (`android/.../data/PlaybackSession.kt:186-190`). abs-ref 2.36.0 agrees
///   — an anonymous cover request there answers `404`, not `401`.
pub fn public_router<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::new()
		.route("/ping", get(identity::ping))
		.route("/healthcheck", get(identity::healthcheck))
		.route("/status", get(identity::status))
		.route("/login", post(identity::login))
		.route("/logout", post(identity::logout))
		.route("/auth/refresh", post(identity::refresh))
		.route(
			"/public/session/{session_id}/track/{track}",
			get(items::public_track),
		)
		.route("/api/items/{item_id}/cover", get(items::cover))
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
		// The official app suffixes an episode id for a podcast
		// (`ApiHandler.kt:688,696`). The profile serves no podcast library,
		// so the book has no episode to address and the pair answers `404`,
		// as abs-ref does for an episode id that is not in the item.
		.route(
			"/me/progress/{item_id}/{episode_id}",
			get(me::episode_progress).patch(me::episode_progress),
		)
		.route("/me/items-in-progress", get(me::items_in_progress))
		.route("/me/listening-sessions", get(me::listening_sessions))
		.route("/me/listening-stats", get(me::listening_stats))
		.route(
			"/me/item/listening-sessions/{item_id}",
			get(me::item_listening_sessions),
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
		.route("/libraries/{library_id}/series", get(libraries::series))
		.route(
			"/libraries/{library_id}/collections",
			get(libraries::collections),
		)
		.route(
			"/libraries/{library_id}/playlists",
			get(playlists::list_for_library),
		)
		.route("/libraries/{library_id}/search", get(libraries::search))
		.route("/items/batch/get", post(items::batch_get))
		.route("/items/{item_id}", get(items::detail))
		.route("/items/{item_id}/file/{ino}", get(items::file))
		.route(
			"/items/{item_id}/file/{ino}/download",
			get(items::file_download),
		)
		.route("/items/{item_id}/download", get(items::download))
		.route("/items/{item_id}/ebook", get(items::ebook))
		.route("/items/{item_id}/ebook/{file_id}", get(items::ebook_file))
		.route(
			"/items/{item_id}/ebook/{file_id}/status",
			patch(items::ebook_status),
		)
		.route("/items/{item_id}/play", post(items::play))
		.route("/session/{session_id}", get(session::detail))
		.route("/session/{session_id}/sync", post(session::sync))
		.route("/session/{session_id}/close", post(session::close))
		.route("/session/local", post(session::local))
		.route("/session/local-all", post(session::local_all))
		.route("/authors/{author_id}", get(items::author))
		.route("/authors/{author_id}/image", get(items::author_image))
		.route(
			"/playlists",
			post(playlists::create).get(playlists::list_all),
		)
		.route(
			"/playlists/{playlist_id}",
			get(playlists::detail)
				.patch(playlists::update)
				.delete(playlists::remove),
		)
		.route("/playlists/{playlist_id}/item", post(playlists::add_item))
		.route(
			"/playlists/{playlist_id}/item/{item_id}",
			delete(playlists::remove_item),
		)
		.route(
			"/playlists/{playlist_id}/batch/add",
			post(playlists::batch_add),
		)
		.route(
			"/playlists/{playlist_id}/batch/remove",
			post(playlists::batch_remove),
		)
}
