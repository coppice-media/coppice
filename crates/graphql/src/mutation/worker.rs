use async_graphql::{Context, Object, Result};
use models::shared::enums::UserPermission;

use crate::{data::CoreContext, guard::PermissionGuard, object::worker::WorkerJob};

#[derive(Default)]
pub struct WorkerMutation;

#[Object]
impl WorkerMutation {
	/// Stop a worker job.
	///
	/// The holder is told to stop and the row lands `FAILED` with the
	/// canceller's name in `error` — cancellation is a terminal failure, not a
	/// seventh status, because the status set is the protocol's contract and a
	/// cancelled job is exactly a job that will not produce its output.
	/// Cancelling a finished job is a no-op that returns the row unchanged.
	#[graphql(guard = "PermissionGuard::new(&[UserPermission::ManageJobs])")]
	async fn cancel_worker_job(
		&self,
		ctx: &Context<'_>,
		id: String,
	) -> Result<WorkerJob> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let reason = format!("cancelled by {}", user.username);
		Ok(WorkerJob::from(
			core.worker_jobs().cancel(&id, &reason).await?,
		))
	}
}
