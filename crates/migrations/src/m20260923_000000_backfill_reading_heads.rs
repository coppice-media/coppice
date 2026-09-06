//! Phase 2 of the unified reading state: materialize a `reading_heads` row for
//! every `(user, media)` pair whose reading position only exists as
//! `reading_sessions` history.
//!
//! `m20260914_000000_add_reading_heads` added the head/provenance tables but
//! left them empty, so every database upgraded from a pre-head release reports
//! "no progress" on the protocol lanes that read the head (KOReader, Komga,
//! Kobo, OPDS 2.0, liseur-sync). This migration replays the newest session of
//! each pair as one protocol event and materializes its head, reproducing the
//! projection/conflict rules of `models::services::reading_state::apply`
//! (`models::domain::reading_state::{project, resolve}`) in plain SQL + Rust so
//! the migration keeps working when those entities change.
//!
//! The mapping is documented in
//! `docs/content/docs/developer/unified-reading-state.mdx` ("Phase 2 —
//! deterministic backfill"). It is idempotent: a pair that already has a head
//! is skipped, so re-running inserts nothing. The literal `'stump'`,
//! `'koreader'`, `'server'` and `'FINISHED'` strings below are the stored
//! representations of `SourceProtocol`, `TimestampKind` and `ReadingStatus`.

use std::collections::HashMap;

use chrono::Utc;
use sea_orm::{
	prelude::{Date, DateTimeWithTimeZone, Decimal},
	ConnectionTrait, DbBackend, QueryResult, Statement, TransactionTrait, Value,
};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Marks the provenance rows this migration created, so a backfilled head can
/// be told apart from one a protocol wrote.
const BACKFILL: &str = "m20260923_000000_backfill_reading_heads";

/// The newest session of every `(user, media)` pair that has no head yet, in
/// the order their events must be inserted: the `reading_head_events.id`
/// sequence is the `server_seq` of the conflict rule, so equal timestamps are
/// ordered by the existing session id. The winner is picked by one ranking
/// pass, like the other data migrations in this crate.
const WINNING_SESSIONS: &str = r#"
	WITH ranked AS (
		SELECT
			rs.*,
			ROW_NUMBER() OVER (
				PARTITION BY rs.user_id, rs.media_id
				ORDER BY COALESCE(rs.updated_at, rs.created_at) DESC, rs.id DESC
			) AS session_rank
		FROM reading_sessions rs
	)
	SELECT
		rs.id AS session_id,
		rs.user_id AS user_id,
		rs.media_id AS media_id,
		rs.session_date AS session_date,
		rs.notes AS notes,
		rs.start_locator AS start_locator,
		rs.end_locator AS end_locator,
		rs.start_page AS start_page,
		rs.end_page AS end_page,
		CAST(rs.start_percentage AS DOUBLE PRECISION) AS start_percentage,
		CAST(rs.end_percentage AS DOUBLE PRECISION) AS end_percentage,
		rs.koreader_progress AS koreader_progress,
		rs.kobo_state AS kobo_state,
		rs.elapsed_seconds AS elapsed_seconds,
		rs.readthrough_number AS readthrough_number,
		rs.status AS status,
		rs.device_ids AS device_ids,
		rs.created_at AS created_at,
		rs.updated_at AS updated_at,
		m.pages AS pages,
		m.koreader_hash AS koreader_hash
	FROM ranked rs
	JOIN media m ON m.id = rs.media_id
	WHERE rs.session_rank = 1
	AND NOT EXISTS (
		SELECT 1 FROM reading_heads rh
		WHERE rh.user_id = rs.user_id AND rh.media_id = rs.media_id
	)
	ORDER BY COALESCE(rs.updated_at, rs.created_at) ASC, rs.id ASC
"#;

/// Completed readthroughs per pair. Completion is sticky in the unified head
/// (`resolve` only clears it on an explicit un-read), so a pair whose newest
/// session is a re-read still reports the finished readthrough it already had.
const FINISHED_READTHROUGHS: &str = r#"
	SELECT user_id AS user_id, media_id AS media_id, COUNT(*) AS finished
	FROM reading_sessions
	WHERE status = 'FINISHED'
	GROUP BY user_id, media_id
"#;

/// The device registry, used to resolve a session's contributing device ids to
/// a head/event device. A session may name a device that no longer exists.
const REGISTERED_DEVICES: &str =
	"SELECT id AS id, user_id AS user_id, name AS name FROM devices";

/// The event of every pair that still has no head. The id sequence is
/// monotonic, so after the inserts below `MAX(id)` is the row this migration
/// wrote for the pair, even when a cleared head left older events behind.
const BACKFILLED_EVENT_IDS: &str = r#"
	SELECT rhe.user_id AS user_id, rhe.media_id AS media_id, MAX(rhe.id) AS event_id
	FROM reading_head_events rhe
	WHERE NOT EXISTS (
		SELECT 1 FROM reading_heads rh
		WHERE rh.user_id = rhe.user_id AND rh.media_id = rhe.media_id
	)
	GROUP BY rhe.user_id, rhe.media_id
"#;

const INSERT_EVENT: &str = r#"
	INSERT INTO reading_head_events (
		user_id, media_id, protocol, device_id, raw_payload, locator,
		progression, page, completed, timestamp_kind, source_updated_at,
		received_at, applied
	)
	VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
"#;

const INSERT_HEAD: &str = r#"
	INSERT INTO reading_heads (
		user_id, media_id, locator, progression, page, completed, updated_at,
		created_at, changed_at, source_protocol, source_device_id, revision,
		event_id
	)
	VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let conn = manager.get_connection();
		let backend = conn.get_database_backend();

		let sessions = conn
			.query_all(Statement::from_string(
				backend,
				WINNING_SESSIONS.to_string(),
			))
			.await?;
		if sessions.is_empty() {
			return Ok(());
		}

		let finished = finished_readthroughs(conn, backend).await?;
		let devices = registered_devices(conn, backend).await?;

		let backfilled = sessions
			.iter()
			.map(|session| Backfilled::project(session, &finished, &devices))
			.collect::<Result<Vec<_>, DbErr>>()?;

		let now = DateTimeWithTimeZone::from(Utc::now());
		let tx = conn.begin().await?;

		for row in &backfilled {
			tx.execute(Statement::from_sql_and_values(
				backend,
				INSERT_EVENT,
				[
					row.user_id.clone().into(),
					row.media_id.clone().into(),
					row.protocol.into(),
					row.device_id.clone().into(),
					Value::Json(Some(Box::new(row.raw_payload.clone()))),
					Value::Json(row.locator.clone().map(Box::new)),
					row.asserted_progression.into(),
					row.page.into(),
					row.completed.then_some(true).into(),
					// A session timestamp is server ingestion time; no Stump or
					// KOReader write carries a device clock.
					"server".into(),
					row.source_updated_at.into(),
					now.into(),
					true.into(),
				],
			))
			.await?;
		}

		let event_ids = tx
			.query_all(Statement::from_string(
				backend,
				BACKFILLED_EVENT_IDS.to_string(),
			))
			.await?
			.iter()
			.map(|row| {
				Ok((
					(row.try_get("", "user_id")?, row.try_get("", "media_id")?),
					row.try_get::<i64>("", "event_id")?,
				))
			})
			.collect::<Result<HashMap<(String, String), i64>, DbErr>>()?;

		for row in &backfilled {
			let event_id = event_ids
				.get(&(row.user_id.clone(), row.media_id.clone()))
				.copied()
				.ok_or_else(|| {
					DbErr::Custom(format!(
						"reading-head backfill lost the event it inserted for ({}, {})",
						row.user_id, row.media_id
					))
				})?;
			tx.execute(Statement::from_sql_and_values(
				backend,
				INSERT_HEAD,
				[
					row.user_id.clone().into(),
					row.media_id.clone().into(),
					Value::Json(row.locator.clone().map(Box::new)),
					row.progression.into(),
					row.page.into(),
					row.completed.into(),
					row.source_updated_at.into(),
					now.into(),
					now.into(),
					row.protocol.into(),
					row.device_id.clone().into(),
					1i32.into(),
					event_id.into(),
				],
			))
			.await?;
		}

		tx.commit().await
	}

	async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
		// A data backfill has no safe inverse: from the moment the heads exist
		// every protocol write advances them, so deleting the rows created here
		// would discard progress recorded after this migration ran. The rows
		// stay attributable through `raw_payload -> 'backfill'`, and `up` is
		// idempotent, so a rollback/re-apply cycle is a no-op.
		Ok(())
	}
}

/// One `(reading_head_event, reading_head)` pair derived from a session.
struct Backfilled {
	user_id: String,
	media_id: String,
	protocol: &'static str,
	device_id: Option<String>,
	raw_payload: serde_json::Value,
	locator: Option<serde_json::Value>,
	page: Option<i32>,
	/// The progression the event asserts; `None` when the session carried no
	/// percentage and none could be derived, matching `project`.
	asserted_progression: Option<f64>,
	/// The materialized head progression, which is never null.
	progression: f64,
	completed: bool,
	source_updated_at: DateTimeWithTimeZone,
}

impl Backfilled {
	fn project(
		session: &QueryResult,
		finished: &HashMap<(String, String), i64>,
		devices: &HashMap<String, Device>,
	) -> Result<Self, DbErr> {
		let session_id = session.try_get::<i32>("", "session_id")?;
		let user_id = session.try_get::<String>("", "user_id")?;
		let media_id = session.try_get::<String>("", "media_id")?;
		let pages = session.try_get::<i32>("", "pages")?;

		let end_locator =
			session.try_get::<Option<serde_json::Value>>("", "end_locator")?;
		let end_page = session.try_get::<Option<i32>>("", "end_page")?;
		let end_percentage = session.try_get::<Option<f64>>("", "end_percentage")?;
		let koreader_progress =
			session.try_get::<Option<String>>("", "koreader_progress")?;
		let koreader_hash = session.try_get::<Option<String>>("", "koreader_hash")?;
		let device_ids =
			session.try_get::<Option<serde_json::Value>>("", "device_ids")?;
		let created_at = session.try_get::<DateTimeWithTimeZone>("", "created_at")?;
		let updated_at =
			session.try_get::<Option<DateTimeWithTimeZone>>("", "updated_at")?;

		// Completion is sticky in `resolve`, so it is the pair's aggregate: a
		// re-read of a finished book stays finished.
		let finished_readthroughs = finished
			.get(&(user_id.clone(), media_id.clone()))
			.copied()
			.unwrap_or(0);
		let completed = finished_readthroughs > 0;

		// `project`: an asserted percentage wins, then the locator's own total
		// progression, then the page coordinate. A locator is never synthesized
		// for a position the session did not record.
		let asserted = end_percentage.and_then(clamp_unit);
		let (locator, page, progression) = match (&end_locator, end_page) {
			(Some(locator), _) => {
				let locations = locator.get("locations");
				let position =
					json_i32(locations.and_then(|locations| locations.get("position")));
				let total = json_f64(
					locations.and_then(|locations| locations.get("totalProgression")),
				);
				let progression = asserted
					.or_else(|| total.and_then(clamp_unit))
					.or_else(|| position.and_then(|page| page_progression(page, pages)));
				(Some(locator.clone()), position, progression)
			},
			(None, Some(page)) => (
				Some(page_locator(&media_id, page, pages)),
				Some(page),
				asserted.or_else(|| page_progression(page, pages)),
			),
			(None, None) => (None, None, asserted),
		};
		// `project`: completion without any position lands on the last page.
		let (page, progression) = if completed && page.is_none() && progression.is_none()
		{
			((pages > 0).then_some(pages), Some(1.0))
		} else {
			(page, progression)
		};

		// The last contributing device is the most recently registered one; the
		// complete set stays in the raw payload.
		let device = device_ids
			.as_ref()
			.and_then(|ids| ids.as_array())
			.into_iter()
			.flatten()
			.rev()
			.filter_map(|id| id.as_str())
			.find_map(|id| devices.get(id).filter(|device| device.user_id == user_id));

		let mut raw_payload = serde_json::json!({
			"backfill": BACKFILL,
			"finished_readthroughs": finished_readthroughs,
			"session": {
				"id": session_id,
				"session_date": session
					.try_get::<Date>("", "session_date")?
					.to_string(),
				"notes": session.try_get::<Option<String>>("", "notes")?,
				"start_locator": session
					.try_get::<Option<serde_json::Value>>("", "start_locator")?,
				"end_locator": end_locator,
				"start_page": session.try_get::<Option<i32>>("", "start_page")?,
				"end_page": end_page,
				"start_percentage": session
					.try_get::<Option<f64>>("", "start_percentage")?,
				"end_percentage": end_percentage,
				"koreader_progress": koreader_progress,
				"kobo_state": session
					.try_get::<Option<serde_json::Value>>("", "kobo_state")?,
				"elapsed_seconds": session
					.try_get::<Option<i64>>("", "elapsed_seconds")?,
				"readthrough_number": session
					.try_get::<i32>("", "readthrough_number")?,
				"status": session.try_get::<String>("", "status")?,
				"device_ids": device_ids,
				"media_id": media_id,
				"user_id": user_id,
				"created_at": created_at.to_rfc3339(),
				"updated_at": updated_at.map(|at| at.to_rfc3339()),
			},
		});

		// A session carrying `koreader_progress` was last written by the
		// KOReader lane, so its event replays the native KOReader request: the
		// document hash, the verbatim progress string (a page or an x-pointer),
		// the percentage, and the device. `get_progress` reads those fields
		// back off the winning event.
		let protocol = match &koreader_progress {
			Some(progress) => {
				let payload = raw_payload
					.as_object_mut()
					.expect("the payload is a JSON object");
				payload.insert("document".to_string(), koreader_hash.into());
				payload.insert("progress".to_string(), progress.clone().into());
				payload.insert("percentage".to_string(), progression.into());
				payload.insert(
					"device".to_string(),
					device.map(|device| device.name.clone()).into(),
				);
				payload.insert(
					"device_id".to_string(),
					device.map(|device| device.id.clone()).into(),
				);
				"koreader"
			},
			None => "stump",
		};

		Ok(Self {
			user_id,
			media_id,
			protocol,
			device_id: device.map(|device| device.id.clone()),
			raw_payload,
			locator,
			page,
			asserted_progression: progression,
			// `resolve` with no existing head: an absent progression is 0, or 1
			// when the update reports completion.
			progression: progression.unwrap_or(if completed { 1.0 } else { 0.0 }),
			completed,
			source_updated_at: updated_at.unwrap_or(created_at),
		})
	}
}

struct Device {
	id: String,
	user_id: String,
	name: String,
}

async fn finished_readthroughs<C: ConnectionTrait>(
	conn: &C,
	backend: DbBackend,
) -> Result<HashMap<(String, String), i64>, DbErr> {
	conn.query_all(Statement::from_string(
		backend,
		FINISHED_READTHROUGHS.to_string(),
	))
	.await?
	.iter()
	.map(|row| {
		Ok((
			(row.try_get("", "user_id")?, row.try_get("", "media_id")?),
			row.try_get::<i64>("", "finished")?,
		))
	})
	.collect()
}

async fn registered_devices<C: ConnectionTrait>(
	conn: &C,
	backend: DbBackend,
) -> Result<HashMap<String, Device>, DbErr> {
	conn.query_all(Statement::from_string(
		backend,
		REGISTERED_DEVICES.to_string(),
	))
	.await?
	.iter()
	.map(|row| {
		let device = Device {
			id: row.try_get("", "id")?,
			user_id: row.try_get("", "user_id")?,
			name: row.try_get("", "name")?,
		};
		Ok((device.id.clone(), device))
	})
	.collect()
}

/// `page_progression`: whole-publication progression of a 1-based page.
fn page_progression(page: i32, pages: i32) -> Option<f64> {
	(pages > 0).then(|| (f64::from(page) / f64::from(pages)).clamp(0.0, 1.0))
}

fn clamp_unit(value: f64) -> Option<f64> {
	value.is_finite().then(|| value.clamp(0.0, 1.0))
}

/// `page_locator`: the provider-derived anchor a page-addressed position gets.
/// The decimals are formatted the way `Option<Decimal>` serializes so a
/// backfilled locator is byte-identical to one a live write produced.
fn page_locator(media_id: &str, page: i32, pages: i32) -> serde_json::Value {
	let progression = page_progression(page, pages)
		.and_then(|value| <Decimal as TryFrom<f64>>::try_from(value).ok())
		.map(|value| value.to_string());
	serde_json::json!({
		"chapterTitle": "",
		"href": format!("/api/v2/media/{media_id}/page/{page}"),
		"title": format!("Page {page}"),
		"locations": {
			"fragments": None::<Vec<String>>,
			"progression": progression,
			"position": page,
			"totalProgression": progression,
			"cssSelector": None::<String>,
			"partialCfi": None::<String>,
		},
		"text": None::<String>,
		"koboSpan": None::<String>,
		"type": "image/jpeg",
	})
}

/// A stored `Decimal` is a JSON string, a value written by SQL is a number.
fn json_f64(value: Option<&serde_json::Value>) -> Option<f64> {
	match value? {
		serde_json::Value::Number(number) => number.as_f64(),
		serde_json::Value::String(text) => text.parse().ok(),
		_ => None,
	}
}

fn json_i32(value: Option<&serde_json::Value>) -> Option<i32> {
	match value? {
		serde_json::Value::Number(number) => number
			.as_i64()
			.and_then(|value| <i32 as TryFrom<i64>>::try_from(value).ok()),
		serde_json::Value::String(text) => text.parse().ok(),
		_ => None,
	}
}
