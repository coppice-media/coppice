//! Remote provider host: user-enabled source instances, catalog source health,
//! and the provider identity columns on `libraries`/`series`/`media`.
//!
//! `path` columns stay NOT NULL: provider-backed rows store a
//! `provider://<source>[/<remote_id>[/<chapter>]]` URI, so no table rebuild is
//! required on SQLite and every existing path consumer keeps decoding rows.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(ProviderSources::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(ProviderSources::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(ProviderSources::Implementation)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(ProviderSources::CatalogId).text())
					.col(ColumnDef::new(ProviderSources::Name).text().not_null())
					.col(ColumnDef::new(ProviderSources::Lang).text().not_null())
					.col(ColumnDef::new(ProviderSources::BaseUrl).text().not_null())
					.col(
						ColumnDef::new(ProviderSources::Enabled)
							.boolean()
							.not_null()
							.default(true),
					)
					.col(ColumnDef::new(ProviderSources::CreatedBy).text())
					.col(
						ColumnDef::new(ProviderSources::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.col(
						ColumnDef::new(ProviderSources::UpdatedAt)
							.timestamp_with_time_zone(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk_provider_sources_created_by")
							.from(ProviderSources::Table, ProviderSources::CreatedBy)
							.to(Users::Table, Users::Id)
							.on_delete(ForeignKeyAction::SetNull)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(SourceHealth::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(SourceHealth::SourceId)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(SourceHealth::Name).text().not_null())
					.col(ColumnDef::new(SourceHealth::Lang).text().not_null())
					.col(ColumnDef::new(SourceHealth::BaseUrl).text().not_null())
					.col(ColumnDef::new(SourceHealth::Theme).text())
					.col(
						ColumnDef::new(SourceHealth::Status)
							.text()
							.not_null()
							.default("UNKNOWN"),
					)
					.col(ColumnDef::new(SourceHealth::HttpStatus).integer())
					.col(ColumnDef::new(SourceHealth::LatencyMs).integer())
					.col(ColumnDef::new(SourceHealth::RedirectUrl).text())
					.col(ColumnDef::new(SourceHealth::LatestPathOk).boolean())
					.col(
						ColumnDef::new(SourceHealth::ConsecutiveFailures)
							.integer()
							.not_null()
							.default(0),
					)
					.col(ColumnDef::new(SourceHealth::Error).text())
					.col(
						ColumnDef::new(SourceHealth::CheckedAt)
							.timestamp_with_time_zone(),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("idx_source_health_base_url")
					.table(SourceHealth::Table)
					.col(SourceHealth::BaseUrl)
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(Libraries::Table)
					.add_column(ColumnDef::new(Libraries::SourceProvider).text())
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(Series::Table)
					.add_column(ColumnDef::new(Series::SourceProvider).text())
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(Series::Table)
					.add_column(ColumnDef::new(Series::RemoteId).text())
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(Media::Table)
					.add_column(ColumnDef::new(Media::SourceProvider).text())
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(Media::Table)
					.add_column(ColumnDef::new(Media::RemoteId).text())
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(Media::Table)
					.add_column(ColumnDef::new(Media::RemoteChapterId).text())
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("idx_series_source_provider_remote_id")
					.table(Series::Table)
					.col(Series::SourceProvider)
					.col(Series::RemoteId)
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("idx_media_source_provider_remote_chapter_id")
					.table(Media::Table)
					.col(Media::SourceProvider)
					.col(Media::RemoteChapterId)
					.to_owned(),
			)
			.await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_index(
				Index::drop()
					.name("idx_media_source_provider_remote_chapter_id")
					.table(Media::Table)
					.to_owned(),
			)
			.await?;
		manager
			.drop_index(
				Index::drop()
					.name("idx_series_source_provider_remote_id")
					.table(Series::Table)
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(Media::Table)
					.drop_column(Media::RemoteChapterId)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(Media::Table)
					.drop_column(Media::RemoteId)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(Media::Table)
					.drop_column(Media::SourceProvider)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(Series::Table)
					.drop_column(Series::RemoteId)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(Series::Table)
					.drop_column(Series::SourceProvider)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(Libraries::Table)
					.drop_column(Libraries::SourceProvider)
					.to_owned(),
			)
			.await?;

		manager
			.drop_table(Table::drop().table(SourceHealth::Table).to_owned())
			.await?;
		manager
			.drop_table(Table::drop().table(ProviderSources::Table).to_owned())
			.await?;

		Ok(())
	}
}

#[derive(DeriveIden)]
enum ProviderSources {
	#[sea_orm(iden = "provider_sources")]
	Table,
	Id,
	Implementation,
	CatalogId,
	Name,
	Lang,
	BaseUrl,
	Enabled,
	CreatedBy,
	CreatedAt,
	UpdatedAt,
}

#[derive(DeriveIden)]
enum SourceHealth {
	#[sea_orm(iden = "source_health")]
	Table,
	SourceId,
	Name,
	Lang,
	BaseUrl,
	Theme,
	Status,
	HttpStatus,
	LatencyMs,
	RedirectUrl,
	LatestPathOk,
	ConsecutiveFailures,
	Error,
	CheckedAt,
}

#[derive(DeriveIden)]
enum Series {
	#[sea_orm(iden = "series")]
	Table,
	SourceProvider,
	RemoteId,
}

#[derive(DeriveIden)]
enum Media {
	#[sea_orm(iden = "media")]
	Table,
	SourceProvider,
	RemoteId,
	RemoteChapterId,
}

#[derive(DeriveIden)]
enum Users {
	#[sea_orm(iden = "users")]
	Table,
	Id,
}

#[derive(DeriveIden)]
enum Libraries {
	#[sea_orm(iden = "libraries")]
	Table,
	SourceProvider,
}
