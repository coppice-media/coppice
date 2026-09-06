//! `media_audio_chapters`: the chapter marks of one audiobook.
//!
//! Chapters are publication-relative time marks, not files: an embedded `chpl`
//! atom can put four chapters inside a single `.m4b`, and a folder book can
//! have one chapter per file. Keeping them in their own table (rather than
//! folding them into `media_audio_tracks`) is what lets both shapes coexist,
//! and it keeps the mark independent of the file layout so a re-mux does not
//! invalidate the chapter list.
//!
//! `start_ms`/`end_ms` are offsets from the start of the *publication*, the
//! same unit as a `reading_heads.position_ms`, so a "resume at chapter" jump is
//! a comparison and never a per-track conversion. `end_ms` is nullable: most
//! container formats only carry start marks and the last chapter simply runs to
//! `media_audio.duration_ms`.
//!
//! `index` is the 0-based ordinal of the chapter; it is a SQL keyword, and
//! `DeriveIden` quotes it. The unique index on `(media_id, index)` makes a
//! re-probe idempotent.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(MediaAudioChapters::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(MediaAudioChapters::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(MediaAudioChapters::MediaId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(MediaAudioChapters::Index)
							.integer()
							.not_null(),
					)
					.col(ColumnDef::new(MediaAudioChapters::Title).text().null())
					.col(
						ColumnDef::new(MediaAudioChapters::StartMs)
							.big_integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(MediaAudioChapters::EndMs)
							.big_integer()
							.null(),
					)
					.foreign_key(
						ForeignKey::create()
							.from(MediaAudioChapters::Table, MediaAudioChapters::MediaId)
							.to(Media::Table, Media::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("idx_media_audio_chapters_media_index")
					.table(MediaAudioChapters::Table)
					.col(MediaAudioChapters::MediaId)
					.col(MediaAudioChapters::Index)
					.unique()
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(
				Table::drop()
					.table(MediaAudioChapters::Table)
					.if_exists()
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum MediaAudioChapters {
	Table,
	Id,
	MediaId,
	Index,
	Title,
	StartMs,
	EndMs,
}

#[derive(DeriveIden)]
enum Media {
	Table,
	Id,
}
