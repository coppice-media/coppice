//! `ingest_drop_items.{drop_group_id, sidecar_paths, audio_analysis}` — the
//! three things a drop item needs once one dropped file can produce several
//! items.
//!
//! **`drop_group_id`** is the identity of the *delivery*. A `.rar` holding an
//! EPUB, a MOBI and a cover explodes into two items, and an audiobook `.rar`
//! dropped beside its `Ebooks.rar` produces items nothing else on the row
//! relates: `relative_path` is the path inside the container (often the same
//! for every member), `source_sha256` is per-file by definition, and
//! `idempotency_key` belongs to one upload request. Without a group id the
//! editor cannot show "these three arrived together", and the ebook and the
//! audiobook of one book — the strongest edition-pair signal ingest ever sees —
//! are two unrelated rows. `NULL` is a drop of one file, which is most of them.
//!
//! **`sidecar_paths`** is the JSON array of staged files that describe an item
//! without being it: `cover.jpg`, an `.nfo`, a `.cue`, and the original MP3
//! parts kept beside an assembled M4B. They cannot be items (a cover is not a
//! book) and they cannot be discarded (the cover is the item's artwork), so
//! they are owned by the item they describe and removed with it.
//!
//! **`audio_analysis`** is the probe result: duration, codec, per-track
//! offsets, chapter marks and their provenance, tags, and the assembled M4B
//! when one was produced. Demuxing is the expensive part of an audiobook —
//! symphonia walks every container, and a 62-part folder book means 62 walks —
//! so the answer is persisted once per item instead of recomputed for every
//! screen that shows a chapter list. `NULL` for every non-audio item and for an
//! audio item whose analysis has not run yet.
//!
//! One `ALTER TABLE` per column: SQLite adds columns one at a time, and a
//! multi-column `add_column` chain is silently truncated to the first.

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
						ColumnDef::new(IngestDropItems::DropGroupId).text().null(),
					)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(IngestDropItems::Table)
					.add_column(
						ColumnDef::new(IngestDropItems::SidecarPaths).json().null(),
					)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(IngestDropItems::Table)
					.add_column(
						ColumnDef::new(IngestDropItems::AudioAnalysis).json().null(),
					)
					.to_owned(),
			)
			.await?;
		// The sibling strip is a lookup by group on every drop-item page, and
		// the group is sparse (most items have none), so the index is small
		// and the scan it replaces is over the whole table.
		manager
			.create_index(
				Index::create()
					.name("idx_ingest_drop_items_drop_group_id")
					.table(IngestDropItems::Table)
					.col(IngestDropItems::DropGroupId)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_index(
				Index::drop()
					.name("idx_ingest_drop_items_drop_group_id")
					.table(IngestDropItems::Table)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(IngestDropItems::Table)
					.drop_column(IngestDropItems::AudioAnalysis)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(IngestDropItems::Table)
					.drop_column(IngestDropItems::SidecarPaths)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(IngestDropItems::Table)
					.drop_column(IngestDropItems::DropGroupId)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum IngestDropItems {
	Table,
	DropGroupId,
	SidecarPaths,
	AudioAnalysis,
}
