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

async fn assert_table_exists(db: &sea_orm::DatabaseConnection, table: &str) {
	let rows = sqlite_rows(
		db,
		format!(
			"SELECT name FROM sqlite_master WHERE type = 'table' AND name = '{table}'"
		),
	)
	.await;
	assert_eq!(rows.len(), 1, "expected {table} table to exist");
}

async fn assert_table_absent(db: &sea_orm::DatabaseConnection, table: &str) {
	let rows = sqlite_rows(
		db,
		format!(
			"SELECT name FROM sqlite_master WHERE type = 'table' AND name = '{table}'"
		),
	)
	.await;
	assert!(rows.is_empty(), "expected {table} table to be absent");
}

async fn assert_columns(
	db: &sea_orm::DatabaseConnection,
	table: &str,
	expected: &[(&str, i32, i32)],
) {
	let rows = sqlite_rows(db, format!("PRAGMA table_info('{table}')")).await;
	for (name, not_null, primary_key) in expected {
		let column = rows
			.iter()
			.find(|row| row.try_get::<String>("", "name").unwrap() == *name)
			.unwrap_or_else(|| panic!("expected {table}.{name} column"));
		assert_eq!(
			column.try_get::<i32>("", "notnull").unwrap(),
			*not_null,
			"unexpected nullability for {table}.{name}"
		);
		assert_eq!(
			column.try_get::<i32>("", "pk").unwrap(),
			*primary_key,
			"unexpected primary-key flag for {table}.{name}"
		);
	}
}

async fn assert_index(
	db: &sea_orm::DatabaseConnection,
	table: &str,
	index: &str,
	unique: bool,
	expected_columns: &[&str],
) {
	let rows = sqlite_rows(db, format!("PRAGMA index_list('{table}')")).await;
	let row = rows
		.iter()
		.find(|row| row.try_get::<String>("", "name").unwrap() == index)
		.unwrap_or_else(|| panic!("expected {table}.{index} index"));
	assert_eq!(
		row.try_get::<i32>("", "unique").unwrap(),
		unique as i32,
		"unexpected uniqueness for {table}.{index}"
	);

	let columns = sqlite_rows(db, format!("PRAGMA index_info('{index}')")).await;
	let actual_columns = columns
		.iter()
		.map(|row| row.try_get::<String>("", "name").unwrap())
		.collect::<Vec<_>>();
	assert_eq!(actual_columns, expected_columns);
}

async fn assert_foreign_key(
	db: &sea_orm::DatabaseConnection,
	table: &str,
	parent_table: &str,
	child_column: &str,
	on_delete: &str,
) {
	let rows = sqlite_rows(db, format!("PRAGMA foreign_key_list('{table}')")).await;
	assert!(
		rows.iter().any(|row| {
			row.try_get::<String>("", "table").unwrap() == parent_table
				&& row.try_get::<String>("", "from").unwrap() == child_column
				&& row.try_get::<String>("", "on_delete").unwrap() == on_delete
		}),
		"expected {table}.{child_column} -> {parent_table} with ON DELETE {on_delete}"
	);
}

#[tokio::test]
async fn reading_lists_and_collections_migration_has_neutral_schema() {
	let db = Database::connect("sqlite::memory:")
		.await
		.expect("SQLite connection should succeed");

	Migrator::up(&db, None)
		.await
		.expect("all migrations should apply on SQLite");

	for table in [
		"reading_lists",
		"reading_list_items",
		"reading_list_rules",
		"collections",
		"collection_series",
	] {
		assert_table_exists(&db, table).await;
	}

	assert_columns(
		&db,
		"reading_lists",
		&[
			("id", 1, 1),
			("name", 1, 0),
			("description", 0, 0),
			("updated_at", 1, 0),
			("visibility", 1, 0),
			("ordering", 1, 0),
			("creating_user_id", 1, 0),
		],
	)
	.await;
	assert_columns(
		&db,
		"reading_list_items",
		&[
			("id", 1, 1),
			("display_order", 1, 0),
			("media_id", 1, 0),
			("reading_list_id", 1, 0),
		],
	)
	.await;
	assert_columns(
		&db,
		"reading_list_rules",
		&[
			("id", 1, 1),
			("role", 1, 0),
			("user_id", 1, 0),
			("reading_list_id", 1, 0),
		],
	)
	.await;
	assert_columns(
		&db,
		"collections",
		&[
			("id", 1, 1),
			("name", 1, 0),
			("description", 0, 0),
			("updated_at", 1, 0),
			("ordered", 1, 0),
			("creating_user_id", 1, 0),
		],
	)
	.await;
	assert_columns(
		&db,
		"collection_series",
		&[
			("id", 1, 1),
			("display_order", 1, 0),
			("series_id", 1, 0),
			("collection_id", 1, 0),
		],
	)
	.await;

	assert_foreign_key(&db, "reading_lists", "users", "creating_user_id", "CASCADE")
		.await;
	assert_foreign_key(
		&db,
		"reading_list_items",
		"reading_lists",
		"reading_list_id",
		"RESTRICT",
	)
	.await;
	assert_foreign_key(&db, "reading_list_items", "media", "media_id", "RESTRICT").await;
	assert_foreign_key(
		&db,
		"reading_list_rules",
		"reading_lists",
		"reading_list_id",
		"RESTRICT",
	)
	.await;
	assert_foreign_key(&db, "collections", "users", "creating_user_id", "CASCADE").await;
	assert_foreign_key(
		&db,
		"collection_series",
		"collections",
		"collection_id",
		"CASCADE",
	)
	.await;
	assert_foreign_key(&db, "collection_series", "series", "series_id", "RESTRICT").await;

	assert_index(
		&db,
		"reading_lists",
		"idx-reading_lists-creating_user_id",
		false,
		&["creating_user_id"],
	)
	.await;
	assert_index(
		&db,
		"reading_list_items",
		"idx-reading_list_items-reading_list_order",
		false,
		&["reading_list_id", "display_order"],
	)
	.await;
	assert_index(
		&db,
		"reading_list_items",
		"uq-reading_list_items-reading_list_media",
		true,
		&["reading_list_id", "media_id"],
	)
	.await;
	assert_index(
		&db,
		"reading_list_rules",
		"idx-reading_list_rules-user_id",
		false,
		&["user_id"],
	)
	.await;
	assert_index(
		&db,
		"reading_list_rules",
		"uq-reading_list_rules-reading_list_user",
		true,
		&["reading_list_id", "user_id"],
	)
	.await;
	assert_index(
		&db,
		"collections",
		"idx-collections-creating_user_id",
		false,
		&["creating_user_id"],
	)
	.await;
	assert_index(
		&db,
		"collection_series",
		"uq-collection_series-collection_series",
		true,
		&["collection_id", "series_id"],
	)
	.await;
	assert_index(
		&db,
		"collection_series",
		"uq-collection_series-collection_order",
		true,
		&["collection_id", "display_order"],
	)
	.await;

	Migrator::down(&db, Some(1))
		.await
		.expect("latest migration should roll back on SQLite");
	for table in [
		"reading_lists",
		"reading_list_items",
		"reading_list_rules",
		"collections",
		"collection_series",
	] {
		assert_table_absent(&db, table).await;
	}
}
