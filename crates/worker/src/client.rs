//! The protocol client: what `stump-worker` runs.
//!
//! One task, one socket, one job at a time. Concurrency is deliberately not
//! here: a transcode or an alignment saturates the machine it runs on, so a
//! worker that claimed three would finish all three later than it finishes
//! them one after another, and the queue is already the place that decides
//! what is next.
//!
//! The client dials *out*, so the server never needs a route to the worker's
//! machine — the whole point of the design. A dropped socket is reconnected
//! with capped backoff and the server re-offers whatever the worker still
//! holds, so a network blip costs one reconnect and not a lost encode.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::header, Message};

use crate::protocol::{encode, ServerFrame, WorkerFrame};

/// The shortest and longest reconnect delay.
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// What a worker was asked to do, handed to a [`JobRunner`].
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
	pub id: String,
	pub kind: String,
	pub input: Value,
	pub priority: i32,
}

/// Reports progress on the job being run. Cheap to clone; a send after the job
/// finished is dropped, so a runner never has to check whether it still may
/// report.
#[derive(Clone)]
pub struct Progress {
	job_id: String,
	tx: mpsc::UnboundedSender<String>,
}

impl Progress {
	pub fn report(&self, fraction: f64, message: impl Into<String>) {
		let _ = self.tx.send(encode(&WorkerFrame::Progress {
			job_id: self.job_id.clone(),
			fraction: fraction.clamp(0.0, 1.0),
			message: Some(message.into()),
		}));
	}
}

/// Runs the job kinds this worker advertises.
#[async_trait::async_trait]
pub trait JobRunner: Send + Sync {
	/// The `hello` payload: what this worker can do.
	fn capabilities(&self) -> Value;

	/// Produce the job's output, uploading any bytes it makes before it
	/// returns. The returned value is the `result` frame's `output`.
	async fn run(
		&self,
		assignment: Assignment,
		progress: Progress,
	) -> Result<Value, String>;
}

/// Where and as whom to connect.
#[derive(Debug, Clone)]
pub struct ClientConfig {
	/// The server's base URL, e.g. `http://127.0.0.1:25600`.
	pub server: String,
	/// The worker device's API key.
	pub api_key: String,
	/// Reported in `hello`, for the console's Workers page.
	pub name: Option<String>,
}

impl ClientConfig {
	/// The socket URL derived from the base URL: `http` → `ws`, `https` →
	/// `wss`. A base URL that already names a websocket scheme is left alone,
	/// so an operator behind a proxy can point at it directly.
	#[must_use]
	pub fn socket_url(&self) -> String {
		let base = self.server.trim_end_matches('/');
		let base = if let Some(rest) = base.strip_prefix("https://") {
			format!("wss://{rest}")
		} else if let Some(rest) = base.strip_prefix("http://") {
			format!("ws://{rest}")
		} else {
			base.to_string()
		};
		format!("{base}/api/v2/workers/socket")
	}

	/// Where a job's produced bytes are uploaded.
	#[must_use]
	pub fn upload_url(&self, job_id: &str) -> String {
		format!(
			"{}/api/v2/workers/jobs/{job_id}/output",
			self.server.trim_end_matches('/')
		)
	}
}

/// Connect, serve jobs, and reconnect forever.
///
/// Returns only when `run_once` fails in a way that cannot be a transient
/// network problem — today, never; the loop is the worker's whole lifetime and
/// `Ctrl-C` is what ends it.
pub async fn run(config: ClientConfig, runner: Arc<dyn JobRunner>) -> ! {
	let mut backoff = BACKOFF_MIN;
	loop {
		match run_once(&config, runner.clone()).await {
			Ok(()) => {
				tracing::warn!("Worker socket closed by the server; reconnecting");
				backoff = BACKOFF_MIN;
			},
			Err(error) => {
				tracing::warn!(%error, ?backoff, "Worker socket failed; reconnecting");
			},
		}
		tokio::time::sleep(backoff).await;
		backoff = (backoff * 2).min(BACKOFF_MAX);
	}
}

/// One connection's lifetime: dial, `hello`, then serve frames until the socket
/// closes.
pub async fn run_once(
	config: &ClientConfig,
	runner: Arc<dyn JobRunner>,
) -> Result<(), String> {
	let mut request = config
		.socket_url()
		.into_client_request()
		.map_err(|error| format!("invalid server URL: {error}"))?;
	request.headers_mut().insert(
		header::AUTHORIZATION,
		format!("Bearer {}", config.api_key)
			.parse()
			.map_err(|_| "invalid API key".to_string())?,
	);

	let (socket, _) = tokio_tungstenite::connect_async(request)
		.await
		.map_err(|error| error.to_string())?;
	tracing::info!(url = %config.socket_url(), "Worker socket connected");
	let (mut writer, mut reader) = socket.split();

	// Every outbound frame goes through one channel: the runner's progress
	// reports are produced on a task that must not own the socket writer.
	let (tx, mut rx) = mpsc::unbounded_channel::<String>();
	tx.send(encode(&WorkerFrame::Hello {
		capabilities: runner.capabilities(),
		name: config.name.clone(),
		version: Some(env!("CARGO_PKG_VERSION").to_string()),
	}))
	.map_err(|_| "outbound channel closed".to_string())?;

	let writer_task = tokio::spawn(async move {
		while let Some(text) = rx.recv().await {
			if writer.send(Message::Text(text.into())).await.is_err() {
				break;
			}
		}
		let _ = writer.close().await;
	});

	// One job runs at a time, but an offer that arrives while busy is *queued*,
	// never dropped: the server has already assigned it, and a worker that
	// silently ignored it would leave the row `queued` until the next
	// reconnect. The job task signals completion here so the next offer starts
	// the moment the current one ends, rather than when the next frame happens
	// to arrive.
	let (done_tx, mut done_rx) = mpsc::unbounded_channel::<String>();
	let mut current: Option<(String, tokio::task::JoinHandle<()>)> = None;
	let mut pending: std::collections::VecDeque<Assignment> =
		std::collections::VecDeque::new();

	loop {
		let message = tokio::select! {
			finished = done_rx.recv() => {
				if let Some(finished) = finished {
					if current.as_ref().is_some_and(|(id, _)| id == &finished) {
						current = None;
					}
					if current.is_none() {
						if let Some(next) = pending.pop_front() {
							let id = next.id.clone();
							let handle = spawn_job(
								runner.clone(),
								tx.clone(),
								done_tx.clone(),
								next,
							);
							current = Some((id, handle));
						}
					}
				}
				continue;
			},
			message = reader.next() => match message {
				Some(Ok(message)) => message,
				Some(Err(error)) => {
					writer_task.abort();
					return Err(error.to_string());
				},
				None => break,
			},
		};
		let text = match message {
			Message::Text(text) => text.to_string(),
			Message::Close(_) => break,
			// Ping/pong are answered by the library; binary frames are not
			// part of the protocol and are ignored rather than fatal.
			_ => continue,
		};

		match crate::protocol::parse_server_frame(&text) {
			Ok(ServerFrame::Job {
				id,
				kind,
				input,
				priority,
			}) => {
				// A resume for something already in flight or already queued
				// is a no-op; the server re-offers on every `hello`.
				if current.as_ref().is_some_and(|(running, _)| running == &id)
					|| pending.iter().any(|queued| queued.id == id)
				{
					continue;
				}
				// Claim immediately, even when busy: the claim is what tells
				// the server this worker holds the job, and it is what makes a
				// reconnect resume it by id instead of stranding it.
				let _ = tx.send(encode(&WorkerFrame::Claim { job_id: id.clone() }));
				let assignment = Assignment {
					id: id.clone(),
					kind,
					input,
					priority,
				};
				if current.is_some() {
					tracing::debug!(job_id = %id, "Queued an offer behind the running job");
					pending.push_back(assignment);
					continue;
				}
				let handle =
					spawn_job(runner.clone(), tx.clone(), done_tx.clone(), assignment);
				current = Some((id, handle));
			},
			Ok(ServerFrame::Cancel { job_id }) => {
				pending.retain(|queued| queued.id != job_id);
				if let Some((running, handle)) = current.take() {
					if running == job_id {
						tracing::info!(%job_id, "Cancelling the running job");
						handle.abort();
						// `abort` sends no completion, so the next queued job
						// would otherwise wait for a frame that never comes.
						if let Some(next) = pending.pop_front() {
							let id = next.id.clone();
							let handle = spawn_job(
								runner.clone(),
								tx.clone(),
								done_tx.clone(),
								next,
							);
							current = Some((id, handle));
						}
					} else {
						current = Some((running, handle));
					}
				}
			},
			Err(error) => {
				tracing::warn!(%error, frame = %text, "Ignoring an unparseable frame");
			},
		}
	}

	writer_task.abort();
	Ok(())
}

/// Run one job on its own task and report its terminal frame.
fn spawn_job(
	runner: Arc<dyn JobRunner>,
	tx: mpsc::UnboundedSender<String>,
	done: mpsc::UnboundedSender<String>,
	assignment: Assignment,
) -> tokio::task::JoinHandle<()> {
	tokio::spawn(async move {
		let job_id = assignment.id.clone();
		let progress = Progress {
			job_id: job_id.clone(),
			tx: tx.clone(),
		};
		tracing::info!(%job_id, kind = %assignment.kind, "Running job");
		let frame = match runner.run(assignment, progress).await {
			Ok(output) => WorkerFrame::Result { job_id, output },
			Err(error) => {
				tracing::warn!(%error, "Job failed");
				WorkerFrame::Fail { job_id, error }
			},
		};
		let _ = tx.send(encode(&frame));
		let _ = done.send(job_id_of(&frame));
	})
}

fn job_id_of(frame: &WorkerFrame) -> String {
	frame.job_id().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn socket_and_upload_urls_are_derived_from_one_base() {
		let config = ClientConfig {
			server: "http://127.0.0.1:25600/".into(),
			api_key: "k".into(),
			name: None,
		};
		assert_eq!(
			config.socket_url(),
			"ws://127.0.0.1:25600/api/v2/workers/socket"
		);
		assert_eq!(
			config.upload_url("j1"),
			"http://127.0.0.1:25600/api/v2/workers/jobs/j1/output"
		);

		let tls = ClientConfig {
			server: "https://stump.example".into(),
			api_key: "k".into(),
			name: None,
		};
		assert_eq!(
			tls.socket_url(),
			"wss://stump.example/api/v2/workers/socket"
		);
	}
}
