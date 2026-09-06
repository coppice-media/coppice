//! Audiobookshelf compatibility storage.
//!
//! `abs_ids` allocates the stable uuids Audiobookshelf clients expect for the
//! three ABS entities that have no Stump row to borrow an id from: authors
//! (Stump keeps writers as `media_metadata` strings), ABS's second `media.id`
//! beside a library item, and ABS's `libraryFolder.id` beside a library.
//! Libraries, library items and series are served under their Stump uuid
//! verbatim and are absent here.
//!
//! `abs_sessions` keeps playback sessions, which Audiobookshelf holds
//! server-side: a client syncs against the session id
//! (`POST /api/session/{id}/sync`), so the row has to outlive the play request
//! that created it, and `time_listening_ms` is the wall-clock time one client
//! reported rather than a reading position. Closing a session keeps the row
//! and stamps `closed_at`.
//!
//! Both tables mirror `stump_abs::CREATE_ABS_IDS_SQL` and
//! `stump_abs::CREATE_ABS_SESSIONS_SQL` column for column, so the crate's
//! in-memory tests and a migrated database agree. The DDL is written out here
//! rather than imported: `migrations` does not depend on `stump_abs`, and the
//! compat crate is optional at the server.
//!
//! No secondary index is added: `abs_ids` is read by uuid (its primary key)
//! and by `(kind, stump_id)` (the unique index below), and `abs_sessions` is
//! read by `(id, user_id)`, which the primary key already covers. Neither
//! table is a foreign key holder, matching the crate's DDL — a session row
//! outliving its media is preferable to a cascade deleting listening history.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(AbsIds::Table)
					.if_not_exists()
					.col(ColumnDef::new(AbsIds::Id).text().not_null().primary_key())
					.col(ColumnDef::new(AbsIds::Kind).text().not_null())
					.col(ColumnDef::new(AbsIds::StumpId).text().not_null())
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("uq-abs-ids-kind-stump-id")
					.table(AbsIds::Table)
					.col(AbsIds::Kind)
					.col(AbsIds::StumpId)
					.unique()
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(AbsSessions::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(AbsSessions::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(AbsSessions::UserId).text().not_null())
					.col(ColumnDef::new(AbsSessions::MediaId).text().not_null())
					.col(ColumnDef::new(AbsSessions::LibraryId).text().not_null())
					.col(ColumnDef::new(AbsSessions::DeviceId).text())
					.col(ColumnDef::new(AbsSessions::ClientName).text())
					.col(ColumnDef::new(AbsSessions::ClientVersion).text())
					.col(ColumnDef::new(AbsSessions::MediaPlayer).text())
					.col(
						ColumnDef::new(AbsSessions::CurrentTimeMs)
							.big_integer()
							.not_null()
							.default(0),
					)
					.col(
						ColumnDef::new(AbsSessions::TimeListeningMs)
							.big_integer()
							.not_null()
							.default(0),
					)
					.col(
						ColumnDef::new(AbsSessions::StartedAt)
							.big_integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(AbsSessions::UpdatedAt)
							.big_integer()
							.not_null(),
					)
					.col(ColumnDef::new(AbsSessions::ClosedAt).big_integer())
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(Table::drop().table(AbsSessions::Table).to_owned())
			.await?;
		manager
			.drop_table(Table::drop().table(AbsIds::Table).to_owned())
			.await
	}
}

#[derive(DeriveIden)]
enum AbsIds {
	#[sea_orm(iden = "abs_ids")]
	Table,
	Id,
	Kind,
	StumpId,
}

#[derive(DeriveIden)]
enum AbsSessions {
	#[sea_orm(iden = "abs_sessions")]
	Table,
	Id,
	UserId,
	MediaId,
	LibraryId,
	DeviceId,
	ClientName,
	ClientVersion,
	MediaPlayer,
	CurrentTimeMs,
	TimeListeningMs,
	StartedAt,
	UpdatedAt,
	ClosedAt,
}
