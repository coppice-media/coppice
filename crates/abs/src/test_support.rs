//! Shared in-memory fixtures for the route tests: a SQLite database with the
//! profile's own tables materialised, a stub [`AbsBackend`] over it, and a
//! helper that drives a request through the composed router.

use std::{collections::HashMap, sync::Arc};

use axum::{
	body::Body,
	http::{HeaderMap, Response, StatusCode},
	Extension, Router,
};
use chrono::{DateTime, TimeZone, Utc};
use models::{
	entity::{
		library, library_config, media, media_metadata, series, user, user::AuthUser,
	},
	shared::{enums::LibraryType, image::ImageRef},
};
use parking_lot::Mutex;
use sea_orm::{
	ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseBackend,
	DatabaseConnection, EntityTrait, QueryFilter, Statement,
};
use tower::ServiceExt;

use crate::{
	auth::mint_tokens,
	errors::{AbsError, AbsResult},
	ids::{AbsIds, IdKind, CREATE_ABS_IDS_SQL},
	model::{
		AbsAudio, AbsAudioChapter, AbsAudioTrack, AbsBookmark, AbsEbookFile, AbsImage,
		AbsPlaylist, AbsPositionUpdate, AbsProgress,
	},
	routes::{AbsBackend, AbsSession},
	sessions::AbsSessions,
};

/// The token secret the stub signs with; a fixed value so a test can mint a
/// token by hand and expect the server to accept it.
pub(crate) const SECRET: &[u8] = b"abs-test-secret";

pub(crate) async fn db() -> DatabaseConnection {
	let conn = ::tests::db::test_database().await;
	for sql in [CREATE_ABS_IDS_SQL, crate::sessions::CREATE_ABS_SESSIONS_SQL] {
		conn.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
			.await
			.unwrap();
	}
	conn
}

pub(crate) fn auth_user(user: &models::entity::user::Model) -> AuthUser {
	AuthUser {
		id: user.id.clone(),
		avatar_path: None,
		avatar: ImageRef::default(),
		username: user.username.clone(),
		is_server_owner: user.is_server_owner,
		is_locked: false,
		permissions: Vec::new(),
		age_restriction: None,
		preferences: None,
		device_library_scope: None,
	}
}

/// A library of the given type; `fake_data::Library` always builds the
/// default (`Mixed`) config, so the type is set afterwards.
pub(crate) async fn library_of_type(
	conn: &DatabaseConnection,
	library_type: LibraryType,
) -> library::Model {
	let row = ::tests::fake_data::Library::default().insert(conn).await;
	let config = library_config::Entity::find_by_id(row.config_id)
		.one(conn)
		.await
		.unwrap()
		.expect("library config");
	library_config::ActiveModel {
		library_type: Set(library_type),
		..config.into()
	}
	.update(conn)
	.await
	.unwrap();
	row
}

/// A series holding one media row per `(name, extension)`.
pub(crate) async fn series_with_files(
	conn: &DatabaseConnection,
	library_id: &str,
	name: &str,
	files: &[(&str, &str)],
) -> (series::Model, Vec<media::Model>) {
	let series_row = ::tests::fake_data::Series {
		name: Some(name.to_owned()),
		library_id: Some(library_id.to_owned()),
		..Default::default()
	}
	.insert(conn)
	.await;
	let mut rows = Vec::with_capacity(files.len());
	for (name, extension) in files {
		rows.push(
			::tests::fake_data::Media {
				series_id: series_row.id.clone(),
				name: Some((*name).to_owned()),
				extension: Some((*extension).to_owned()),
				..Default::default()
			}
			.insert(conn)
			.await,
		);
	}
	(series_row, rows)
}

/// Attach `media_metadata` to a book.
pub(crate) async fn metadata(
	conn: &DatabaseConnection,
	media_id: &str,
	title: &str,
	writers: Option<&str>,
) -> media_metadata::Model {
	media_metadata::ActiveModel {
		media_id: Set(Some(media_id.to_owned())),
		title: Set(Some(title.to_owned())),
		writers: Set(writers.map(str::to_owned)),
		..Default::default()
	}
	.insert(conn)
	.await
	.unwrap()
}

/// A single-track audiobook of `duration_ms`, whose track path is the media
/// path (the `isFile: true` shape).
pub(crate) fn one_track_audio(path: &str, duration_ms: i64) -> AbsAudio {
	AbsAudio {
		duration_ms,
		codec: "aac".to_owned(),
		sample_rate: Some(44_100),
		channels: Some(2),
		bitrate: Some(64_000),
		tracks: vec![AbsAudioTrack {
			index: 0,
			path: path.to_owned(),
			duration_ms,
			start_offset_ms: 0,
			byte_size: 3_781,
			mime: "audio/mp4".to_owned(),
		}],
		chapters: vec![AbsAudioChapter {
			index: 0,
			title: Some("Chapter One".to_owned()),
			start_ms: 0,
			end_ms: Some(duration_ms),
		}],
	}
}

/// A folder audiobook: two tracks below the item path.
pub(crate) fn two_track_audio(folder: &str) -> AbsAudio {
	AbsAudio {
		duration_ms: 8_000,
		codec: "mp3".to_owned(),
		sample_rate: Some(44_100),
		channels: Some(2),
		bitrate: Some(64_000),
		tracks: vec![
			AbsAudioTrack {
				index: 0,
				path: format!("{folder}/01 - Pass 1.mp3"),
				duration_ms: 4_000,
				start_offset_ms: 0,
				byte_size: 1_000,
				mime: "audio/mpeg".to_owned(),
			},
			AbsAudioTrack {
				index: 1,
				path: format!("{folder}/02 - Pass 2.mp3"),
				duration_ms: 4_000,
				start_offset_ms: 4_000,
				byte_size: 1_100,
				mime: "audio/mpeg".to_owned(),
			},
		],
		chapters: vec![
			AbsAudioChapter {
				index: 0,
				title: Some("Pass 1".to_owned()),
				start_ms: 0,
				end_ms: Some(4_000),
			},
			AbsAudioChapter {
				index: 1,
				title: Some("Pass 2".to_owned()),
				start_ms: 4_000,
				end_ms: Some(8_000),
			},
		],
	}
}

/// An [`AbsBackend`] answering persistence from the in-memory database and
/// audio/progress/bookmark/session state from maps a test seeds.
pub(crate) struct TestBackend {
	pub conn: DatabaseConnection,
	pub audio: Mutex<HashMap<String, AbsAudio>>,
	pub progress: Mutex<HashMap<String, HashMap<String, AbsProgress>>>,
	pub bookmarks: Mutex<HashMap<String, HashMap<String, Vec<AbsBookmark>>>>,
	/// Every position update the routes applied, in order: what a test
	/// asserts the reading state would have received.
	pub applied: Mutex<Vec<(String, AbsPositionUpdate)>>,
	pub covers: Mutex<HashMap<String, AbsImage>>,
	/// `(media_id, track_index, Range, device_id)` per track-route call: what
	/// a test asserts the delivery layer would have received.
	pub served: Mutex<Vec<(String, i32, Option<String>, Option<String>)>>,
	/// The device the request's credential resolves to, as the server's auth
	/// middleware would have put on the [`AuthContext`].
	pub device_id: Mutex<Option<String>>,
	/// The socket bus, when a test mounted one. The stub plays the part the
	/// server adapter plays in production: an accepted head is announced by
	/// whoever wrote it, never by the route.
	pub events: Mutex<Option<crate::socket::AbsEvents>>,
	/// Confirmed audiobook -> EPUB pairs, per user id.
	pub ebooks: Mutex<HashMap<String, HashMap<String, AbsEbookFile>>>,
	/// Reading lists, per user id, newest last.
	pub playlists: Mutex<HashMap<String, Vec<AbsPlaylist>>>,
	/// `(media_id, Range)` per served ebook and `media_id` per zipped
	/// download: what a test asserts the delivery layer received.
	pub served_ebooks: Mutex<Vec<(String, Option<String>)>>,
	pub downloaded: Mutex<Vec<(String, Option<String>)>>,
}

impl TestBackend {
	pub(crate) fn new(conn: DatabaseConnection) -> Arc<Self> {
		Arc::new(Self {
			conn,
			audio: Mutex::new(HashMap::new()),
			progress: Mutex::new(HashMap::new()),
			bookmarks: Mutex::new(HashMap::new()),
			applied: Mutex::new(Vec::new()),
			covers: Mutex::new(HashMap::new()),
			served: Mutex::new(Vec::new()),
			device_id: Mutex::new(None),
			events: Mutex::new(None),
			ebooks: Mutex::new(HashMap::new()),
			playlists: Mutex::new(HashMap::new()),
			served_ebooks: Mutex::new(Vec::new()),
			downloaded: Mutex::new(Vec::new()),
		})
	}

	pub(crate) fn set_audio(&self, media_id: &str, audio: AbsAudio) {
		self.audio.lock().insert(media_id.to_owned(), audio);
	}

	/// Announce accepted reading-head changes on this bus, the way the
	/// server adapter forwards `ReadingHeadChanged` to the socket lane.
	pub(crate) fn set_events(&self, events: crate::socket::AbsEvents) {
		*self.events.lock() = Some(events);
	}

	/// Authenticate every later request as this device, so the per-device
	/// audio transform preset is in play.
	pub(crate) fn set_device(&self, device_id: &str) {
		*self.device_id.lock() = Some(device_id.to_owned());
	}

	pub(crate) fn set_cover(&self, media_id: &str, image: AbsImage) {
		self.covers.lock().insert(media_id.to_owned(), image);
	}

	/// Every position update the routes applied, in order.
	pub(crate) fn applied_updates(&self) -> Vec<(String, AbsPositionUpdate)> {
		self.applied.lock().clone()
	}

	/// Seed a bookmark without going through a route.
	pub(crate) fn upsert_bookmark_for_test(
		&self,
		user_id: &str,
		media_id: &str,
		position_ms: i64,
		title: &str,
	) {
		self.bookmarks
			.lock()
			.entry(user_id.to_owned())
			.or_default()
			.entry(media_id.to_owned())
			.or_default()
			.push(AbsBookmark {
				position_ms,
				title: title.to_owned(),
				created_at: fixed(1_788_699_586_221),
			});
	}

	pub(crate) fn set_progress(
		&self,
		user_id: &str,
		media_id: &str,
		progress: AbsProgress,
	) {
		self.progress
			.lock()
			.entry(user_id.to_owned())
			.or_default()
			.insert(media_id.to_owned(), progress);
	}

	/// Record a confirmed audiobook -> EPUB pair for `user_id`.
	pub(crate) fn set_ebook(&self, user_id: &str, media_id: &str, ebook: AbsEbookFile) {
		self.ebooks
			.lock()
			.entry(user_id.to_owned())
			.or_default()
			.insert(media_id.to_owned(), ebook);
	}
}

fn fixed(millis: i64) -> DateTime<Utc> {
	Utc.timestamp_millis_opt(millis).unwrap()
}

#[async_trait::async_trait]
impl AbsBackend for TestBackend {
	fn conn(&self) -> &DatabaseConnection {
		&self.conn
	}

	async fn token_secret(&self) -> AbsResult<Vec<u8>> {
		Ok(SECRET.to_vec())
	}

	async fn authenticate_password(
		&self,
		username: &str,
		password: &str,
	) -> AbsResult<(AuthUser, Option<String>)> {
		if password != "correct-horse" {
			return Err(AbsError::Unauthorized);
		}
		let row = user::Entity::find()
			.filter(user::Column::Username.eq(username))
			.one(&self.conn)
			.await?
			.ok_or(AbsError::Unauthorized)?;
		Ok((auth_user(&row), Some("device-abs".to_owned())))
	}

	fn is_docker(&self) -> bool {
		false
	}

	async fn user(&self, user_id: &str) -> AbsResult<(AuthUser, DateTime<Utc>)> {
		let row = user::Entity::find_by_id(user_id)
			.one(&self.conn)
			.await?
			.ok_or(AbsError::Unauthorized)?;
		Ok((auth_user(&row), fixed(1_788_699_170_786)))
	}

	async fn audio(&self, media_id: &str) -> AbsResult<Option<AbsAudio>> {
		Ok(self.audio.lock().get(media_id).cloned())
	}

	async fn audio_batch(
		&self,
		media_ids: &[String],
	) -> AbsResult<HashMap<String, AbsAudio>> {
		let audio = self.audio.lock();
		Ok(media_ids
			.iter()
			.filter_map(|id| audio.get(id).map(|audio| (id.clone(), audio.clone())))
			.collect())
	}

	async fn progress(
		&self,
		user: &AuthUser,
		media_id: &str,
	) -> AbsResult<Option<AbsProgress>> {
		Ok(self
			.progress
			.lock()
			.get(&user.id)
			.and_then(|rows| rows.get(media_id))
			.cloned())
	}

	async fn progress_all(
		&self,
		user: &AuthUser,
	) -> AbsResult<Vec<(String, AbsProgress)>> {
		Ok(self
			.progress
			.lock()
			.get(&user.id)
			.map(|rows| {
				rows.iter()
					.map(|(id, progress)| (id.clone(), progress.clone()))
					.collect()
			})
			.unwrap_or_default())
	}

	/// Applies the **real** conflict rule
	/// ([`models::domain::reading_state::resolve`]) rather than a second
	/// opinion about it, so a route test that asserts an offline session was
	/// or was not merged is asserting what the server would do.
	async fn apply_position(
		&self,
		user: &AuthUser,
		media_id: &str,
		update: AbsPositionUpdate,
	) -> AbsResult<bool> {
		use models::domain::reading_state::{resolve, HeadState, Outcome, Projection};

		self.applied.lock().push((media_id.to_owned(), update));

		let mut progress = self.progress.lock();
		let rows = progress.entry(user.id.clone()).or_default();
		let existing = rows.get(media_id).cloned();
		let duration_ms = update.duration_ms;
		let progression = (duration_ms > 0)
			.then(|| (update.position_ms as f64 / duration_ms as f64).clamp(0.0, 1.0));
		let head = existing.as_ref().map(|progress| HeadState {
			updated_at: progress.last_update,
			progression: if duration_ms > 0 {
				(progress.position_ms as f64 / duration_ms as f64).clamp(0.0, 1.0)
			} else {
				0.0
			},
			completed: progress.is_finished,
		});
		let incoming_at = update.at.unwrap_or_else(Utc::now);
		let resolved = resolve(
			head,
			&Projection {
				position_ms: Some(update.position_ms),
				track_index: update.track_index,
				progression,
				completed: update.is_finished,
				..Default::default()
			},
			incoming_at,
		);
		if resolved.outcome != Outcome::Accepted {
			// Provenance only: the update was recorded (`applied`) and the
			// head did not move.
			return Ok(false);
		}

		let finished = resolved.completed
			|| (update.is_finished.is_none()
				&& duration_ms > 0
				&& update.position_ms >= duration_ms);
		rows.insert(
			media_id.to_owned(),
			AbsProgress {
				position_ms: update.position_ms,
				track_index: update.track_index,
				is_finished: finished,
				started_at: existing
					.as_ref()
					.map(|progress| progress.started_at)
					.unwrap_or(incoming_at),
				last_update: incoming_at,
				finished_at: finished.then_some(incoming_at),
			},
		);
		drop(progress);

		if let Some(events) = self.events.lock().clone() {
			events.send(crate::socket::AbsEvent::ProgressChanged {
				user_id: user.id.clone(),
				media_id: media_id.to_owned(),
			});
		}
		Ok(true)
	}

	async fn bookmarks(
		&self,
		user: &AuthUser,
		media_id: &str,
	) -> AbsResult<Vec<AbsBookmark>> {
		Ok(self
			.bookmarks
			.lock()
			.get(&user.id)
			.and_then(|rows| rows.get(media_id))
			.cloned()
			.unwrap_or_default())
	}

	async fn bookmarks_all(
		&self,
		user: &AuthUser,
	) -> AbsResult<Vec<(String, AbsBookmark)>> {
		Ok(self
			.bookmarks
			.lock()
			.get(&user.id)
			.map(|rows| {
				rows.iter()
					.flat_map(|(media_id, bookmarks)| {
						bookmarks
							.iter()
							.map(|bookmark| (media_id.clone(), bookmark.clone()))
							.collect::<Vec<_>>()
					})
					.collect()
			})
			.unwrap_or_default())
	}

	async fn upsert_bookmark(
		&self,
		user: &AuthUser,
		media_id: &str,
		position_ms: i64,
		title: &str,
	) -> AbsResult<AbsBookmark> {
		let mut bookmarks = self.bookmarks.lock();
		let rows = bookmarks
			.entry(user.id.clone())
			.or_default()
			.entry(media_id.to_owned())
			.or_default();
		let created_at = rows
			.iter()
			.find(|bookmark| bookmark.position_ms == position_ms)
			.map(|bookmark| bookmark.created_at)
			.unwrap_or_else(|| fixed(1_788_699_586_221));
		rows.retain(|bookmark| bookmark.position_ms != position_ms);
		let bookmark = AbsBookmark {
			position_ms,
			title: title.to_owned(),
			created_at,
		};
		rows.push(bookmark.clone());
		rows.sort_by_key(|bookmark| bookmark.position_ms);
		Ok(bookmark)
	}

	async fn delete_bookmark(
		&self,
		user: &AuthUser,
		media_id: &str,
		position_ms: i64,
	) -> AbsResult<()> {
		if let Some(rows) = self
			.bookmarks
			.lock()
			.get_mut(&user.id)
			.and_then(|rows| rows.get_mut(media_id))
		{
			rows.retain(|bookmark| bookmark.position_ms != position_ms);
		}
		Ok(())
	}

	async fn cover(
		&self,
		_user: Option<&AuthUser>,
		media_id: &str,
	) -> AbsResult<AbsImage> {
		self.covers
			.lock()
			.get(media_id)
			.cloned()
			.ok_or_else(|| AbsError::NotFound(format!("No cover for {media_id}")))
	}

	async fn serve_track(
		&self,
		headers: HeaderMap,
		media_id: &str,
		track_index: i32,
		device_id: Option<&str>,
	) -> AbsResult<Response<Body>> {
		let audio = self
			.audio
			.lock()
			.get(media_id)
			.cloned()
			.ok_or_else(|| AbsError::NotFound(format!("No audio for {media_id}")))?;
		let track = audio
			.track(track_index)
			.ok_or_else(|| AbsError::NotFound(format!("No file {track_index}")))?
			.clone();
		let range = headers
			.get(axum::http::header::RANGE)
			.and_then(|value| value.to_str().ok())
			.map(str::to_owned);
		self.served.lock().push((
			media_id.to_owned(),
			track_index,
			range.clone(),
			device_id.map(str::to_owned),
		));

		let status = if range.is_some() {
			StatusCode::PARTIAL_CONTENT
		} else {
			StatusCode::OK
		};
		Ok(Response::builder()
			.status(status)
			.header(axum::http::header::CONTENT_TYPE, track.mime)
			.header(axum::http::header::ACCEPT_RANGES, "bytes")
			.body(Body::from(vec![0u8; 8]))
			.unwrap())
	}

	// The session store is exercised for real: the stub goes through the same
	// `abs_sessions` SQL the server adapter uses, against the in-memory
	// database, so a route test covers the table as well as the handler.
	async fn create_session(&self, session: AbsSession) -> AbsResult<()> {
		Ok(AbsSessions::insert(&self.conn, &session).await?)
	}

	async fn session(
		&self,
		user_id: &str,
		session_id: &str,
	) -> AbsResult<Option<AbsSession>> {
		Ok(AbsSessions::get(&self.conn, user_id, session_id).await?)
	}

	async fn sessions(
		&self,
		user_id: &str,
		media_id: Option<&str>,
	) -> AbsResult<Vec<AbsSession>> {
		Ok(AbsSessions::list(&self.conn, user_id, media_id).await?)
	}

	async fn update_session(
		&self,
		session_id: &str,
		current_time_ms: i64,
		time_listening_ms: i64,
	) -> AbsResult<()> {
		Ok(AbsSessions::update(
			&self.conn,
			session_id,
			current_time_ms,
			time_listening_ms,
		)
		.await?)
	}

	async fn close_session(&self, session_id: &str) -> AbsResult<()> {
		Ok(AbsSessions::close(&self.conn, session_id).await?)
	}

	async fn book_ids(&self, media_ids: &[String]) -> AbsResult<HashMap<String, String>> {
		AbsIds::resolve_many(&self.conn, IdKind::Book, media_ids)
			.await
			.map_err(AbsError::from)
	}

	async fn folder_id(&self, library_id: &str) -> AbsResult<String> {
		Ok(AbsIds::resolve(&self.conn, IdKind::Folder, library_id).await?)
	}

	async fn author_ids(&self, names: &[String]) -> AbsResult<HashMap<String, String>> {
		AbsIds::resolve_many(&self.conn, IdKind::Author, names)
			.await
			.map_err(AbsError::from)
	}

	async fn author_name(&self, author_id: &str) -> AbsResult<Option<String>> {
		Ok(AbsIds::lookup(&self.conn, IdKind::Author, author_id).await?)
	}

	async fn session_by_id(&self, session_id: &str) -> AbsResult<Option<AbsSession>> {
		Ok(AbsSessions::get_any(&self.conn, session_id).await?)
	}

	async fn ebook_editions(
		&self,
		user: &AuthUser,
		media_ids: &[String],
	) -> AbsResult<HashMap<String, AbsEbookFile>> {
		let ebooks = self.ebooks.lock();
		let Some(rows) = ebooks.get(&user.id) else {
			return Ok(HashMap::new());
		};
		Ok(media_ids
			.iter()
			.filter_map(|id| rows.get(id).map(|ebook| (id.clone(), ebook.clone())))
			.collect())
	}

	async fn paired_ebook_media_ids(&self, user: &AuthUser) -> AbsResult<Vec<String>> {
		Ok(self
			.ebooks
			.lock()
			.get(&user.id)
			.map(|rows| rows.keys().cloned().collect())
			.unwrap_or_default())
	}

	async fn serve_ebook(
		&self,
		headers: HeaderMap,
		_user: &AuthUser,
		ebook: &AbsEbookFile,
	) -> AbsResult<Response<Body>> {
		let range = headers
			.get(axum::http::header::RANGE)
			.and_then(|value| value.to_str().ok())
			.map(str::to_owned);
		self.served_ebooks
			.lock()
			.push((ebook.media_id.clone(), range.clone()));
		let status = if range.is_some() {
			StatusCode::PARTIAL_CONTENT
		} else {
			StatusCode::OK
		};
		Ok(Response::builder()
			.status(status)
			.header(axum::http::header::CONTENT_TYPE, "application/epub+zip")
			.header(axum::http::header::ACCEPT_RANGES, "bytes")
			.body(Body::from(vec![0u8; 8]))
			.unwrap())
	}

	async fn download_item(
		&self,
		_headers: HeaderMap,
		_user: &AuthUser,
		media_id: &str,
		ebook: Option<&AbsEbookFile>,
	) -> AbsResult<Response<Body>> {
		self.downloaded.lock().push((
			media_id.to_owned(),
			ebook.map(|ebook| ebook.media_id.clone()),
		));
		Ok(Response::builder()
			.status(StatusCode::OK)
			.header(axum::http::header::CONTENT_TYPE, "application/zip")
			.header(
				axum::http::header::CONTENT_DISPOSITION,
				format!("attachment; filename=\"{media_id}.zip\""),
			)
			.body(Body::from(vec![0u8; 8]))
			.unwrap())
	}

	async fn playlists(&self, user: &AuthUser) -> AbsResult<Vec<AbsPlaylist>> {
		Ok(self
			.playlists
			.lock()
			.get(&user.id)
			.cloned()
			.unwrap_or_default())
	}

	async fn playlist(
		&self,
		user: &AuthUser,
		playlist_id: &str,
	) -> AbsResult<Option<AbsPlaylist>> {
		Ok(self
			.playlists
			.lock()
			.get(&user.id)
			.and_then(|rows| rows.iter().find(|row| row.id == playlist_id).cloned()))
	}

	async fn create_playlist(
		&self,
		user: &AuthUser,
		name: &str,
		description: Option<&str>,
		media_ids: &[String],
	) -> AbsResult<AbsPlaylist> {
		let playlist = AbsPlaylist {
			id: format!("playlist-{}", uuid::Uuid::new_v4()),
			name: name.to_owned(),
			description: description.map(str::to_owned),
			media_ids: media_ids.to_vec(),
			created_at: fixed(1_788_699_586_221),
			updated_at: fixed(1_788_699_586_221),
		};
		self.playlists
			.lock()
			.entry(user.id.clone())
			.or_default()
			.push(playlist.clone());
		Ok(playlist)
	}

	async fn update_playlist(
		&self,
		user: &AuthUser,
		playlist_id: &str,
		name: Option<&str>,
		description: Option<Option<&str>>,
		media_ids: Option<&[String]>,
	) -> AbsResult<AbsPlaylist> {
		let mut playlists = self.playlists.lock();
		let row = playlists
			.get_mut(&user.id)
			.and_then(|rows| rows.iter_mut().find(|row| row.id == playlist_id))
			.ok_or_else(|| AbsError::NotFound(format!("No playlist {playlist_id}")))?;
		if let Some(name) = name {
			row.name = name.to_owned();
		}
		if let Some(description) = description {
			row.description = description.map(str::to_owned);
		}
		if let Some(media_ids) = media_ids {
			row.media_ids = media_ids.to_vec();
		}
		row.updated_at = fixed(1_788_699_600_000);
		Ok(row.clone())
	}

	async fn delete_playlist(&self, user: &AuthUser, playlist_id: &str) -> AbsResult<()> {
		if let Some(rows) = self.playlists.lock().get_mut(&user.id) {
			rows.retain(|row| row.id != playlist_id);
		}
		Ok(())
	}
}

/// The composed router, mounted the way the server mounts it: the public
/// routes and the socket endpoint at the root, the authenticated ones under
/// `/api`, with the user already resolved.
pub(crate) fn router_with_events(
	backend: Arc<TestBackend>,
	user: &AuthUser,
	events: crate::socket::AbsEvents,
) -> Router {
	let auth = stump_auth::AuthContext {
		user: user.clone(),
		api_key: None,
		device_id: backend.device_id.lock().clone(),
	};
	crate::routes::public_router::<()>()
		.merge(crate::socket::router::<()>())
		.merge(Router::new().nest("/api", crate::routes::authenticated_router::<()>()))
		.layer(Extension(user.clone()))
		.layer(Extension(auth))
		.layer(Extension(events))
		.layer(Extension(backend as Arc<dyn AbsBackend>))
}

/// The same router with **no** user resolved: what the server serves on the
/// public prefix, where the auth middleware never ran.
pub(crate) fn router_anonymous(backend: Arc<TestBackend>) -> Router {
	crate::routes::public_router::<()>()
		.merge(Router::new().nest("/api", crate::routes::authenticated_router::<()>()))
		.layer(Extension(crate::socket::AbsEvents::new()))
		.layer(Extension(backend as Arc<dyn AbsBackend>))
}

/// Drive one anonymous request through the router.
pub(crate) async fn request_anonymous(
	backend: Arc<TestBackend>,
	method: &str,
	uri: &str,
	headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, Vec<u8>) {
	let app = router_anonymous(backend);
	let mut builder = axum::http::Request::builder().method(method).uri(uri);
	for (name, value) in headers {
		builder = builder.header(*name, *value);
	}
	let response = app
		.oneshot(builder.body(Body::empty()).unwrap())
		.await
		.expect("router response");
	let status = response.status();
	let headers = response.headers().clone();
	let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
		.await
		.expect("response body");
	(status, headers, bytes.to_vec())
}

/// Drive one request through the router and decode the JSON body. Going
/// through the router (rather than calling a handler) is what exercises route
/// registration, query parsing and the serialised DTOs.
pub(crate) async fn request(
	backend: Arc<TestBackend>,
	user: &AuthUser,
	method: &str,
	uri: &str,
	body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
	let (status, _, value) = request_full(backend, user, method, uri, body, &[]).await;
	(status, value)
}

/// The same, keeping the response headers and accepting request headers — for
/// the range and token-header cases.
pub(crate) async fn request_full(
	backend: Arc<TestBackend>,
	user: &AuthUser,
	method: &str,
	uri: &str,
	body: Option<serde_json::Value>,
	headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, serde_json::Value) {
	let events = backend
		.events
		.lock()
		.clone()
		.unwrap_or_else(crate::socket::AbsEvents::new);
	let app = router_with_events(backend, user, events);
	let mut builder = axum::http::Request::builder().method(method).uri(uri);
	for (name, value) in headers {
		builder = builder.header(*name, *value);
	}
	let request = match body {
		Some(body) => builder
			.header(axum::http::header::CONTENT_TYPE, "application/json")
			.body(Body::from(serde_json::to_vec(&body).unwrap()))
			.unwrap(),
		None => builder.body(Body::empty()).unwrap(),
	};

	let response = app.oneshot(request).await.expect("router response");
	let status = response.status();
	let headers = response.headers().clone();
	let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
		.await
		.expect("response body");
	let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
	(status, headers, value)
}

/// A bearer token the profile itself would mint, for the token-shape tests.
pub(crate) fn access_token(user: &AuthUser) -> String {
	mint_tokens(SECRET, &user.id, &user.username, None)
		.unwrap()
		.access_token
}
