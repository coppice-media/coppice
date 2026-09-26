//! Personal display-name overrides for scan-owned Liseur catalog series.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(LiseurSyncSeriesNames::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(LiseurSyncSeriesNames::UserId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(LiseurSyncSeriesNames::SeriesId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(LiseurSyncSeriesNames::Name)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(LiseurSyncSeriesNames::NormalizedName)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(LiseurSyncSeriesNames::UpdatedAt)
							.text()
							.not_null(),
					)
					.primary_key(
						Index::create()
							.col(LiseurSyncSeriesNames::UserId)
							.col(LiseurSyncSeriesNames::SeriesId),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-liseur-sync-series-names-user")
							.from(
								LiseurSyncSeriesNames::Table,
								LiseurSyncSeriesNames::UserId,
							)
							.to(Users::Table, Users::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-liseur-sync-series-names-series")
							.from(
								LiseurSyncSeriesNames::Table,
								LiseurSyncSeriesNames::SeriesId,
							)
							.to(Series::Table, Series::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("uq-liseur-sync-series-name-user-normalized")
					.table(LiseurSyncSeriesNames::Table)
					.col(LiseurSyncSeriesNames::UserId)
					.col(LiseurSyncSeriesNames::NormalizedName)
					.unique()
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(
				Table::drop()
					.table(LiseurSyncSeriesNames::Table)
					.if_exists()
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum LiseurSyncSeriesNames {
	#[sea_orm(iden = "liseur_sync_series_names")]
	Table,
	UserId,
	SeriesId,
	Name,
	NormalizedName,
	UpdatedAt,
}

#[derive(DeriveIden)]
enum Users {
	Table,
	Id,
}

#[derive(DeriveIden)]
enum Series {
	Table,
	Id,
}
