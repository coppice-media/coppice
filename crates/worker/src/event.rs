//! What the rest of the server learns about a job.
//!
//! One event per persisted transition, carrying everything a live list needs to
//! redraw a row without refetching it. The host forwards it as
//! `CoreEvent::WorkerJobChanged`, the same way `stump_devices::DeviceSeen`
//! becomes `CoreEvent::DeviceSeen`; this crate never touches the core bus.

use serde::{Deserialize, Serialize};

use models::shared::enums::WorkerJobStatus;

/// A `worker_jobs` row changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct WorkerJobChanged {
	pub id: String,
	pub kind: String,
	pub status: WorkerJobStatus,
	/// The worker holding the job, or `None` for an unassigned or locally-run
	/// job — the console distinguishes "served by the GPU box" from "the
	/// server did it itself".
	pub worker_id: Option<String>,
	pub progress: f64,
	pub message: Option<String>,
	pub error: Option<String>,
}

impl From<&crate::entity::Model> for WorkerJobChanged {
	fn from(job: &crate::entity::Model) -> Self {
		Self {
			id: job.id.clone(),
			kind: job.kind.clone(),
			status: job.status,
			worker_id: job.worker_id.clone(),
			progress: job.progress,
			message: job.progress_message.clone(),
			error: job.error.clone(),
		}
	}
}

/// The callback a host installs to receive [`WorkerJobChanged`].
pub type ChangeListener = std::sync::Arc<dyn Fn(WorkerJobChanged) + Send + Sync>;
