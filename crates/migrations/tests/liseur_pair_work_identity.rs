use migrations::{Migrator, MigratorTrait};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};

const REPAIR: &str = "m20260963_000000_repair_liseur_pair_work_identity";

const SPLIT: &str = r#"
INSERT INTO users (id, username, hashed_password, is_server_owner, is_locked, created_at)
VALUES ('user-1', 'identity-owner', 'unused', TRUE, FALSE, '2026-09-01T00:00:00Z');

INSERT INTO media (id, name, size, extension, pages, is_oneshot, path, status, created_at)
VALUES
    ('media-lottery', 'The Lottery', 100, 'epub', 10, FALSE, '/tmp/lottery.epub', 'READY', '2026-09-01T00:00:00Z'),
    ('media-unrelated', 'Other Book', 100, 'epub', 10, FALSE, '/tmp/other.epub', 'READY', '2026-09-01T00:00:00Z'),
    ('media-pair-only', 'Paired Edition', 100, 'epub', 10, FALSE, '/tmp/pair-only.epub', 'READY', '2026-09-04T00:00:00Z');

INSERT INTO liseur_sync_works (id, user_id, title, author, pending, created_at)
VALUES
    ('identity-work', 'user-1', 'The Lottery', 'Shirley Jackson', FALSE, '2026-09-01T00:00:00Z'),
    ('pair-work', 'user-1', 'The Lottery', 'Shirley Jackson', FALSE, '2026-09-04T00:00:00Z'),
    ('unrelated-work', 'user-1', 'Other Book', 'Someone Else', FALSE, '2026-09-04T00:00:00Z');

INSERT INTO liseur_sync_editions (id, user_id, edition_sha, work_id, media_id, created_at)
VALUES
    ('identity-edition', 'user-1', 'sha-lottery', 'identity-work', 'media-lottery', '2026-09-01T00:00:00Z'),
    ('pair-only-edition', 'user-1', 'sha-pair-only', 'pair-work', 'media-pair-only', '2026-09-04T00:00:00Z');
INSERT INTO liseur_sync_aliases (id, user_id, kind, value, work_id, edition_sha, created_at)
VALUES ('identity-alias', 'user-1', 'sha256', 'sha-lottery', 'identity-work', 'sha-lottery', '2026-09-01T00:00:00Z');

INSERT INTO liseur_sync_media_links
    (id, user_id, media_id, work_id, edition_sha, resolution_status, created_at, pair_status, pair_evidence)
VALUES
    ('pair-link', 'user-1', 'media-lottery', 'pair-work', 'sha-lottery', 'unverified', '2026-09-04T00:00:00Z', 'suggested', 'title_author'),
    ('unrelated-link', 'user-1', 'media-unrelated', 'unrelated-work', '', 'unverified', '2026-09-04T00:00:00Z', 'suggested', 'title_author');

INSERT INTO liseur_sync_counters (user_id, op_seq, annotation_seq, projected_annotation_seq)
VALUES ('user-1', 42, 41, 36);
INSERT INTO liseur_sync_ops
    (id, user_id, seq, op_id, work_id, edition_sha, device_id, client_ts,
     progression, locator, foreign_pos, origin, received_at)
VALUES
    ('position-row', 'user-1', 42, 'op-lottery', 'pair-work', 'sha-lottery',
     'device-1', '2026-09-12T00:00:00Z', 0.42, '{"href":"chapter.xhtml"}', NULL,
     'koreader', '2026-09-12T00:00:01Z');
INSERT INTO liseur_sync_sessions
    (id, user_id, session_id, work_id, edition_sha, device_id, started_at,
     ended_at, start_progression, end_progression, idle_ms, payload, received_at)
VALUES
    ('session-row', 'user-1', 'session-lottery', 'pair-work', 'sha-lottery',
     'device-1', '2026-09-12T00:00:00Z', '2026-09-12T00:05:00Z', 0.2, 0.42, 0,
     '{}', '2026-09-12T00:05:01Z');
INSERT INTO liseur_sync_annotations
    (user_id, annotation_id, rev, seq, work_id, edition_sha, kind, locator,
     progression, excerpt, color, drawer, body, device_id, origin_device_id,
     client_ts, updated_at, deleted, deleted_at, payload)
VALUES
    ('user-1', 'koreader-lottery', 3, 41, 'pair-work', 'sha-lottery', 'highlight',
     '{"href":"chapter.xhtml"}', 0.42, 'lottery excerpt', 'yellow', NULL,
     'Liseur note', 'device-1', 'device-1', '2026-09-12T00:00:00Z',
     '2026-09-12T00:00:00Z', FALSE, NULL, '{}');
INSERT INTO media_annotations (id, locator, annotation_text, color, media_id, user_id, created_at, updated_at)
VALUES
    ('home-native-highlight', '{"href":"chapter.xhtml","locations":{"position":7}}',
     'Home note', 'yellow', 'media-lottery', 'user-1', '2026-09-12T00:00:00Z', '2026-09-12T00:00:00Z');

INSERT INTO book_work_metadata
    (work_id, user_id, title, author, metadata, locked_fields, created_at, updated_at)
VALUES
    ('identity-work', 'user-1', 'The Lottery', 'Shirley Jackson',
     '{"shared":"identity","identityOnly":true}', '["title"]',
     '2026-09-02T00:00:00Z', '2026-09-03T00:00:00Z'),
    ('pair-work', 'user-1', 'Pair title', 'Pair author',
     '{"shared":"pair","pairOnly":true}', '["author"]',
     '2026-09-01T00:00:00Z', '2026-09-04T00:00:00Z');
INSERT INTO book_reviews
    (id, work_id, media_id, user_id, rating, content, is_private, created_at, updated_at)
VALUES
    ('identity-review', 'identity-work', NULL, 'user-1', 4, 'Identity review', FALSE,
     '2026-09-02T00:00:00Z', '2026-09-03T00:00:00Z'),
    ('pair-review', 'pair-work', NULL, 'user-1', 5, 'Pair review', TRUE,
     '2026-09-01T00:00:00Z', '2026-09-04T00:00:00Z');
"#;

const ALIAS_SPLIT: &str = r#"
INSERT INTO users (id, username, hashed_password, is_server_owner, is_locked, created_at)
VALUES ('user-1', 'identity-owner', 'unused', TRUE, FALSE, '2026-09-01T00:00:00Z');

INSERT INTO media
    (id, name, size, extension, pages, is_oneshot, path, status, created_at, koreader_hash)
VALUES
    ('media-lottery', 'The Lottery', 100, 'epub', 10, FALSE, '/tmp/lottery.epub', 'READY', '2026-09-01T00:00:00Z', 'partial-lottery'),
    ('media-copy', 'The Lottery copy', 100, 'epub', 10, FALSE, '/tmp/lottery-copy.epub', 'READY', '2026-09-01T00:00:00Z', 'partial-lottery'),
    ('media-audio', 'The Lottery audiobook', 100, 'm4b', -1, FALSE, '/tmp/lottery.m4b', 'READY', '2026-09-01T00:00:00Z', NULL),
    ('media-location', 'The Lottery location hash', 100, 'epub', 10, FALSE, '/tmp/location.epub', 'READY', '2026-09-01T00:00:00Z', NULL),
    ('media-ambiguous', 'Ambiguous copy', 100, 'epub', 10, FALSE, '/tmp/ambiguous.epub', 'READY', '2026-09-01T00:00:00Z', 'partial-lottery'),
    ('media-weak', 'Weak title match', 100, 'epub', 10, FALSE, '/tmp/weak.epub', 'READY', '2026-09-01T00:00:00Z', NULL);
INSERT INTO liseur_sync_works (id, user_id, title, author, pending, created_at)
VALUES
    ('identity-work', 'user-1', 'The Lottery', 'Shirley Jackson', FALSE, '2026-09-01T00:00:00Z'),
    ('pair-work', 'user-1', 'The Lottery', 'Shirley Jackson', FALSE, '2026-09-04T00:00:00Z'),
    ('conflict-work', 'user-1', 'Other identity', 'Someone Else', FALSE, '2026-09-01T00:00:00Z'),
    ('ambiguous-pair', 'user-1', 'The Lottery', 'Shirley Jackson', FALSE, '2026-09-04T00:00:00Z'),
    ('weak-pair', 'user-1', 'The Lottery', 'Shirley Jackson', FALSE, '2026-09-04T00:00:00Z');

INSERT INTO liseur_sync_editions (id, user_id, edition_sha, work_id, media_id, created_at)
VALUES
    ('identity-edition', 'user-1', 'sha-lottery', 'identity-work', NULL, '2026-09-01T00:00:00Z');

INSERT INTO liseur_sync_aliases (id, user_id, kind, value, work_id, edition_sha, created_at)
VALUES
    ('sha-alias-unmapped', 'user-1', 'sha256', 'sha-lottery', 'identity-work', 'sha-lottery', '2026-09-01T00:00:00Z'),
    ('sha-alias-location', 'user-1', 'sha256', 'sha-location', 'identity-work', NULL, '2026-09-01T00:00:00Z'),
    ('komga-source-alias', 'user-1', 'source', 'komga:media-lottery', 'identity-work', NULL, '2026-09-01T00:00:00Z'),
    ('raw-source-alias', 'user-1', 'source', 'media-copy', 'identity-work', NULL, '2026-09-01T00:00:00Z'),
    ('partial-md5-alias', 'user-1', 'partial-md5', 'partial-lottery', 'identity-work', NULL, '2026-09-01T00:00:00Z'),
    ('weak-title-alias', 'user-1', 'ta', 'the lottery|shirley jackson', 'identity-work', NULL, '2026-09-01T00:00:00Z'),
    ('competing-source-alias', 'user-1', 'source', 'komga:media-ambiguous', 'conflict-work', NULL, '2026-09-01T00:00:00Z');

INSERT INTO liseur_sync_media_links
    (id, user_id, media_id, work_id, edition_sha, resolution_status, created_at, pair_status, pair_evidence)
VALUES
    ('pair-link-source', 'user-1', 'media-lottery', 'pair-work', '', 'unverified', '2026-09-04T00:00:00Z', 'suggested', 'title_author'),
    ('pair-link-partial', 'user-1', 'media-copy', 'pair-work', '', 'unverified', '2026-09-04T00:00:00Z', 'suggested', 'title_author'),
    ('pair-link-audio', 'user-1', 'media-audio', 'pair-work', '', 'unverified', '2026-09-04T00:00:00Z', 'suggested', 'title_author'),
    ('pair-link-location', 'user-1', 'media-location', 'pair-work', '', 'unverified', '2026-09-04T00:00:00Z', 'suggested', 'title_author'),
    ('ambiguous-link', 'user-1', 'media-ambiguous', 'ambiguous-pair', '', 'unverified', '2026-09-04T00:00:00Z', 'suggested', 'title_author'),
    ('weak-link', 'user-1', 'media-weak', 'weak-pair', '', 'unverified', '2026-09-04T00:00:00Z', 'suggested', 'title_author');

INSERT INTO media_locations
    (id, media_id, kind, sha256, content_version, health, durability_role, created_at, updated_at)
VALUES
    ('location-row', 'media-location', 'remote', 'sha-location', 'sha256:sha-location',
     'online', 'remote', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z');

INSERT INTO liseur_sync_annotations
    (user_id, annotation_id, rev, seq, work_id, edition_sha, kind, locator,
     progression, excerpt, color, drawer, body, device_id, origin_device_id,
     client_ts, updated_at, deleted, deleted_at, payload)
VALUES
    ('user-1', 'annotation-split', 7, 19, 'pair-work', 'sha-lottery', 'highlight',
     '{"href":"chapter.xhtml"}', 0.42, 'excerpt', 'yellow', NULL,
     'note', 'device-1', 'device-1', '2026-09-12T00:00:00Z',
     '2026-09-12T00:00:00Z', FALSE, NULL, '{}');
"#;

async fn migrated_database() -> DatabaseConnection {
	let db = Database::connect("sqlite::memory:")
		.await
		.expect("SQLite connection should succeed");
	Migrator::up(&db, None)
		.await
		.expect("all migrations should apply on SQLite");
	db
}

async fn rerun_repair(db: &DatabaseConnection) {
	db.execute(Statement::from_string(
		DbBackend::Sqlite,
		format!("DELETE FROM seaql_migrations WHERE version = '{REPAIR}'"),
	))
	.await
	.expect("repair migration row should be removable for the regression");
	Migrator::up(db, None)
		.await
		.expect("the pairing identity repair should apply");
}

async fn rerun_alias_split_repair(db: &DatabaseConnection) {
	db.execute(Statement::from_string(
		DbBackend::Sqlite,
		"DELETE FROM seaql_migrations WHERE version = 'm20260966_000000_repair_liseur_alias_split'"
			.to_owned(),
	))
	.await
	.expect("alias split migration row should be removable for the regression");
	Migrator::up(db, None)
		.await
		.expect("the alias split repair should apply");
}

#[tokio::test]
async fn split_pair_work_moves_cas_rows_and_preserves_media_projections() {
	let db = migrated_database().await;
	db.execute_unprepared(SPLIT)
		.await
		.expect("the pre-repair split fixture should insert");

	rerun_repair(&db).await;

	let link = db
		.query_one(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT id, work_id, pair_status, pair_evidence FROM liseur_sync_media_links WHERE id = 'pair-link'".to_owned(),
		))
		.await
		.unwrap()
		.expect("the media binding should remain");
	assert_eq!(link.try_get::<String>("", "id").unwrap(), "pair-link");
	assert_eq!(
		link.try_get::<String>("", "work_id").unwrap(),
		"identity-work"
	);
	assert_eq!(
		link.try_get::<String>("", "pair_status").unwrap(),
		"suggested"
	);
	assert_eq!(
		link.try_get::<String>("", "pair_evidence").unwrap(),
		"title_author"
	);

	for (table, id_col, id) in [
		("liseur_sync_editions", "id", "pair-only-edition"),
		(
			"liseur_sync_annotations",
			"annotation_id",
			"koreader-lottery",
		),
		("liseur_sync_ops", "id", "position-row"),
		("liseur_sync_sessions", "id", "session-row"),
	] {
		let sql = format!("SELECT work_id FROM {table} WHERE {id_col} = '{id}'");
		let row = db
			.query_one(Statement::from_string(DbBackend::Sqlite, sql))
			.await
			.unwrap()
			.expect("the Liseur state row should remain");
		assert_eq!(
			row.try_get::<String>("", "work_id").unwrap(),
			"identity-work"
		);
	}
	let cas = db
		.query_one(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT row_id, rev, seq FROM liseur_sync_annotations WHERE annotation_id = 'koreader-lottery'".to_owned(),
		))
		.await
		.unwrap()
		.unwrap();
	assert_eq!(cas.try_get::<i64>("", "row_id").unwrap(), 1);
	assert_eq!(cas.try_get::<i64>("", "rev").unwrap(), 3);
	assert_eq!(cas.try_get::<i64>("", "seq").unwrap(), 41);
	let counters = db
		.query_one(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT op_seq, annotation_seq, projected_annotation_seq FROM liseur_sync_counters WHERE user_id = 'user-1'".to_owned(),
		))
		.await
		.unwrap()
		.unwrap();
	assert_eq!(counters.try_get::<i64>("", "op_seq").unwrap(), 42);
	assert_eq!(counters.try_get::<i64>("", "annotation_seq").unwrap(), 41);
	assert_eq!(
		counters
			.try_get::<i64>("", "projected_annotation_seq")
			.unwrap(),
		36
	);

	let metadata = db
		.query_one(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT work_id, title, author, metadata, locked_fields FROM book_work_metadata WHERE work_id = 'identity-work'".to_owned(),
		))
		.await
		.unwrap()
		.unwrap();
	assert_eq!(
		metadata.try_get::<String>("", "work_id").unwrap(),
		"identity-work"
	);
	assert_eq!(
		metadata.try_get::<String>("", "title").unwrap(),
		"The Lottery"
	);
	let extra: serde_json::Value = metadata.try_get("", "metadata").unwrap();
	assert_eq!(extra["shared"], "identity");
	assert_eq!(extra["identityOnly"], true);
	assert_eq!(extra["pairOnly"], true);
	let locked: serde_json::Value = metadata.try_get("", "locked_fields").unwrap();
	assert_eq!(locked, serde_json::json!(["title", "author"]));
	let review = db
		.query_one(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT rating, content, is_private FROM book_reviews WHERE user_id = 'user-1' AND work_id = 'identity-work'".to_owned(),
		))
		.await
		.unwrap()
		.unwrap();
	assert_eq!(review.try_get::<i32>("", "rating").unwrap(), 5);
	let content: String = review.try_get("", "content").unwrap();
	assert!(content.contains("Identity review"));
	assert!(content.contains("Pair review"));
	assert!(review.try_get::<bool>("", "is_private").unwrap());

	let native = db
		.query_one(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT id, media_id FROM media_annotations WHERE id = 'home-native-highlight'".to_owned(),
		))
		.await
		.unwrap()
		.expect("the media-keyed Home projection should remain");
	assert_eq!(
		native.try_get::<String>("", "media_id").unwrap(),
		"media-lottery"
	);
	let unrelated = db
		.query_one(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT id FROM liseur_sync_works WHERE id = 'unrelated-work'".to_owned(),
		))
		.await
		.unwrap();
	assert!(
		unrelated.is_some(),
		"non-overlapping pairing work must remain"
	);
	let losing = db
		.query_one(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT id FROM liseur_sync_works WHERE id = 'pair-work'".to_owned(),
		))
		.await
		.unwrap();
	assert!(
		losing.is_none(),
		"the duplicate alias-less work must be removed"
	);

	// A repeated application is a no-op after the unique media link is moved.
	rerun_repair(&db).await;
	let annotations = db
		.query_one(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT work_id, rev, seq FROM liseur_sync_annotations WHERE annotation_id = 'koreader-lottery'".to_owned(),
		))
		.await
		.unwrap()
		.unwrap();
	assert_eq!(
		annotations.try_get::<String>("", "work_id").unwrap(),
		"identity-work"
	);
	assert_eq!(annotations.try_get::<i64>("", "rev").unwrap(), 3);
	assert_eq!(annotations.try_get::<i64>("", "seq").unwrap(), 41);
}

#[tokio::test]
async fn alias_tied_split_repairs_from_strong_aliases_and_stored_file_hashes() {
	let db = migrated_database().await;
	db.execute_unprepared(ALIAS_SPLIT)
		.await
		.expect("the alias-tied split fixture should insert");

	rerun_alias_split_repair(&db).await;

	let links = db
		.query_all(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT id, work_id FROM liseur_sync_media_links ORDER BY id".to_owned(),
		))
		.await
		.unwrap();
	let linked_work_ids = links
		.into_iter()
		.map(|row| {
			(
				row.try_get::<String>("", "id").unwrap(),
				row.try_get::<String>("", "work_id").unwrap(),
			)
		})
		.collect::<std::collections::HashMap<_, _>>();
	for link_id in [
		"pair-link-source",
		"pair-link-partial",
		"pair-link-audio",
		"pair-link-location",
	] {
		assert_eq!(linked_work_ids[link_id], "identity-work");
	}
	assert_eq!(linked_work_ids["ambiguous-link"], "ambiguous-pair");
	assert_eq!(linked_work_ids["weak-link"], "weak-pair");

	let editions = db
		.query_all(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT id, work_id, media_id FROM liseur_sync_editions ORDER BY id"
				.to_owned(),
		))
		.await
		.unwrap();
	assert_eq!(editions.len(), 1);
	assert!(editions
		.iter()
		.all(|row| { row.try_get::<String>("", "work_id").unwrap() == "identity-work" }));
	let unmapped = editions
		.iter()
		.find(|row| row.try_get::<String>("", "id").unwrap() == "identity-edition")
		.unwrap();
	assert!(unmapped
		.try_get::<Option<String>>("", "media_id")
		.unwrap()
		.is_none());

	let annotation = db
		.query_one(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT work_id, rev, seq FROM liseur_sync_annotations WHERE annotation_id = 'annotation-split'"
				.to_owned(),
		))
		.await
		.unwrap()
		.unwrap();
	assert_eq!(
		annotation.try_get::<String>("", "work_id").unwrap(),
		"identity-work"
	);
	assert_eq!(annotation.try_get::<i64>("", "rev").unwrap(), 7);
	assert_eq!(annotation.try_get::<i64>("", "seq").unwrap(), 19);

	let losing = db
		.query_one(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT id FROM liseur_sync_works WHERE id = 'pair-work'".to_owned(),
		))
		.await
		.unwrap();
	assert!(
		losing.is_none(),
		"the merged alias-less work should be removed"
	);

	rerun_alias_split_repair(&db).await;
	let link = db
		.query_one(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT work_id FROM liseur_sync_media_links WHERE id = 'pair-link-audio'"
				.to_owned(),
		))
		.await
		.unwrap()
		.unwrap();
	assert_eq!(
		link.try_get::<String>("", "work_id").unwrap(),
		"identity-work"
	);
}
