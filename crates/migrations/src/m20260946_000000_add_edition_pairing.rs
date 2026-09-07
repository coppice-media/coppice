//! Edition pairing: which audiobook is which ebook, and how their chapters
//! line up.
//!
//! There is no pair table. A work already owns its editions
//! (`m20260904_000000_add_liseur_sync`: `liseur_sync_works` /
//! `liseur_sync_editions` / `liseur_sync_media_links`, with
//! `uq-liseur-sync-media-links-user-media` making a media row belong to at
//! most one work per user), so "these two books are the same work" is already
//! expressible as two `liseur_sync_media_links` rows sharing `work_id`. A
//! second, parallel pair table would let the two disagree.
//!
//! What the existing row cannot express is that a link is a *guess*.
//! `resolution_status` is not the carrier: it means "was the edition hash
//! verified against the file" (`verified`/`unverified`, written by
//! `POST /v1/works/resolve`), and the liseur lane rewrites it whenever a
//! device re-resolves. So pairing state gets its own column,
//! `pair_status`, with the three states a suggestion moves through:
//!
//! - `confirmed` — an edition of the work. The default, so every row that
//!   exists today (a client asserted the work with an edition hash, which is
//!   stronger evidence than anything the pairing heuristics produce) and
//!   every future liseur `INSERT`, which does not name this column, keep
//!   their current meaning. This is what `Media.editions` returns.
//! - `suggested` — computed on demand by `stump_library::editions::pair_editions`
//!   and cached here; shown as a suggestion, never as an edition.
//! - `rejected` — the user said no. Kept as a row, not deleted, because the
//!   suggestion is recomputed on every book-page query and a deleted
//!   rejection comes straight back.
//!
//! `pair_evidence` records *why* (`work_id`, `provider_edition_list`,
//! `title_author`, `same_drop`) so the console can explain a suggestion and
//! so a weak signal is never presented as a strong one. It is null on rows
//! that predate pairing.
//!
//! `media_chapter_map` is the tier-1 chapter map: the only storage read-time
//! position conversion needs. It is keyed by the two media rows rather than by
//! a `pair_id`, because the pair has no row of its own to point at; the
//! composite unique index is what makes recomputing a map idempotent and what
//! lets one entry be edited in the console.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(LiseurSyncMediaLinks::Table)
					.add_column(
						ColumnDef::new(LiseurSyncMediaLinks::PairStatus)
							.text()
							.not_null()
							.default("confirmed"),
					)
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(LiseurSyncMediaLinks::Table)
					.add_column(
						ColumnDef::new(LiseurSyncMediaLinks::PairEvidence)
							.text()
							.null(),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(MediaChapterMap::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(MediaChapterMap::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(MediaChapterMap::EbookMediaId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(MediaChapterMap::AudioMediaId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(MediaChapterMap::EbookSpineIndex)
							.integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(MediaChapterMap::AudioChapterIndex)
							.integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(MediaChapterMap::Confidence)
							.double()
							.not_null(),
					)
					.col(ColumnDef::new(MediaChapterMap::CreatedAt).text().not_null())
					.foreign_key(
						ForeignKey::create()
							.name("fk-media-chapter-map-ebook")
							.from(MediaChapterMap::Table, MediaChapterMap::EbookMediaId)
							.to(Media::Table, Media::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-media-chapter-map-audio")
							.from(MediaChapterMap::Table, MediaChapterMap::AudioMediaId)
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
					.name("uq_media_chapter_map_pair_spine")
					.table(MediaChapterMap::Table)
					.col(MediaChapterMap::EbookMediaId)
					.col(MediaChapterMap::AudioMediaId)
					.col(MediaChapterMap::EbookSpineIndex)
					.unique()
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(
				Table::drop()
					.table(MediaChapterMap::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(LiseurSyncMediaLinks::Table)
					.drop_column(LiseurSyncMediaLinks::PairEvidence)
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(LiseurSyncMediaLinks::Table)
					.drop_column(LiseurSyncMediaLinks::PairStatus)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum LiseurSyncMediaLinks {
	#[sea_orm(iden = "liseur_sync_media_links")]
	Table,
	PairStatus,
	PairEvidence,
}

#[derive(DeriveIden)]
enum MediaChapterMap {
	#[sea_orm(iden = "media_chapter_map")]
	Table,
	Id,
	EbookMediaId,
	AudioMediaId,
	EbookSpineIndex,
	AudioChapterIndex,
	Confidence,
	CreatedAt,
}

#[derive(DeriveIden)]
enum Media {
	Table,
	Id,
}
