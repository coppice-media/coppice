//! End-to-end protocol tests: a real `stump-worker` client against a real hub,
//! over a real WebSocket listener on `127.0.0.1`.
//!
//! The transport is deliberately not faked. The whole risk in this crate is the
//! seam between the frame handler and the socket — a claim that races a
//! disconnect, a resume that offers a job twice, a `result` that arrives after
//! a cancel — and none of that is exercised by handing the service a `Vec` of
//! frames. So the test mounts the same axum `ws` upgrade the server mounts, on
//! the same path the client derives, and drives it with `client::run_once`.
//!
//! Authentication is the one thing stubbed: the test route reads the device id
//! straight out of the `Authorization` header, where the server's auth
//! middleware would have resolved it. The real auth path (a device credential
//! is required, a non-`Worker` device is refused) is asserted in
//! `apps/server/tests/worker/mod.rs`, against the real middleware.

use std::sync::Arc;
use std::time::Duration;

use axum::{
	extract::{
		ws::{Message, WebSocket, WebSocketUpgrade},
		State,
	},
	http::HeaderMap,
	response::Response,
	routing::get,
	Router,
};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::{
	client::{self, Assignment, ClientConfig, JobRunner, Progress},
	kind::{LocalJob, LocalRunner},
	protocol::parse_worker_frame,
	service::{JobOutcome, WorkerJobs, INTERACTIVE_PRIORITY},
	KindRegistry, WorkerJobStatus,
};

/// Long enough that a loaded machine does not flake, short enough that a real
/// hang fails the suite instead of hanging it.
const SETTLE: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------- harness

struct Harness {
	service: Arc<WorkerJobs>,
	base_url: String,
	_dir: tempfile::TempDir,
}

async fn harness(registry: KindRegistry) -> Harness {
	use migrations::MigratorTrait;

	let conn = Arc::new(
		sea_orm::Database::connect("sqlite::memory:")
			.await
			.expect("sqlite"),
	);
	migrations::Migrator::up(conn.as_ref(), None)
		.await
		.expect("migrations");
	let dir = tempfile::tempdir().expect("temp dir");
	let service = Arc::new(
		WorkerJobs::new(conn, dir.path().join("worker-output")).with_registry(registry),
	);

	let app = Router::new()
		.route("/api/v2/workers/socket", get(upgrade))
		.with_state(service.clone());
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
		.await
		.expect("bind");
	let addr = listener.local_addr().expect("addr");
	tokio::spawn(async move {
		let _ = axum::serve(listener, app).await;
	});

	Harness {
		service,
		base_url: format!("http://{addr}"),
		_dir: dir,
	}
}

/// The reference transport: what `apps/server` mounts, minus the auth.
async fn upgrade(
	State(service): State<Arc<WorkerJobs>>,
	headers: HeaderMap,
	upgrade: WebSocketUpgrade,
) -> Response {
	let device_id = headers
		.get(axum::http::header::AUTHORIZATION)
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.strip_prefix("Bearer "))
		.unwrap_or("unknown")
		.to_string();
	upgrade.on_upgrade(move |socket| pump(service, device_id, socket))
}

async fn pump(service: Arc<WorkerJobs>, device_id: String, mut socket: WebSocket) {
	let Some(Ok(Message::Text(hello))) = socket.recv().await else {
		return;
	};
	let Ok(hello) = parse_worker_frame(&hello) else {
		return;
	};
	let Ok((mut outbound, epoch)) = service
		.attach_worker(&device_id, "test worker", hello)
		.await
	else {
		return;
	};

	let (mut writer, mut reader) = socket.split();
	let writes = tokio::spawn(async move {
		use futures_util::SinkExt;
		while let Some(text) = outbound.recv().await {
			if writer.send(Message::Text(text.into())).await.is_err() {
				break;
			}
		}
	});

	use futures_util::StreamExt;
	while let Some(Ok(message)) = reader.next().await {
		let Message::Text(text) = message else {
			continue;
		};
		let Ok(frame) = parse_worker_frame(&text) else {
			continue;
		};
		if let Err(error) = service.handle_frame(&device_id, frame).await {
			tracing::debug!(?error, "frame refused");
		}
	}

	writes.abort();
	let _ = service.detach_worker(&device_id, epoch).await;
}

/// A worker that reports one progress line and returns what it was told to.
struct ScriptedRunner {
	capabilities: Value,
	/// Frames observed, so a test can assert the client's own behaviour.
	seen: Arc<Mutex<Vec<String>>>,
	/// Held until released, to keep a job "in flight" across a disconnect.
	gate: Option<Arc<tokio::sync::Semaphore>>,
}

impl ScriptedRunner {
	fn new(capabilities: Value) -> Self {
		Self {
			capabilities,
			seen: Arc::new(Mutex::new(Vec::new())),
			gate: None,
		}
	}
}

#[async_trait::async_trait]
impl JobRunner for ScriptedRunner {
	fn capabilities(&self) -> Value {
		self.capabilities.clone()
	}

	async fn run(
		&self,
		assignment: Assignment,
		progress: Progress,
	) -> Result<Value, String> {
		self.seen.lock().await.push(assignment.id.clone());
		progress.report(0.5, "halfway");
		if let Some(gate) = self.gate.as_ref() {
			// Blocks forever unless the test adds a permit.
			let _permit = gate.acquire().await.map_err(|error| error.to_string())?;
		}
		Ok(json!({ "ran": assignment.id }))
	}
}

fn spawn_client(
	base_url: &str,
	api_key: &str,
	runner: Arc<dyn JobRunner>,
) -> tokio::task::JoinHandle<()> {
	let config = ClientConfig {
		server: base_url.to_string(),
		api_key: api_key.to_string(),
		name: Some("test worker".into()),
	};
	tokio::spawn(async move {
		let _ = client::run_once(&config, runner).await;
	})
}

/// Poll a job until `predicate` holds, or fail the test.
async fn await_job(
	service: &WorkerJobs,
	id: &str,
	predicate: impl Fn(&crate::entity::Model) -> bool,
	what: &str,
) -> crate::entity::Model {
	let deadline = tokio::time::Instant::now() + SETTLE;
	loop {
		let job = service
			.get(id)
			.await
			.expect("query")
			.expect("the job exists");
		if predicate(&job) {
			return job;
		}
		assert!(
			tokio::time::Instant::now() < deadline,
			"timed out waiting for {what}; job is {:?}",
			job.status
		);
		tokio::time::sleep(Duration::from_millis(20)).await;
	}
}

/// Wait until a worker has completed its `hello`, so an enqueue that follows
/// is genuinely offered rather than racing the handshake.
async fn await_connected(service: &WorkerJobs, device_id: &str) {
	let deadline = tokio::time::Instant::now() + SETTLE;
	while !service.hub().is_connected(device_id).await {
		assert!(
			tokio::time::Instant::now() < deadline,
			"the worker never connected"
		);
		tokio::time::sleep(Duration::from_millis(10)).await;
	}
}

// ------------------------------------------------------------------ tests

/// The happy path over a real socket: `hello` registers the worker, the offer
/// reaches it, its `claim` moves the row, its `progress` is persisted, and its
/// `result` completes the job and wakes the caller that was awaiting it.
#[tokio::test]
async fn a_job_round_trips_over_the_socket() {
	let harness = harness(KindRegistry::new()).await;
	let runner = Arc::new(ScriptedRunner::new(json!({ "transcode": {} })));
	let client = spawn_client(&harness.base_url, "dev-worker", runner.clone());
	await_connected(&harness.service, "dev-worker").await;

	let workers = harness.service.hub().connected().await;
	assert_eq!(workers.len(), 1);
	assert_eq!(workers[0].device_id, "dev-worker");
	assert_eq!(workers[0].capabilities, json!({ "transcode": {} }));

	let outcome = harness
		.service
		.enqueue_and_wait(
			crate::TRANSCODE,
			json!({ "media_id": "m1" }),
			crate::transcode_requires(),
			INTERACTIVE_PRIORITY,
			SETTLE,
		)
		.await
		.expect("the job completes");

	let JobOutcome::Done { result, .. } = outcome else {
		panic!("expected the worker to complete the job, got {outcome:?}");
	};
	let ran = result["ran"].as_str().expect("the runner echoes its id");

	let job = harness
		.service
		.get(ran)
		.await
		.expect("query")
		.expect("the job exists");
	assert_eq!(job.status, WorkerJobStatus::Done);
	assert_eq!(job.worker_id.as_deref(), Some("dev-worker"));
	assert_eq!(job.progress, 1.0);
	// The `progress` frame landed on its way through, so the console had
	// something to show while the job ran.
	assert_eq!(job.progress_message.as_deref(), Some("halfway"));
	assert!(job.started_at.is_some() && job.finished_at.is_some());

	client.abort();
}

/// A job whose requirements nobody advertises, and whose kind has no local
/// implementation, must come to rest in `needs_worker` — visible, not silently
/// queued forever — and must tell the caller so immediately rather than making
/// it wait out a timeout.
#[tokio::test]
async fn a_job_with_no_capable_worker_and_no_fallback_needs_a_worker() {
	let harness = harness(KindRegistry::new()).await;
	// A worker is connected; it just cannot do this.
	let runner = Arc::new(ScriptedRunner::new(json!({ "transcode": {} })));
	let client = spawn_client(&harness.base_url, "dev-worker", runner);
	await_connected(&harness.service, "dev-worker").await;

	let started = tokio::time::Instant::now();
	let outcome = harness
		.service
		.enqueue_and_wait(
			crate::ALIGN,
			json!({}),
			json!({ "align": { "device": "cuda" } }),
			INTERACTIVE_PRIORITY,
			SETTLE,
		)
		.await
		.expect("routing answers");
	assert_eq!(outcome, JobOutcome::NeedsWorker);
	assert!(
		started.elapsed() < SETTLE,
		"needs_worker must answer at once, not time out"
	);

	let parked = harness
		.service
		.list(Some(WorkerJobStatus::NeedsWorker), 10)
		.await
		.expect("list");
	assert_eq!(parked.len(), 1);
	assert_eq!(parked[0].kind, crate::ALIGN);
	assert!(parked[0].worker_id.is_none());

	client.abort();
}

/// The same job, once a worker that *can* run it arrives, must leave
/// `needs_worker` on its own — the operator starts the GPU box, the queue
/// drains, nobody re-enqueues anything.
#[tokio::test]
async fn a_parked_job_is_dispatched_when_a_capable_worker_arrives() {
	let harness = harness(KindRegistry::new()).await;
	let job = harness
		.service
		.enqueue(
			crate::TRANSCODE,
			json!({ "media_id": "m1" }),
			crate::transcode_requires(),
			INTERACTIVE_PRIORITY,
		)
		.await
		.expect("enqueue");
	assert_eq!(job.status, WorkerJobStatus::NeedsWorker);

	let runner = Arc::new(ScriptedRunner::new(json!({ "transcode": {} })));
	let client = spawn_client(&harness.base_url, "dev-worker", runner);

	let done = await_job(
		&harness.service,
		&job.id,
		|job| job.status == WorkerJobStatus::Done,
		"the parked job to be picked up",
	)
	.await;
	assert_eq!(done.worker_id.as_deref(), Some("dev-worker"));

	client.abort();
}

/// A worker that drops mid-job keeps the job; the next `hello` is re-offered it
/// by id. Without this a network blip silently loses an hour of encoding.
#[tokio::test]
async fn reconnect_resumes_a_claimed_job() {
	let harness = harness(KindRegistry::new()).await;
	let gate = Arc::new(tokio::sync::Semaphore::new(0));
	let blocked = Arc::new(ScriptedRunner {
		capabilities: json!({ "transcode": {} }),
		seen: Arc::new(Mutex::new(Vec::new())),
		gate: Some(gate.clone()),
	});
	let first = spawn_client(&harness.base_url, "dev-worker", blocked.clone());
	await_connected(&harness.service, "dev-worker").await;

	let job = harness
		.service
		.enqueue(
			crate::TRANSCODE,
			json!({ "media_id": "m1" }),
			crate::transcode_requires(),
			INTERACTIVE_PRIORITY,
		)
		.await
		.expect("enqueue");
	let claimed = await_job(
		&harness.service,
		&job.id,
		|job| job.status == WorkerJobStatus::Running,
		"the first worker to start the job",
	)
	.await;
	assert_eq!(claimed.worker_id.as_deref(), Some("dev-worker"));

	// Kill the socket mid-job. The row must stay assigned: the worker may be
	// halfway through an encode and about to come back.
	first.abort();
	tokio::time::sleep(Duration::from_millis(100)).await;
	let orphaned = harness
		.service
		.get(&job.id)
		.await
		.expect("query")
		.expect("row");
	assert_eq!(orphaned.worker_id.as_deref(), Some("dev-worker"));
	assert!(orphaned.status.is_assigned());

	// The same device reconnects; the server re-offers by id and the fresh
	// runner completes it.
	let resumed = Arc::new(ScriptedRunner::new(json!({ "transcode": {} })));
	let second = spawn_client(&harness.base_url, "dev-worker", resumed.clone());

	let done = await_job(
		&harness.service,
		&job.id,
		|job| job.status == WorkerJobStatus::Done,
		"the reconnected worker to finish the resumed job",
	)
	.await;
	assert_eq!(done.worker_id.as_deref(), Some("dev-worker"));
	assert_eq!(
		resumed.seen.lock().await.as_slice(),
		&[job.id.clone()],
		"the resumed job must be handed over exactly once"
	);

	second.abort();
	drop(gate);
}

/// Cancelling reaches the worker and lands the row terminal with a reason. The
/// status set is the contract, so a cancel is a `failed`, never a seventh
/// status the console would have to learn.
#[tokio::test]
async fn cancel_stops_the_worker_and_marks_the_job_failed() {
	let harness = harness(KindRegistry::new()).await;
	let gate = Arc::new(tokio::sync::Semaphore::new(0));
	let blocked = Arc::new(ScriptedRunner {
		capabilities: json!({ "transcode": {} }),
		seen: Arc::new(Mutex::new(Vec::new())),
		gate: Some(gate.clone()),
	});
	let client = spawn_client(&harness.base_url, "dev-worker", blocked);
	await_connected(&harness.service, "dev-worker").await;

	let job = harness
		.service
		.enqueue(
			crate::TRANSCODE,
			json!({ "media_id": "m1" }),
			crate::transcode_requires(),
			INTERACTIVE_PRIORITY,
		)
		.await
		.expect("enqueue");
	await_job(
		&harness.service,
		&job.id,
		|job| job.status == WorkerJobStatus::Running,
		"the worker to start",
	)
	.await;

	let cancelled = harness
		.service
		.cancel(&job.id, "cancelled by al")
		.await
		.expect("cancel");
	assert_eq!(cancelled.status, WorkerJobStatus::Failed);
	assert_eq!(cancelled.error.as_deref(), Some("cancelled by al"));
	assert!(cancelled.finished_at.is_some());

	// Releasing the gate lets the aborted runner's task, if it somehow
	// survived, try to report; the terminal row must not move.
	gate.add_permits(1);
	tokio::time::sleep(Duration::from_millis(100)).await;
	let after = harness
		.service
		.get(&job.id)
		.await
		.expect("query")
		.expect("row");
	assert_eq!(after.status, WorkerJobStatus::Failed);

	client.abort();
}

/// With no worker connected, a kind that has a local implementation runs here
/// and writes its bytes to the same path an upload would have landed at — which
/// is what lets the audio route await one job and publish one file without
/// knowing who ran it.
#[tokio::test]
async fn a_kind_with_a_local_fallback_runs_on_the_server() {
	struct Local;

	#[async_trait::async_trait]
	impl LocalRunner for Local {
		async fn run(&self, job: LocalJob) -> Result<Value, String> {
			tokio::fs::write(&job.output_path, b"locally encoded")
				.await
				.map_err(|error| error.to_string())?;
			Ok(json!({ "bytes": 15 }))
		}
	}

	let harness =
		harness(KindRegistry::new().with_local(crate::TRANSCODE, Arc::new(Local))).await;

	let outcome = harness
		.service
		.enqueue_and_wait(
			crate::TRANSCODE,
			json!({ "media_id": "m1" }),
			crate::transcode_requires(),
			INTERACTIVE_PRIORITY,
			SETTLE,
		)
		.await
		.expect("the local fallback completes the job");

	let JobOutcome::Done {
		result,
		output_path,
	} = outcome
	else {
		panic!("expected the local fallback to run, got {outcome:?}");
	};
	assert_eq!(result["bytes"], json!(15));
	assert_eq!(
		tokio::fs::read(&output_path).await.expect("output"),
		b"locally encoded"
	);

	let jobs = harness.service.list(None, 10).await.expect("list");
	assert_eq!(jobs.len(), 1);
	assert_eq!(jobs[0].status, WorkerJobStatus::Done);
	assert!(
		jobs[0].worker_id.is_none(),
		"a locally-run job names no worker, which is how the console tells the two apart"
	);
}

/// A frame naming a job the sender does not hold is refused. Otherwise any
/// paired worker could complete — or corrupt — another worker's job.
#[tokio::test]
async fn a_worker_cannot_touch_a_job_it_does_not_hold() {
	let harness = harness(KindRegistry::new()).await;
	let runner = Arc::new(ScriptedRunner::new(json!({ "transcode": {} })));
	let client = spawn_client(&harness.base_url, "dev-a", runner);
	await_connected(&harness.service, "dev-a").await;

	let job = harness
		.service
		.enqueue(
			crate::TRANSCODE,
			json!({ "media_id": "m1" }),
			crate::transcode_requires(),
			INTERACTIVE_PRIORITY,
		)
		.await
		.expect("enqueue");

	let stolen = harness
		.service
		.handle_frame(
			"dev-b",
			crate::WorkerFrame::Result {
				job_id: job.id.clone(),
				output: json!({ "stolen": true }),
			},
		)
		.await;
	assert!(matches!(
		stolen,
		Err(crate::WorkerError::NotAssigned { .. })
	));

	// And the upload lane refuses the same way, so the bytes cannot be
	// substituted either.
	assert!(matches!(
		harness.service.upload_path(&job.id, "dev-b").await,
		Err(crate::WorkerError::NotAssigned { .. })
	));

	client.abort();
}

/// Two jobs in a row, the second offered only after the first has finished.
///
/// The regression: the client used to reap its finished job *after* matching
/// the frame, so the first offer arriving once a job completed hit the
/// busy guard and was silently dropped — and the server, having already
/// assigned the row, left it `queued` until the next reconnect. Any
/// deployment that gets two jobs in a row hit it; one job at a time hid it.
#[tokio::test]
async fn an_offer_arriving_after_a_completion_is_accepted() {
	let harness = harness(KindRegistry::new()).await;
	let runner = Arc::new(ScriptedRunner::new(json!({ "transcode": {} })));
	let client = spawn_client(&harness.base_url, "dev-worker", runner.clone());
	await_connected(&harness.service, "dev-worker").await;

	let first = harness
		.service
		.enqueue(
			crate::TRANSCODE,
			json!({ "media_id": "m1" }),
			crate::transcode_requires(),
			INTERACTIVE_PRIORITY,
		)
		.await
		.expect("enqueue");
	await_job(
		&harness.service,
		&first.id,
		|job| job.status == WorkerJobStatus::Done,
		"the first job to finish",
	)
	.await;

	let second = harness
		.service
		.enqueue(
			crate::TRANSCODE,
			json!({ "media_id": "m2" }),
			crate::transcode_requires(),
			INTERACTIVE_PRIORITY,
		)
		.await
		.expect("enqueue");
	let done = await_job(
		&harness.service,
		&second.id,
		|job| job.status == WorkerJobStatus::Done,
		"the second job to be accepted after the first finished",
	)
	.await;
	assert_eq!(done.worker_id.as_deref(), Some("dev-worker"));
	assert_eq!(
		runtime_ids(&runner).await,
		vec![first.id.clone(), second.id.clone()],
		"both jobs must have run, in order"
	);

	client.abort();
}

/// Two jobs offered while the worker is still busy with the first.
///
/// The client runs one at a time, but it must *queue* what it was offered
/// rather than drop it: the server has already assigned the row, and there is
/// no decline frame for it to change its mind with.
#[tokio::test]
async fn an_offer_arriving_while_busy_is_queued_not_dropped() {
	let harness = harness(KindRegistry::new()).await;
	let gate = Arc::new(tokio::sync::Semaphore::new(0));
	let runner = Arc::new(ScriptedRunner {
		capabilities: json!({ "transcode": {} }),
		seen: Arc::new(Mutex::new(Vec::new())),
		gate: Some(gate.clone()),
	});
	let client = spawn_client(&harness.base_url, "dev-worker", runner.clone());
	await_connected(&harness.service, "dev-worker").await;

	let mut ids = Vec::new();
	for media in ["m1", "m2"] {
		ids.push(
			harness
				.service
				.enqueue(
					crate::TRANSCODE,
					json!({ "media_id": media }),
					crate::transcode_requires(),
					INTERACTIVE_PRIORITY,
				)
				.await
				.expect("enqueue")
				.id,
		);
	}

	// Both are claimed even though only one can run: the claim is what makes a
	// reconnect resume them by id instead of stranding the queued one.
	for id in &ids {
		await_job(
			&harness.service,
			id,
			|job| job.status.is_assigned() && job.worker_id.is_some(),
			"both jobs to be claimed",
		)
		.await;
	}

	gate.add_permits(2);
	for id in &ids {
		await_job(
			&harness.service,
			id,
			|job| job.status == WorkerJobStatus::Done,
			"both jobs to finish",
		)
		.await;
	}

	client.abort();
}

async fn runtime_ids(runner: &ScriptedRunner) -> Vec<String> {
	runner.seen.lock().await.clone()
}
