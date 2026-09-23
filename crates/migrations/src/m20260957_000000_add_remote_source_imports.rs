//! Durable proposals and decisions for importing a verified source item.
//!
//! A proposal snapshots the worker lineage, server-verified digest, and byte
//! size.  Approval code must compare those values with the live observation
//! before linking or staging; matching never publishes media by itself.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(RemoteSourceImports::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(RemoteSourceImports::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::SourceId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::SourceItemId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::TargetMediaId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::WorkerItemId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::WorkerContentVersion)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::SourceSha256)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::SourceSize)
							.big_integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::MatchKind)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::MatchScore)
							.integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::MatchEvidence)
							.json()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::Status)
							.text()
							.not_null()
							.default("proposed"),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::DecisionActorId)
							.text()
							.null(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::DecisionAt)
							.timestamp_with_time_zone()
							.null(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::DecisionReason)
							.text()
							.null(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::AppliedLocationId)
							.text()
							.null(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::StagedItemId)
							.text()
							.null(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceImports::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-remote-source-imports-source")
							.from(
								RemoteSourceImports::Table,
								RemoteSourceImports::SourceId,
							)
							.to(RemoteSources::Table, RemoteSources::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-remote-source-imports-source-item")
							.from(
								RemoteSourceImports::Table,
								RemoteSourceImports::SourceItemId,
							)
							.to(RemoteSourceItems::Table, RemoteSourceItems::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-remote-source-imports-media")
							.from(
								RemoteSourceImports::Table,
								RemoteSourceImports::TargetMediaId,
							)
							.to(Media::Table, Media::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-remote-source-imports-decision-actor")
							.from(
								RemoteSourceImports::Table,
								RemoteSourceImports::DecisionActorId,
							)
							.to(Users::Table, Users::Id)
							.on_delete(ForeignKeyAction::SetNull)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("idx-remote-source-imports-identity")
					.table(RemoteSourceImports::Table)
					.col(RemoteSourceImports::SourceItemId)
					.col(RemoteSourceImports::TargetMediaId)
					.col(RemoteSourceImports::WorkerContentVersion)
					.col(RemoteSourceImports::SourceSha256)
					.unique()
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx-remote-source-imports-status")
					.table(RemoteSourceImports::Table)
					.col(RemoteSourceImports::Status)
					.col(RemoteSourceImports::CreatedAt)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(Table::drop().table(RemoteSourceImports::Table).to_owned())
			.await
	}
}

#[derive(DeriveIden)]
enum RemoteSources {
	Table,
	Id,
}

#[derive(DeriveIden)]
enum RemoteSourceItems {
	Table,
	Id,
}

#[derive(DeriveIden)]
enum Media {
	Table,
	Id,
}

#[derive(DeriveIden)]
enum Users {
	Table,
	Id,
}

#[derive(DeriveIden)]
enum RemoteSourceImports {
	#[sea_orm(iden = "remote_source_imports")]
	Table,
	Id,
	SourceId,
	SourceItemId,
	TargetMediaId,
	WorkerItemId,
	WorkerContentVersion,
	SourceSha256,
	SourceSize,
	MatchKind,
	MatchScore,
	MatchEvidence,
	Status,
	DecisionActorId,
	DecisionAt,
	DecisionReason,
	AppliedLocationId,
	StagedItemId,
	CreatedAt,
	UpdatedAt,
}
