use migrations::{Migrator, MigratorTrait};
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

async fn sqlite_rows(
	db: &sea_orm::DatabaseConnection,
	sql: &str,
) -> Vec<sea_orm::QueryResult> {
	db.query_all(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
		.await
		.expect("SQLite schema query should succeed")
}

async fn assert_columns(
	db: &sea_orm::DatabaseConnection,
	table: &str,
	expected: &[(&str, i32, i32, Option<&str>)],
) {
	let rows = sqlite_rows(db, &format!("PRAGMA table_info('{table}')")).await;
	let described = rows
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
	for &(name, not_null, primary_key, default) in expected {
		let column = described
			.iter()
			.find(|(column, ..)| column == name)
			.unwrap_or_else(|| panic!("expected {table}.{name}: {described:?}"));
		assert_eq!(column.1, not_null, "nullability of {table}.{name}");
		assert_eq!(column.2, primary_key, "pk flag of {table}.{name}");
		assert_eq!(
			column.3.as_deref().map(str::to_ascii_uppercase),
			default.map(str::to_ascii_uppercase),
			"default of {table}.{name}"
		);
	}
	assert_eq!(described.len(), expected.len(), "columns of {table}");
}

async fn assert_unique_index(
	db: &sea_orm::DatabaseConnection,
	table: &str,
	index: &str,
	expected_columns: &[&str],
) {
	let indexes = sqlite_rows(db, &format!("PRAGMA index_list('{table}')")).await;
	let row = indexes
		.iter()
		.find(|row| row.try_get::<String>("", "name").unwrap() == index)
		.unwrap_or_else(|| panic!("expected {table} index {index}: {indexes:?}"));
	assert_eq!(row.try_get::<i32>("", "unique").unwrap(), 1, "{index}");
	let columns = sqlite_rows(db, &format!("PRAGMA index_info('{index}')"))
		.await
		.iter()
		.map(|row| row.try_get::<String>("", "name").unwrap())
		.collect::<Vec<_>>();
	assert_eq!(columns, expected_columns, "columns of {index}");
}

#[tokio::test]
async fn remote_source_migration_creates_exact_registry_schema() {
	let db = Database::connect("sqlite::memory:")
		.await
		.expect("SQLite connection should succeed");
	Migrator::up(&db, None)
		.await
		.expect("all migrations should apply on SQLite");

	assert_columns(
		&db,
		"remote_sources",
		&[
			("id", 1, 1, None),
			("device_id", 1, 0, None),
			("root_id", 1, 0, None),
			("label", 1, 0, None),
			("kind", 1, 0, None),
			("privacy_mode", 1, 0, None),
			("transport", 1, 0, None),
			("direct_base_url", 0, 0, None),
			("current_revision", 1, 0, Some("0")),
			("last_seen_at", 1, 0, None),
			("health", 1, 0, None),
			("created_at", 1, 0, None),
			("updated_at", 1, 0, None),
		],
	)
	.await;
	assert_columns(
		&db,
		"remote_source_items",
		&[
			("id", 1, 1, None),
			("source_id", 1, 0, None),
			("worker_item_id", 1, 0, None),
			("worker_content_version", 1, 0, None),
			("relative_path", 0, 0, None),
			("size", 1, 0, None),
			("modified_at", 0, 0, None),
			("media_type", 0, 0, None),
			("quick_fingerprint", 0, 0, None),
			("sha256", 0, 0, None),
			("metadata", 0, 0, None),
			("retention", 0, 0, None),
			("observation_state", 1, 0, None),
			("last_seen_revision", 1, 0, None),
			("last_seen_at", 1, 0, None),
			("created_at", 1, 0, None),
			("updated_at", 1, 0, None),
			("imported_media_id", 0, 0, None),
		],
	)
	.await;
	assert_columns(
		&db,
		"media_locations",
		&[
			("id", 1, 1, None),
			("media_id", 1, 0, None),
			("source_item_id", 0, 0, None),
			("kind", 1, 0, None),
			("sha256", 1, 0, None),
			("content_version", 1, 0, None),
			("health", 1, 0, None),
			("durability_role", 1, 0, None),
			("cache_path", 0, 0, None),
			("verified_at", 0, 0, None),
			("last_seen_at", 0, 0, None),
			("created_at", 1, 0, None),
			("updated_at", 1, 0, None),
		],
	)
	.await;
	assert_columns(
		&db,
		"remote_source_imports",
		&[
			("id", 1, 1, None),
			("source_id", 1, 0, None),
			("source_item_id", 1, 0, None),
			("target_media_id", 1, 0, None),
			("worker_item_id", 1, 0, None),
			("worker_content_version", 1, 0, None),
			("source_sha256", 1, 0, None),
			("source_size", 1, 0, None),
			("match_kind", 1, 0, None),
			("match_score", 1, 0, None),
			("match_evidence", 1, 0, None),
			("status", 1, 0, Some("'proposed'")),
			("decision_actor_id", 0, 0, None),
			("decision_at", 0, 0, None),
			("decision_reason", 0, 0, None),
			("applied_location_id", 0, 0, None),
			("staged_item_id", 0, 0, None),
			("created_at", 1, 0, None),
			("updated_at", 1, 0, None),
		],
	)
	.await;

	assert_unique_index(
		&db,
		"remote_sources",
		"idx-remote-sources-device-root",
		&["device_id", "root_id"],
	)
	.await;
	assert_unique_index(
		&db,
		"remote_source_items",
		"idx-remote-source-items-source-worker-item",
		&["source_id", "worker_item_id"],
	)
	.await;
	assert_unique_index(
		&db,
		"media_locations",
		"idx-media-locations-media-source-item-kind",
		&["media_id", "source_item_id", "kind"],
	)
	.await;
	assert_unique_index(
		&db,
		"remote_source_imports",
		"idx-remote-source-imports-identity",
		&[
			"source_item_id",
			"target_media_id",
			"worker_content_version",
			"source_sha256",
		],
	)
	.await;

	for (table, target, from, on_delete) in [
		("remote_sources", "devices", "device_id", "CASCADE"),
		(
			"remote_source_items",
			"remote_sources",
			"source_id",
			"CASCADE",
		),
		(
			"remote_source_items",
			"media",
			"imported_media_id",
			"SET NULL",
		),
		("media_locations", "media", "media_id", "CASCADE"),
		(
			"media_locations",
			"remote_source_items",
			"source_item_id",
			"SET NULL",
		),
		(
			"remote_source_imports",
			"remote_sources",
			"source_id",
			"CASCADE",
		),
		(
			"remote_source_imports",
			"remote_source_items",
			"source_item_id",
			"CASCADE",
		),
		(
			"remote_source_imports",
			"media",
			"target_media_id",
			"CASCADE",
		),
		(
			"remote_source_imports",
			"users",
			"decision_actor_id",
			"SET NULL",
		),
	] {
		let foreign_keys =
			sqlite_rows(&db, &format!("PRAGMA foreign_key_list('{table}')")).await;
		assert!(
			foreign_keys.iter().any(|row| {
				row.try_get::<String>("", "table").unwrap() == target
					&& row.try_get::<String>("", "from").unwrap() == from
					&& row.try_get::<String>("", "on_delete").unwrap() == on_delete
			}),
			"expected {table}.{from} -> {target} ON DELETE {on_delete}: {foreign_keys:?}"
		);
	}
}
