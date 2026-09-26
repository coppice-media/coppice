use migrations::{Migrator, MigratorTrait};
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

#[tokio::test]
async fn liseur_annotation_projection_schema_preserves_scope_and_backfill_state() {
	let db = Database::connect("sqlite::memory:")
		.await
		.expect("SQLite connection should succeed");
	Migrator::up(&db, None)
		.await
		.expect("all migrations should apply on SQLite");

	let annotation_columns = db
		.query_all(Statement::from_string(
			DbBackend::Sqlite,
			"PRAGMA table_info('media_annotations')".to_owned(),
		))
		.await
		.expect("annotation schema query should succeed");
	let color = annotation_columns
		.iter()
		.find(|row| row.try_get::<String>("", "name").unwrap() == "color")
		.expect("native annotations should store color");
	assert_eq!(color.try_get::<i32>("", "notnull").unwrap(), 0);

	let counter_columns = db
		.query_all(Statement::from_string(
			DbBackend::Sqlite,
			"PRAGMA table_info('liseur_sync_counters')".to_owned(),
		))
		.await
		.expect("counter schema query should succeed");
	let projected_seq = counter_columns
		.iter()
		.find(|row| {
			row.try_get::<String>("", "name").unwrap() == "projected_annotation_seq"
		})
		.expect("projection backfill cursor should exist");
	assert_eq!(projected_seq.try_get::<i32>("", "notnull").unwrap(), 1);
	assert_eq!(
		projected_seq
			.try_get::<Option<String>>("", "dflt_value")
			.unwrap()
			.as_deref(),
		Some("0")
	);
	let bridge_columns = db
		.query_all(Statement::from_string(
			DbBackend::Sqlite,
			"PRAGMA table_info('liseur_sync_annotations')".to_owned(),
		))
		.await
		.expect("bridge annotation schema query should succeed");
	let drawer = bridge_columns
		.iter()
		.find(|row| row.try_get::<String>("", "name").unwrap() == "drawer")
		.expect("KOReader highlight styles should have a storage column");
	assert_eq!(drawer.try_get::<i32>("", "notnull").unwrap(), 0);

	let name_columns = db
		.query_all(Statement::from_string(
			DbBackend::Sqlite,
			"PRAGMA table_info('liseur_sync_series_names')".to_owned(),
		))
		.await
		.expect("series-name schema query should succeed");
	let primary_keys = name_columns
		.iter()
		.filter_map(|row| {
			let position = row.try_get::<i32>("", "pk").unwrap();
			(position > 0).then(|| (position, row.try_get::<String>("", "name").unwrap()))
		})
		.collect::<Vec<_>>();
	assert_eq!(
		primary_keys,
		vec![(1, "user_id".to_owned()), (2, "series_id".to_owned()),]
	);

	let foreign_keys = db
		.query_all(Statement::from_string(
			DbBackend::Sqlite,
			"PRAGMA foreign_key_list('liseur_sync_series_names')".to_owned(),
		))
		.await
		.expect("series-name foreign-key query should succeed");
	for (table, column) in [("users", "user_id"), ("series", "series_id")] {
		assert!(foreign_keys.iter().any(|row| {
			row.try_get::<String>("", "table").unwrap() == table
				&& row.try_get::<String>("", "from").unwrap() == column
				&& row.try_get::<String>("", "on_delete").unwrap() == "CASCADE"
		}));
	}
}
