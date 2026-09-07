//! `WorkerJobs`: the queue, the dispatcher, and the frame handler.
//!
//! Every transition is one conditional `UPDATE` guarded by the status it is
//! leaving, so two frames that cross on the wire cannot both win and no
//! transaction is needed for a read-then-write. A frame that names a job the
//! sender does not hold, or a status it has already left, is dropped with a
//! log line rather than propagated as an error: a `result` and a `cancel` can
//! genuinely cross, and killing the connection over it would lose the jobs the
//! worker is still holding.
//!
//! Routing, once per dispatch:
//!
//! 1. a connected worker whose capabilities satisfy `requires` → offer it
//!    (`job` frame; the row is assigned but stays `queued` until the worker
//!    answers `claim`);
//! 2. else the kind's local implementation, when it has one → run it here;
//! 3. else `needs_worker`, which is a resting state the console displays.
//!
//! Bytes take one path whoever runs the job: the output lands at
//! [`WorkerJobs::output_path`], either because a remote worker `PUT` it there
//! or because the local runner wrote it there. The caller that awaited the job
//! then publishes that one file, and never learns which happened.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use sea_orm::{
	prelude::*, ActiveValue::Set, DatabaseConnection, Order, QueryOrder, QuerySelect,
};
use serde_json::Value;
use tokio::sync::{oneshot, Mutex};

use models::{entity::worker_job as entity, shared::enums::WorkerJobStatus};

use crate::{
	error::{WorkerError, WorkerResult},
	event::{ChangeListener, WorkerJobChanged},
	hub::{ConnectedWorker, SharedHub, WorkerHub},
	kind::{KindRegistry, LocalJob},
	protocol::{ServerFrame, WorkerFrame},
};

/// The default priority of a job nobody asked for interactively. Negative so
/// an alignment can never elbow ahead of work a user is waiting on.
pub const BACKGROUND_PRIORITY: i32 = -10;

/// The priority of work a request is blocked on.
pub const INTERACTIVE_PRIORITY: i32 = 0;

/// How a job ended, as the caller that awaited it sees it.
#[derive(Debug, Clone, PartialEq)]
pub enum JobOutcome {
	/// The job produced `result`; its bytes, if any, are at `output_path`.
	Done {
		result: Value,
		output_path: PathBuf,
	},
	Failed {
		error: String,
	},
	/// Nobody can run this kind right now. Distinct from `Failed` because the
	/// caller's answer differs: a failure is worth logging, a missing worker is
	/// the documented degraded mode.
	NeedsWorker,
}

/// The queue and its dispatcher.
pub struct WorkerJobs {
	conn: Arc<DatabaseConnection>,
	hub: SharedHub,
	registry: KindRegistry,
	/// Where a job's produced bytes land. A *subdirectory* of the transform
	/// cache, so the cache's LRU sweep — which enumerates only the files
	/// directly in its own directory — can neither serve nor evict an
	/// in-flight upload.
	output_dir: PathBuf,
	on_change: Option<ChangeListener>,
	waiters: Waiters,
}

/// The completion channels `enqueue_and_wait` is blocked on, shared with the
/// tasks spawned for local runs.
type Waiters = Arc<Mutex<HashMap<String, Vec<oneshot::Sender<JobOutcome>>>>>;

impl WorkerJobs {
	#[must_use]
	pub fn new(conn: Arc<DatabaseConnection>, output_dir: PathBuf) -> Self {
		Self {
			conn,
			hub: Arc::new(WorkerHub::new()),
			registry: KindRegistry::new(),
			output_dir,
			on_change: None,
			waiters: Waiters::default(),
		}
	}

	/// Install the job-kind registry (the local fallbacks).
	#[must_use]
	pub fn with_registry(mut self, registry: KindRegistry) -> Self {
		self.registry = registry;
		self
	}

	/// Install the transition listener the host forwards onto its event bus.
	#[must_use]
	pub fn with_change_listener(mut self, listener: ChangeListener) -> Self {
		self.on_change = Some(listener);
		self
	}

	/// The shared hub, for the transport and for the console's `workers` query.
	#[must_use]
	pub fn hub(&self) -> SharedHub {
		self.hub.clone()
	}

	/// Where job `id`'s bytes are, whoever produced them.
	#[must_use]
	pub fn output_path(&self, id: &str) -> PathBuf {
		self.output_dir.join(id)
	}

	/// The path an upload for `id` may be written to by `worker_id`.
	///
	/// The authorisation is the assignment: only the worker the job was offered
	/// to may upload for it, and only while it still holds it. Without that
	/// check any paired worker could overwrite any other job's output.
	pub async fn upload_path(&self, id: &str, worker_id: &str) -> WorkerResult<PathBuf> {
		let job = self
			.get(id)
			.await?
			.ok_or_else(|| WorkerError::NotFound(id.to_string()))?;
		if job.worker_id.as_deref() != Some(worker_id) || !job.status.is_assigned() {
			return Err(WorkerError::NotAssigned {
				job_id: id.to_string(),
				worker_id: worker_id.to_string(),
			});
		}
		tokio::fs::create_dir_all(&self.output_dir).await?;
		Ok(self.output_path(id))
	}

	// -- queries ---------------------------------------------------------

	pub async fn get(&self, id: &str) -> WorkerResult<Option<entity::Model>> {
		Ok(entity::Entity::find_by_id(id)
			.one(self.conn.as_ref())
			.await?)
	}

	/// Jobs, newest first, optionally narrowed to one status.
	pub async fn list(
		&self,
		status: Option<WorkerJobStatus>,
		limit: u64,
	) -> WorkerResult<Vec<entity::Model>> {
		let mut query = entity::Entity::find();
		if let Some(status) = status {
			query = query.filter(entity::Column::Status.eq(status.as_str()));
		}
		Ok(query
			.order_by(entity::Column::CreatedAt, Order::Desc)
			.limit(limit)
			.all(self.conn.as_ref())
			.await?)
	}

	/// Jobs of `kind`, newest first, optionally narrowed to one status.
	///
	/// `list`'s bound is over every kind at once, which makes it unusable for
	/// "the newest finished job of *this* kind": a server doing audio work
	/// would bury one finished solve under a page of transcodes. Filtering in
	/// SQL is what makes the limit mean something.
	pub async fn list_of_kind(
		&self,
		kind: &str,
		status: Option<WorkerJobStatus>,
		limit: u64,
	) -> WorkerResult<Vec<entity::Model>> {
		let mut query = entity::Entity::find().filter(entity::Column::Kind.eq(kind));
		if let Some(status) = status {
			query = query.filter(entity::Column::Status.eq(status.as_str()));
		}
		Ok(query
			.order_by(entity::Column::CreatedAt, Order::Desc)
			.limit(limit)
			.all(self.conn.as_ref())
			.await?)
	}

	/// Every job of `kind` that has not reached a terminal status, oldest
	/// first.
	///
	/// The dedupe primitive: a caller that must not enqueue a second job for
	/// something already in flight reads this and decides for itself what
	/// "already in flight" means for its kind. The policy stays with the
	/// caller — a transcode of the same track is harmless, a second challenge
	/// solve for the same source is not — but the query does not, because
	/// assembling it from `list` is four round-trips and races its own limit.
	pub async fn active_of_kind(&self, kind: &str) -> WorkerResult<Vec<entity::Model>> {
		Ok(entity::Entity::find()
			.filter(entity::Column::Kind.eq(kind))
			.filter(entity::Column::Status.is_in([
				WorkerJobStatus::Queued.as_str(),
				WorkerJobStatus::Claimed.as_str(),
				WorkerJobStatus::Running.as_str(),
				WorkerJobStatus::NeedsWorker.as_str(),
			]))
			.order_by(entity::Column::CreatedAt, Order::Asc)
			.all(self.conn.as_ref())
			.await?)
	}

	// -- lifecycle -------------------------------------------------------

	/// Create a job and route it. Returns the row as it stands after routing,
	/// so a caller that only wants to know "did this land in `needs_worker`"
	/// does not have to re-read it.
	pub async fn enqueue(
		&self,
		kind: &str,
		input: Value,
		requires: Value,
		priority: i32,
	) -> WorkerResult<entity::Model> {
		let now = Utc::now().fixed_offset();
		let job = entity::ActiveModel {
			id: Set(uuid::Uuid::new_v4().to_string()),
			kind: Set(kind.to_string()),
			input: Set(input),
			requires: Set(requires),
			status: Set(WorkerJobStatus::Queued),
			worker_id: Set(None),
			priority: Set(priority),
			progress: Set(0.0),
			progress_message: Set(None),
			result: Set(None),
			error: Set(None),
			created_at: Set(now),
			updated_at: Set(now),
			started_at: Set(None),
			finished_at: Set(None),
		}
		.insert(self.conn.as_ref())
		.await?;

		self.announce(&job);
		self.dispatch(&job).await
	}

	/// Enqueue and wait for the job to finish.
	///
	/// The waiter is registered *before* the job is routed, because a local
	/// fallback can finish before this function returns from `enqueue`.
	/// `needs_worker` resolves immediately rather than waiting out the
	/// timeout: nothing is going to happen.
	pub async fn enqueue_and_wait(
		&self,
		kind: &str,
		input: Value,
		requires: Value,
		priority: i32,
		timeout: std::time::Duration,
	) -> WorkerResult<JobOutcome> {
		let now = Utc::now().fixed_offset();
		let id = uuid::Uuid::new_v4().to_string();
		let (tx, rx) = oneshot::channel();
		self.waiters
			.lock()
			.await
			.entry(id.clone())
			.or_default()
			.push(tx);

		let job = entity::ActiveModel {
			id: Set(id.clone()),
			kind: Set(kind.to_string()),
			input: Set(input),
			requires: Set(requires),
			status: Set(WorkerJobStatus::Queued),
			worker_id: Set(None),
			priority: Set(priority),
			progress: Set(0.0),
			progress_message: Set(None),
			result: Set(None),
			error: Set(None),
			created_at: Set(now),
			updated_at: Set(now),
			started_at: Set(None),
			finished_at: Set(None),
		}
		.insert(self.conn.as_ref())
		.await?;
		self.announce(&job);

		let routed = self.dispatch(&job).await?;
		if routed.status == WorkerJobStatus::NeedsWorker {
			self.waiters.lock().await.remove(&id);
			return Ok(JobOutcome::NeedsWorker);
		}

		match tokio::time::timeout(timeout, rx).await {
			Ok(Ok(outcome)) => Ok(outcome),
			// The sender was dropped without an outcome, which only happens if
			// the waiter map was cleared out from under us.
			Ok(Err(_)) => Err(WorkerError::Timeout(id)),
			Err(_) => {
				self.waiters.lock().await.remove(&id);
				Err(WorkerError::Timeout(id))
			},
		}
	}

	/// Offer the job to a worker, run it locally, or park it in `needs_worker`.
	async fn dispatch(&self, job: &entity::Model) -> WorkerResult<entity::Model> {
		if let Some(worker) = self.hub.capable_worker(&job.requires).await {
			if self.offer(job, &worker).await? {
				let mut offered = job.clone();
				offered.worker_id = Some(worker.device_id);
				return Ok(offered);
			}
		}

		if let Some(runner) = self.registry.local(&job.kind) {
			self.run_locally(job.clone(), runner);
			let mut running = job.clone();
			running.status = WorkerJobStatus::Running;
			return Ok(running);
		}

		self.transition(
			&job.id,
			&[WorkerJobStatus::Queued],
			WorkerJobStatus::NeedsWorker,
			Transition::default(),
		)
		.await?
		.ok_or_else(|| WorkerError::NotFound(job.id.clone()))
	}

	/// Assign the job to `worker` and push the offer. `false` when the socket
	/// closed between selection and send, which the caller treats as "no
	/// worker".
	async fn offer(
		&self,
		job: &entity::Model,
		worker: &ConnectedWorker,
	) -> WorkerResult<bool> {
		let frame = ServerFrame::Job {
			id: job.id.clone(),
			kind: job.kind.clone(),
			input: job.input.clone(),
			priority: job.priority,
		};
		if !self.hub.send(&worker.device_id, &frame).await {
			return Ok(false);
		}
		let updated = self
			.transition(
				&job.id,
				&[WorkerJobStatus::Queued, WorkerJobStatus::NeedsWorker],
				WorkerJobStatus::Queued,
				Transition {
					worker_id: Some(Some(worker.device_id.clone())),
					..Transition::default()
				},
			)
			.await?;
		if let Some(job) = updated.as_ref() {
			tracing::debug!(
				job_id = %job.id,
				kind = %job.kind,
				worker = %worker.device_id,
				"Offered a worker job"
			);
		}
		Ok(updated.is_some())
	}

	/// Run a job in this process. Spawned rather than awaited so `enqueue`
	/// answers as soon as the row is routed; the waiter is what the caller
	/// blocks on.
	fn run_locally(&self, job: entity::Model, runner: Arc<dyn crate::kind::LocalRunner>) {
		let conn = self.conn.clone();
		let output_path = self.output_path(&job.id);
		let output_dir = self.output_dir.clone();
		let on_change = self.on_change.clone();
		let waiters = self.waiters.clone();
		tokio::spawn(async move {
			let local = LocalJob {
				id: job.id.clone(),
				kind: job.kind.clone(),
				input: job.input.clone(),
				output_path,
			};
			if let Err(error) = tokio::fs::create_dir_all(&output_dir).await {
				tracing::error!(?error, "Failed to create the worker output directory");
			}
			// A spawned task cannot borrow the service, so the two shared
			// writes are free functions rather than methods: duplicating the
			// SQL is how the local and the remote path would drift.
			match update_status(
				conn.as_ref(),
				&job.id,
				&[WorkerJobStatus::Queued],
				WorkerJobStatus::Running,
				Transition {
					started_at: true,
					..Transition::default()
				},
			)
			.await
			{
				Ok(Some(row)) => announce_with(on_change.as_ref(), &row),
				// Cancelled between routing and starting.
				Ok(None) => return,
				Err(error) => {
					tracing::error!(?error, "Failed to mark a local worker job running");
					return;
				},
			}

			tracing::debug!(
				job_id = %job.id,
				kind = %job.kind,
				"Running a worker job locally"
			);
			let outcome = runner.run(local).await;
			match finish(conn.as_ref(), &job.id, outcome).await {
				Ok(Some(row)) => {
					announce_with(on_change.as_ref(), &row);
					wake_waiters(&waiters, &row, &output_dir).await;
				},
				Ok(None) => {},
				Err(error) => {
					tracing::error!(?error, "Failed to finish a local worker job");
				},
			}
		});
	}

	// -- transport callbacks ---------------------------------------------

	/// Accept a worker's `hello`: register the connection and immediately
	/// re-offer everything it still holds.
	///
	/// Every transport calls exactly this — the axum route in `apps/server` and
	/// the crate's own test alike — so "what happens on hello" is written once
	/// and a transport is left with nothing but socket plumbing. Anything but a
	/// `hello` here is a protocol error: the hub cannot address a worker whose
	/// capabilities it does not know.
	pub async fn attach_worker(
		&self,
		device_id: &str,
		device_name: &str,
		hello: WorkerFrame,
	) -> WorkerResult<(crate::hub::Outbound, u64)> {
		let WorkerFrame::Hello {
			capabilities,
			name,
			version,
		} = hello
		else {
			return Err(WorkerError::Invalid(
				"the first frame on a worker socket must be `hello`".to_string(),
			));
		};
		let (outbound, epoch) = self
			.hub
			.attach(
				device_id.to_string(),
				name.unwrap_or_else(|| device_name.to_string()),
				version,
				capabilities,
			)
			.await;
		self.on_worker_connected(device_id).await?;
		Ok((outbound, epoch))
	}

	/// Release a worker's connection. A no-op when the connection was already
	/// replaced by a newer one, so a slow teardown cannot evict a live worker.
	pub async fn detach_worker(&self, device_id: &str, epoch: u64) -> WorkerResult<()> {
		if self.hub.detach(device_id, epoch).await {
			self.on_worker_disconnected(device_id).await?;
		}
		Ok(())
	}

	/// A worker completed its `hello`: re-offer what it still holds, then see
	/// whether anything parked in `needs_worker` is now runnable.
	pub async fn on_worker_connected(&self, device_id: &str) -> WorkerResult<usize> {
		let assigned = entity::Entity::find()
			.filter(entity::Column::WorkerId.eq(device_id))
			.filter(entity::Column::Status.is_in([
				WorkerJobStatus::Queued.as_str(),
				WorkerJobStatus::Claimed.as_str(),
				WorkerJobStatus::Running.as_str(),
			]))
			.order_by(entity::Column::CreatedAt, Order::Asc)
			.all(self.conn.as_ref())
			.await?;

		let mut resumed = 0;
		for job in &assigned {
			let frame = ServerFrame::Job {
				id: job.id.clone(),
				kind: job.kind.clone(),
				input: job.input.clone(),
				priority: job.priority,
			};
			if self.hub.send(device_id, &frame).await {
				resumed += 1;
			}
		}
		if resumed > 0 {
			tracing::info!(worker = %device_id, resumed, "Resumed claimed worker jobs");
		}

		let parked = entity::Entity::find()
			.filter(entity::Column::Status.eq(WorkerJobStatus::NeedsWorker.as_str()))
			.order_by(entity::Column::Priority, Order::Desc)
			.order_by(entity::Column::CreatedAt, Order::Asc)
			.all(self.conn.as_ref())
			.await?;
		for job in parked {
			if self.hub.capable_worker(&job.requires).await.is_none() {
				continue;
			}
			// Back to `queued` first, so `offer` has a status to leave.
			if let Some(requeued) = self
				.transition(
					&job.id,
					&[WorkerJobStatus::NeedsWorker],
					WorkerJobStatus::Queued,
					Transition::default(),
				)
				.await?
			{
				self.dispatch(&requeued).await?;
			}
		}

		Ok(resumed)
	}

	/// A worker's socket closed. An *offered but unclaimed* job goes back on
	/// the queue immediately — nobody has started it. A claimed or running job
	/// stays assigned, because the worker may be mid-encode and reconnecting;
	/// it is resumed by id on the next `hello`.
	pub async fn on_worker_disconnected(&self, device_id: &str) -> WorkerResult<()> {
		let unclaimed = entity::Entity::find()
			.filter(entity::Column::WorkerId.eq(device_id))
			.filter(entity::Column::Status.eq(WorkerJobStatus::Queued.as_str()))
			.all(self.conn.as_ref())
			.await?;
		for job in unclaimed {
			if let Some(released) = self
				.transition(
					&job.id,
					&[WorkerJobStatus::Queued],
					WorkerJobStatus::Queued,
					Transition {
						worker_id: Some(None),
						..Transition::default()
					},
				)
				.await?
			{
				self.dispatch(&released).await?;
			}
		}
		Ok(())
	}

	/// Apply one frame from `device_id`.
	///
	/// `hello` never reaches here: the transport handles it, because attaching
	/// the connection is what makes the sender addressable in the first place.
	pub async fn handle_frame(
		&self,
		device_id: &str,
		frame: WorkerFrame,
	) -> WorkerResult<()> {
		self.hub.touch(device_id).await;
		match frame {
			WorkerFrame::Hello { .. } => {
				tracing::warn!(
					worker = %device_id,
					"Ignoring a second hello on an established worker socket"
				);
				Ok(())
			},
			WorkerFrame::Claim { job_id } => {
				self.apply(
					device_id,
					&job_id,
					&[WorkerJobStatus::Queued],
					WorkerJobStatus::Claimed,
					Transition {
						started_at: true,
						..Transition::default()
					},
				)
				.await
			},
			WorkerFrame::Progress {
				job_id,
				fraction,
				message,
			} => {
				self.apply(
					device_id,
					&job_id,
					&[WorkerJobStatus::Claimed, WorkerJobStatus::Running],
					WorkerJobStatus::Running,
					Transition {
						progress: Some(fraction.clamp(0.0, 1.0)),
						message: Some(message),
						..Transition::default()
					},
				)
				.await
			},
			WorkerFrame::Result { job_id, output } => {
				self.complete(device_id, &job_id, Ok(output)).await
			},
			WorkerFrame::Fail { job_id, error } => {
				self.complete(device_id, &job_id, Err(error)).await
			},
		}
	}

	/// Give up on a job: tell the holder to stop, mark it failed, and drop any
	/// bytes it staged.
	///
	/// Cancellation is a terminal *failure*, not a seventh status. The status
	/// set is the documented contract, and a cancelled job is exactly a job
	/// that will not produce its output; the reason is in `error`.
	pub async fn cancel(&self, id: &str, reason: &str) -> WorkerResult<entity::Model> {
		let job = self
			.get(id)
			.await?
			.ok_or_else(|| WorkerError::NotFound(id.to_string()))?;
		if job.status.is_terminal() {
			return Ok(job);
		}
		if let Some(worker_id) = job.worker_id.as_deref() {
			self.hub
				.send(
					worker_id,
					&ServerFrame::Cancel {
						job_id: id.to_string(),
					},
				)
				.await;
		}
		let cancelled = self
			.transition(
				id,
				&[
					WorkerJobStatus::Queued,
					WorkerJobStatus::Claimed,
					WorkerJobStatus::Running,
					WorkerJobStatus::NeedsWorker,
				],
				WorkerJobStatus::Failed,
				Transition {
					error: Some(reason.to_string()),
					finished_at: true,
					..Transition::default()
				},
			)
			.await?
			.unwrap_or(job);
		let _ = tokio::fs::remove_file(self.output_path(id)).await;
		self.wake(&cancelled).await;
		Ok(cancelled)
	}

	// -- internals -------------------------------------------------------

	async fn apply(
		&self,
		device_id: &str,
		job_id: &str,
		from: &[WorkerJobStatus],
		to: WorkerJobStatus,
		change: Transition,
	) -> WorkerResult<()> {
		self.assert_holder(device_id, job_id).await?;
		match self.transition(job_id, from, to, change).await? {
			Some(job) => {
				self.announce(&job);
				Ok(())
			},
			None => {
				tracing::debug!(
					job_id,
					worker = %device_id,
					?to,
					"Dropped a worker frame for a job that had already moved on"
				);
				Ok(())
			},
		}
	}

	async fn complete(
		&self,
		device_id: &str,
		job_id: &str,
		outcome: Result<Value, String>,
	) -> WorkerResult<()> {
		self.assert_holder(device_id, job_id).await?;
		let failed = outcome.is_err();
		let Some(job) = finish(self.conn.as_ref(), job_id, outcome).await? else {
			tracing::debug!(
				job_id,
				worker = %device_id,
				"Dropped a terminal frame for a job that had already finished"
			);
			return Ok(());
		};
		if failed {
			let _ = tokio::fs::remove_file(self.output_path(job_id)).await;
		}
		self.announce(&job);
		self.wake(&job).await;
		Ok(())
	}

	async fn assert_holder(&self, device_id: &str, job_id: &str) -> WorkerResult<()> {
		let job = self
			.get(job_id)
			.await?
			.ok_or_else(|| WorkerError::NotFound(job_id.to_string()))?;
		if job.worker_id.as_deref() == Some(device_id) {
			return Ok(());
		}
		Err(WorkerError::NotAssigned {
			job_id: job_id.to_string(),
			worker_id: device_id.to_string(),
		})
	}

	async fn transition(
		&self,
		id: &str,
		from: &[WorkerJobStatus],
		to: WorkerJobStatus,
		change: Transition,
	) -> WorkerResult<Option<entity::Model>> {
		update_status(self.conn.as_ref(), id, from, to, change).await
	}

	fn announce(&self, job: &entity::Model) {
		announce_with(self.on_change.as_ref(), job);
	}

	async fn wake(&self, job: &entity::Model) {
		wake_waiters(&self.waiters, job, &self.output_dir).await;
	}
}

/// The columns one transition writes beyond `status` and `updated_at`.
///
/// `Option<Option<_>>` is the difference between "leave it alone" and "set it
/// to NULL", which the release-on-disconnect path needs for `worker_id`.
#[derive(Default)]
struct Transition {
	worker_id: Option<Option<String>>,
	progress: Option<f64>,
	message: Option<Option<String>>,
	error: Option<String>,
	started_at: bool,
	finished_at: bool,
}

/// One guarded status write. Returns the row when it moved, `None` when the
/// guard did not match — the caller's signal that a frame lost a race.
async fn update_status<C: ConnectionTrait>(
	conn: &C,
	id: &str,
	from: &[WorkerJobStatus],
	to: WorkerJobStatus,
	change: Transition,
) -> WorkerResult<Option<entity::Model>> {
	let now = Utc::now().fixed_offset();
	let mut update = entity::Entity::update_many()
		.filter(entity::Column::Id.eq(id))
		.filter(
			entity::Column::Status.is_in(
				from.iter()
					.map(|status| status.as_str())
					.collect::<Vec<_>>(),
			),
		)
		.col_expr(entity::Column::Status, Expr::value(to.as_str()))
		.col_expr(entity::Column::UpdatedAt, Expr::value(now));
	if let Some(worker_id) = change.worker_id {
		update = update.col_expr(entity::Column::WorkerId, Expr::value(worker_id));
	}
	if let Some(progress) = change.progress {
		update = update.col_expr(entity::Column::Progress, Expr::value(progress));
	}
	if let Some(message) = change.message {
		update = update.col_expr(entity::Column::ProgressMessage, Expr::value(message));
	}
	if let Some(error) = change.error {
		update = update.col_expr(entity::Column::Error, Expr::value(Some(error)));
	}
	if change.started_at {
		update = update.col_expr(entity::Column::StartedAt, Expr::value(Some(now)));
	}
	if change.finished_at {
		update = update.col_expr(entity::Column::FinishedAt, Expr::value(Some(now)));
	}

	if update.exec(conn).await?.rows_affected == 0 {
		return Ok(None);
	}
	Ok(entity::Entity::find_by_id(id).one(conn).await?)
}

/// The terminal write shared by the remote and the local path.
async fn finish<C: ConnectionTrait>(
	conn: &C,
	id: &str,
	outcome: Result<Value, String>,
) -> WorkerResult<Option<entity::Model>> {
	let from = [
		WorkerJobStatus::Queued,
		WorkerJobStatus::Claimed,
		WorkerJobStatus::Running,
	];
	let now = Utc::now().fixed_offset();
	let (to, result, error, progress) = match outcome {
		Ok(result) => (WorkerJobStatus::Done, Some(result), None, 1.0),
		Err(error) => (WorkerJobStatus::Failed, None, Some(error), 0.0),
	};
	let mut update = entity::Entity::update_many()
		.filter(entity::Column::Id.eq(id))
		.filter(
			entity::Column::Status.is_in(
				from.iter()
					.map(|status| status.as_str())
					.collect::<Vec<_>>(),
			),
		)
		.col_expr(entity::Column::Status, Expr::value(to.as_str()))
		.col_expr(entity::Column::UpdatedAt, Expr::value(now))
		.col_expr(entity::Column::FinishedAt, Expr::value(Some(now)))
		.col_expr(entity::Column::Result, Expr::value(result))
		.col_expr(entity::Column::Error, Expr::value(error));
	if progress > 0.0 {
		update = update.col_expr(entity::Column::Progress, Expr::value(progress));
	}
	if update.exec(conn).await?.rows_affected == 0 {
		return Ok(None);
	}
	Ok(entity::Entity::find_by_id(id).one(conn).await?)
}

fn announce_with(listener: Option<&ChangeListener>, job: &entity::Model) {
	if let Some(listener) = listener {
		listener(WorkerJobChanged::from(job));
	}
}

/// Resolve every `enqueue_and_wait` blocked on `job`. Called from both the
/// frame path and the spawned local run, which is why it takes the map rather
/// than the service.
async fn wake_waiters(waiters: &Waiters, job: &entity::Model, output_dir: &Path) {
	let Some(blocked) = waiters.lock().await.remove(&job.id) else {
		return;
	};
	let outcome = outcome_of(job, output_dir);
	for waiter in blocked {
		let _ = waiter.send(outcome.clone());
	}
}

fn outcome_of(job: &entity::Model, output_dir: &Path) -> JobOutcome {
	match job.status {
		WorkerJobStatus::Done => JobOutcome::Done {
			result: job.result.clone().unwrap_or(Value::Null),
			output_path: output_dir.join(&job.id),
		},
		WorkerJobStatus::NeedsWorker => JobOutcome::NeedsWorker,
		_ => JobOutcome::Failed {
			error: job
				.error
				.clone()
				.unwrap_or_else(|| job.status.as_str().to_string()),
		},
	}
}
