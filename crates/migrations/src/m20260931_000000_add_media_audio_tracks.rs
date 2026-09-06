//! `media_audio_tracks`: the ordered files that make up one audiobook.
//!
//! A folder audiobook is dozens of `.m4b`/`.mp3` files that together form a
//! single publication, so the files cannot be `media` rows (that would shatter
//! one book into dozens of library entries) and cannot be a JSON blob on
//! `media_audio` (a range request has to resolve a byte offset to exactly one
//! file, which is a lookup, not a scan).
//!
//! `start_offset_ms` is the running sum of the preceding `duration_ms` values,
//! stored rather than computed so a publication-relative position (the unit a
//! reading head is expressed in) maps to a file with one indexed read instead
//! of a prefix scan. `byte_size` is kept so a range request can be answered
//! without stat-ing the file.
//!
//! `index` is the 0-based position of the file within the publication and is
//! deliberately named after the concept even though it is a SQL keyword —
//! `DeriveIden` quotes every identifier, so the emitted DDL and DML are valid.
//! The unique index on `(media_id, index)` is what makes a re-scan idempotent:
//! a track number can only be claimed once per book.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(MediaAudioTracks::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(MediaAudioTracks::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(MediaAudioTracks::MediaId).text().not_null())
					.col(ColumnDef::new(MediaAudioTracks::Index).integer().not_null())
					.col(ColumnDef::new(MediaAudioTracks::Path).text().not_null())
					.col(
						ColumnDef::new(MediaAudioTracks::DurationMs)
							.big_integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(MediaAudioTracks::StartOffsetMs)
							.big_integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(MediaAudioTracks::ByteSize)
							.big_integer()
							.not_null(),
					)
					.col(ColumnDef::new(MediaAudioTracks::Mime).text().not_null())
					.foreign_key(
						ForeignKey::create()
							.from(MediaAudioTracks::Table, MediaAudioTracks::MediaId)
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
					.name("idx_media_audio_tracks_media_index")
					.table(MediaAudioTracks::Table)
					.col(MediaAudioTracks::MediaId)
					.col(MediaAudioTracks::Index)
					.unique()
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(
				Table::drop()
					.table(MediaAudioTracks::Table)
					.if_exists()
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum MediaAudioTracks {
	Table,
	Id,
	MediaId,
	Index,
	Path,
	DurationMs,
	StartOffsetMs,
	ByteSize,
	Mime,
}

#[derive(DeriveIden)]
enum Media {
	Table,
	Id,
}
