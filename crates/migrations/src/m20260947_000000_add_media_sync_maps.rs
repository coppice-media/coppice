//! Persist validated tier-2 ebook↔audiobook timing maps.
//!
//! A map belongs to two media rows and is identified by the exact input and
//! generator provenance that produced it. There is no pair id: edition pairing
//! remains the two confirmed liseur-sync links, while this table is a reusable
//! artifact cache. `source` distinguishes an operator import from a future
//! worker result, and `job_id` keeps that result traceable without making the
//! worker history a foreign-key dependency.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(MediaSyncMaps::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(MediaSyncMaps::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(MediaSyncMaps::EbookMediaId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(MediaSyncMaps::AudioMediaId)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(MediaSyncMaps::Granularity).text().not_null())
					.col(ColumnDef::new(MediaSyncMaps::Generator).text().not_null())
					.col(
						ColumnDef::new(MediaSyncMaps::GeneratorVersion)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(MediaSyncMaps::Algorithm).text())
					.col(ColumnDef::new(MediaSyncMaps::Model).text())
					.col(ColumnDef::new(MediaSyncMaps::TextDigest).text().not_null())
					.col(
						ColumnDef::new(MediaSyncMaps::AudioManifestDigest)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(MediaSyncMaps::CueCount).integer().not_null())
					.col(ColumnDef::new(MediaSyncMaps::Coverage).double())
					.col(ColumnDef::new(MediaSyncMaps::Map).json().not_null())
					.col(ColumnDef::new(MediaSyncMaps::Source).text().not_null())
					.col(ColumnDef::new(MediaSyncMaps::JobId).text())
					.col(
						ColumnDef::new(MediaSyncMaps::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-media-sync-maps-ebook")
							.from(MediaSyncMaps::Table, MediaSyncMaps::EbookMediaId)
							.to(Media::Table, Media::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-media-sync-maps-audio")
							.from(MediaSyncMaps::Table, MediaSyncMaps::AudioMediaId)
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
					.name("uq_media_sync_maps_identity")
					.table(MediaSyncMaps::Table)
					.col(MediaSyncMaps::EbookMediaId)
					.col(MediaSyncMaps::AudioMediaId)
					.col(MediaSyncMaps::Granularity)
					.col(MediaSyncMaps::TextDigest)
					.col(MediaSyncMaps::AudioManifestDigest)
					.col(MediaSyncMaps::Generator)
					.col(MediaSyncMaps::GeneratorVersion)
					.unique()
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(
				Table::drop()
					.table(MediaSyncMaps::Table)
					.if_exists()
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum MediaSyncMaps {
	#[sea_orm(iden = "media_sync_maps")]
	Table,
	Id,
	EbookMediaId,
	AudioMediaId,
	Granularity,
	Generator,
	GeneratorVersion,
	Algorithm,
	Model,
	TextDigest,
	AudioManifestDigest,
	CueCount,
	Coverage,
	Map,
	Source,
	JobId,
	CreatedAt,
}

#[derive(DeriveIden)]
enum Media {
	Table,
	Id,
}
