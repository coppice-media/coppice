//! Per-user Connections state: Kindle destinations and Hardcover credentials/links.
//!
//! SMTP remains the server's single `emailers` sender.  Kindle destinations are
//! user-owned recipients; delivery rows keep owner and target snapshots so
//! deleting a destination never erases history.  Hardcover credentials are
//! encrypted at the application boundary and this migration intentionally
//! stores no plaintext provider response.

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(KindleDestinations::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(KindleDestinations::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(KindleDestinations::UserId).text().not_null())
					.col(ColumnDef::new(KindleDestinations::Name).text().not_null())
					.col(ColumnDef::new(KindleDestinations::Email).text().not_null())
					.col(
						ColumnDef::new(KindleDestinations::IsDefault)
							.boolean()
							.not_null()
							.default(false),
					)
					.col(
						ColumnDef::new(KindleDestinations::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.col(
						ColumnDef::new(KindleDestinations::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-kindle-destinations-user")
							.from(KindleDestinations::Table, KindleDestinations::UserId)
							.to(Users::Table, Users::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx-kindle-destinations-user")
					.table(KindleDestinations::Table)
					.col(KindleDestinations::UserId)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("uq-kindle-destinations-user-email")
					.table(KindleDestinations::Table)
					.col(KindleDestinations::UserId)
					.col(KindleDestinations::Email)
					.unique()
					.to_owned(),
			)
			.await?;

		// Keep each new column in its own alter_table call. This is important for
		// SQLite's table-copy implementation and for migration inspection tooling.
		for column in [
			ColumnDef::new(KindleDeliveries::UserId).text().null(),
			ColumnDef::new(KindleDeliveries::DestinationId)
				.text()
				.null(),
			ColumnDef::new(KindleDeliveries::DestinationName)
				.text()
				.null(),
		] {
			manager
				.alter_table(
					Table::alter()
						.table(KindleDeliveries::Table)
						.add_column(column)
						.to_owned(),
				)
				.await?;
		}

		// Existing rows were device sends. Preserve their owner before allowing
		// destination-only rows to use the compatibility empty device id.
		manager
			.get_connection()
			.execute(Statement::from_string(
				manager.get_database_backend(),
				"UPDATE kindle_deliveries SET user_id = (SELECT user_id FROM devices WHERE devices.id = kindle_deliveries.device_id) WHERE user_id IS NULL".to_string(),
			))
			.await?;

		// Destination sends have no device; rebuild/alter the column while
		// retaining the foreign key for every non-null legacy device id.
		match manager.get_database_backend() {
			DbBackend::Sqlite => make_sqlite_delivery_device_nullable(manager).await?,
			DbBackend::Postgres => {
				manager
					.get_connection()
					.execute(Statement::from_string(
						DbBackend::Postgres,
						"ALTER TABLE kindle_deliveries ALTER COLUMN device_id DROP NOT NULL".to_string(),
					))
					.await?;
			},
			_ => {},
		}

		// Legacy addresses become destinations, but device.kindle_email remains
		// intact for old mobile clients and the legacy mutation.
		manager
			.get_connection()
			.execute(Statement::from_string(
				manager.get_database_backend(),
				"INSERT INTO kindle_destinations (id, user_id, name, email, is_default, created_at, updated_at) SELECT 'legacy-' || id, user_id, name, kindle_email, CASE WHEN id = (SELECT MIN(d2.id) FROM devices d2 WHERE d2.user_id = devices.user_id AND d2.kindle_email IS NOT NULL) THEN TRUE ELSE FALSE END, created_at, created_at FROM devices WHERE kindle_email IS NOT NULL".to_string(),
			))
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx-kindle-deliveries-user-sent-at")
					.table(KindleDeliveries::Table)
					.col(KindleDeliveries::UserId)
					.col(KindleDeliveries::SentAt)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(HardcoverConnections::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(HardcoverConnections::UserId)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(HardcoverConnections::EncryptedApiToken)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(HardcoverConnections::CredentialVersion)
							.big_integer()
							.not_null()
							.default(1),
					)
					.col(
						ColumnDef::new(HardcoverConnections::RemoteUserId)
							.text()
							.null(),
					)
					.col(
						ColumnDef::new(HardcoverConnections::RemoteUsername)
							.text()
							.null(),
					)
					.col(ColumnDef::new(HardcoverConnections::Scopes).json().null())
					.col(
						ColumnDef::new(HardcoverConnections::Capabilities)
							.json()
							.null(),
					)
					.col(
						ColumnDef::new(HardcoverConnections::UseForMetadata)
							.boolean()
							.not_null()
							.default(true),
					)
					.col(
						ColumnDef::new(HardcoverConnections::ImportJournals)
							.boolean()
							.not_null()
							.default(false),
					)
					.col(
						ColumnDef::new(HardcoverConnections::SyncProgress)
							.boolean()
							.not_null()
							.default(false),
					)
					.col(
						ColumnDef::new(HardcoverConnections::ConnectedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.col(
						ColumnDef::new(HardcoverConnections::VerifiedAt)
							.timestamp_with_time_zone()
							.null(),
					)
					.col(
						ColumnDef::new(HardcoverConnections::LastSyncAt)
							.timestamp_with_time_zone()
							.null(),
					)
					.col(
						ColumnDef::new(HardcoverConnections::LastError)
							.text()
							.null(),
					)
					.col(
						ColumnDef::new(HardcoverConnections::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-hardcover-connections-user")
							.from(
								HardcoverConnections::Table,
								HardcoverConnections::UserId,
							)
							.to(Users::Table, Users::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(HardcoverMediaLinks::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(HardcoverMediaLinks::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(HardcoverMediaLinks::UserId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(HardcoverMediaLinks::MediaId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(HardcoverMediaLinks::RemoteId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(HardcoverMediaLinks::RemoteTitle)
							.text()
							.null(),
					)
					.col(
						ColumnDef::new(HardcoverMediaLinks::LinkedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.col(
						ColumnDef::new(HardcoverMediaLinks::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-hardcover-media-links-user")
							.from(HardcoverMediaLinks::Table, HardcoverMediaLinks::UserId)
							.to(Users::Table, Users::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-hardcover-media-links-media")
							.from(
								HardcoverMediaLinks::Table,
								HardcoverMediaLinks::MediaId,
							)
							.to(Media::Table, Media::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("uq-hardcover-media-links-user-media")
					.table(HardcoverMediaLinks::Table)
					.col(HardcoverMediaLinks::UserId)
					.col(HardcoverMediaLinks::MediaId)
					.unique()
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("uq-hardcover-media-links-user-remote")
					.table(HardcoverMediaLinks::Table)
					.col(HardcoverMediaLinks::UserId)
					.col(HardcoverMediaLinks::RemoteId)
					.unique()
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(HardcoverMetadataCache::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(HardcoverMetadataCache::Provider)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(HardcoverMetadataCache::QueryKey)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(HardcoverMetadataCache::SchemaVersion)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(HardcoverMetadataCache::Payload)
							.json()
							.not_null(),
					)
					.col(
						ColumnDef::new(HardcoverMetadataCache::ExpiresAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(HardcoverMetadataCache::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.primary_key(
						Index::create()
							.col(HardcoverMetadataCache::Provider)
							.col(HardcoverMetadataCache::QueryKey)
							.col(HardcoverMetadataCache::SchemaVersion),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(HardcoverJournalProvenance::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(HardcoverJournalProvenance::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(HardcoverJournalProvenance::UserId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(HardcoverJournalProvenance::RemoteEntryId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(HardcoverJournalProvenance::MediaId)
							.text()
							.null(),
					)
					.col(
						ColumnDef::new(HardcoverJournalProvenance::LocalAnnotationId)
							.text()
							.null(),
					)
					.col(
						ColumnDef::new(HardcoverJournalProvenance::LocatorKey)
							.text()
							.null(),
					)
					.col(
						ColumnDef::new(HardcoverJournalProvenance::Status)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(HardcoverJournalProvenance::UnresolvedReason)
							.text()
							.null(),
					)
					.col(
						ColumnDef::new(HardcoverJournalProvenance::Payload)
							.json()
							.null(),
					)
					.col(
						ColumnDef::new(HardcoverJournalProvenance::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.col(
						ColumnDef::new(HardcoverJournalProvenance::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-hardcover-journal-provenance-user")
							.from(
								HardcoverJournalProvenance::Table,
								HardcoverJournalProvenance::UserId,
							)
							.to(Users::Table, Users::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-hardcover-journal-provenance-media")
							.from(
								HardcoverJournalProvenance::Table,
								HardcoverJournalProvenance::MediaId,
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
					.name("uq-hardcover-journal-provenance-user-entry")
					.table(HardcoverJournalProvenance::Table)
					.col(HardcoverJournalProvenance::UserId)
					.col(HardcoverJournalProvenance::RemoteEntryId)
					.unique()
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(
				Table::drop()
					.table(HardcoverJournalProvenance::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		manager
			.drop_table(
				Table::drop()
					.table(HardcoverMetadataCache::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		manager
			.drop_table(
				Table::drop()
					.table(HardcoverMediaLinks::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		manager
			.drop_table(
				Table::drop()
					.table(HardcoverConnections::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		manager
			.drop_table(
				Table::drop()
					.table(KindleDestinations::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		Ok(())
	}
}

async fn make_sqlite_delivery_device_nullable(
	manager: &SchemaManager<'_>,
) -> Result<(), DbErr> {
	let db = manager.get_connection();
	for statement in [
		"CREATE TABLE kindle_deliveries_connections_new (id TEXT NOT NULL PRIMARY KEY, media_id TEXT NOT NULL, device_id TEXT, recipient TEXT NOT NULL, format TEXT NOT NULL, bytes BIGINT NOT NULL, converted BOOLEAN NOT NULL, note TEXT, error TEXT, sent_at TIMESTAMP WITH TIME ZONE NOT NULL, user_id TEXT, destination_id TEXT, destination_name TEXT, FOREIGN KEY(media_id) REFERENCES media(id) ON UPDATE CASCADE ON DELETE CASCADE, FOREIGN KEY(device_id) REFERENCES devices(id) ON UPDATE CASCADE ON DELETE CASCADE)",
		"INSERT INTO kindle_deliveries_connections_new (id, media_id, device_id, recipient, format, bytes, converted, note, error, sent_at, user_id, destination_id, destination_name) SELECT id, media_id, device_id, recipient, format, bytes, converted, note, error, sent_at, user_id, destination_id, destination_name FROM kindle_deliveries",
		"DROP TABLE kindle_deliveries",
		"ALTER TABLE kindle_deliveries_connections_new RENAME TO kindle_deliveries",
		"CREATE INDEX kindle_deliveries_device_id_sent_at_idx ON kindle_deliveries(device_id, sent_at)",
		"CREATE INDEX kindle_deliveries_media_id_sent_at_idx ON kindle_deliveries(media_id, sent_at)",
	] {
		db.execute(Statement::from_string(DbBackend::Sqlite, statement.to_string()))
			.await?;
	}
	Ok(())
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
enum KindleDestinations {
	Table,
	Id,
	UserId,
	Name,
	Email,
	IsDefault,
	CreatedAt,
	UpdatedAt,
}

#[derive(DeriveIden)]
enum KindleDeliveries {
	Table,
	UserId,
	DestinationId,
	DestinationName,
	SentAt,
}

#[derive(DeriveIden)]
enum HardcoverConnections {
	Table,
	UserId,
	EncryptedApiToken,
	CredentialVersion,
	RemoteUserId,
	RemoteUsername,
	Scopes,
	Capabilities,
	UseForMetadata,
	ImportJournals,
	SyncProgress,
	ConnectedAt,
	VerifiedAt,
	LastSyncAt,
	LastError,
	UpdatedAt,
}

#[derive(DeriveIden)]
enum HardcoverMediaLinks {
	Table,
	Id,
	UserId,
	MediaId,
	RemoteId,
	RemoteTitle,
	LinkedAt,
	UpdatedAt,
}

#[derive(DeriveIden)]
enum HardcoverMetadataCache {
	Table,
	Provider,
	QueryKey,
	SchemaVersion,
	Payload,
	ExpiresAt,
	CreatedAt,
}

#[derive(DeriveIden)]
enum HardcoverJournalProvenance {
	Table,
	Id,
	UserId,
	RemoteEntryId,
	MediaId,
	LocalAnnotationId,
	LocatorKey,
	Status,
	UnresolvedReason,
	Payload,
	CreatedAt,
	UpdatedAt,
}
