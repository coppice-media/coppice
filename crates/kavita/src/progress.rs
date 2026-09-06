//! Reading-progress translation between Kavita's per-chapter `pagesRead` and
//! Stump's reading sessions.
//!
//! Kavita's `ProgressDto.pageNum` is the zero-based index of the page being
//! read and `pageNum == pages` means the chapter is finished; Stump's
//! `reading_sessions.end_page` is the one-based page being read and the
//! session status carries completion. Both directions are covered here so the
//! mapping is defined once.

use std::collections::HashMap;

use chrono::Utc;
use models::{
	domain::reading_progress::compute_page_based_percentage,
	entity::{reading_session, user::AuthUser},
	services::reading_progress::NormalizedProgression,
};
use rust_decimal::prelude::ToPrimitive;
use sea_orm::{
	prelude::*, ConnectionTrait, DatabaseConnection, QueryOrder, Statement, Value,
};

/// Latest reading session per media for one user, keyed by media id.
pub async fn latest_sessions(
	conn: &DatabaseConnection,
	user: &AuthUser,
	media_ids: &[String],
) -> Result<HashMap<String, reading_session::Model>, DbErr> {
	if media_ids.is_empty() {
		return Ok(HashMap::new());
	}
	let rows = reading_session::Entity::find()
		.filter(reading_session::Column::UserId.eq(user.id.clone()))
		.filter(reading_session::Column::MediaId.is_in(media_ids.to_vec()))
		.order_by_desc(reading_session::Column::UpdatedAt)
		.order_by_desc(reading_session::Column::CreatedAt)
		.order_by_desc(reading_session::Column::Id)
		.all(conn)
		.await?;
	let mut latest: HashMap<String, reading_session::Model> =
		HashMap::with_capacity(media_ids.len());
	for row in rows {
		let key = (
			row.updated_at.unwrap_or(row.created_at),
			row.created_at,
			row.id,
		);
		let replace = latest.get(&row.media_id).is_none_or(|current| {
			key > (
				current.updated_at.unwrap_or(current.created_at),
				current.created_at,
				current.id,
			)
		});
		if replace {
			latest.insert(row.media_id.clone(), row);
		}
	}
	Ok(latest)
}

/// Kavita `pagesRead` for a media item with `pages` pages.
pub fn pages_read(session: Option<&reading_session::Model>, pages: i32) -> i32 {
	let pages = pages.max(0);
	let Some(session) = session else {
		return 0;
	};
	if session.is_complete() {
		return pages;
	}
	if let Some(page) = session.end_page {
		return (page - 1).clamp(0, pages);
	}
	session
		.end_percentage
		.and_then(|percentage| percentage.to_f64())
		.map(|percentage| {
			((percentage * f64::from(pages)).floor() as i32).clamp(0, pages)
		})
		.unwrap_or(0)
}

/// The instant Kavita reports as `lastModifiedUtc`/`lastReadingProgressUtc`.
pub fn last_progress_at(
	session: Option<&reading_session::Model>,
) -> Option<chrono::DateTime<Utc>> {
	session.map(|session| {
		session
			.updated_at
			.unwrap_or(session.created_at)
			.with_timezone(&Utc)
	})
}

/// Translate a Kavita `pageNum` into Stump's normalized progression.
///
/// `page_num` is clamped to `0..=pages` like `ReaderService.CapPageToChapter`.
pub fn progression_for_page(page_num: i32, pages: i32) -> NormalizedProgression {
	let pages = pages.max(0);
	let page_num = page_num.clamp(0, pages);
	let did_complete = pages > 0 && page_num >= pages;
	let stump_page = if pages == 0 {
		None
	} else {
		Some((page_num + 1).min(pages))
	};
	NormalizedProgression {
		page: stump_page,
		locator: None,
		percentage: stump_page.map(|page| compute_page_based_percentage(page, pages)),
		elapsed_seconds_delta: None,
		did_complete,
		device_id: None,
		reset_elapsed_seconds: false,
	}
}

/// Progression for `mark-read`: the last page, completed.
pub fn finished_progression(pages: i32) -> NormalizedProgression {
	progression_for_page(pages.max(0), pages)
}

fn statement(conn: &impl ConnectionTrait, sql: &str, values: Vec<Value>) -> Statement {
	Statement::from_sql_and_values(conn.get_database_backend(), sql, values)
}

/// Accessors over the `kavita_progress` side table (EPUB `bookScrollId`).
pub struct KavitaProgress;

impl KavitaProgress {
	pub async fn scroll_id(
		conn: &impl ConnectionTrait,
		user_id: &str,
		media_id: &str,
	) -> Result<Option<String>, DbErr> {
		let row = conn
			.query_one(statement(
				conn,
				"SELECT book_scroll_id FROM kavita_progress WHERE user_id = $1 AND media_id = $2",
				vec![user_id.into(), media_id.into()],
			))
			.await?;
		Ok(row
			.map(|row| row.try_get::<Option<String>>("", "book_scroll_id"))
			.transpose()?
			.flatten())
	}

	pub async fn set_scroll_id(
		conn: &impl ConnectionTrait,
		user_id: &str,
		media_id: &str,
		book_scroll_id: Option<&str>,
	) -> Result<(), DbErr> {
		conn.execute(statement(
			conn,
			"INSERT INTO kavita_progress (user_id, media_id, book_scroll_id, updated_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT(user_id, media_id) DO UPDATE SET
                book_scroll_id = excluded.book_scroll_id,
                updated_at = excluded.updated_at",
			vec![
				user_id.into(),
				media_id.into(),
				book_scroll_id.into(),
				Utc::now().to_rfc3339().into(),
			],
		))
		.await?;
		Ok(())
	}

	pub async fn clear(
		conn: &impl ConnectionTrait,
		user_id: &str,
		media_ids: &[String],
	) -> Result<(), DbErr> {
		for media_id in media_ids {
			conn.execute(statement(
				conn,
				"DELETE FROM kavita_progress WHERE user_id = $1 AND media_id = $2",
				vec![user_id.into(), media_id.as_str().into()],
			))
			.await?;
		}
		Ok(())
	}
}

/// `CREATE TABLE` used by the migration and by in-memory tests.
pub const CREATE_KAVITA_PROGRESS_SQL: &str =
	"CREATE TABLE IF NOT EXISTS kavita_progress (
    user_id TEXT NOT NULL,
    media_id TEXT NOT NULL,
    book_scroll_id TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id, media_id)
)";

#[cfg(test)]
mod tests {
	use super::*;
	use chrono::NaiveDate;
	use models::shared::enums::ReadingStatus;
	use rust_decimal::Decimal;
	use sea_orm::{Database, DatabaseBackend};

	fn session(
		end_page: Option<i32>,
		end_percentage: Option<Decimal>,
		status: ReadingStatus,
	) -> reading_session::Model {
		let now = Utc::now().fixed_offset();
		reading_session::Model {
			id: 1,
			session_date: NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(),
			start_locator: None,
			end_locator: None,
			start_page: None,
			end_page,
			start_percentage: None,
			end_percentage,
			// Audio position; a Kavita chapter is always paged or located.
			end_position_ms: None,
			koreader_progress: None,
			elapsed_seconds: None,
			readthrough_number: 1,
			status,
			notes: None,
			kobo_state: None,
			device_ids: None,
			media_id: "m".to_owned(),
			user_id: "u".to_owned(),
			created_at: now,
			updated_at: None,
		}
	}

	#[test]
	fn pages_read_round_trips_kavita_page_numbers() {
		let pages = 36;
		for page_num in [0, 1, 17, 35, 36] {
			let progression = progression_for_page(page_num, pages);
			let status = if progression.did_complete {
				ReadingStatus::Finished
			} else {
				ReadingStatus::Reading
			};
			let model = session(progression.page, progression.percentage, status);
			assert_eq!(pages_read(Some(&model), pages), page_num, "page {page_num}");
		}
		assert_eq!(pages_read(None, pages), 0);
		assert_eq!(progression_for_page(99, pages).page, Some(36));
		assert!(progression_for_page(99, pages).did_complete);
		assert!(!progression_for_page(35, pages).did_complete);
		assert_eq!(progression_for_page(3, 0).page, None);
		assert!(!progression_for_page(3, 0).did_complete);
	}

	#[test]
	fn stump_native_progress_maps_onto_pages_read() {
		let finished = session(Some(36), None, ReadingStatus::Finished);
		assert_eq!(pages_read(Some(&finished), 36), 36);
		let epub = session(None, Some(Decimal::new(50, 2)), ReadingStatus::Reading);
		assert_eq!(pages_read(Some(&epub), 30), 15);
		let abandoned = session(Some(10), None, ReadingStatus::Abandoned);
		assert_eq!(pages_read(Some(&abandoned), 30), 9);
		assert_eq!(finished_progression(36).page, Some(36));
	}

	#[tokio::test]
	async fn scroll_ids_persist_per_user_and_media() {
		let conn = Database::connect("sqlite::memory:").await.unwrap();
		conn.execute(Statement::from_string(
			DatabaseBackend::Sqlite,
			CREATE_KAVITA_PROGRESS_SQL,
		))
		.await
		.unwrap();
		assert_eq!(
			KavitaProgress::scroll_id(&conn, "u", "m").await.unwrap(),
			None
		);
		KavitaProgress::set_scroll_id(&conn, "u", "m", Some("//div[1]"))
			.await
			.unwrap();
		KavitaProgress::set_scroll_id(&conn, "u", "m", Some("//div[2]"))
			.await
			.unwrap();
		assert_eq!(
			KavitaProgress::scroll_id(&conn, "u", "m").await.unwrap(),
			Some("//div[2]".to_owned())
		);
		assert_eq!(
			KavitaProgress::scroll_id(&conn, "v", "m").await.unwrap(),
			None
		);
		KavitaProgress::clear(&conn, "u", &["m".to_owned()])
			.await
			.unwrap();
		assert_eq!(
			KavitaProgress::scroll_id(&conn, "u", "m").await.unwrap(),
			None
		);
	}
}
