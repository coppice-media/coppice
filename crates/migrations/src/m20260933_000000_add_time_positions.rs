//! Time-addressed reading positions on the unified reading-state tables.
//!
//! A page ordinal cannot carry an audio position: an audiobook has no pages,
//! and rounding a millisecond offset into `progression` alone loses the only
//! thing a listener needs to resume. So `reading_heads` and its provenance
//! stream gain `position_ms` (milliseconds from the start of the
//! *publication*, matching `media_audio.duration_ms`) plus `track_index`, the
//! file the position fell in for a multi-file book.
//!
//! `track_index` is stored beside `position_ms` rather than derived from
//! `media_audio_tracks.start_offset_ms` because it is what the *client*
//! reported: a device that resumes on a re-scanned book must be able to say
//! which file it was in, even if the track split has since changed underneath
//! it. It is never a page ordinal, which is why it is a separate column from
//! `page` instead of reusing it.
//!
//! Every column is nullable with no default, so existing rows are untouched
//! and a page- or locator-addressed protocol keeps writing exactly what it did
//! before. `reading_sessions.end_position_ms` is the session-history twin of
//! `end_page`/`end_locator`, so listening statistics can report where a
//! session stopped.
//!
//! One `alter_table` per column: SQLite only accepts a single `ADD COLUMN` per
//! `ALTER TABLE` statement.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(ReadingHeads::Table)
					.add_column(
						ColumnDef::new(ReadingHeads::PositionMs)
							.big_integer()
							.null(),
					)
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(ReadingHeads::Table)
					.add_column(ColumnDef::new(ReadingHeads::TrackIndex).integer().null())
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(ReadingHeadEvents::Table)
					.add_column(
						ColumnDef::new(ReadingHeadEvents::PositionMs)
							.big_integer()
							.null(),
					)
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(ReadingHeadEvents::Table)
					.add_column(
						ColumnDef::new(ReadingHeadEvents::TrackIndex)
							.integer()
							.null(),
					)
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(ReadingSessions::Table)
					.add_column(
						ColumnDef::new(ReadingSessions::EndPositionMs)
							.big_integer()
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
					.table(ReadingSessions::Table)
					.drop_column(ReadingSessions::EndPositionMs)
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(ReadingHeadEvents::Table)
					.drop_column(ReadingHeadEvents::TrackIndex)
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(ReadingHeadEvents::Table)
					.drop_column(ReadingHeadEvents::PositionMs)
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(ReadingHeads::Table)
					.drop_column(ReadingHeads::TrackIndex)
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(ReadingHeads::Table)
					.drop_column(ReadingHeads::PositionMs)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum ReadingHeads {
	Table,
	PositionMs,
	TrackIndex,
}

#[derive(DeriveIden)]
enum ReadingHeadEvents {
	Table,
	PositionMs,
	TrackIndex,
}

#[derive(DeriveIden)]
enum ReadingSessions {
	Table,
	EndPositionMs,
}
