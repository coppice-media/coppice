//! Server-side worker authorization: initial upgrade, live revocation, and
//! guarded uploads. The WebSocket regression uses a real listener, the real
//! migrated database, and an already-established worker connection.

use axum::http::StatusCode;
use futures_util::{SinkExt, StreamExt};
use migrations::{Migrator, MigratorTrait};
use models::{entity::user::AuthUser, shared::enums::DeviceKind};
use sea_orm::{
	ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, EntityTrait,
	Statement,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use stump_core::config::StumpConfig;
use stump_server::{config::session::get_session_layer, routers};
use stump_worker::{
	kind::TranscodeOutput,
	protocol::{ServerFrame, WorkerFrame},
	TranscodeInput, TRANSCODE,
};
use tempfile::TempDir;
use tokio_tungstenite::{
	connect_async,
	tungstenite::{client::IntoClientRequest, Message},
};

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
	// `401`, not `403`: revocation deletes the credential row, so the key
	// cannot authenticate a new request. A separate live-socket regression
	// below proves that already upgraded connections lose that authority too.
	assert_eq!(
		revoked.status_code(),
		StatusCode::UNAUTHORIZED,
		"a revoked worker must not be able to reconnect"
	);
}

/// A socket which already received a real job offer cannot claim that job or
/// receive a fresh offer once its account, device, or key has been revoked.
#[tokio::test]
async fn live_worker_socket_stops_on_account_device_and_key_revocation() {
	use std::time::Duration;

	for reason in ["account", "device", "key"] {
		let fixture = Fixture::new().await;
		let (device_id, key) = if reason == "account" {
			let permissions =
				stump_devices::required_permissions(DeviceKind::Worker, false);
			let stored_permissions = permissions
				.iter()
				.map(ToString::to_string)
				.collect::<Vec<_>>()
				.join(",");
			fixture.app.conn().execute(Statement::from_string(
				DatabaseBackend::Sqlite,
				format!("INSERT INTO users (id, username, hashed_password, is_server_owner, created_at, is_locked, permissions) VALUES ('worker-member', 'worker-member', 'hash', 0, CURRENT_TIMESTAMP, 0, '{stored_permissions}')"),
			)).await.expect("member");
			let member = AuthUser {
				id: "worker-member".into(),
				username: "worker-member".into(),
				permissions,
				..Default::default()
			};
			let (device, issued) = fixture
				.app
				.ctx
				.devices()
				.create_device(
					&member,
					stump_devices::CredentialIssuance::InteractiveSession,
					DeviceKind::Worker,
					Some("member worker".into()),
				)
				.await
				.expect("member worker");
			(device.id, issued.secret)
		} else {
			fixture.device(DeviceKind::Worker, "live worker").await
		};

		let state = fixture.app.ctx.clone();
		let router = routers::mount(state.clone())
			.await
			.with_state(state.clone())
			.layer(get_session_layer(state));
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
			.await
			.expect("listener");
		let addr = listener.local_addr().expect("address");
		let server = tokio::spawn(async move {
			axum::serve(listener, router)
				.await
				.expect("worker test server");
		});

		let mut request = format!("ws://{addr}{SOCKET}")
			.into_client_request()
			.expect("websocket request");
		request.headers_mut().insert(
			"Authorization",
			format!("Bearer {key}").parse().expect("bearer"),
		);
		request.headers_mut().insert(
			"user-agent",
			"stump-server-tests".parse().expect("user agent"),
		);
		let (mut socket, _) = connect_async(request).await.expect("live worker socket");
		let jobs = fixture.app.ctx.worker_jobs();
		let first = queued_guarded_job(&fixture).await;
		socket
			.send(Message::Text(
				stump_worker::protocol::encode(&WorkerFrame::Hello {
					capabilities: json!({ "transcode": true }),
					name: None,
					version: None,
				})
				.into(),
			))
			.await
			.expect("worker hello");
		let offer = tokio::time::timeout(Duration::from_secs(5), socket.next())
			.await
			.expect("initial offer timeout")
			.expect("socket closed")
			.expect("offer frame");
		let ServerFrame::Job { id, .. } =
			serde_json::from_str(offer.to_text().expect("text offer"))
				.expect("worker offer")
		else {
			panic!("expected real job offer")
		};
		assert_eq!(id, first.id);

		match reason {
			"account" => {
				let result = fixture
					.app
					.execute_gql(
						"mutation { deleteUser(id: \"worker-member\") { id } }",
						None,
					)
					.await;
				assert_eq!(
					result["data"]["deleteUser"]["id"], "worker-member",
					"{result}"
				);
			},
			"device" => {
				fixture
					.app
					.ctx
					.devices()
					.revoke(&fixture.owner().await, &device_id)
					.await
					.expect("revoke device");
			},
			"key" => {
				fixture
					.app
					.ctx
					.devices()
					.rotate_credential(
						&fixture.owner().await,
						stump_devices::CredentialIssuance::InteractiveSession,
						&device_id,
					)
					.await
					.expect("rotate key");
			},
			_ => unreachable!(),
		}

		// Both directions matter: outbound job bytes and an inbound claim are
		// forbidden even though the original upgrade was valid.
		let later = queued_guarded_job(&fixture).await;
		let _ = socket
			.send(Message::Text(
				stump_worker::protocol::encode(&WorkerFrame::Claim {
					job_id: first.id.clone(),
				})
				.into(),
			))
			.await;
		let next = tokio::time::timeout(Duration::from_secs(5), socket.next())
			.await
			.expect("revoked socket did not close");
		assert!(
			matches!(next, None | Some(Ok(Message::Close(_))) | Some(Err(_))),
			"revoked socket received a new frame: {next:?} ({reason})"
		);
		assert_ne!(
			jobs.get(&first.id)
				.await
				.expect("first job")
				.expect("first row")
				.status,
			stump_worker::WorkerJobStatus::Claimed,
			"{reason} must not claim work"
		);
		assert_ne!(
			jobs.get(&later.id)
				.await
				.expect("later job")
				.expect("later row")
				.status,
			stump_worker::WorkerJobStatus::Claimed,
			"{reason} must not receive work"
		);
		server.abort();
	}
}

/// No local executor is registered for this kind: it must be offered over the
/// socket, unlike transcoding on a test host with an ffmpeg fallback.
async fn queued_guarded_job(fixture: &Fixture) -> stump_worker::entity::Model {
	fixture
		.app
		.ctx
		.worker_jobs()
		.enqueue(
			"credential_guard_regression",
			json!({ "private": "worker-only payload" }),
			stump_worker::transcode_requires(),
			0,
		)
		.await
		.expect("enqueue worker-only job")
}

/// An established remote-source socket must neither reconcile another
/// manifest revision nor retain a live inventory after credential revocation.
#[tokio::test]
async fn live_source_socket_stops_on_account_device_and_key_revocation() {
	use models::entity::remote_source;
	use sea_orm::{ColumnTrait, QueryFilter};
	use std::time::Duration;
	use stump_worker::source_protocol::{
		encode_source_frame, SourceManifestChunk, SourceRootHello, SourceTransport,
		SourceWorkerFrame,
	};

	for reason in ["account", "device", "key"] {
		let fixture = Fixture::new().await;
		let (device_id, key) = if reason == "account" {
			let permissions =
				stump_devices::required_permissions(DeviceKind::SourceWorker, false);
			let stored_permissions = permissions
				.iter()
				.map(ToString::to_string)
				.collect::<Vec<_>>()
				.join(",");
			fixture.app.conn().execute(Statement::from_string(DatabaseBackend::Sqlite,
				format!("INSERT INTO users (id, username, hashed_password, is_server_owner, created_at, is_locked, permissions) VALUES ('source-member', 'source-member', 'hash', 0, CURRENT_TIMESTAMP, 0, '{stored_permissions}')")))
				.await.expect("source member");
			let member = AuthUser {
				id: "source-member".into(),
				username: "source-member".into(),
				permissions,
				..Default::default()
			};
			let (device, issued) = fixture
				.app
				.ctx
				.devices()
				.create_device(
					&member,
					stump_devices::CredentialIssuance::InteractiveSession,
					DeviceKind::SourceWorker,
					Some("source member".into()),
				)
				.await
				.expect("source member device");
			(device.id, issued.secret)
		} else {
			fixture
				.device(DeviceKind::SourceWorker, "live source")
				.await
		};

		let state = fixture.app.ctx.clone();
		let router = routers::mount(state.clone())
			.await
			.with_state(state.clone())
			.layer(get_session_layer(state));
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
			.await
			.expect("listener");
		let addr = listener.local_addr().expect("address");
		let server = tokio::spawn(async move {
			axum::serve(listener, router)
				.await
				.expect("source test server");
		});
		let mut request = format!("ws://{addr}/api/v2/source-workers/socket")
			.into_client_request()
			.expect("source request");
		request.headers_mut().insert(
			"Authorization",
			format!("Bearer {key}").parse().expect("bearer"),
		);
		request.headers_mut().insert(
			"user-agent",
			"stump-server-tests".parse().expect("user agent"),
		);
		let (mut socket, _) = connect_async(request).await.expect("live source socket");
		let hello = SourceWorkerFrame::Hello {
			name: None,
			version: None,
			roots: vec![SourceRootHello {
				root_id: "library".into(),
				label: "Library".into(),
				kind: "ebooks".into(),
				privacy_mode: "catalog".into(),
				transport: SourceTransport::Tunnel,
				direct_base_url: None,
			}],
		};
		socket
			.send(Message::Text(encode_source_frame(&hello).into()))
			.await
			.expect("source hello");
		let chunk = |revision| {
			SourceWorkerFrame::ManifestChunk(SourceManifestChunk {
				batch_id: format!("batch-{revision}"),
				root_id: "library".into(),
				revision,
				sequence: 0,
				terminal: true,
				items: vec![],
			})
		};
		socket
			.send(Message::Text(encode_source_frame(&chunk(1)).into()))
			.await
			.expect("initial manifest");
		let source = tokio::time::timeout(Duration::from_secs(5), async {
			loop {
				let source = remote_source::Entity::find()
					.filter(remote_source::Column::DeviceId.eq(&device_id))
					.one(fixture.app.conn())
					.await
					.expect("source query");
				if let Some(source) = source.filter(|source| source.current_revision == 1)
				{
					break source;
				}
				tokio::time::sleep(Duration::from_millis(10)).await;
			}
		})
		.await
		.expect("first manifest committed");
		let initial_ack = tokio::time::timeout(Duration::from_secs(5), socket.next())
			.await
			.expect("initial manifest acknowledgement timeout")
			.expect("source socket closed before acknowledgement")
			.expect("source acknowledgement frame");
		let initial_ack: serde_json::Value =
			serde_json::from_str(initial_ack.to_text().expect("text acknowledgement"))
				.expect("decode initial acknowledgement");
		assert_eq!(initial_ack["type"], "manifest_ack");
		assert_eq!(initial_ack["root_id"], "library");
		assert_eq!(initial_ack["revision"], 1);

		match reason {
			"account" => {
				let result = fixture
					.app
					.execute_gql(
						"mutation { deleteUser(id: \"source-member\") { id } }",
						None,
					)
					.await;
				assert_eq!(
					result["data"]["deleteUser"]["id"], "source-member",
					"{result}"
				);
			},
			"device" => {
				fixture
					.app
					.ctx
					.devices()
					.revoke(&fixture.owner().await, &device_id)
					.await
					.expect("revoke source device");
			},
			"key" => {
				fixture
					.app
					.ctx
					.devices()
					.rotate_credential(
						&fixture.owner().await,
						stump_devices::CredentialIssuance::InteractiveSession,
						&device_id,
					)
					.await
					.expect("rotate source key");
			},
			_ => unreachable!(),
		}
		let _ = socket
			.send(Message::Text(encode_source_frame(&chunk(2)).into()))
			.await;
		let next = tokio::time::timeout(Duration::from_secs(6), socket.next())
			.await
			.expect("revoked source socket did not close");
		assert!(
			matches!(next, None | Some(Ok(Message::Close(_))) | Some(Err(_))),
			"revoked source socket received a frame: {next:?} ({reason})"
		);
		let after = remote_source::Entity::find_by_id(source.id)
			.one(fixture.app.conn())
			.await
			.expect("source query")
			.expect("retained source inventory");
		assert_eq!(
			after.current_revision, 1,
			"{reason} must not ingest another manifest"
		);
		assert_eq!(
			after.health, "offline",
			"{reason} must mark revoked source offline"
		);
		server.abort();
	}
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
