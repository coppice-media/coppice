//! `media_audio`: the audio facts of one audiobook, one row per media.
//!
//! An audiobook is a `media` row like any other book, so the audio-only facts
//! live in a side table keyed by `media_id` rather than as nullable columns on
//! `media`: a comic or an EPUB would carry six permanently-null columns
//! otherwise, and `media_analysis` is the *page* analysis surface, which has no
//! meaning for a waveform.
//!
//! The primary key is `media_id` itself (not a surrogate id) because the
//! relation is strictly 1:1 — a media item is one publication with one
//! duration and one codec. `duration_ms` is the whole-publication duration
//! (the sum of `media_audio_tracks.duration_ms` for a multi-file book), which
//! is what a reading position is expressed against.
//!
//! `chapter_source` records *how* the chapters in `media_audio_chapters` were
//! obtained (`mp4_chpl`, `mp4_chapter_track`, `id3_chap`, `vorbis_comment`,
//! `per_track`, `none`). It is provenance, not a preference: a book whose
//! chapters were synthesized one-per-file must never be presented as if the
//! publisher shipped them.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(MediaAudio::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(MediaAudio::MediaId)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(MediaAudio::DurationMs)
							.big_integer()
							.not_null(),
					)
					.col(ColumnDef::new(MediaAudio::Codec).text().not_null())
					.col(ColumnDef::new(MediaAudio::SampleRate).integer().null())
					.col(ColumnDef::new(MediaAudio::Channels).integer().null())
					.col(ColumnDef::new(MediaAudio::Bitrate).integer().null())
					.col(ColumnDef::new(MediaAudio::ChapterSource).text().not_null())
					.foreign_key(
						ForeignKey::create()
							.from(MediaAudio::Table, MediaAudio::MediaId)
							.to(Media::Table, Media::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(Table::drop().table(MediaAudio::Table).if_exists().to_owned())
			.await
	}
}

#[derive(DeriveIden)]
enum MediaAudio {
	Table,
	MediaId,
	DurationMs,
	Codec,
	SampleRate,
	Channels,
	Bitrate,
	ChapterSource,
}

#[derive(DeriveIden)]
enum Media {
	Table,
	Id,
}
