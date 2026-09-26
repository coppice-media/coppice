//! Native annotation colors, KOReader styles, and bridge-projection backfill state.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(MediaAnnotations::Table)
					.add_column(ColumnDef::new(MediaAnnotations::Color).text().null())
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(LiseurSyncCounters::Table)
					.add_column(
						ColumnDef::new(LiseurSyncCounters::ProjectedAnnotationSeq)
							.big_integer()
							.not_null()
							.default(0),
					)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(LiseurSyncAnnotations::Table)
					.add_column(
						ColumnDef::new(LiseurSyncAnnotations::Drawer).text().null(),
					)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(LiseurSyncAnnotations::Table)
					.drop_column(LiseurSyncAnnotations::Drawer)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(LiseurSyncCounters::Table)
					.drop_column(LiseurSyncCounters::ProjectedAnnotationSeq)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(MediaAnnotations::Table)
					.drop_column(MediaAnnotations::Color)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum MediaAnnotations {
	Table,
	Color,
}

#[derive(DeriveIden)]
enum LiseurSyncCounters {
	#[sea_orm(iden = "liseur_sync_counters")]
	Table,
	ProjectedAnnotationSeq,
}
#[derive(DeriveIden)]
enum LiseurSyncAnnotations {
	Table,
	Drawer,
}
