//! The audiobook playback lane: `GET /api/v2/media/{id}/audio/manifest` and
//! `GET /api/v2/media/{id}/audio/track/{index}`.
//!
//! The manifest is the contract every player builds against, so the assertions
//! are on the exact camelCase wire shape rather than on the entities. The
//! track route exists to be seeked, so the `Range` behaviour is asserted
//! rather than assumed: a route that ignored `Range` would force a full
//! download per seek and no client would notice until playback.

use axum::http::{header, StatusCode};
use models::services::audio::{AudioFacts, ChapterFacts, TrackFacts};
use serde_json::Value;
use tempfile::TempDir;
use tests::fake_data;

use crate::common::{series::setup_single_series_with_n_books, TestApp};

/// The bytes each fixture track is written with; short enough that a `Range`
/// assertion can name exact offsets.
const TRACK_BYTES: &[&[u8]] = &[b"first-track-bytes", b"second-track-bytes"];

struct Fixture {
	app: TestApp,
	book_id: String,
	/// Held so the track files outlive the test.
	_dir: TempDir,
}

impl Fixture {
	/// The on-disk path of one fixture track, for a test that needs to
	/// replace its bytes.
	fn track_path(&self, index: usize) -> std::path::PathBuf {
		self._dir.path().join(format!("{index:02}.mp3"))
	}
}

/// A two-track audiobook whose track files really exist on disk, because the
/// track route serves real bytes through `ServeFile`.
async fn audiobook() -> Fixture {
	let app = TestApp::new_with_default_user().await;
	let (_series, books) =
		setup_single_series_with_n_books(&app, fake_data::Series::default(), 1).await;
	let book_id = books[0].id.clone();

	let dir = TempDir::new().expect("failed to create temp dir");
	let mut tracks = Vec::new();
	for (index, bytes) in TRACK_BYTES.iter().enumerate() {
		let path = dir.path().join(format!("{index:02}.mp3"));
		std::fs::write(&path, bytes).expect("failed to write track");
		tracks.push(TrackFacts {
			path: path.to_string_lossy().to_string(),
			duration_ms: 2_000 + index as i64,
			byte_size: bytes.len() as i64,
			mime: "audio/mpeg".to_string(),
		});
	}

	let facts = AudioFacts {
		duration_ms: tracks.iter().map(|track| track.duration_ms).sum(),
		codec: "mp3".to_string(),
		sample_rate: Some(22_050),
		channels: Some(1),
		bitrate: Some(64_000),
		chapter_source: models::domain::audio::AudioChapterSource::PerTrack,
		tracks,
		chapters: vec![
			ChapterFacts {
				title: Some("Opening".to_string()),
				start_ms: 0,
				end_ms: Some(2_000),
			},
			ChapterFacts {
				title: Some("Closing".to_string()),
				start_ms: 2_000,
				end_ms: Some(4_001),
			},
		],
	};

	models::services::audio::replace(app.conn(), &book_id, &facts)
		.await
		.expect("failed to persist audio facts");

	Fixture {
		app,
		book_id,
		_dir: dir,
	}
}

/// The manifest is the player's contract: the camelCase field names, the
/// publication-relative offsets, the running `startOffsetMs` sum and the
/// per-track URL all have to be exactly right or a client cannot seek.
#[tokio::test]
async fn audio_manifest_describes_the_publication() {
	let fixture = audiobook().await;
	let response = fixture
		.app
		.get(&format!("/api/v2/media/{}/audio/manifest", fixture.book_id))
		.await;
	response.assert_status_ok();
	let body: Value = response.json();

	assert_eq!(body["mediaId"], fixture.book_id.as_str());
	assert_eq!(body["durationMs"], 4_001);
	assert_eq!(body["codec"], "mp3");
	assert_eq!(body["sampleRate"], 22_050);
	assert_eq!(body["channels"], 1);
	assert_eq!(body["bitrate"], 64_000);
	// Provenance, not a preference: a client that hides synthesized chapters
	// branches on exactly this value.
	assert_eq!(body["chapterSource"], "per_track");

	let tracks = body["tracks"].as_array().expect("tracks must be an array");
	assert_eq!(tracks.len(), 2);
	assert_eq!(tracks[0]["index"], 0);
	assert_eq!(tracks[0]["mime"], "audio/mpeg");
	assert_eq!(tracks[0]["durationMs"], 2_000);
	assert_eq!(tracks[0]["startOffsetMs"], 0);
	assert_eq!(tracks[0]["byteSize"], TRACK_BYTES[0].len());
	assert_eq!(
		tracks[0]["url"],
		format!("/api/v2/media/{}/audio/track/0", fixture.book_id)
	);
	// The running sum is what maps a publication offset onto a file.
	assert_eq!(tracks[1]["index"], 1);
	assert_eq!(tracks[1]["startOffsetMs"], 2_000);

	let chapters = body["chapters"]
		.as_array()
		.expect("chapters must be an array");
	assert_eq!(chapters.len(), 2);
	assert_eq!(chapters[0]["index"], 0);
	assert_eq!(chapters[0]["title"], "Opening");
	assert_eq!(chapters[0]["startMs"], 0);
	assert_eq!(chapters[0]["endMs"], 2_000);
	assert_eq!(chapters[1]["startMs"], 2_000);
}

/// A book that is not an audiobook must be distinguishable from an audiobook
/// with no tracks, so it is a 404 rather than an empty manifest.
#[tokio::test]
async fn audio_manifest_is_not_found_for_a_non_audio_book() {
	let app = TestApp::new_with_default_user().await;
	let (_series, books) =
		setup_single_series_with_n_books(&app, fake_data::Series::default(), 1).await;

	let response = app
		.get(&format!("/api/v2/media/{}/audio/manifest", books[0].id))
		.await;
	assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}

/// A player seeks by byte range. The route must answer a full request with
/// `200` + `Accept-Ranges: bytes` and a ranged request with `206` and only the
/// requested bytes, and it must report the stored MIME rather than a guess
/// from the extension so the manifest and the bytes never disagree.
#[tokio::test]
async fn audio_track_serves_bytes_and_honours_range() {
	let fixture = audiobook().await;
	let url = format!("/api/v2/media/{}/audio/track/1", fixture.book_id);

	let response = fixture.app.get(&url).await;
	response.assert_status_ok();
	assert_eq!(
		response.headers().get(header::ACCEPT_RANGES).unwrap(),
		"bytes"
	);
	assert_eq!(
		response.headers().get(header::CONTENT_TYPE).unwrap(),
		"audio/mpeg"
	);
	assert_eq!(response.as_bytes().as_ref(), TRACK_BYTES[1]);

	let ranged = fixture.app.get_with_range(&url, "bytes=0-4").await;
	assert_eq!(ranged.status_code(), StatusCode::PARTIAL_CONTENT);
	assert_eq!(ranged.as_bytes().as_ref(), &TRACK_BYTES[1][..5]);
}

/// A track index the publication does not have is a 404: an out-of-range
/// index is a client bug, and serving track 0 instead would silently play the
/// wrong part of the book.
#[tokio::test]
async fn audio_track_is_not_found_for_an_unknown_index() {
	let fixture = audiobook().await;

	let response = fixture
		.app
		.get(&format!("/api/v2/media/{}/audio/track/9", fixture.book_id))
		.await;
	assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}

/// The `ffmpeg` the Opus lane shells out to, when the host has one.
///
/// Delivery degrades to the stored bytes without it, so its absence changes
/// what this test can assert but never whether it runs.
fn ffmpeg_path() -> Option<std::path::PathBuf> {
	let path = std::env::var_os("PATH")?;
	std::env::split_paths(&path)
		.map(|dir| dir.join("ffmpeg"))
		.find(|candidate| candidate.is_file())
}

/// Register a device carrying the `phone-opus` preset and return the secret
/// its credential authenticates with, so a request bearing that key resolves
/// to it.
///
/// The registry mints the credential rather than the test inserting one: the
/// key format, the hash and the credential row are what the auth middleware
/// looks the device up by, and a hand-built row would authenticate a device
/// the real registry would not.
async fn opus_device(app: &TestApp) -> String {
	use models::{entity::user::AuthUser, shared::enums::DeviceKind};
	use sea_orm::EntityTrait;

	let owner = models::entity::user::Entity::find()
		.one(app.conn())
		.await
		.expect("user query")
		.expect("the default user exists");
	let owner = AuthUser {
		id: owner.id.clone(),
		username: owner.username.clone(),
		is_server_owner: true,
		..Default::default()
	};

	let devices = app.ctx.devices();
	let (device, issued) = devices
		.create_device(&owner, DeviceKind::Api, Some("Opus phone".to_string()))
		.await
		.expect("device");
	devices
		.set_transform_profile(
			&owner,
			&device.id,
			Some(serde_json::json!({ "preset": "phone-opus" })),
		)
		.await
		.expect("preset");

	issued.secret
}

/// The per-device audio preset is negotiated on the track route: a plain
/// request gets the stored encoding, and a request authenticated by a device
/// carrying an Opus preset gets Opus.
///
/// Both halves matter. The stored-MIME half is the default every existing
/// client depends on — a regression there would silently re-encode every
/// download — and the Opus half is the whole point of the preset. Without
/// `ffmpeg` the transcode cannot run, and the documented behaviour is exactly
/// the fallback the plain request gets, so the assertion follows the host.
#[tokio::test]
async fn audio_track_negotiates_the_requesting_devices_preset() {
	let fixture = audiobook().await;
	let url = format!("/api/v2/media/{}/audio/track/1", fixture.book_id);

	// A session/token request carries no device, so nothing is negotiated.
	let plain = fixture.app.get(&url).await;
	plain.assert_status_ok();
	assert_eq!(
		plain.headers().get(header::CONTENT_TYPE).unwrap(),
		"audio/mpeg",
		"a request with no device must get the stored encoding"
	);

	let api_key = opus_device(&fixture.app).await;
	let ffmpeg = ffmpeg_path();
	if let Some(ffmpeg) = &ffmpeg {
		// The fixture's placeholder bytes are not decodable audio; the
		// transcode needs a real stream to produce one.
		let track = fixture.track_path(1);
		let built = std::process::Command::new(ffmpeg)
			.args([
				"-hide_banner",
				"-loglevel",
				"error",
				"-y",
				"-f",
				"lavfi",
				"-i",
				"sine=frequency=440:duration=1",
				"-c:a",
				"libmp3lame",
			])
			.arg(&track)
			.status()
			.expect("ffmpeg run");
		assert!(built.success(), "failed to build the MP3 fixture");
	}

	let negotiated = fixture
		.app
		.server
		.get(&url)
		.add_header("Authorization", format!("Bearer {api_key}"))
		.await;
	negotiated.assert_status_ok();
	let content_type = negotiated
		.headers()
		.get(header::CONTENT_TYPE)
		.expect("content type")
		.to_str()
		.expect("ascii content type")
		.to_string();

	if ffmpeg.is_some() {
		assert_eq!(
			content_type, "audio/ogg",
			"a device with an Opus preset must be served Opus, not the stored MP3"
		);
		assert!(
			!negotiated.as_bytes().is_empty(),
			"the transcode must have a body"
		);
		// `Range` still works on the transcode: a player seeks in it too.
		let ranged = fixture
			.app
			.server
			.get(&url)
			.add_header("Authorization", format!("Bearer {api_key}"))
			.add_header("Range", "bytes=0-9")
			.await;
		assert_eq!(ranged.status_code(), StatusCode::PARTIAL_CONTENT);
	} else {
		assert_eq!(
			content_type, "audio/mpeg",
			"without ffmpeg the Opus preset must degrade to the stored bytes"
		);
	}
}

/// Both routes are user-scoped like every other media route: a book the user
/// cannot see is a 404, never a 403 that confirms it exists.
#[tokio::test]
async fn audio_routes_are_scoped_to_visible_books() {
	let fixture = audiobook().await;

	for path in [
		format!("/api/v2/media/does-not-exist/audio/manifest"),
		format!("/api/v2/media/does-not-exist/audio/track/0"),
	] {
		let response = fixture.app.get(&path).await;
		assert_eq!(
			response.status_code(),
			StatusCode::NOT_FOUND,
			"{path} must not leak"
		);
	}
}

/// GraphQL exposes the same shape, ungated, so a headless build can answer it.
#[tokio::test]
async fn audio_is_exposed_on_the_graphql_media_object() {
	let fixture = audiobook().await;

	let result = fixture
		.app
		.execute_gql(
			r#"
			query BookAudio($id: ID!) {
				mediaById(id: $id) {
					audio {
						durationMs
						codec
						chapterSource
						manifestUrl
						tracks { index startOffsetMs url }
						chapters { index title startMs }
					}
				}
			}
			"#,
			Some(serde_json::json!({ "id": fixture.book_id })),
		)
		.await;

	let audio = &result["data"]["mediaById"]["audio"];
	assert!(!audio.is_null(), "expected audio, got {result:?}");
	assert_eq!(audio["durationMs"], 4_001);
	assert_eq!(audio["codec"], "mp3");
	assert_eq!(audio["chapterSource"], "PER_TRACK");
	assert_eq!(
		audio["manifestUrl"],
		format!("/api/v2/media/{}/audio/manifest", fixture.book_id)
	);
	assert_eq!(audio["tracks"].as_array().unwrap().len(), 2);
	assert_eq!(audio["tracks"][1]["startOffsetMs"], 2_000);
	assert_eq!(audio["chapters"][0]["title"], "Opening");
}

/// Liseur reads paginated documents: an audiobook listed in its catalogue is
/// a book it can open and never track, and a folder book's "download" is a
/// directory - the harness saw `ServeFile` answer that with a truncated 200.
/// The ebook profile must not see audio rows at all.
#[tokio::test]
async fn audiobooks_are_not_liseur_catalogue_books() {
	use migrations::{Migrator, MigratorTrait};
	use sea_orm::{ActiveModelTrait, ActiveValue::Set, IntoActiveModel};

	// The liseur token tables have no entities, so this runs on the real schema.
	let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
	Migrator::up(&db, None).await.unwrap();
	let app = TestApp::with_parts(db, stump_core::config::StumpConfig::debug()).await;
	let token = app.create_initial_account().await;
	*app.access_token.write().await = Some(token);
	let library = fake_data::Library {
		name: Some("Shelf".to_string()),
		..Default::default()
	}
	.insert(app.conn())
	.await;
	let (_series, books) = setup_single_series_with_n_books(
		&app,
		fake_data::Series {
			library_id: Some(library.id.clone()),
			..Default::default()
		},
		2,
	)
	.await;
	let mut audiobook = books[0].clone().into_active_model();
	audiobook.extension = Set("m4b".to_string());
	audiobook.pages = Set(-1);
	audiobook.update(app.conn()).await.unwrap();
	let ebook_id = books[1].id.clone();
	let audiobook_id = books[0].id.clone();

	let login: Value = app
		.server
		.post("/v1/login")
		.json(
			&serde_json::json!({ "username": "initial-server-admin", "password": "password" }),
		)
		.await
		.json();
	let session = format!("Bearer {}", login["auth_token"].as_str().unwrap());
	let minted: Value = app
		.server
		.post("/v1/tokens")
		.add_header("Authorization", session)
		.json(&serde_json::json!({ "name": "probe", "scopes": ["sync", "library-read"] }))
		.await
		.json();
	let bearer = format!("Bearer {}", minted["secret"].as_str().unwrap());
	let folders: Value = app
		.server
		.get("/v1/folders")
		.add_header("Authorization", bearer.clone())
		.await
		.json();
	let folder_id = folders["folders"][0]["folder_id"]
		.as_str()
		.unwrap_or_else(|| panic!("{folders:?}"));

	let listed: Value = app
		.server
		.get(&format!("/v1/folders/{folder_id}/books?limit=10"))
		.add_header("Authorization", bearer.clone())
		.await
		.json();
	let ids = listed["books"]
		.as_array()
		.unwrap()
		.iter()
		.map(|book| book["book_id"].as_str().unwrap().to_string())
		.collect::<Vec<_>>();
	assert_eq!(
		ids,
		vec![ebook_id],
		"only the paginated book is a catalogue book"
	);

	app.server
		.get(&format!("/v1/books/{audiobook_id}/download"))
		.add_header("Authorization", bearer)
		.await
		.assert_status(StatusCode::NOT_FOUND);
}
