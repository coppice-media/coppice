use sea_orm::{DbBackend, Statement};
use sea_orm_migration::prelude::*;

/// Introduces the unified `devices` + `device_credentials` model and folds the
/// protocol-registered `reading_devices` rows into it.
///
/// `reading_devices` had no owner column; a row is attributed to the user whose
/// reading sessions reference it (most recently updated user wins when several
/// do) and is dropped when no session references it, since nothing else does.
/// The kind is inferred from the referencing sessions: KOReader when any of
/// them carries a `koreader_progress`, otherwise a generic OPDS reader.
///
/// `reading_sessions.device_ids` keeps pointing at these ids as a JSON array;
/// replacing it with a junction table carrying a foreign key to `devices` is a
/// follow-up.
#[derive(DeriveMigrationName)]
pub struct Migration;

const SQLITE_FOLD_READING_DEVICES: &str = r#"
	INSERT INTO devices (
		id, user_id, name, kind, transform_profile, created_at,
		last_seen_at, last_sync_at, last_sync_summary, revoked_at
	)
	SELECT
		rd.id,
		owner.user_id,
		rd.name,
		CASE WHEN owner.koreader_sessions > 0 THEN 'koreader' ELSE 'opds' END,
		NULL,
		owner.first_seen,
		owner.last_seen,
		owner.last_seen,
		NULL,
		NULL
	FROM reading_devices rd
	JOIN (
		SELECT
			device_id,
			user_id,
			first_seen,
			last_seen,
			koreader_sessions,
			ROW_NUMBER() OVER (PARTITION BY device_id ORDER BY last_seen DESC) AS rn
		FROM (
			SELECT
				je.value AS device_id,
				rs.user_id,
				MIN(rs.created_at) AS first_seen,
				MAX(COALESCE(rs.updated_at, rs.created_at)) AS last_seen,
				SUM(CASE WHEN rs.koreader_progress IS NOT NULL THEN 1 ELSE 0 END) AS koreader_sessions
			FROM reading_sessions rs, json_each(rs.device_ids) je
			WHERE rs.device_ids IS NOT NULL
			GROUP BY je.value, rs.user_id
		)
	) owner ON owner.device_id = rd.id AND owner.rn = 1;
"#;

const POSTGRES_FOLD_READING_DEVICES: &str = r#"
	INSERT INTO devices (
		id, user_id, name, kind, transform_profile, created_at,
		last_seen_at, last_sync_at, last_sync_summary, revoked_at
	)
	SELECT
		rd.id,
		owner.user_id,
		rd.name,
		CASE WHEN owner.koreader_sessions > 0 THEN 'koreader' ELSE 'opds' END,
		NULL,
		owner.first_seen,
		owner.last_seen,
		owner.last_seen,
		NULL,
		NULL
	FROM reading_devices rd
	JOIN (
		SELECT
			device_id,
			user_id,
			first_seen,
			last_seen,
			koreader_sessions,
			ROW_NUMBER() OVER (PARTITION BY device_id ORDER BY last_seen DESC) AS rn
		FROM (
			SELECT
				je.value AS device_id,
				rs.user_id,
				MIN(rs.created_at) AS first_seen,
				MAX(COALESCE(rs.updated_at, rs.created_at)) AS last_seen,
				SUM(CASE WHEN rs.koreader_progress IS NOT NULL THEN 1 ELSE 0 END) AS koreader_sessions
			FROM reading_sessions rs
			CROSS JOIN LATERAL json_array_elements_text(rs.device_ids::json) AS je(value)
			WHERE rs.device_ids IS NOT NULL
			GROUP BY je.value, rs.user_id
		) grouped
	) owner ON owner.device_id = rd.id AND owner.rn = 1;
"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(Devices::Table)
					.if_not_exists()
					.col(ColumnDef::new(Devices::Id).text().not_null().primary_key())
					.col(ColumnDef::new(Devices::UserId).text().not_null())
					.col(ColumnDef::new(Devices::Name).text().not_null())
					.col(ColumnDef::new(Devices::Kind).text().not_null())
					.col(ColumnDef::new(Devices::TransformProfile).json())
					.col(
						ColumnDef::new(Devices::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.col(ColumnDef::new(Devices::LastSeenAt).timestamp_with_time_zone())
					.col(ColumnDef::new(Devices::LastSyncAt).timestamp_with_time_zone())
					.col(ColumnDef::new(Devices::LastSyncSummary).json())
					.col(ColumnDef::new(Devices::RevokedAt).timestamp_with_time_zone())
					.foreign_key(
						ForeignKey::create()
							.name("fk-devices-user")
							.from(Devices::Table, Devices::UserId)
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
					.name("idx-devices-user")
					.table(Devices::Table)
					.col(Devices::UserId)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(DeviceCredentials::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(DeviceCredentials::Id)
							.integer()
							.not_null()
							.auto_increment()
							.primary_key(),
					)
					.col(
						ColumnDef::new(DeviceCredentials::DeviceId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(DeviceCredentials::Protocol)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(DeviceCredentials::CredentialKind)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(DeviceCredentials::CredentialRef)
							.text()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-device_credentials-device")
							.from(DeviceCredentials::Table, DeviceCredentials::DeviceId)
							.to(Devices::Table, Devices::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		// Every `touch` resolves a credential by kind + reference.
		manager
			.create_index(
				Index::create()
					.name("idx-device_credentials-kind-ref")
					.table(DeviceCredentials::Table)
					.col(DeviceCredentials::CredentialKind)
					.col(DeviceCredentials::CredentialRef)
					.unique()
					.to_owned(),
			)
			.await?;

		let conn = manager.get_connection();
		let backend = conn.get_database_backend();
		let fold = match backend {
			DbBackend::Postgres => POSTGRES_FOLD_READING_DEVICES,
			_ => SQLITE_FOLD_READING_DEVICES,
		};
		conn.execute(Statement::from_string(backend, fold.to_string()))
			.await?;

		manager
			.drop_table(Table::drop().table(ReadingDevices::Table).to_owned())
			.await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		// Recreate the legacy table and put the folded rows back. Devices that
		// were minted through the service (credentials, API kinds) have no legacy
		// representation and are dropped with the table.
		manager
			.create_table(
				Table::create()
					.table(ReadingDevices::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(ReadingDevices::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(ReadingDevices::Name)
							.text()
							.not_null()
							.unique_key(),
					)
					.col(ColumnDef::new(ReadingDevices::Kind).text())
					.col(ColumnDef::new(ReadingDevices::Email).text())
					.to_owned(),
			)
			.await?;

		let conn = manager.get_connection();
		conn.execute(Statement::from_string(
			conn.get_database_backend(),
			r#"
				INSERT INTO reading_devices (id, name, kind, email)
				SELECT id, name, NULL, NULL
				FROM (
					SELECT
						d.id,
						d.name,
						ROW_NUMBER() OVER (PARTITION BY d.name ORDER BY d.created_at, d.id) AS rn
					FROM devices d
					WHERE d.kind IN ('koreader', 'opds')
						AND NOT EXISTS (
							SELECT 1 FROM device_credentials dc WHERE dc.device_id = d.id
						)
				) legacy
				WHERE legacy.rn = 1;
			"#
			.to_string(),
		))
		.await?;

		manager
			.drop_table(Table::drop().table(DeviceCredentials::Table).to_owned())
			.await?;
		manager
			.drop_table(Table::drop().table(Devices::Table).to_owned())
			.await?;

		Ok(())
	}
}

#[derive(DeriveIden)]
enum Devices {
	Table,
	Id,
	UserId,
	Name,
	Kind,
	TransformProfile,
	CreatedAt,
	LastSeenAt,
	LastSyncAt,
	LastSyncSummary,
	RevokedAt,
}

#[derive(DeriveIden)]
enum DeviceCredentials {
	Table,
	Id,
	DeviceId,
	Protocol,
	CredentialKind,
	CredentialRef,
}

#[derive(DeriveIden)]
enum ReadingDevices {
	Table,
	Id,
	Name,
	Kind,
	Email,
}

#[derive(DeriveIden)]
enum Users {
	Table,
	Id,
}
