//! The liseur-sync attachment lane: `PUT`/`GET
//! /v1/annotations/{id}/attachments[/{kind}]` and the authenticated download
//! at `GET /api/v2/annotations/{id}/attachments/{attachment_id}`.
//!
//! The suite runs against the real migrated schema because the liseur-sync
//! tables have no SeaORM entities, and against a throwaway config directory
//! because attachments write real files.

use std::path::PathBuf;

use axum::http::{Method, StatusCode};
use axum_test::TestResponse;
use migrations::{Migrator, MigratorTrait};
use models::{entity::user, services::annotation_attachment as attachment_rows};
use sea_orm::{Database, DatabaseConnection, EntityTrait};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use stump_core::config::StumpConfig;
use tempfile::TempDir;

use crate::common::TestApp;

const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M1 1"/></svg>"#;
const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46];
const ANNOTATION_ID: &str = "ks1-0123456789abcdef";
/// Small enough to exercise `413` with a body the harness can build.
const MAX_BYTES: usize = 1024;

fn digest(bytes: &[u8]) -> String {
	format!("{:x}", Sha256::digest(bytes))
}

async fn migrated_database() -> DatabaseConnection {
	let db = Database::connect("sqlite::memory:")
		.await
		.expect("failed to connect to sqlite");
	Migrator::up(&db, None)
		.await
		.expect("failed to run migrations");
	db
}

/// A server with a device token, a resolved work, and one live annotation.
struct Fixture {
	app: TestApp,
	config_dir: TempDir,
	device_secret: String,
}

impl Fixture {
	async fn new() -> Self {
		let config_dir = TempDir::new().expect("failed to create a config directory");
		let mut config = StumpConfig::debug();
		config.config_dir = config_dir.path().to_string_lossy().into_owned();
		config.protocols.attachment_max_bytes = MAX_BYTES;

		let app = TestApp::with_parts(migrated_database().await, config).await;
		let token = app.create_initial_account().await;
		*app.access_token.write().await = Some(token);

		let login = app
			.server
			.post("/v1/login")
			.json(&json!({
				"username": "initial-server-admin",
				"password": "password",
			}))
			.await;
		login.assert_status_ok();
		let login: Value = login.json();
		let session = login["auth_token"].as_str().unwrap().to_string();

		let minted = app
			.server
			.post("/v1/tokens")
			.add_header("Authorization", format!("Bearer {session}"))
			.json(&json!({ "name": "Kobo Elipsa", "scopes": ["sync", "library-read"] }))
			.await;
		minted.assert_status(StatusCode::CREATED);
		let minted: Value = minted.json();
		let device_secret = minted["secret"].as_str().unwrap().to_string();

		let fixture = Self {
			app,
			config_dir,
			device_secret,
		};

		let resolved = fixture
			.app
			.server
			.post("/v1/works/resolve")
			.add_header("Authorization", fixture.bearer())
			.json(&json!({
				"identifiers": [{ "kind": "sha256", "value": "a".repeat(64) }],
				"title": "Attachment Fixture",
				"confirmed": true,
			}))
			.await;
		let resolved: Value = resolved.json();
		let work_id = resolved["work_id"].as_str().unwrap().to_string();

		let pushed = fixture
			.app
			.server
			.post("/v1/annotations")
			.add_header("Authorization", fixture.bearer())
			.json(&json!({
				"annotations": [{
					"id": ANNOTATION_ID,
					"base_rev": 0,
					"work_id": work_id,
					"kind": "highlight",
					"locator": { "href": "ch01.xhtml" },
					"client_ts": "2026-01-01T00:00:00Z",
				}],
			}))
			.await;
		pushed.assert_status_ok();
		let pushed: Value = pushed.json();
		assert_eq!(pushed["results"][0]["status"], "applied", "{pushed:#}");

		fixture
	}

	fn bearer(&self) -> String {
		format!("Bearer {}", self.device_secret)
	}

	async fn upload(
		&self,
		annotation_id: &str,
		kind: &str,
		media_type: &str,
		bytes: &[u8],
		sha256: &str,
	) -> TestResponse {
		self.app
			.server
			.method(
				Method::PUT,
				&format!("/v1/annotations/{annotation_id}/attachments/{kind}"),
			)
			.add_header("Authorization", self.bearer())
			.add_header("x-attachment-sha256", sha256)
			.content_type(media_type)
			.bytes(bytes.to_vec().into())
			.await
	}

	async fn put_markup(&self) -> Value {
		let response = self
			.upload(
				ANNOTATION_ID,
				"markup-svg",
				"image/svg+xml",
				SVG,
				&digest(SVG),
			)
			.await;
		response.assert_status_ok();
		response.json()
	}

	async fn attachments(&self) -> TestResponse {
		self.app
			.server
			.get(&format!("/v1/annotations/{ANNOTATION_ID}/attachments"))
			.add_header("Authorization", self.bearer())
			.await
	}

	fn stored_path(&self, sha256: &str, extension: &str) -> PathBuf {
		self.config_dir
			.path()
			.join("attachments")
			.join(ANNOTATION_ID)
			.join(format!("{sha256}.{extension}"))
	}

	/// The single account this fixture creates.
	async fn owner_id(&self) -> String {
		user::Entity::find()
			.one(self.app.conn())
			.await
			.expect("the user table is readable")
			.expect("the fixture created an account")
			.id
	}
}

#[tokio::test]
async fn upload_is_idempotent_by_digest_and_never_moves_the_annotation_revision() {
	let fixture = Fixture::new().await;

	let first = fixture.put_markup().await;
	assert_eq!(first["sha256"], digest(SVG));
	assert_eq!(first["byte_size"], SVG.len());
	let attachment_id = first["id"].as_str().unwrap().to_string();
	assert!(fixture.stored_path(&digest(SVG), "svg").is_file());

	// A replayed batch must not accumulate rows or mint a second id.
	let second = fixture.put_markup().await;
	assert_eq!(second["id"], attachment_id.as_str());

	let page_digest = digest(JPEG);
	let page = fixture
		.upload(
			ANNOTATION_ID,
			"markup-page",
			"image/jpeg",
			JPEG,
			&page_digest,
		)
		.await;
	page.assert_status_ok();

	let listed = fixture.attachments().await;
	listed.assert_status_ok();
	let listed: Value = listed.json();
	let attachments = listed["attachments"].as_array().unwrap();
	assert_eq!(attachments.len(), 2, "{listed:#}");
	let svg = attachments
		.iter()
		.find(|entry| entry["kind"] == "markup-svg")
		.expect("the markup is listed");
	assert_eq!(svg["id"], attachment_id.as_str());
	assert_eq!(svg["media_type"], "image/svg+xml");
	assert_eq!(svg["byte_size"], SVG.len());
	assert_eq!(svg["sha256"], digest(SVG));
	assert_eq!(svg["annotation_id"], ANNOTATION_ID);

	// Attachments are side objects: the CAS record is untouched by all of it.
	let live = fixture
		.app
		.server
		.get("/v1/annotations/changes?since=0")
		.add_header("Authorization", fixture.bearer())
		.await;
	live.assert_status_ok();
	let live: Value = live.json();
	let record = live["annotations"]
		.as_array()
		.unwrap()
		.iter()
		.find(|entry| entry["id"] == ANNOTATION_ID)
		.expect("the annotation is in the feed");
	assert_eq!(record["rev"], 1, "attachments must not bump the revision");
}

#[tokio::test]
async fn a_new_digest_replaces_the_slot_and_unlinks_the_previous_bytes() {
	let fixture = Fixture::new().await;
	let first = fixture.put_markup().await;

	let edited = br#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M2 2"/></svg>"#;
	let replaced = fixture
		.upload(
			ANNOTATION_ID,
			"markup-svg",
			"image/svg+xml",
			edited,
			&digest(edited),
		)
		.await;
	replaced.assert_status_ok();
	let replaced: Value = replaced.json();
	assert_eq!(replaced["id"], first["id"], "the slot keeps its identity");
	assert_eq!(replaced["sha256"], digest(edited));

	assert!(fixture.stored_path(&digest(edited), "svg").is_file());
	assert!(
		!fixture.stored_path(&digest(SVG), "svg").exists(),
		"the superseded bytes must be unlinked"
	);

	let listed: Value = fixture.attachments().await.json();
	assert_eq!(listed["attachments"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn download_serves_the_bytes_to_the_owner_by_session_or_device() {
	let fixture = Fixture::new().await;
	let uploaded = fixture.put_markup().await;
	let attachment_id = uploaded["id"].as_str().unwrap();
	let path = format!("/api/v2/annotations/{ANNOTATION_ID}/attachments/{attachment_id}");

	let by_device = fixture
		.app
		.server
		.get(&path)
		.add_header("Authorization", fixture.bearer())
		.await;
	by_device.assert_status_ok();
	assert_eq!(by_device.as_bytes().as_ref(), SVG);
	assert_eq!(
		by_device
			.headers()
			.get("content-type")
			.and_then(|value| value.to_str().ok()),
		Some("image/svg+xml")
	);

	// The web UI arrives with the Stump access token instead.
	let by_session = fixture.app.get(&path).await;
	by_session.assert_status_ok();
	assert_eq!(by_session.as_bytes().as_ref(), SVG);

	let unknown = fixture
		.app
		.get(&format!(
			"/api/v2/annotations/{ANNOTATION_ID}/attachments/does-not-exist"
		))
		.await;
	unknown.assert_status(StatusCode::NOT_FOUND);

	let unauthenticated = fixture.app.server.get(&path).await;
	unauthenticated.assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn uploads_are_rejected_on_digest_mismatch_size_kind_media_type_and_identity() {
	let fixture = Fixture::new().await;

	let mismatch = fixture
		.upload(
			ANNOTATION_ID,
			"markup-svg",
			"image/svg+xml",
			SVG,
			&digest(JPEG),
		)
		.await;
	mismatch.assert_status(StatusCode::CONFLICT);

	let oversized = vec![b'x'; MAX_BYTES + 1];
	let too_large = fixture
		.upload(
			ANNOTATION_ID,
			"markup-page",
			"image/jpeg",
			&oversized,
			&digest(&oversized),
		)
		.await;
	too_large.assert_status(StatusCode::PAYLOAD_TOO_LARGE);

	let unknown_kind = fixture
		.upload(
			ANNOTATION_ID,
			"scribble",
			"image/svg+xml",
			SVG,
			&digest(SVG),
		)
		.await;
	unknown_kind.assert_status(StatusCode::BAD_REQUEST);

	let wrong_media_type = fixture
		.upload(ANNOTATION_ID, "markup-svg", "image/jpeg", SVG, &digest(SVG))
		.await;
	wrong_media_type.assert_status(StatusCode::BAD_REQUEST);

	let unknown_annotation = fixture
		.upload(
			"ks1-nothing-here",
			"markup-svg",
			"image/svg+xml",
			SVG,
			&digest(SVG),
		)
		.await;
	unknown_annotation.assert_status(StatusCode::NOT_FOUND);

	// An id that is not a safe path segment cannot name a storage directory.
	// The separator arrives percent-encoded, so routing hands the handler a
	// single segment that decodes to `ks1/escape`.
	let traversing = fixture
		.upload(
			"ks1%2Fescape",
			"markup-svg",
			"image/svg+xml",
			SVG,
			&digest(SVG),
		)
		.await;
	traversing.assert_status(StatusCode::BAD_REQUEST);

	let unauthenticated = fixture
		.app
		.server
		.method(
			Method::PUT,
			&format!("/v1/annotations/{ANNOTATION_ID}/attachments/markup-svg"),
		)
		.add_header("x-attachment-sha256", digest(SVG))
		.content_type("image/svg+xml")
		.bytes(SVG.to_vec().into())
		.await;
	unauthenticated.assert_status(StatusCode::UNAUTHORIZED);

	// None of the refusals may leave a row or a file behind.
	let listed: Value = fixture.attachments().await.json();
	assert_eq!(listed["attachments"].as_array().unwrap().len(), 0);
	assert!(!fixture.config_dir.path().join("attachments").exists());
}

#[tokio::test]
async fn tombstoning_the_annotation_cascades_rows_and_files() {
	let fixture = Fixture::new().await;
	let uploaded = fixture.put_markup().await;
	let attachment_id = uploaded["id"].as_str().unwrap().to_string();
	let stored = fixture.stored_path(&digest(SVG), "svg");
	assert!(stored.is_file());

	let deleted = fixture
		.app
		.server
		.delete(&format!("/v1/annotations/{ANNOTATION_ID}?rev=1"))
		.add_header("Authorization", fixture.bearer())
		.await;
	deleted.assert_status_ok();
	let deleted: Value = deleted.json();
	assert_eq!(deleted["status"], "applied");

	assert!(!stored.exists(), "the attachment bytes must be unlinked");
	let rows = attachment_rows::list(
		fixture.app.conn(),
		&fixture.owner_id().await,
		ANNOTATION_ID,
	)
	.await
	.expect("attachment rows are readable");
	assert!(rows.is_empty(), "attachment rows must be gone");

	// The lane reports the annotation, not an empty attachment list.
	fixture
		.attachments()
		.await
		.assert_status(StatusCode::NOT_FOUND);
	fixture
		.app
		.get(&format!(
			"/api/v2/annotations/{ANNOTATION_ID}/attachments/{attachment_id}"
		))
		.await
		.assert_status(StatusCode::NOT_FOUND);
}
