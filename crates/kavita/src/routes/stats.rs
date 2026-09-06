//! `StatisticsController` reads: the per-user totals and reading history
//! Inkita renders.
//!
//! Both routes are the pre-0.9 spellings Inkita is pinned to
//! (`GET /api/Stats/user/{userId}/read`, `KavitaApi.kt:162`, and
//! `GET /api/Stats/user/reading-history`, `KavitaApi.kt:287`); `kavita-ref`
//! 0.9.1.4 renamed them to `/api/Stats/user-read?userId` and
//! `/api/Stats/reading-history` and answers `404` on the old paths. The DTOs
//! follow the reference's `UserReadStatistics` and add the two fields Inkita
//! reads (`chaptersRead`, `lastActive`).
//!
//! Numbers come from Stump's reading sessions — the rows every protocol
//! writes through — using the same `pagesRead` translation the rest of the
//! profile uses, so a chapter counted as read here is a chapter Kavita's
//! reader reports as read. Stump records no word counts, so `totalWordsRead`
//! is `0` exactly as `wordCount` is elsewhere in the profile.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{extract::Path, extract::Query, routing::get, Extension, Json, Router};
use chrono::Utc;
use models::entity::{media, reading_session, user, user::AuthUser};
use sea_orm::{prelude::*, QueryOrder, QuerySelect};
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::{
	dto::{KavitaDateTime, KavitaFloat, ReadHistoryEventDto, UserReadStatisticsDto},
	errors::{APIError, APIResult},
	ids::{IdKind, KavitaIds, LOOKUP_CHUNK},
	mapper::DEFAULT_CHAPTER_NUMBER,
	progress::{latest_sessions, pages_read},
};

use super::{query::group_by_media, route_ci, KavitaBackend};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserIdQuery {
	#[serde(default)]
	user_id: Option<i32>,
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(
		router,
		"/api/Stats/user/reading-history",
		get(reading_history),
	);
	route_ci(router, "/api/Stats/user/{userId}/read", get(user_read))
}

/// The user a `userId` names. Kavita lets an admin ask about anybody and
/// everyone else only about themselves; `0`/absent means the caller.
async fn stats_subject(
	ctx: &dyn KavitaBackend,
	auth: &AuthContext,
	user_id: i32,
) -> APIResult<AuthUser> {
	let caller = auth.user();
	if user_id == 0 {
		return Ok(caller);
	}
	let requested = KavitaIds::lookup(ctx.conn(), IdKind::User, user_id)
		.await?
		.ok_or_else(|| APIError::BadRequest("User does not exist".to_owned()))?;
	if requested == caller.id {
		return Ok(caller);
	}
	if !caller.is_server_owner {
		return Err(APIError::Forbidden(
			"You are not permitted to view another user's statistics".to_owned(),
		));
	}
	let row = user::Entity::find_by_id(requested)
		.one(ctx.conn())
		.await?
		.ok_or_else(|| APIError::BadRequest("User does not exist".to_owned()))?;
	Ok(AuthUser {
		id: row.id,
		avatar_path: None,
		avatar: Default::default(),
		username: row.username,
		is_server_owner: row.is_server_owner,
		is_locked: row.is_locked,
		permissions: Vec::new(),
		age_restriction: None,
		preferences: None,
		// The subject is another user, not the authenticated request: it
		// carries no device and therefore no device scope.
		device_library_scope: None,
	})
}

/// The media a user has any reading session for.
async fn read_media_ids(
	ctx: &dyn KavitaBackend,
	user_id: &str,
) -> APIResult<Vec<String>> {
	let mut ids = reading_session::Entity::find()
		.select_only()
		.column(reading_session::Column::MediaId)
		.filter(reading_session::Column::UserId.eq(user_id))
		.into_tuple::<String>()
		.all(ctx.conn())
		.await?;
	ids.sort();
	ids.dedup();
	Ok(ids)
}

/// `GET /api/Stats/user/{userId}/read`.
async fn user_read(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(user_id): Path<i32>,
) -> APIResult<Json<UserReadStatisticsDto>> {
	let ctx = ctx.as_ref();
	let subject = stats_subject(ctx, &auth, user_id).await?;
	Ok(Json(read_statistics(ctx, &subject).await?))
}

/// The per-user totals behind [`user_read`].
pub(crate) async fn read_statistics(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
) -> APIResult<UserReadStatisticsDto> {
	let media_ids = read_media_ids(ctx, &user.id).await?;
	let sessions = latest_sessions(ctx.conn(), user, &media_ids).await?;
	let mut pages: HashMap<String, i32> = HashMap::with_capacity(media_ids.len());
	for chunk in media_ids.chunks(LOOKUP_CHUNK) {
		pages.extend(
			media::Entity::find()
				.select_only()
				.column(media::Column::Id)
				.column(media::Column::Pages)
				.filter(media::Column::Id.is_in(chunk.to_vec()))
				.into_tuple::<(String, i32)>()
				.all(ctx.conn())
				.await?,
		);
	}
	let mut total_pages_read: i64 = 0;
	let mut chapters_read: i64 = 0;
	for (media_id, session) in &sessions {
		let total = pages.get(media_id).copied().unwrap_or(0);
		let read = pages_read(Some(session), total);
		total_pages_read += i64::from(read);
		if total > 0 && read >= total {
			chapters_read += 1;
		}
	}

	// Elapsed time and activity span cover every session, not only the
	// latest per book: a re-read still counts as time spent reading.
	let all_sessions = reading_session::Entity::find()
		.filter(reading_session::Column::UserId.eq(user.id.clone()))
		.all(ctx.conn())
		.await?;
	let time_spent_reading: i64 = all_sessions
		.iter()
		.filter_map(|session| session.elapsed_seconds)
		.sum();
	let last_active = all_sessions
		.iter()
		.map(|session| session.updated_at.unwrap_or(session.created_at))
		.max()
		.map(|at| at.with_timezone(&Utc));
	let first_active = all_sessions
		.iter()
		.map(|session| session.created_at)
		.min()
		.map(|at| at.with_timezone(&Utc));
	let weeks = first_active
		.map(|first| {
			let days = (Utc::now() - first).num_days().max(1) as f64;
			(days / 7.0).max(1.0)
		})
		.unwrap_or(1.0);
	let avg_hours_per_week_spent_reading = if time_spent_reading == 0 {
		0.0
	} else {
		(time_spent_reading as f64 / 3600.0) / weeks
	};

	Ok(UserReadStatisticsDto {
		total_pages_read,
		total_words_read: 0,
		time_spent_reading,
		chapters_read,
		last_active: KavitaDateTime::new(last_active),
		last_active_utc: KavitaDateTime::new(last_active),
		avg_hours_per_week_spent_reading,
	})
}

/// `GET /api/Stats/user/reading-history?userId`: one event per reading
/// session, newest first.
///
/// Every Stump media item is a single-file volume holding one chapter, so
/// `chapterNumber` is `Parser.DefaultChapterNumber` (`-100000`), the number
/// the rest of the profile reports for that chapter.
async fn reading_history(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<UserIdQuery>,
) -> APIResult<Json<Vec<ReadHistoryEventDto>>> {
	let ctx = ctx.as_ref();
	let subject = stats_subject(ctx, &auth, query.user_id.unwrap_or_default()).await?;
	let kavita_user_id =
		KavitaIds::resolve(ctx.conn(), IdKind::User, &subject.id).await?;
	let sessions = reading_session::Entity::find()
		.filter(reading_session::Column::UserId.eq(subject.id.clone()))
		.order_by_desc(reading_session::Column::SessionDate)
		.order_by_desc(reading_session::Column::Id)
		.all(ctx.conn())
		.await?;
	if sessions.is_empty() {
		return Ok(Json(Vec::new()));
	}
	let grouped =
		group_by_media(ctx, &subject, sessions, |session| session.media_id.as_str())
			.await?;
	Ok(Json(
		grouped
			.rows
			.iter()
			.map(|(session, series_index, media_index)| {
				let input = &grouped.inputs[*series_index];
				let at = session
					.updated_at
					.unwrap_or(session.created_at)
					.with_timezone(&Utc);
				ReadHistoryEventDto {
					user_id: kavita_user_id,
					user_name: subject.username.clone(),
					library_id: input.library_id,
					series_id: input.id,
					series_name: input.name(),
					read_date: KavitaDateTime::new(at),
					read_date_utc: KavitaDateTime::new(at),
					chapter_id: input.media[*media_index].id,
					chapter_number: KavitaFloat(DEFAULT_CHAPTER_NUMBER),
				}
			})
			.collect(),
	))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_support::{
		auth_user, db, library_of_type, request, series_with_files, TestBackend,
	};
	use ::tests::fake_data;
	use axum::http::StatusCode;
	use models::shared::enums::{LibraryType as StumpLibraryType, ReadingStatus};
	use sea_orm::{DbBackend, Statement};

	#[tokio::test]
	async fn user_read_totals_come_from_reading_sessions() {
		let conn = db().await;
		let user_row = fake_data::User::new("reader").insert(&conn).await;
		let user = auth_user(&user_row);
		let library = library_of_type(&conn, StumpLibraryType::Comic).await;
		let (_, files) = series_with_files(
			&conn,
			&library.id,
			"Zeta",
			&[("v01", "cbz", 20), ("v02", "cbz", 30)],
		)
		.await;
		// One finished book (20 pages) and one half-read (15 of 30).
		fake_data::ReadingSession::completed(&files[0].id, &user_row.id)
			.insert(&conn)
			.await;
		let open = fake_data::ReadingSession {
			media_id: files[1].id.clone(),
			user_id: user_row.id.clone(),
			end_percentage: 0.5,
			status: ReadingStatus::Reading,
			..Default::default()
		}
		.insert(&conn)
		.await;
		conn.execute(Statement::from_sql_and_values(
			DbBackend::Sqlite,
			"UPDATE reading_sessions SET elapsed_seconds = 3600 WHERE id = ?",
			[open.id.into()],
		))
		.await
		.unwrap();

		let backend = std::sync::Arc::new(TestBackend::new(conn));
		let kavita_user = KavitaIds::resolve(backend.conn(), IdKind::User, &user_row.id)
			.await
			.unwrap();
		let (status, body) = request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Stats/user/{kavita_user}/read"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(body["totalPagesRead"], 35);
		assert_eq!(body["chaptersRead"], 1);
		assert_eq!(body["timeSpentReading"], 3600);
		// Stump records no word counts, like the rest of the profile.
		assert_eq!(body["totalWordsRead"], 0);
		assert!(body["lastActive"].as_str().unwrap() != KavitaDateTime::UNSET);
		assert_eq!(body["lastActive"], body["lastActiveUtc"]);

		// A non-admin may not read another user's statistics.
		let other_row = fake_data::User::new("nosy").insert(backend.conn()).await;
		let other = AuthUser {
			is_server_owner: false,
			..auth_user(&other_row)
		};
		let (status, _) = request(
			backend.clone(),
			&other,
			"GET",
			&format!("/api/Stats/user/{kavita_user}/read"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::FORBIDDEN);

		// A user with no sessions reports zeroes and the unset instant.
		let (status, body) =
			request(backend, &other, "GET", "/api/stats/user/0/read", None).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(body["totalPagesRead"], 0);
		assert_eq!(body["lastActive"], KavitaDateTime::UNSET);
		assert_eq!(body["avgHoursPerWeekSpentReading"], 0.0);
	}

	#[tokio::test]
	async fn reading_history_reports_one_event_per_session_newest_first() {
		let conn = db().await;
		let user_row = fake_data::User::new("historian").insert(&conn).await;
		let user = auth_user(&user_row);
		let library = library_of_type(&conn, StumpLibraryType::Comic).await;
		let (_, files) = series_with_files(
			&conn,
			&library.id,
			"Zeta",
			&[("v01", "cbz", 20), ("v02", "cbz", 30)],
		)
		.await;
		for file in &files {
			fake_data::ReadingSession::completed(&file.id, &user_row.id)
				.insert(&conn)
				.await;
		}
		let backend = std::sync::Arc::new(TestBackend::new(conn));

		let (status, body) = request(
			backend.clone(),
			&user,
			"GET",
			"/api/Stats/user/reading-history",
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		let events = body.as_array().unwrap();
		assert_eq!(events.len(), 2);
		for event in events {
			assert_eq!(event["seriesName"], "Zeta");
			assert_eq!(event["userName"], "historian");
			// One chapter per single-file volume: `Parser.DefaultChapterNumber`.
			assert_eq!(event["chapterNumber"], -100000);
			assert!(event["chapterId"].as_i64().unwrap() > 0);
			assert!(event["libraryId"].as_i64().unwrap() > 0);
			assert_eq!(event["readDate"], event["readDateUtc"]);
		}
		// Newest first: the second session is listed before the first.
		assert_ne!(events[0]["chapterId"], events[1]["chapterId"]);

		let no_sessions = fake_data::User::new("fresh").insert(backend.conn()).await;
		let (status, body) = request(
			backend,
			&auth_user(&no_sessions),
			"GET",
			"/api/Stats/user/reading-history",
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert!(body.as_array().unwrap().is_empty());
	}
}
