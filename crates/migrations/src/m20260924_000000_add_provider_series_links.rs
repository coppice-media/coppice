//! Cross-source dedupe for materialised provider series.
//!
//! `provider_series_identity` is the dedupe key of every materialised
//! provider series: the normalised title plus, when the source carries one,
//! a cross-source external id (`al:<id>`, `mal:<id>`, ...). Both are indexed
//! so a materialisation is one lookup, never a table scan.
//!
//! `provider_series_links` records that a series is the same work as an
//! earlier (canonical) one from another source. It is advisory: nothing is
//! merged until an operator calls `mergeProviderSeries`, so a false positive
//! is visible rather than destructive.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(ProviderSeriesIdentity::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(ProviderSeriesIdentity::SeriesId)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(ProviderSeriesIdentity::SourceProvider)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(ProviderSeriesIdentity::NormalisedTitle)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(ProviderSeriesIdentity::ExternalKey).text())
					.col(
						ColumnDef::new(ProviderSeriesIdentity::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk_provider_series_identity_series")
							.from(
								ProviderSeriesIdentity::Table,
								ProviderSeriesIdentity::SeriesId,
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
					.name("idx_provider_series_identity_normalised_title")
					.table(ProviderSeriesIdentity::Table)
					.col(ProviderSeriesIdentity::NormalisedTitle)
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("idx_provider_series_identity_external_key")
					.table(ProviderSeriesIdentity::Table)
					.col(ProviderSeriesIdentity::ExternalKey)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(ProviderSeriesLinks::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(ProviderSeriesLinks::SeriesId)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(ProviderSeriesLinks::CanonicalSeriesId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(ProviderSeriesLinks::Reason)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(ProviderSeriesLinks::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk_provider_series_links_series")
							.from(
								ProviderSeriesLinks::Table,
								ProviderSeriesLinks::SeriesId,
							)
							.to(Series::Table, Series::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk_provider_series_links_canonical")
							.from(
								ProviderSeriesLinks::Table,
								ProviderSeriesLinks::CanonicalSeriesId,
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
					.name("idx_provider_series_links_canonical_series_id")
					.table(ProviderSeriesLinks::Table)
					.col(ProviderSeriesLinks::CanonicalSeriesId)
					.to_owned(),
			)
			.await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_index(
				Index::drop()
					.name("idx_provider_series_links_canonical_series_id")
					.table(ProviderSeriesLinks::Table)
					.to_owned(),
			)
			.await?;
		manager
			.drop_table(Table::drop().table(ProviderSeriesLinks::Table).to_owned())
			.await?;
		manager
			.drop_index(
				Index::drop()
					.name("idx_provider_series_identity_external_key")
					.table(ProviderSeriesIdentity::Table)
					.to_owned(),
			)
			.await?;
		manager
			.drop_index(
				Index::drop()
					.name("idx_provider_series_identity_normalised_title")
					.table(ProviderSeriesIdentity::Table)
					.to_owned(),
			)
			.await?;
		manager
			.drop_table(
				Table::drop()
					.table(ProviderSeriesIdentity::Table)
					.to_owned(),
			)
			.await?;
		Ok(())
	}
}

#[derive(DeriveIden)]
enum ProviderSeriesIdentity {
	#[sea_orm(iden = "provider_series_identity")]
	Table,
	SeriesId,
	SourceProvider,
	NormalisedTitle,
	ExternalKey,
	CreatedAt,
}

#[derive(DeriveIden)]
enum ProviderSeriesLinks {
	#[sea_orm(iden = "provider_series_links")]
	Table,
	SeriesId,
	CanonicalSeriesId,
	Reason,
	CreatedAt,
}

#[derive(DeriveIden)]
enum Series {
	#[sea_orm(iden = "series")]
	Table,
	Id,
}
