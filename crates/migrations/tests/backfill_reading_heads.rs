//! `m20260923_000000_backfill_reading_heads` on a database whose reading
//! positions only exist as `reading_sessions` rows.

use migrations::{Migrator, MigratorTrait};
use models::{
	domain::reading_state::{SourceProtocol, TimestampKind},
	entity::{reading_head, reading_head_event},
	services::reading_state,
};
use sea_orm::{
	prelude::Decimal, ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement,
};

const BACKFILL: &str = "m20260923_000000_backfill_reading_heads";

/// The x-pointer KOReader stores for a DOM-based book: not a page, not a
/// Readium locator.
const XPOINTER: &str = "/body/DocFragment[11]/body/div/p[6]/text().123";

/// A database as a release before the unified reading state left it: sessions,
/// devices and the KOReader hash, but no head. Timestamps use the RFC 3339
/// form `sqlx` writes for a `DateTime<FixedOffset>`.
const SEED: &str = r#"
INSERT INTO users (id, username, hashed_password, is_server_owner, is_locked, created_at)
VALUES
	('user-1', 'reader', '', 1, 0, '2026-09-01T09:00:00+00:00'),
	('user-2', 'guest', '', 0, 0, '2026-09-01T09:00:00+00:00');

INSERT INTO media (id, name, size, extension, pages, created_at, path, status, koreader_hash)
VALUES
	('media-percentage', 'Percentage', 1, 'epub', 300, '2026-09-01T09:00:00+00:00', '/books/percentage.epub', 'READY', NULL),
	('media-locator', 'Locator', 1, 'epub', 0, '2026-09-01T09:00:00+00:00', '/books/locator.epub', 'READY', NULL),
	('media-koreader', 'KOReader', 1, 'epub', 0, '2026-09-01T09:00:00+00:00', '/books/koreader.epub', 'READY', 'koreader-hash-1'),
	('media-completed', 'Completed', 1, 'cbz', 940, '2026-09-01T09:00:00+00:00', '/books/completed.cbz', 'READY', NULL),
	('media-blank', 'Blank', 1, 'cbz', 12, '2026-09-01T09:00:00+00:00', '/books/blank.cbz', 'READY', NULL),
	('media-reread', 'Reread', 1, 'cbz', 100, '2026-09-01T09:00:00+00:00', '/books/reread.cbz', 'READY', NULL),
	('media-headed', 'Headed', 1, 'cbz', 100, '2026-09-01T09:00:00+00:00', '/books/headed.cbz', 'READY', NULL),
	('media-cleared', 'Cleared', 1, 'cbz', 50, '2026-09-01T09:00:00+00:00', '/books/cleared.cbz', 'READY', NULL),
	('media-unread', 'Unread', 1, 'cbz', 50, '2026-09-01T09:00:00+00:00', '/books/unread.cbz', 'READY', NULL);

INSERT INTO devices (id, user_id, name, kind, created_at)
VALUES
	('device-web', 'user-1', 'Firefox', 'web', '2026-09-01T09:00:00+00:00'),
	('device-koreader', 'user-1', 'Kobo Clara', 'koreader', '2026-09-01T09:00:00+00:00'),
	('device-foreign', 'user-2', 'Guest tablet', 'koreader', '2026-09-01T09:00:00+00:00');

INSERT INTO reading_sessions (
	id, session_date, end_locator, end_page, end_percentage, koreader_progress,
	elapsed_seconds, readthrough_number, status, device_ids, media_id, user_id,
	created_at, updated_at
)
VALUES
	-- superseded by session 2: the newest session of a pair wins
	(1, '2026-09-01', NULL, NULL, 0.1, NULL, 600, 1, 'READING', '["device-web"]', 'media-percentage', 'user-1', '2026-09-01T09:00:00+00:00', '2026-09-01T09:30:00+00:00'),
	(2, '2026-09-02', NULL, NULL, 0.42, NULL, 900, 1, 'READING', '["device-web"]', 'media-percentage', 'user-1', '2026-09-02T09:00:00+00:00', '2026-09-02T09:30:00+00:00'),
	-- the same book for another user is another head
	(3, '2026-09-02', NULL, NULL, 0.15, NULL, 120, 1, 'READING', NULL, 'media-percentage', 'user-2', '2026-09-02T10:00:00+00:00', '2026-09-02T10:15:00+00:00'),
	-- an EPUB locator with a total progression and a device owned by user-2
	(4, '2026-09-03', '{"chapterTitle":"Chapter 3","href":"chapter3.xhtml","title":"Chapter 3","locations":{"fragments":null,"progression":"0.5","position":null,"totalProgression":"0.73","cssSelector":null,"partialCfi":null},"text":null,"koboSpan":null,"type":"application/xhtml+xml"}', NULL, NULL, NULL, 300, 1, 'READING', '["device-foreign"]', 'media-locator', 'user-1', '2026-09-03T09:00:00+00:00', '2026-09-03T09:05:00+00:00'),
	-- KOReader: an x-pointer plus a percentage
	(5, '2026-09-04', NULL, NULL, 0.66, '/body/DocFragment[11]/body/div/p[6]/text().123', 1800, 1, 'READING', '["device-koreader"]', 'media-koreader', 'user-1', '2026-09-04T09:00:00+00:00', '2026-09-04T09:30:00+00:00'),
	-- a finished paged readthrough; the last contributing device wins
	(6, '2026-09-05', NULL, 940, 1.0, NULL, 3600, 1, 'FINISHED', '["device-web","device-koreader"]', 'media-completed', 'user-1', '2026-09-05T09:00:00+00:00', '2026-09-05T10:00:00+00:00'),
	-- finished without any recorded position
	(7, '2026-09-06', NULL, NULL, NULL, NULL, NULL, 1, 'FINISHED', NULL, 'media-blank', 'user-1', '2026-09-06T09:00:00+00:00', '2026-09-06T09:10:00+00:00'),
	-- a completed readthrough followed by an in-progress re-read
	(8, '2026-09-01', NULL, 100, 1.0, NULL, 3600, 1, 'FINISHED', NULL, 'media-reread', 'user-1', '2026-09-01T09:00:00+00:00', '2026-09-01T11:00:00+00:00'),
	(9, '2026-09-07', NULL, 20, 0.2, NULL, 600, 2, 'READING', NULL, 'media-reread', 'user-1', '2026-09-07T09:00:00+00:00', '2026-09-07T09:10:00+00:00'),
	-- a pair that already has a head
	(10, '2026-09-07', NULL, 90, 0.9, NULL, 600, 1, 'READING', NULL, 'media-headed', 'user-1', '2026-09-07T10:00:00+00:00', '2026-09-07T10:10:00+00:00'),
	-- a pair whose head was cleared, leaving provenance behind
	(11, '2026-09-07', NULL, 15, 0.3, NULL, 600, 1, 'READING', NULL, 'media-cleared', 'user-1', '2026-09-07T11:00:00+00:00', '2026-09-07T11:10:00+00:00');

INSERT INTO reading_head_events (
	id, user_id, media_id, protocol, device_id, raw_payload, locator, progression,
	page, completed, timestamp_kind, source_updated_at, received_at, applied
)
VALUES
	(41, 'user-1', 'media-headed', 'stump', NULL, '{"page":50}', NULL, 0.5, 50, NULL, 'server', '2026-09-07T10:05:00+00:00', '2026-09-07T10:05:00+00:00', 1),
	(42, 'user-1', 'media-cleared', 'stump', NULL, '{"cleared":true}', NULL, NULL, NULL, 0, 'server', '2026-09-07T11:05:00+00:00', '2026-09-07T11:05:00+00:00', 1);

INSERT INTO reading_heads (
	user_id, media_id, locator, progression, page, completed, updated_at,
	created_at, changed_at, source_protocol, source_device_id, revision, event_id
)
VALUES
	('user-1', 'media-headed', NULL, 0.5, 50, 0, '2026-09-07T10:05:00+00:00', '2026-09-07T10:05:00+00:00', '2026-09-07T10:05:00+00:00', 'stump', NULL, 3, 41);
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

/// Re-applies only the backfill. Forgetting its row in the migration table
/// makes it the single pending migration again, which — unlike `down(1)` —
/// stays correct when further migrations are appended after it.
async fn run_backfill(db: &DatabaseConnection) {
	db.execute(Statement::from_string(
		DbBackend::Sqlite,
		format!("DELETE FROM seaql_migrations WHERE version = '{BACKFILL}'"),
	))
	.await
	.expect("the backfill should be forgettable");
	Migrator::up(db, None)
		.await
		.expect("the backfill should re-apply");
}

/// A migrated database, seeded as a v2-era one, with the backfill applied to
/// the seeded rows.
async fn backfilled() -> DatabaseConnection {
	let db = migrated_database().await;
	db.execute_unprepared(SEED)
		.await
		.expect("the v2-era seed should insert");
	run_backfill(&db).await;
	db
}

async fn count(db: &DatabaseConnection, sql: &str) -> i64 {
	db.query_one(Statement::from_string(DbBackend::Sqlite, sql.to_string()))
		.await
		.expect("the count should run")
		.expect("a count always returns a row")
		.try_get::<i64>("", "total")
		.expect("the count should be an integer")
}

async fn head_of(db: &DatabaseConnection, media_id: &str) -> reading_head::Model {
	reading_state::head(db, "user-1", media_id)
		.await
		.expect("the head query should run")
		.unwrap_or_else(|| panic!("{media_id} should have a backfilled head"))
}

async fn event_of(
	db: &DatabaseConnection,
	head: &reading_head::Model,
) -> reading_head_event::Model {
	reading_state::winning_event(db, head)
		.await
		.expect("the event query should run")
		.expect("a head always points at its provenance row")
}

#[tokio::test]
async fn backfill_projects_a_percentage_only_session() {
	let db = backfilled().await;

	let head = head_of(&db, "media-percentage").await;
	assert_eq!(head.progression, 0.42, "the newest session wins");
	assert_eq!(head.page, None, "no page was recorded");
	assert_eq!(head.locator, None, "a locator is never fabricated");
	assert!(!head.completed);
	assert_eq!(head.source_protocol, SourceProtocol::Stump);
	assert_eq!(head.source_device_id.as_deref(), Some("device-web"));
	assert_eq!(head.revision, 1);
	assert_eq!(
		head.updated_at.to_rfc3339(),
		"2026-09-02T09:30:00+00:00",
		"the head keeps the session's effective time"
	);
	assert!(
		head.changed_at > head.updated_at,
		"changed_at is the backfill's server time"
	);

	let event = event_of(&db, &head).await;
	assert_eq!(event.protocol, SourceProtocol::Stump);
	assert_eq!(event.progression, Some(0.42));
	assert_eq!(event.page, None);
	assert_eq!(
		event.completed, None,
		"an unfinished session asserts nothing about completion"
	);
	assert!(event.applied);
	assert_eq!(
		event.timestamp_kind,
		TimestampKind::Server,
		"session timestamps are server ingestion times"
	);
	assert_eq!(event.source_updated_at, head.updated_at);
	assert_eq!(event.raw_payload["backfill"], BACKFILL);
	assert_eq!(
		event.raw_payload["session"]["id"], 2,
		"the winning session is retained verbatim"
	);
	assert_eq!(event.raw_payload["session"]["elapsed_seconds"], 900);
	assert_eq!(event.raw_payload["session"]["device_ids"][0], "device-web");
	assert_eq!(
		event.raw_payload["session"]["updated_at"],
		"2026-09-02T09:30:00+00:00"
	);

	let other_user = reading_state::head(&db, "user-2", "media-percentage")
		.await
		.expect("the head query should run")
		.expect("the other user's session is its own head");
	assert_eq!(other_user.progression, 0.15);
	assert_eq!(other_user.source_device_id, None);
}

#[tokio::test]
async fn backfill_keeps_an_epub_locator_verbatim() {
	let db = backfilled().await;

	let head = head_of(&db, "media-locator").await;
	let locator = head.locator.as_ref().expect("the locator is carried over");
	assert_eq!(locator.href, "chapter3.xhtml");
	assert_eq!(locator.chapter_title, "Chapter 3");
	assert_eq!(locator.r#type, "application/xhtml+xml");
	let locations = locator
		.locations
		.as_ref()
		.expect("locations are carried over");
	assert_eq!(locations.total_progression, Some(Decimal::new(73, 2)));
	assert_eq!(locations.progression, Some(Decimal::new(5, 1)));
	assert_eq!(
		head.progression, 0.73,
		"an absent percentage falls back to the locator's total progression"
	);
	assert_eq!(head.page, None, "the locator has no position");
	assert_eq!(
		head.source_device_id, None,
		"a device owned by another user is not the head's device"
	);
}

#[tokio::test]
async fn backfill_replays_a_koreader_session_as_a_koreader_event() {
	let db = backfilled().await;

	let head = head_of(&db, "media-koreader").await;
	assert_eq!(head.source_protocol, SourceProtocol::Koreader);
	assert_eq!(head.progression, 0.66);
	assert_eq!(head.page, None, "an x-pointer is not a page");
	assert_eq!(head.locator, None, "an x-pointer is not a Readium locator");
	assert_eq!(head.source_device_id.as_deref(), Some("device-koreader"));

	let event = event_of(&db, &head).await;
	assert_eq!(event.protocol, SourceProtocol::Koreader);
	assert_eq!(event.raw_payload["document"], "koreader-hash-1");
	assert_eq!(
		event.raw_payload["progress"], XPOINTER,
		"the native progress string is replayable"
	);
	assert_eq!(event.raw_payload["percentage"], 0.66);
	assert_eq!(event.raw_payload["device"], "Kobo Clara");
	assert_eq!(event.raw_payload["device_id"], "device-koreader");
	assert_eq!(event.raw_payload["session"]["koreader_progress"], XPOINTER);
}

#[tokio::test]
async fn backfill_projects_a_finished_paged_session() {
	let db = backfilled().await;

	let head = head_of(&db, "media-completed").await;
	assert!(head.completed);
	assert_eq!(head.progression, 1.0);
	assert_eq!(head.page, Some(940));
	let locator = head.locator.as_ref().expect("a page gets a page locator");
	assert_eq!(locator.href, "/api/v2/media/media-completed/page/940");
	assert_eq!(locator.title.as_deref(), Some("Page 940"));
	assert_eq!(locator.r#type, "image/jpeg");
	let locations = locator
		.locations
		.as_ref()
		.expect("the page locator locates");
	assert_eq!(locations.position, Some(940));
	assert_eq!(locations.total_progression, Some(Decimal::ONE));
	assert_eq!(
		head.source_device_id.as_deref(),
		Some("device-koreader"),
		"the last contributing device is the head's device"
	);

	let event = event_of(&db, &head).await;
	assert_eq!(
		event.protocol,
		SourceProtocol::Stump,
		"a session without a KOReader progress string is a Stump event"
	);
	assert_eq!(event.completed, Some(true));
	assert_eq!(
		event.raw_payload["session"]["device_ids"],
		serde_json::json!(["device-web", "device-koreader"]),
		"the complete contributing set stays in provenance"
	);
}

#[tokio::test]
async fn backfill_lands_a_positionless_finished_session_on_the_last_page() {
	let db = backfilled().await;

	let head = head_of(&db, "media-blank").await;
	assert!(head.completed);
	assert_eq!(head.progression, 1.0);
	assert_eq!(head.page, Some(12), "completion lands on the last page");
	assert_eq!(
		head.locator, None,
		"the session recorded no position, so no locator is synthesized"
	);
}

#[tokio::test]
async fn backfill_keeps_completion_of_an_earlier_readthrough() {
	let db = backfilled().await;

	let head = head_of(&db, "media-reread").await;
	assert_eq!(head.progression, 0.2, "the re-read is the current position");
	assert_eq!(head.page, Some(20));
	assert!(
		head.completed,
		"completion is sticky: the first readthrough finished"
	);

	let event = event_of(&db, &head).await;
	assert_eq!(event.raw_payload["session"]["id"], 9);
	assert_eq!(event.raw_payload["finished_readthroughs"], 1);
}

#[tokio::test]
async fn backfill_orders_events_by_session_time() {
	let db = backfilled().await;

	let percentage = head_of(&db, "media-percentage").await;
	let locator = head_of(&db, "media-locator").await;
	let koreader = head_of(&db, "media-koreader").await;
	let completed = head_of(&db, "media-completed").await;
	assert!(
		percentage.event_id < locator.event_id
			&& locator.event_id < koreader.event_id
			&& koreader.event_id < completed.event_id,
		"the event id is the server sequence, seeded in session-time order"
	);
	assert!(
		percentage.event_id > 42,
		"the backfill appends to the existing provenance stream"
	);
}

#[tokio::test]
async fn backfill_skips_a_pair_that_already_has_a_head() {
	let db = backfilled().await;

	let head = head_of(&db, "media-headed").await;
	assert_eq!(head.progression, 0.5, "an existing head is authoritative");
	assert_eq!(head.revision, 3);
	assert_eq!(head.event_id, 41);
	assert_eq!(
		count(
			&db,
			"SELECT COUNT(*) AS total FROM reading_head_events WHERE media_id = 'media-headed'"
		)
		.await,
		1,
		"no provenance is invented for a pair that already has a head"
	);
	assert!(
		reading_state::head(&db, "user-1", "media-unread")
			.await
			.expect("the head query should run")
			.is_none(),
		"a book without sessions gets no head"
	);
}

#[tokio::test]
async fn backfill_appends_to_a_cleared_provenance_stream() {
	let db = backfilled().await;

	let head = head_of(&db, "media-cleared").await;
	assert_eq!(head.progression, 0.3);
	assert!(
		head.event_id > 42,
		"the head points at the backfilled event, not the cleared one"
	);
	assert_eq!(
		count(
			&db,
			"SELECT COUNT(*) AS total FROM reading_head_events WHERE media_id = 'media-cleared'"
		)
		.await,
		2,
		"the cleared event is kept"
	);
}

#[tokio::test]
async fn backfill_is_idempotent() {
	let db = backfilled().await;

	let heads = count(&db, "SELECT COUNT(*) AS total FROM reading_heads").await;
	let events = count(&db, "SELECT COUNT(*) AS total FROM reading_head_events").await;
	assert_eq!(heads, 9, "one head per (user, media) pair with sessions");
	assert_eq!(
		events, 10,
		"eight backfilled events plus the two seeded ones"
	);
	let before = head_of(&db, "media-percentage").await;

	run_backfill(&db).await;

	assert_eq!(
		count(&db, "SELECT COUNT(*) AS total FROM reading_heads").await,
		heads,
		"re-running the backfill inserts no head"
	);
	assert_eq!(
		count(&db, "SELECT COUNT(*) AS total FROM reading_head_events").await,
		events,
		"re-running the backfill inserts no provenance"
	);
	assert_eq!(head_of(&db, "media-percentage").await, before);
}

#[tokio::test]
async fn backfill_is_a_no_op_on_an_empty_database() {
	let db = migrated_database().await;

	run_backfill(&db).await;

	assert_eq!(
		count(&db, "SELECT COUNT(*) AS total FROM reading_heads").await,
		0
	);
}
