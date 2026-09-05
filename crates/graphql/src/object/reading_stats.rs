use async_graphql::{Enum, Result, SimpleObject};
use chrono::{Duration, NaiveDate};
use models::shared::enums::DeviceKind;
use sea_orm::{prelude::*, DatabaseConnection, FromQueryResult};

use crate::utils::db_statement;

/// How far back reading statistics reach, counted in logical reading days
/// ending today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ReadingStatsSpan {
	/// The last 7 days
	Week,
	/// The last 30 days
	Month,
	/// The last 90 days
	Quarter,
	/// The last 365 days
	Year,
	AllTime,
}

impl ReadingStatsSpan {
	fn days(self) -> Option<i64> {
		match self {
			Self::Week => Some(7),
			Self::Month => Some(30),
			Self::Quarter => Some(90),
			Self::Year => Some(365),
			Self::AllTime => None,
		}
	}

	/// The first logical day inside the span, if it is bounded
	pub fn starts_on(self, today: NaiveDate) -> Option<NaiveDate> {
		self.days().map(|days| today - Duration::days(days - 1))
	}
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct ReadingStats {
	pub span: ReadingStatsSpan,
	/// The first logical day counted; `null` for an unbounded span with no sessions
	pub from: Option<NaiveDate>,
	/// The last logical day counted (today)
	pub to: NaiveDate,
	/// Reading sessions inside the span
	pub sessions: i64,
	/// Accumulated reading time inside the span
	pub minutes: i64,
	/// Pages turned inside the span (end page minus start page per session)
	pub pages: i64,
	/// Sessions inside the span that finished a book
	pub books_finished: i64,
	/// Consecutive logical days with at least one session, ending today or
	/// yesterday. Not bounded by the span.
	pub streak_days: i64,
	/// Days with activity inside the span, ascending; days without sessions
	/// are omitted
	pub days: Vec<ReadingStatsDay>,
	/// Activity per contributing device inside the span, most sessions first.
	/// A session that several devices contributed to counts toward each.
	pub devices: Vec<ReadingStatsDevice>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct ReadingStatsDay {
	pub date: NaiveDate,
	pub sessions: i64,
	pub minutes: i64,
	pub pages: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct ReadingStatsDevice {
	pub device_id: String,
	/// `null` when the id no longer resolves to a registered device
	pub name: Option<String>,
	pub kind: Option<DeviceKind>,
	pub sessions: i64,
	pub minutes: i64,
	pub pages: i64,
}

#[derive(Debug, FromQueryResult)]
struct TotalsRow {
	sessions: i64,
	seconds: i64,
	pages: i64,
	books_finished: i64,
}

#[derive(Debug, FromQueryResult)]
struct DayRow {
	date: NaiveDate,
	sessions: i64,
	seconds: i64,
	pages: i64,
}

#[derive(Debug, FromQueryResult)]
struct DeviceRow {
	device_id: String,
	name: Option<String>,
	kind: Option<DeviceKind>,
	sessions: i64,
	seconds: i64,
	pages: i64,
}

#[derive(Debug, FromQueryResult)]
struct DateRow {
	date: NaiveDate,
}

/// The shared `WHERE` clause: `$1` user id, `$2` first day (or NULL), `$3`
/// device id (or NULL, matching sessions any device contributed to).
const SESSION_FILTER: &str = r"
	rs.user_id = $1
	AND ($2 IS NULL OR rs.session_date >= $2)
	AND (
		$3 IS NULL
		OR EXISTS (SELECT 1 FROM json_each(rs.device_ids) je WHERE je.value = $3)
	)
";

const AGGREGATES: &str = r"
	COUNT(*) AS sessions,
	COALESCE(CAST(SUM(rs.elapsed_seconds) AS BIGINT), 0) AS seconds,
	COALESCE(CAST(SUM(
		CASE
			WHEN rs.start_page IS NOT NULL AND rs.end_page IS NOT NULL AND rs.end_page > rs.start_page
			THEN rs.end_page - rs.start_page
			ELSE 0
		END
	) AS BIGINT), 0) AS pages
";

/// The most recent distinct reading days considered for the streak
const STREAK_WINDOW: u64 = 400;

impl ReadingStats {
	/// Aggregates `user_id`'s sessions over `span` (logical days ending on
	/// `today`), optionally narrowed to sessions `device_id` contributed to.
	pub async fn fetch(
		conn: &DatabaseConnection,
		user_id: &str,
		today: NaiveDate,
		span: ReadingStatsSpan,
		device_id: Option<&str>,
	) -> Result<Self> {
		let from = span.starts_on(today);
		let params = || {
			[
				user_id.into(),
				from.into(),
				device_id.map(str::to_string).into(),
			]
		};

		let totals = conn
			.query_one(db_statement(
				conn,
				format!(
					"SELECT {AGGREGATES},
						COUNT(CASE WHEN rs.status = 'FINISHED' THEN 1 END) AS books_finished
					FROM reading_sessions rs
					WHERE {SESSION_FILTER}"
				),
				params(),
			))
			.await?
			.ok_or("Reading stats failed to be calculated")?;
		let totals = TotalsRow::from_query_result(&totals, "")?;

		let days = conn
			.query_all(db_statement(
				conn,
				format!(
					"SELECT rs.session_date AS date, {AGGREGATES}
					FROM reading_sessions rs
					WHERE {SESSION_FILTER}
					GROUP BY rs.session_date
					ORDER BY rs.session_date ASC"
				),
				params(),
			))
			.await?
			.iter()
			.map(|row| DayRow::from_query_result(row, ""))
			.collect::<std::result::Result<Vec<_>, _>>()?;

		let devices = conn
			.query_all(db_statement(
				conn,
				format!(
					"SELECT je.value AS device_id, d.name AS name, d.kind AS kind, {AGGREGATES}
					FROM reading_sessions rs
					JOIN json_each(rs.device_ids) je
					LEFT JOIN devices d ON d.id = je.value
					WHERE {SESSION_FILTER}
					GROUP BY je.value, d.name, d.kind
					ORDER BY sessions DESC, je.value ASC"
				),
				params(),
			))
			.await?
			.iter()
			.map(|row| DeviceRow::from_query_result(row, ""))
			.collect::<std::result::Result<Vec<_>, _>>()?;

		// The streak is not bounded by the span: `$2` is always NULL here.
		let recent_days = conn
			.query_all(db_statement(
				conn,
				format!(
					"SELECT DISTINCT rs.session_date AS date
					FROM reading_sessions rs
					WHERE {SESSION_FILTER}
					ORDER BY rs.session_date DESC
					LIMIT {STREAK_WINDOW}"
				),
				[
					user_id.into(),
					Option::<NaiveDate>::None.into(),
					device_id.map(str::to_string).into(),
				],
			))
			.await?
			.iter()
			.map(|row| DateRow::from_query_result(row, "").map(|row| row.date))
			.collect::<std::result::Result<Vec<_>, _>>()?;

		let earliest_activity = days.first().map(|day| day.date);

		Ok(ReadingStats {
			span,
			from: from.or(earliest_activity),
			to: today,
			sessions: totals.sessions,
			minutes: totals.seconds / 60,
			pages: totals.pages,
			books_finished: totals.books_finished,
			streak_days: streak(today, &recent_days),
			days: days
				.into_iter()
				.map(|row| ReadingStatsDay {
					date: row.date,
					sessions: row.sessions,
					minutes: row.seconds / 60,
					pages: row.pages,
				})
				.collect(),
			devices: devices
				.into_iter()
				.map(|row| ReadingStatsDevice {
					device_id: row.device_id,
					name: row.name,
					kind: row.kind,
					sessions: row.sessions,
					minutes: row.seconds / 60,
					pages: row.pages,
				})
				.collect(),
		})
	}
}

/// Consecutive reading days ending today or yesterday (a streak survives until
/// the current day is over). `recent_days` is distinct and descending.
pub fn streak(today: NaiveDate, recent_days: &[NaiveDate]) -> i64 {
	let Some(&latest) = recent_days.first() else {
		return 0;
	};
	if latest < today - Duration::days(1) {
		return 0;
	}

	let mut streak = 0;
	let mut expected = latest;
	for &day in recent_days {
		if day != expected {
			break;
		}
		streak += 1;
		expected -= Duration::days(1);
	}
	streak
}

#[cfg(test)]
mod tests {
	use super::*;

	fn day(ymd: &str) -> NaiveDate {
		NaiveDate::parse_from_str(ymd, "%Y-%m-%d").unwrap()
	}

	#[test]
	fn streak_counts_consecutive_days_ending_today_or_yesterday() {
		let today = day("2026-09-05");
		assert_eq!(streak(today, &[]), 0);
		assert_eq!(streak(today, &[day("2026-09-05")]), 1);
		assert_eq!(
			streak(
				today,
				&[day("2026-09-05"), day("2026-09-04"), day("2026-09-03")]
			),
			3
		);
		// yesterday keeps the streak alive
		assert_eq!(streak(today, &[day("2026-09-04"), day("2026-09-03")]), 2);
		// a gap ends it
		assert_eq!(
			streak(
				today,
				&[day("2026-09-05"), day("2026-09-04"), day("2026-09-02")]
			),
			2
		);
		// two days of silence resets it
		assert_eq!(streak(today, &[day("2026-09-03"), day("2026-09-02")]), 0);
	}

	#[test]
	fn spans_start_on_the_right_day() {
		let today = day("2026-09-05");
		assert_eq!(ReadingStatsSpan::Week.starts_on(today), Some(day("2026-08-30")));
		assert_eq!(ReadingStatsSpan::Month.starts_on(today), Some(day("2026-08-07")));
		assert_eq!(ReadingStatsSpan::AllTime.starts_on(today), None);
	}

	mod seeded {
		use chrono::{Duration, NaiveDate, Utc};
		use models::{
			entity::{device, reading_session, reading_session::DeviceIds},
			shared::enums::{DeviceKind, ReadingStatus},
		};
		use sea_orm::{prelude::*, ActiveValue::Set, DatabaseConnection};
		use tests::{db::test_database, fake_data};

		use crate::object::reading_stats::{ReadingStats, ReadingStatsSpan};

		struct Session<'a> {
			user_id: &'a str,
			media_id: &'a str,
			days_ago: i64,
			seconds: i64,
			pages: (Option<i32>, Option<i32>),
			status: ReadingStatus,
			devices: &'a [&'a str],
		}

		async fn seed(conn: &DatabaseConnection, today: NaiveDate, session: Session<'_>) {
			let device_ids = (!session.devices.is_empty()).then(|| {
				DeviceIds(session.devices.iter().map(|id| String::from(*id)).collect())
			});
			reading_session::ActiveModel {
				session_date: Set(today - Duration::days(session.days_ago)),
				media_id: Set(session.media_id.to_string()),
				user_id: Set(session.user_id.to_string()),
				elapsed_seconds: Set(Some(session.seconds)),
				start_page: Set(session.pages.0),
				end_page: Set(session.pages.1),
				status: Set(session.status),
				readthrough_number: Set(1),
				device_ids: Set(device_ids),
				..Default::default()
			}
			.insert(conn)
			.await
			.expect("insert session");
		}

		#[tokio::test]
		async fn aggregates_sessions_days_devices_and_streak() {
			let conn = test_database().await;
			let user = fake_data::User::new("al").insert(&conn).await;
			let other = fake_data::User::new("bea").insert(&conn).await;
			let library = fake_data::Library::default().insert(&conn).await;
			let series = fake_data::Series {
				library_id: Some(library.id.clone()),
				..Default::default()
			}
			.insert(&conn)
			.await;
			let book = fake_data::Media {
				series_id: series.id.clone(),
				..Default::default()
			}
			.insert(&conn)
			.await;
			for (id, name) in [("kobo-1", "Clara"), ("koreader-1", "Palma")] {
				device::ActiveModel {
					id: Set(id.to_string()),
					user_id: Set(user.id.clone()),
					name: Set(name.to_string()),
					kind: Set(if id.starts_with("kobo") {
						DeviceKind::Kobo
					} else {
						DeviceKind::Koreader
					}),
					..Default::default()
				}
				.insert(&conn)
				.await
				.expect("insert device");
			}

			let today = Utc::now().date_naive();
			let sessions = [
				// today: 20 min, 10 pages, on the Kobo
				(0, 1200, (Some(10), Some(20)), ReadingStatus::Reading, &["kobo-1"][..]),
				// yesterday: two sessions, one finished the book, both devices on one
				(1, 600, (Some(20), Some(25)), ReadingStatus::Reading, &["kobo-1", "koreader-1"][..]),
				(1, 300, (None, Some(40)), ReadingStatus::Finished, &["koreader-1"][..]),
				// three days ago: breaks the streak, on an unregistered device
				(3, 60, (Some(5), Some(2)), ReadingStatus::Reading, &["ghost"][..]),
				// long ago: outside a week, inside a month
				(20, 3600, (Some(0), Some(100)), ReadingStatus::Reading, &[][..]),
			];
			for (days_ago, seconds, pages, status, devices) in sessions {
				seed(
					&conn,
					today,
					Session {
						user_id: &user.id,
						media_id: &book.id,
						days_ago,
						seconds,
						pages,
						status,
						devices,
					},
				)
				.await;
			}
			// another user's session must never leak in
			seed(
				&conn,
				today,
				Session {
					user_id: &other.id,
					media_id: &book.id,
					days_ago: 0,
					seconds: 9999,
					pages: (Some(0), Some(999)),
					status: ReadingStatus::Reading,
					devices: &["kobo-1"],
				},
			)
			.await;

			let week = ReadingStats::fetch(&conn, &user.id, today, ReadingStatsSpan::Week, None)
				.await
				.expect("week stats");
			assert_eq!(week.from, Some(today - Duration::days(6)));
			assert_eq!(week.to, today);
			assert_eq!(week.sessions, 4);
			assert_eq!(week.minutes, (1200 + 600 + 300 + 60) / 60);
			assert_eq!(week.pages, 10 + 5);
			assert_eq!(week.books_finished, 1);
			assert_eq!(week.streak_days, 2);

			let day_dates: Vec<_> = week.days.iter().map(|day| day.date).collect();
			assert_eq!(
				day_dates,
				vec![today - Duration::days(3), today - Duration::days(1), today]
			);
			let yesterday = &week.days[1];
			assert_eq!(yesterday.sessions, 2);
			assert_eq!(yesterday.minutes, 15);
			assert_eq!(yesterday.pages, 5);

			let by_device: Vec<_> = week
				.devices
				.iter()
				.map(|d| (d.device_id.as_str(), d.name.as_deref(), d.sessions, d.minutes))
				.collect();
			assert_eq!(
				by_device,
				vec![
					("kobo-1", Some("Clara"), 2, 30),
					("koreader-1", Some("Palma"), 2, 15),
					("ghost", None, 1, 1),
				]
			);
			assert_eq!(week.devices[0].kind, Some(DeviceKind::Kobo));
			assert_eq!(week.devices[2].kind, None);

			let month = ReadingStats::fetch(&conn, &user.id, today, ReadingStatsSpan::Month, None)
				.await
				.expect("month stats");
			assert_eq!(month.sessions, 5);
			assert_eq!(month.minutes, week.minutes + 60);
			assert_eq!(month.pages, week.pages + 100);

			let all_time =
				ReadingStats::fetch(&conn, &user.id, today, ReadingStatsSpan::AllTime, None)
					.await
					.expect("all time stats");
			assert_eq!(all_time.from, Some(today - Duration::days(20)));
			assert_eq!(all_time.sessions, 5);

			let kobo_only =
				ReadingStats::fetch(&conn, &user.id, today, ReadingStatsSpan::Week, Some("kobo-1"))
					.await
					.expect("device stats");
			assert_eq!(kobo_only.sessions, 2);
			assert_eq!(kobo_only.minutes, 30);
			assert_eq!(kobo_only.streak_days, 2);
			assert_eq!(kobo_only.devices.len(), 2);

			let nothing =
				ReadingStats::fetch(&conn, "nobody", today, ReadingStatsSpan::AllTime, None)
					.await
					.expect("empty stats");
			assert_eq!(nothing.sessions, 0);
			assert_eq!(nothing.from, None);
			assert_eq!(nothing.streak_days, 0);
			assert!(nothing.days.is_empty() && nothing.devices.is_empty());
		}
	}
}
