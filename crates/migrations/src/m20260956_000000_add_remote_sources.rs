//! Durable registry for privacy-minimized home-library source workers.
//!
//! The three tables keep worker observations and server-verified locations
//! separate. No absolute worker path is stored here; a worker resolves opaque
//! source/item identities from its own local catalog.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(RemoteSources::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(RemoteSources::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(RemoteSources::DeviceId).text().not_null())
					.col(ColumnDef::new(RemoteSources::RootId).text().not_null())
					.col(ColumnDef::new(RemoteSources::Label).text().not_null())
					.col(ColumnDef::new(RemoteSources::Kind).text().not_null())
					.col(ColumnDef::new(RemoteSources::PrivacyMode).text().not_null())
					.col(ColumnDef::new(RemoteSources::Transport).text().not_null())
					.col(ColumnDef::new(RemoteSources::DirectBaseUrl).text().null())
					.col(
						ColumnDef::new(RemoteSources::CurrentRevision)
							.big_integer()
							.not_null()
							.default(0),
					)
					.col(
						ColumnDef::new(RemoteSources::LastSeenAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(ColumnDef::new(RemoteSources::Health).text().not_null())
					.col(
						ColumnDef::new(RemoteSources::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSources::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-remote-sources-device")
							.from(RemoteSources::Table, RemoteSources::DeviceId)
							.to(Devices::Table, Devices::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("idx-remote-sources-device-root")
					.table(RemoteSources::Table)
					.col(RemoteSources::DeviceId)
					.col(RemoteSources::RootId)
					.unique()
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(RemoteSourceItems::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(RemoteSourceItems::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(RemoteSourceItems::SourceId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceItems::WorkerItemId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceItems::WorkerContentVersion)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceItems::RelativePath)
							.text()
							.null(),
					)
					.col(
						ColumnDef::new(RemoteSourceItems::Size)
							.big_integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceItems::ModifiedAt)
							.timestamp_with_time_zone()
							.null(),
					)
					.col(ColumnDef::new(RemoteSourceItems::MediaType).text().null())
					.col(
						ColumnDef::new(RemoteSourceItems::QuickFingerprint)
							.text()
							.null(),
					)
					.col(ColumnDef::new(RemoteSourceItems::Sha256).text().null())
					.col(ColumnDef::new(RemoteSourceItems::Metadata).json().null())
					.col(ColumnDef::new(RemoteSourceItems::Retention).json().null())
					.col(
						ColumnDef::new(RemoteSourceItems::ObservationState)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceItems::LastSeenRevision)
							.big_integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceItems::LastSeenAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceItems::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceItems::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(RemoteSourceItems::ImportedMediaId)
							.text()
							.null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-remote-source-items-source")
							.from(RemoteSourceItems::Table, RemoteSourceItems::SourceId)
							.to(RemoteSources::Table, RemoteSources::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-remote-source-items-media")
							.from(
								RemoteSourceItems::Table,
								RemoteSourceItems::ImportedMediaId,
							)
							.to(Media::Table, Media::Id)
							.on_delete(ForeignKeyAction::SetNull)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("idx-remote-source-items-source-worker-item")
					.table(RemoteSourceItems::Table)
					.col(RemoteSourceItems::SourceId)
					.col(RemoteSourceItems::WorkerItemId)
					.unique()
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(MediaLocations::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(MediaLocations::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(MediaLocations::MediaId).text().not_null())
					.col(ColumnDef::new(MediaLocations::SourceItemId).text().null())
					.col(ColumnDef::new(MediaLocations::Kind).text().not_null())
					.col(ColumnDef::new(MediaLocations::Sha256).text().not_null())
					.col(
						ColumnDef::new(MediaLocations::ContentVersion)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(MediaLocations::Health).text().not_null())
					.col(
						ColumnDef::new(MediaLocations::DurabilityRole)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(MediaLocations::CachePath).text().null())
					.col(
						ColumnDef::new(MediaLocations::VerifiedAt)
							.timestamp_with_time_zone()
							.null(),
					)
					.col(
						ColumnDef::new(MediaLocations::LastSeenAt)
							.timestamp_with_time_zone()
							.null(),
					)
					.col(
						ColumnDef::new(MediaLocations::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(MediaLocations::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-media-locations-media")
							.from(MediaLocations::Table, MediaLocations::MediaId)
							.to(Media::Table, Media::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-media-locations-source-item")
							.from(MediaLocations::Table, MediaLocations::SourceItemId)
							.to(RemoteSourceItems::Table, RemoteSourceItems::Id)
							.on_delete(ForeignKeyAction::SetNull)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("idx-media-locations-media-source-item-kind")
					.table(MediaLocations::Table)
					.col(MediaLocations::MediaId)
					.col(MediaLocations::SourceItemId)
					.col(MediaLocations::Kind)
					.unique()
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(Table::drop().table(MediaLocations::Table).to_owned())
			.await?;
		manager
			.drop_table(Table::drop().table(RemoteSourceItems::Table).to_owned())
			.await?;
		manager
			.drop_table(Table::drop().table(RemoteSources::Table).to_owned())
			.await
	}
}

#[derive(DeriveIden)]
enum Devices {
	Table,
	Id,
}

#[derive(DeriveIden)]
enum Media {
	Table,
	Id,
}

#[derive(DeriveIden)]
enum RemoteSources {
	#[sea_orm(iden = "remote_sources")]
	Table,
	Id,
	DeviceId,
	RootId,
	Label,
	Kind,
	PrivacyMode,
	Transport,
	DirectBaseUrl,
	CurrentRevision,
	LastSeenAt,
	Health,
	CreatedAt,
	UpdatedAt,
}

#[derive(DeriveIden)]
enum RemoteSourceItems {
	#[sea_orm(iden = "remote_source_items")]
	Table,
	Id,
	SourceId,
	WorkerItemId,
	WorkerContentVersion,
	RelativePath,
	Size,
	ModifiedAt,
	MediaType,
	QuickFingerprint,
	Sha256,
	Metadata,
	Retention,
	ObservationState,
	LastSeenRevision,
	LastSeenAt,
	CreatedAt,
	UpdatedAt,
	ImportedMediaId,
}

#[derive(DeriveIden)]
enum MediaLocations {
	#[sea_orm(iden = "media_locations")]
	Table,
	Id,
	MediaId,
	SourceItemId,
	Kind,
	Sha256,
	ContentVersion,
	Health,
	DurabilityRole,
	CachePath,
	VerifiedAt,
	LastSeenAt,
	CreatedAt,
	UpdatedAt,
}
