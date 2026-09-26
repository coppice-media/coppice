use migrations::{Migrator, MigratorTrait};
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

#[tokio::test]
async fn liseur_sync_settings_migration_creates_account_scoped_map() {
	let db = Database::connect("sqlite::memory:")
		.await
		.expect("SQLite connection should succeed");
	Migrator::up(&db, None)
		.await
		.expect("all migrations should apply on SQLite");

	let columns = db
		.query_all(Statement::from_string(
			DbBackend::Sqlite,
			"PRAGMA table_info('liseur_sync_settings')".to_owned(),
		))
		.await
		.expect("settings schema query should succeed")
		.iter()
		.map(|row| {
			(
				row.try_get::<String>("", "name").unwrap(),
				row.try_get::<i32>("", "notnull").unwrap(),
				row.try_get::<i32>("", "pk").unwrap(),
			)
		})
		.collect::<Vec<_>>();
	assert_eq!(
		columns,
		vec![
			("user_id".to_owned(), 1, 1),
			("setting_key".to_owned(), 1, 2),
			("value".to_owned(), 1, 0),
			("updated_at".to_owned(), 1, 0),
		]
	);

	let foreign_keys = db
		.query_all(Statement::from_string(
			DbBackend::Sqlite,
			"PRAGMA foreign_key_list('liseur_sync_settings')".to_owned(),
		))
		.await
		.expect("settings foreign-key query should succeed");
	assert!(foreign_keys.iter().any(|row| {
		row.try_get::<String>("", "table").unwrap() == "users"
			&& row.try_get::<String>("", "from").unwrap() == "user_id"
			&& row.try_get::<String>("", "to").unwrap() == "id"
			&& row.try_get::<String>("", "on_delete").unwrap() == "CASCADE"
	}));
}
