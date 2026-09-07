use async_graphql::{Context, Object, Result};
use models::shared::enums::UserPermission;
use stump_worker::WorkerJobStatus;

use crate::{
	data::CoreContext,
	guard::PermissionGuard,
	object::worker::{Worker, WorkerJob},
};

/// The most jobs one page asks for. The Workers page is a live list, not an
/// archive; a client that wants history filters by status.
const MAX_JOBS: u64 = 200;

#[derive(Default)]
pub struct WorkerQuery;

#[Object]
impl WorkerQuery {
	/// The workers currently holding a socket, with what each advertises.
	///
	/// Read from the hub, not the database: "connected" is a property of this
	/// process, and a `devices` row cannot answer it. A worker that is paired
	/// but offline is a device, and the Devices page is where it shows.
	#[graphql(guard = "PermissionGuard::new(&[UserPermission::ReadJobs])")]
	async fn workers(&self, ctx: &Context<'_>) -> Result<Vec<Worker>> {
		let core = ctx.data::<CoreContext>()?;
		Ok(core
			.worker_jobs()
			.hub()
			.connected()
			.await
			.into_iter()
			.map(Worker::from)
			.collect())
	}

	/// Worker jobs, newest first, optionally narrowed to one status.
	///
	/// `NEEDS_WORKER` is the interesting filter: it is the list of work this
	/// server has been asked for and cannot do, which is the whole reason the
	/// status exists.
	#[graphql(guard = "PermissionGuard::new(&[UserPermission::ReadJobs])")]
	async fn worker_jobs(
		&self,
		ctx: &Context<'_>,
		status: Option<WorkerJobStatus>,
		limit: Option<u64>,
	) -> Result<Vec<WorkerJob>> {
		let core = ctx.data::<CoreContext>()?;
		let limit = limit.unwrap_or(50).clamp(1, MAX_JOBS);
		Ok(core
			.worker_jobs()
			.list(status, limit)
			.await?
			.into_iter()
			.map(WorkerJob::from)
			.collect())
	}
}
