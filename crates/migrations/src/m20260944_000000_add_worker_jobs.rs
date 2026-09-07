//! `worker_jobs`: the queue a remote worker claims from.
//!
//! Heavy or hardware-bound work (audio transcoding today, forced alignment
//! next) is a row here rather than a `jobs` row, because the two queues answer
//! different questions. A `jobs` row is *this server's* background work and its
//! runtime owns the task; a `worker_jobs` row is work that may be executed by a
//! process on another machine, so the row has to carry what a stranger needs to
//! run it (`kind`, `input`) and what it must be able to do (`requires`), and it
//! has to survive both a worker disconnect and a server restart.
//!
//! `status` is `queued` | `claimed` | `running` | `done` | `failed` |
//! `needs_worker`. The last one is the reason the table exists: a job nobody
//! can run is *shown*, not hidden, so the console can say "no worker advertises
//! `align` right now" instead of silently queueing forever.
//!
//! Statuses and kinds are TEXT rather than integers, matching every other
//! Stump status column (`jobs.status`, `library.status`), so a row read with
//! `sqlite3` says what it is. `kind` is deliberately *not* a closed enum in the
//! schema: a job kind is a registry entry in `stump_worker`
//! (`stump_worker::kind`), and adding `align` must be a registry row, not a
//! migration.
//!
//! Two indexes, both for a query the dispatcher runs on every transition: the
//! queue scan orders by `(status, priority DESC, created_at)`, and reconnect
//! resume reads `(worker_id, status)`. No foreign key on `worker_id`: a
//! finished job's history must outlive the device row it was run by, exactly as
//! `abs_sessions` outlives its media.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(WorkerJobs::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(WorkerJobs::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(WorkerJobs::Kind).text().not_null())
					.col(ColumnDef::new(WorkerJobs::Input).json().not_null())
					.col(ColumnDef::new(WorkerJobs::Requires).json().not_null())
					.col(ColumnDef::new(WorkerJobs::Status).text().not_null())
					.col(ColumnDef::new(WorkerJobs::WorkerId).text())
					.col(
						ColumnDef::new(WorkerJobs::Priority)
							.integer()
							.not_null()
							.default(0),
					)
					.col(
						ColumnDef::new(WorkerJobs::Progress)
							.double()
							.not_null()
							.default(0.0),
					)
					.col(ColumnDef::new(WorkerJobs::ProgressMessage).text())
					.col(ColumnDef::new(WorkerJobs::Result).json())
					.col(ColumnDef::new(WorkerJobs::Error).text())
					.col(
						ColumnDef::new(WorkerJobs::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(WorkerJobs::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(ColumnDef::new(WorkerJobs::StartedAt).timestamp_with_time_zone())
					.col(
						ColumnDef::new(WorkerJobs::FinishedAt).timestamp_with_time_zone(),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("idx-worker-jobs-status-priority")
					.table(WorkerJobs::Table)
					.col(WorkerJobs::Status)
					.col(WorkerJobs::Priority)
					.col(WorkerJobs::CreatedAt)
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("idx-worker-jobs-worker-status")
					.table(WorkerJobs::Table)
					.col(WorkerJobs::WorkerId)
					.col(WorkerJobs::Status)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(Table::drop().table(WorkerJobs::Table).to_owned())
			.await
	}
}

#[derive(DeriveIden)]
enum WorkerJobs {
	#[sea_orm(iden = "worker_jobs")]
	Table,
	Id,
	Kind,
	Input,
	Requires,
	Status,
	WorkerId,
	Priority,
	Progress,
	ProgressMessage,
	Result,
	Error,
	CreatedAt,
	UpdatedAt,
	StartedAt,
	FinishedAt,
}
