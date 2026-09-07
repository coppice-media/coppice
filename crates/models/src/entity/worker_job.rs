use sea_orm::entity::prelude::*;

use crate::shared::enums::WorkerJobStatus;

/// One unit of work a remote worker may claim.
///
/// Distinct from [`super::job`] on purpose: a `jobs` row is *this server's*
/// background work and its runtime owns the task, while a `worker_jobs` row may
/// be executed by a process on another machine. So the row carries what a
/// stranger needs to run it (`kind`, `input`) and what it must be able to do
/// (`requires`), and it survives both a worker disconnect and a server restart.
///
/// Every reader goes through `stump_worker::WorkerJobs`, which owns the state
/// machine and the in-memory view of who holds what; a direct write here would
/// leave the hub lying about an assignment.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "worker_jobs")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	/// The registry id of the work to do (`transcode`, later `align`). Free
	/// text rather than an enum: a job kind is a registry entry in
	/// `stump_worker::kind`, and adding one must not be a migration.
	#[sea_orm(column_type = "Text")]
	pub kind: String,
	/// The kind-specific request, e.g. `stump_worker::kind::TranscodeInput`.
	#[sea_orm(column_type = "Json")]
	pub input: Json,
	/// The capability subset a worker must advertise to be offered this job;
	/// see `stump_worker::kind::satisfies`.
	#[sea_orm(column_type = "Json")]
	pub requires: Json,
	#[sea_orm(column_type = "Text")]
	pub status: WorkerJobStatus,
	/// The `devices.id` of the worker holding the job, when one does. `NULL` on
	/// a job the server ran itself, which is how the console tells the two
	/// apart. Deliberately not a foreign key: a finished job's history outlives
	/// the device row it ran on.
	#[sea_orm(column_type = "Text", nullable)]
	pub worker_id: Option<String>,
	/// Higher runs first. Negative for background work, so an alignment never
	/// competes with something a user is waiting on.
	pub priority: i32,
	/// `0.0..=1.0`.
	pub progress: f64,
	/// The last human-readable line the runner reported.
	#[sea_orm(column_type = "Text", nullable)]
	pub progress_message: Option<String>,
	#[sea_orm(column_type = "Json", nullable)]
	pub result: Option<Json>,
	#[sea_orm(column_type = "Text", nullable)]
	pub error: Option<String>,
	pub created_at: DateTimeWithTimeZone,
	pub updated_at: DateTimeWithTimeZone,
	pub started_at: Option<DateTimeWithTimeZone>,
	pub finished_at: Option<DateTimeWithTimeZone>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
