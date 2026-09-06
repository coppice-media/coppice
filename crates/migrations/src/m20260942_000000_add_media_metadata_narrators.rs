//! `media_metadata.narrators`: who reads an audiobook aloud.
//!
//! Every other credit role a book can carry already has a column here —
//! `writers`, `editors`, `inkers`, `letterers`, `colorists`, `cover_artists`,
//! `pencillers` — and the narrator is the one credit an audiobook always has
//! and a comic never does. Until now it had nowhere to go: the probe reads a
//! narrator off the container (`stump_media::audio::ProbedAudio::narrator`,
//! from the `composer`/`©wrt` tag every publisher writes it to) and the
//! Audiobookshelf surface had to answer `narrators: []` on every book because
//! no column held it.
//!
//! Same storage shape as the sibling credit columns: one nullable `TEXT`
//! holding a comma-joined list, so the existing list read/write helpers apply
//! unchanged and no existing row is rewritten.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(MediaMetadata::Table)
					.add_column(ColumnDef::new(MediaMetadata::Narrators).text().null())
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(MediaMetadata::Table)
					.drop_column(MediaMetadata::Narrators)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum MediaMetadata {
	Table,
	Narrators,
}
