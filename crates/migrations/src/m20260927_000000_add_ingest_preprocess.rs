//! `ingest_drop_items.preprocessed_at` — when the optional ingest preprocess
//! hook last completed successfully for this item.
//!
//! The hook rewrites (or replaces at the same path) the staged file before
//! analysis, so the item's `source_sha256` and `byte_size` are updated from the
//! post-hook bytes. That makes every other column on the row useless as an
//! "already preprocessed" marker: a rewritten item is byte-for-byte
//! indistinguishable from a fresh upload of the rewritten file, and `status`,
//! `revision`, and `analysis_job_id` all move again on every retry or
//! re-analysis. A dedicated timestamp is the only thing that keeps the hook to
//! one successful run per item.
//!
//! `NULL` means "never preprocessed successfully" and is also what a failed
//! hook leaves behind, so an operator can fix the command and retry.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(IngestDropItems::Table)
					.add_column(
						ColumnDef::new(IngestDropItems::PreprocessedAt)
							.timestamp_with_time_zone()
							.null(),
					)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(IngestDropItems::Table)
					.drop_column(IngestDropItems::PreprocessedAt)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum IngestDropItems {
	Table,
	PreprocessedAt,
}
