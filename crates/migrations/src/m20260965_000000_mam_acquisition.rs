//! Durable MAM release-search snapshots and explicitly requested grab history.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(MamReleaseSearches::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(MamReleaseSearches::RequestId)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(MamReleaseSearches::CandidatesJson)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(MamReleaseSearches::Found).integer().null())
					.col(
						ColumnDef::new(MamReleaseSearches::Probe)
							.boolean()
							.not_null()
							.default(false),
					)
					.col(
						ColumnDef::new(MamReleaseSearches::SearchedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(MamReleaseSearches::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk_mam_release_searches_request")
							.from(
								MamReleaseSearches::Table,
								MamReleaseSearches::RequestId,
							)
							.to(BookRequests::Table, BookRequests::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(MamAcquisitionGrabs::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(MamAcquisitionGrabs::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(MamAcquisitionGrabs::RequestId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(MamAcquisitionGrabs::TorrentId)
							.big_integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(MamAcquisitionGrabs::BridgeGrabId)
							.text()
							.null(),
					)
					.col(ColumnDef::new(MamAcquisitionGrabs::Title).text().not_null())
					.col(ColumnDef::new(MamAcquisitionGrabs::Phase).text().not_null())
					.col(
						ColumnDef::new(MamAcquisitionGrabs::Progress)
							.double()
							.not_null()
							.default(0.0),
					)
					.col(ColumnDef::new(MamAcquisitionGrabs::Error).text().null())
					.col(
						ColumnDef::new(MamAcquisitionGrabs::IngestItemId)
							.text()
							.null(),
					)
					.col(
						ColumnDef::new(MamAcquisitionGrabs::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(MamAcquisitionGrabs::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk_mam_acquisition_grabs_request")
							.from(
								MamAcquisitionGrabs::Table,
								MamAcquisitionGrabs::RequestId,
							)
							.to(BookRequests::Table, BookRequests::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk_mam_acquisition_grabs_ingest_item")
							.from(
								MamAcquisitionGrabs::Table,
								MamAcquisitionGrabs::IngestItemId,
							)
							.to(IngestDropItems::Table, IngestDropItems::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::SetNull),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("idx_mam_acquisition_grabs_request_created")
					.table(MamAcquisitionGrabs::Table)
					.col(MamAcquisitionGrabs::RequestId)
					.col(MamAcquisitionGrabs::CreatedAt)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx_mam_acquisition_grabs_poll")
					.table(MamAcquisitionGrabs::Table)
					.col(MamAcquisitionGrabs::Phase)
					.col(MamAcquisitionGrabs::UpdatedAt)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(
				Table::drop()
					.table(MamAcquisitionGrabs::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		manager
			.drop_table(
				Table::drop()
					.table(MamReleaseSearches::Table)
					.if_exists()
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum BookRequests {
	#[sea_orm(iden = "book_requests")]
	Table,
	Id,
}

#[derive(DeriveIden)]
enum IngestDropItems {
	#[sea_orm(iden = "ingest_drop_items")]
	Table,
	Id,
}

#[derive(DeriveIden)]
enum MamReleaseSearches {
	#[sea_orm(iden = "mam_release_searches")]
	Table,
	RequestId,
	CandidatesJson,
	Found,
	Probe,
	SearchedAt,
	UpdatedAt,
}

#[derive(DeriveIden)]
enum MamAcquisitionGrabs {
	#[sea_orm(iden = "mam_acquisition_grabs")]
	Table,
	Id,
	RequestId,
	TorrentId,
	BridgeGrabId,
	Title,
	Phase,
	Progress,
	Error,
	IngestItemId,
	CreatedAt,
	UpdatedAt,
}
