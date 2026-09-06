//! `bookmarks.position_ms`: a bookmark placed in an audiobook.
//!
//! A bookmark already stores a `page` and a Readium `locator`; neither can
//! address a moment in a recording. `position_ms` is milliseconds from the
//! start of the *publication* — the same unit as `media_audio.duration_ms` and
//! `reading_heads.position_ms` — so a bookmark and a reading head are
//! comparable without a conversion.
//!
//! Nullable with no default: every existing bookmark stays exactly as it was,
//! and a page- or locator-addressed bookmark never writes this column.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(Bookmarks::Table)
					.add_column(
						ColumnDef::new(Bookmarks::PositionMs).big_integer().null(),
					)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(Bookmarks::Table)
					.drop_column(Bookmarks::PositionMs)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum Bookmarks {
	Table,
	PositionMs,
}
