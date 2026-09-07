//! The GraphQL shape of the worker lane.
//!
//! Two objects: a *connected* worker (in-memory, from the hub) and a job row.
//! They are deliberately separate types rather than one "worker with jobs":
//! a job outlives the connection that ran it, and a worker with nothing to do
//! is still a worker the operator wants to see on the page.

use async_graphql::{Object, SimpleObject, ID};
use stump_worker::{ConnectedWorker, WorkerJob as WorkerJobModel, WorkerJobStatus};

/// A worker currently holding a socket.
pub struct Worker(pub ConnectedWorker);

#[Object]
impl Worker {
	/// The `devices.id` of the paired worker device, so the console can link
	/// straight to its card (and its revoke button).
	async fn id(&self) -> ID {
		ID(self.0.device_id.clone())
	}

	async fn name(&self) -> &str {
		&self.0.name
	}

	/// The `stump-worker` version this host is running, when it reported one.
	async fn version(&self) -> Option<&str> {
		self.0.version.as_deref()
	}

	/// The `hello` payload verbatim, e.g.
	/// `{"transcode":{"ffmpeg":"7.1.0","hwaccel":["vaapi"]}}`. Free-form: the
	/// vocabulary is per job kind and a schema here would have to change every
	/// time a kind does.
	async fn capabilities(&self) -> async_graphql::Json<serde_json::Value> {
		async_graphql::Json(self.0.capabilities.clone())
	}

	/// The job kinds this worker advertises, for a page that wants a chip row
	/// rather than a JSON blob.
	async fn kinds(&self) -> Vec<String> {
		self.0
			.capabilities
			.as_object()
			.map(|caps| caps.keys().cloned().collect())
			.unwrap_or_default()
	}

	async fn connected_at(&self) -> String {
		self.0.connected_at.to_rfc3339()
	}

	/// When this worker last said anything. A worker mid-encode reports
	/// progress, so this is also "is it still working".
	async fn last_seen_at(&self) -> String {
		self.0.last_frame_at.to_rfc3339()
	}
}

impl From<ConnectedWorker> for Worker {
	fn from(worker: ConnectedWorker) -> Self {
		Self(worker)
	}
}

/// One `worker_jobs` row.
#[derive(SimpleObject)]
#[graphql(name = "WorkerJob")]
pub struct WorkerJob {
	pub id: ID,
	/// The registry id of the work: `transcode` today.
	pub kind: String,
	pub status: WorkerJobStatus,
	/// The worker holding (or that ran) the job. `null` on a job the server ran
	/// itself with its local implementation, which is how the console tells
	/// "served by the GPU box" from "the server did it".
	pub worker_id: Option<ID>,
	pub priority: i32,
	/// `0.0..=1.0`.
	pub progress: f64,
	/// The runner's last human-readable line.
	pub message: Option<String>,
	pub input: async_graphql::Json<serde_json::Value>,
	pub requires: async_graphql::Json<serde_json::Value>,
	pub result: Option<async_graphql::Json<serde_json::Value>>,
	pub error: Option<String>,
	pub created_at: String,
	pub updated_at: String,
	pub started_at: Option<String>,
	pub finished_at: Option<String>,
}

impl From<WorkerJobModel> for WorkerJob {
	fn from(job: WorkerJobModel) -> Self {
		Self {
			id: ID(job.id),
			kind: job.kind,
			status: job.status,
			worker_id: job.worker_id.map(ID),
			priority: job.priority,
			progress: job.progress,
			message: job.progress_message,
			input: async_graphql::Json(job.input),
			requires: async_graphql::Json(job.requires),
			result: job.result.map(async_graphql::Json),
			error: job.error,
			created_at: job.created_at.to_rfc3339(),
			updated_at: job.updated_at.to_rfc3339(),
			started_at: job.started_at.map(|at| at.to_rfc3339()),
			finished_at: job.finished_at.map(|at| at.to_rfc3339()),
		}
	}
}
