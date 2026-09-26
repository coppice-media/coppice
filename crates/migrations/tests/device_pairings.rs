use migrations::{Migrator, MigratorTrait};
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

async fn sqlite_rows(
	db: &sea_orm::DatabaseConnection,
	sql: String,
) -> Vec<sea_orm::QueryResult> {
	db.query_all(Statement::from_string(DbBackend::Sqlite, sql))
		.await
		.expect("SQLite schema query should succeed")
}

#[tokio::test]
async fn device_pairings_migration_creates_audit_table() {
	let db = Database::connect("sqlite::memory:")
		.await
		.expect("SQLite connection should succeed");
	Migrator::up(&db, None)
		.await
		.expect("all migrations should apply on SQLite");

	let columns =
		sqlite_rows(&db, "PRAGMA table_info('device_pairings')".to_string()).await;
	let described = columns
		.iter()
		.map(|row| {
			(
				row.try_get::<String>("", "name").unwrap(),
				row.try_get::<i32>("", "notnull").unwrap(),
				row.try_get::<i32>("", "pk").unwrap(),
				row.try_get::<Option<String>>("", "dflt_value").unwrap(),
			)
		})
		.collect::<Vec<_>>();
	let expected = [
		("id", 1, 1, None),
		("kind", 1, 0, None),
		("name", 0, 0, None),
		("code_hash", 1, 0, None),
		("nonce", 1, 0, None),
		("remote_ip", 1, 0, None),
		("user_id", 0, 0, None),
		("status", 1, 0, Some("'PENDING'")),
		("failed_attempts", 1, 0, Some("0")),
		("credential_issued", 1, 0, Some("FALSE")),
		("created_at", 1, 0, Some("CURRENT_TIMESTAMP")),
		("expires_at", 1, 0, None),
		("approved_at", 0, 0, None),
		("allow_komf_metadata_editing", 1, 0, Some("FALSE")),
	];
	for (name, not_null, primary_key, default) in expected {
		let column = described
			.iter()
			.find(|(column, ..)| column == name)
			.unwrap_or_else(|| panic!("expected device_pairings.{name}: {described:?}"));
		assert_eq!(column.1, not_null, "nullability of device_pairings.{name}");
		assert_eq!(column.2, primary_key, "pk flag of device_pairings.{name}");
		assert_eq!(
			column.3.as_deref().map(str::to_ascii_uppercase),
			default.map(str::to_ascii_uppercase),
			"default of device_pairings.{name}"
		);
	}
	assert_eq!(described.len(), expected.len(), "{described:?}");

	let foreign_keys = sqlite_rows(
		&db,
		"PRAGMA foreign_key_list('device_pairings')".to_string(),
	)
	.await;
	assert!(
		foreign_keys.iter().any(|row| {
			row.try_get::<String>("", "table").unwrap() == "users"
				&& row.try_get::<String>("", "from").unwrap() == "user_id"
				&& row.try_get::<String>("", "on_delete").unwrap() == "CASCADE"
		}),
		"expected device_pairings.user_id -> users ON DELETE CASCADE"
	);

	let indexes =
		sqlite_rows(&db, "PRAGMA index_list('device_pairings')".to_string()).await;
	let index_names = indexes
		.iter()
		.map(|row| row.try_get::<String>("", "name").unwrap())
		.collect::<Vec<_>>();
	for index in [
		"idx-device-pairings-status-expires",
		"idx-device-pairings-remote-ip",
	] {
		assert!(index_names.contains(&index.to_string()), "{index_names:?}");
	}
	let status_index = sqlite_rows(
		&db,
		"PRAGMA index_info('idx-device-pairings-status-expires')".to_string(),
	)
	.await
	.iter()
	.map(|row| row.try_get::<String>("", "name").unwrap())
	.collect::<Vec<_>>();
	assert_eq!(status_index, ["status", "expires_at"]);
}
