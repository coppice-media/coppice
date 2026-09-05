//! Kavita compatibility storage.
//!
//! `kavita_ids` allocates the stable integer identities Kavita clients expect
//! for libraries, series, media (volume and chapter share one id) and users;
//! `kavita_progress` keeps the per-user EPUB `bookScrollId` that Kavita's
//! `ProgressDto` carries alongside the page number, which has no counterpart in
//! `reading_sessions`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(KavitaIds::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(KavitaIds::Id)
							.integer()
							.not_null()
							.auto_increment()
							.primary_key(),
					)
					.col(ColumnDef::new(KavitaIds::Kind).text().not_null())
					.col(ColumnDef::new(KavitaIds::StumpId).text().not_null())
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("uq-kavita-ids-kind-stump-id")
					.table(KavitaIds::Table)
					.col(KavitaIds::Kind)
					.col(KavitaIds::StumpId)
					.unique()
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(KavitaProgress::Table)
					.if_not_exists()
					.col(ColumnDef::new(KavitaProgress::UserId).text().not_null())
					.col(ColumnDef::new(KavitaProgress::MediaId).text().not_null())
					.col(ColumnDef::new(KavitaProgress::BookScrollId).text())
					.col(ColumnDef::new(KavitaProgress::UpdatedAt).text().not_null())
					.primary_key(
						Index::create()
							.col(KavitaProgress::UserId)
							.col(KavitaProgress::MediaId),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-kavita-progress-user")
							.from(KavitaProgress::Table, KavitaProgress::UserId)
							.to(Users::Table, Users::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-kavita-progress-media")
							.from(KavitaProgress::Table, KavitaProgress::MediaId)
							.to(Media::Table, Media::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(Table::drop().table(KavitaProgress::Table).to_owned())
			.await?;
		manager
			.drop_table(Table::drop().table(KavitaIds::Table).to_owned())
			.await
	}
}

#[derive(DeriveIden)]
enum Users {
	Table,
	Id,
}

#[derive(DeriveIden)]
enum Media {
	Table,
	Id,
}

#[derive(DeriveIden)]
enum KavitaIds {
	#[sea_orm(iden = "kavita_ids")]
	Table,
	Id,
	Kind,
	StumpId,
}

#[derive(DeriveIden)]
enum KavitaProgress {
	#[sea_orm(iden = "kavita_progress")]
	Table,
	UserId,
	MediaId,
	BookScrollId,
	UpdatedAt,
}
