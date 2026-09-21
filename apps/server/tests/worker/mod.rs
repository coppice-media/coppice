//! The server's half of the worker lane: who may open the socket, and who may
//! upload a job's bytes.
//!
//! The protocol itself is covered in `cargo test -p stump_worker`, against a
//! real listener and a real client. What can only be asserted *here* is the
//! part the crate deliberately does not own: the auth rule. A worker
//! authenticates with a device credential like every other client, so the
//! risk is not the frames — it is that some other credential (a session, a
//! Kobo's key, no credential at all) reaches a route that hands out other
//! users' books as job inputs and takes bytes into the delivery cache.
//!
//! The suite runs against the real migrated schema: `worker_jobs` has no
//! SeaORM entity in `models`, so the entity-built schema of
//! `tests::db::test_database` does not carry the table.

use axum::http::StatusCode;
use migrations::{Migrator, MigratorTrait};
use models::{entity::user::AuthUser, shared::enums::DeviceKind};
use sea_orm::{Database, DatabaseConnection, EntityTrait};
use sha2::{Digest, Sha256};
use stump_core::config::StumpConfig;
use stump_worker::{kind::TranscodeOutput, TranscodeInput, TRANSCODE};
use tempfile::TempDir;

use crate::common::TestApp;

const SOCKET: &str = "/api/v2/workers/socket";

/// The headers that make a `GET` a WebSocket upgrade.
///
/// They are required for the auth rule to be reachable at all: axum's
/// `WebSocketUpgrade` extractor runs before the handler body and rejects a
/// plain `GET` with `400` on its own, so a test that sent one would assert the
/// extractor rather than the credential check.
fn upgrade(request: axum_test::TestRequest) -> axum_test::TestRequest {
	request
		.add_header("Connection", "Upgrade")
		.add_header("Upgrade", "websocket")
		.add_header("Sec-WebSocket-Version", "13")
		.add_header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
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

struct Fixture {
	app: TestApp,
	_config_dir: TempDir,
}

impl Fixture {
	async fn new() -> Self {
		let config_dir = TempDir::new().expect("config dir");
		let mut config = StumpConfig::debug();
		config.config_dir = config_dir.path().to_string_lossy().into_owned();

		let app = TestApp::with_parts(migrated_database().await, config).await;
		let token = app.create_initial_account().await;
		*app.access_token.write().await = Some(token);

		Self {
			app,
			_config_dir: config_dir,
		}
	}

	async fn owner(&self) -> AuthUser {
		let owner = models::entity::user::Entity::find()
			.one(self.app.conn())
			.await
			.expect("user query")
			.expect("the default user exists");
		AuthUser {
			id: owner.id,
			username: owner.username,
			is_server_owner: true,
			..Default::default()
		}
	}

	/// Register a device of `kind` and return `(device_id, api_key)`.
	async fn device(&self, kind: DeviceKind, name: &str) -> (String, String) {
		let owner = self.owner().await;
		let (device, issued) = self
			.app
			.ctx
			.devices()
			.create_device(
				&owner,
				stump_devices::CredentialIssuance::InteractiveSession,
				kind,
				Some(name.to_string()),
			)
			.await
			.expect("device");
		(device.id, issued.secret)
	}
}

fn digest(bytes: &[u8]) -> String {
	format!("{:x}", Sha256::digest(bytes))
}

/// The socket refuses everything that is not a live worker device.
///
/// Each case is a different way in, and each would be a different hole: a
/// session is the browser, a bare token is the user, another device kind is
/// any paired reader, and a revoked device is a credential the operator
/// already withdrew.
#[tokio::test]
async fn the_worker_socket_requires_a_live_worker_device_credential() {
	let fixture = Fixture::new().await;

	// The session/bearer the harness authenticates with is not a device.
	let no_device = upgrade(fixture.app.server.get(SOCKET))
		.add_header("Authorization", fixture.app.auth_header().await)
		.await;
	assert_eq!(
		no_device.status_code(),
		StatusCode::FORBIDDEN,
		"a request with no device credential must not reach the worker lane"
	);

	// A reader's key is a device, but the wrong kind.
	let (_, reader_key) = fixture.device(DeviceKind::Api, "a script").await;
	let wrong_kind = upgrade(fixture.app.server.get(SOCKET))
		.add_header("Authorization", format!("Bearer {reader_key}"))
		.await;
	assert_eq!(
		wrong_kind.status_code(),
		StatusCode::FORBIDDEN,
		"a non-worker device must not be able to claim worker jobs"
	);

	// The right kind gets past the credential check. It cannot get further
	// here: `axum_test` drives the router directly, so hyper never installs
	// the `OnUpgrade` extension and the extractor answers `426`. That is the
	// proof the credential was accepted — the real handshake is exercised over
	// a real listener in `cargo test -p stump_worker`.
	let (_, worker_key) = fixture.device(DeviceKind::Worker, "gpu box").await;
	let worker = upgrade(fixture.app.server.get(SOCKET))
		.add_header("Authorization", format!("Bearer {worker_key}"))
		.await;
	assert_eq!(
		worker.status_code(),
		StatusCode::UPGRADE_REQUIRED,
		"a worker device's credential must pass the auth rule"
	);

	// A revoked worker is a worker no more.
	let (revoked_id, revoked_key) =
		fixture.device(DeviceKind::Worker, "retired box").await;
	let owner = fixture.owner().await;
	fixture
		.app
		.ctx
		.devices()
		.revoke(&owner, &revoked_id)
		.await
		.expect("revoke");
	let revoked = upgrade(fixture.app.server.get(SOCKET))
		.add_header("Authorization", format!("Bearer {revoked_key}"))
		.await;
	// `401`, not `403`: revocation *deletes* the credential rows, so the key
	// no longer authenticates at all and never reaches the worker rule. That
	// is the stronger of the two answers, and it is why the socket needs no
	// revocation check of its own.
	assert_eq!(
		revoked.status_code(),
		StatusCode::UNAUTHORIZED,
		"a revoked worker must not be able to reconnect"
	);
}

/// The upload route is the one place a remote process writes bytes the server
/// will later serve, so it is guarded three ways: the caller must be a worker,
/// it must hold *this* job, and the bytes must match the digest it declared.
#[tokio::test]
async fn the_output_upload_is_guarded_by_assignment_and_digest() {
	let fixture = Fixture::new().await;
	let (worker_id, worker_key) = fixture.device(DeviceKind::Worker, "gpu box").await;
	let (_, other_id_key) = fixture.device(DeviceKind::Worker, "other box").await;

	let jobs = fixture.app.ctx.worker_jobs();
	let input = serde_json::to_value(TranscodeInput {
		media_id: "m1".into(),
		track_index: 0,
		duration_ms: 1000,
		output: TranscodeOutput::Opus {
			bitrate: "64k".into(),
		},
	})
	.expect("input");

	// No worker is connected and this build has no local ffmpeg guarantee, so
	// the row is parked; assignment is set by hand, which is exactly the state
	// a claimed job is in.
	let job = jobs
		.enqueue(TRANSCODE, input, stump_worker::transcode_requires(), 0)
		.await
		.expect("enqueue");
	assign(&fixture, &job.id, &worker_id).await;

	let body = b"transcoded-bytes".to_vec();
	let url = format!("/api/v2/workers/jobs/{}/output", job.id);

	// A worker that does not hold the job cannot substitute its bytes.
	let stolen = fixture
		.app
		.server
		.put(&url)
		.add_header("Authorization", format!("Bearer {other_id_key}"))
		.add_header("x-stump-sha256", digest(&body))
		.bytes(body.clone().into())
		.await;
	assert_eq!(stolen.status_code(), StatusCode::FORBIDDEN);

	// A digest that does not match the body is refused: these bytes go
	// straight into the delivery cache, and a truncated upload would be served
	// to every later request as a valid entry.
	let mismatched = fixture
		.app
		.server
		.put(&url)
		.add_header("Authorization", format!("Bearer {worker_key}"))
		.add_header("x-stump-sha256", digest(b"something-else"))
		.bytes(body.clone().into())
		.await;
	assert_eq!(mismatched.status_code(), StatusCode::BAD_REQUEST);

	// And a missing digest is refused rather than trusted.
	let undeclared = fixture
		.app
		.server
		.put(&url)
		.add_header("Authorization", format!("Bearer {worker_key}"))
		.bytes(body.clone().into())
		.await;
	assert_eq!(undeclared.status_code(), StatusCode::BAD_REQUEST);

	// The holder, with a correct digest, lands the bytes where the awaiting
	// caller will publish them from.
	let accepted = fixture
		.app
		.server
		.put(&url)
		.add_header("Authorization", format!("Bearer {worker_key}"))
		.add_header("x-stump-sha256", digest(&body))
		.bytes(body.clone().into())
		.await;
	accepted.assert_status_ok();
	assert_eq!(
		tokio::fs::read(jobs.output_path(&job.id))
			.await
			.expect("the output is on disk"),
		body
	);

	// An unknown job is a 404, not a 403: it cannot leak whether some other
	// worker holds an id the caller guessed, because the id does not exist.
	let unknown = fixture
		.app
		.server
		.put("/api/v2/workers/jobs/does-not-exist/output")
		.add_header("Authorization", format!("Bearer {worker_key}"))
		.add_header("x-stump-sha256", digest(&body))
		.bytes(body.into())
		.await;
	assert_eq!(unknown.status_code(), StatusCode::NOT_FOUND);
}

/// Set `worker_id` on a job without a socket, which is the state a claimed job
/// is in. Written through the entity rather than a frame because the frame path
/// is the crate's test; here the point is only what the *route* does with an
/// assignment.
async fn assign(fixture: &Fixture, job_id: &str, worker_id: &str) {
	use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
	use stump_worker::entity;

	entity::Entity::update_many()
		.filter(entity::Column::Id.eq(job_id))
		.set(entity::ActiveModel {
			worker_id: Set(Some(worker_id.to_string())),
			status: Set(stump_worker::WorkerJobStatus::Claimed),
			..Default::default()
		})
		.exec(fixture.app.conn())
		.await
		.expect("assign");
}
