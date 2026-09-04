use std::{collections::HashMap, sync::Arc};

use crate::{sse::KomgaEvent, KomgaBookReadProgressUpdateRequest};
use axum::{extract::Path, http::StatusCode, routing::patch, Extension, Json, Router};
use models::{
	domain::reading_progress::compute_page_based_percentage,
	entity::{media, reading_session, series, user::AuthUser},
	services::reading_progress::{upsert_reading_session, NormalizedProgression},
};
use stump_auth::AuthContext;

use super::{KomgaBackend, KomgaEvents};
use crate::errors::{APIError, APIResult};
use sea_orm::{prelude::*, DatabaseConnection, QueryOrder, TransactionTrait};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaSeriesReadProgressDto {
	pub books_count: i32,
	pub books_read_count: i32,
	pub books_unread_count: i32,
	pub books_in_progress_count: i32,
	pub last_read_continuous_number_sort: f64,
	pub max_number_sort: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaSeriesReadProgressUpdateDto {
	pub last_book_number_sort_read: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaReadProgressDto {
	pub books_count: i32,
	pub books_read_count: i32,
	pub books_unread_count: i32,
	pub books_in_progress_count: i32,
	pub last_read_continuous_index: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaReadProgressUpdateDto {
	pub last_book_read: i32,
}

#[derive(Debug, Clone)]
pub(crate) struct ProgressBook {
	pub id: String,
	pub number: Option<f64>,
	pub source_number: bool,
	pub pages: i32,
	pub series_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct ProgressCounts {
	pub books_count: i32,
	pub books_read_count: i32,
	pub books_unread_count: i32,
	pub books_in_progress_count: i32,
	pub continuous_prefix_count: i32,
	pub last_read_continuous_number_sort: f64,
	pub max_number_sort: f32,
}

fn progress_book(book: &media::ModelWithMetadata) -> ProgressBook {
	let number = book
		.metadata
		.as_ref()
		.and_then(|metadata| metadata.number.map(|number| number.to_string()))
		.and_then(|number| number.parse::<f64>().ok())
		.filter(|number| number.is_finite());
	ProgressBook {
		id: book.media.id.clone(),
		source_number: number.is_some(),
		number,
		pages: book.media.pages,
		series_id: book.media.series_id.clone(),
	}
}

pub(crate) fn progress_books(
	books: impl IntoIterator<Item = media::ModelWithMetadata>,
) -> Vec<ProgressBook> {
	books.into_iter().map(|book| progress_book(&book)).collect()
}

async fn latest_sessions(
	conn: &DatabaseConnection,
	user: &AuthUser,
	media_ids: &[String],
) -> APIResult<HashMap<String, reading_session::Model>> {
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
	let mut latest = HashMap::with_capacity(media_ids.len());
	for row in rows {
		let id = row.media_id.clone();
		let key = (
			row.updated_at.unwrap_or(row.created_at),
			row.created_at,
			row.id,
		);
		let replace = latest
			.get(&id)
			.is_none_or(|current: &reading_session::Model| {
				let current_key = (
					current.updated_at.unwrap_or(current.created_at),
					current.created_at,
					current.id,
				);
				key > current_key
			});
		if replace {
			latest.insert(id, row);
		}
	}
	Ok(latest)
}

fn continuous_prefix(
	states: &[(Option<f64>, bool)],
	order_by_number: bool,
) -> (usize, f64) {
	let mut states = states.to_vec();
	if order_by_number {
		states.sort_by(|(left, _), (right, _)| {
			left.unwrap_or(0.0)
				.partial_cmp(&right.unwrap_or(0.0))
				.unwrap_or(std::cmp::Ordering::Equal)
		});
	}
	let prefix = states.iter().take_while(|(_, is_read)| *is_read).count();
	let last = if prefix == 0 {
		0.0
	} else {
		states[prefix - 1].0.unwrap_or(0.0)
	};
	(prefix, last)
}

fn counts_for_books(
	books: &[ProgressBook],
	sessions: &HashMap<String, reading_session::Model>,
	order_by_number: bool,
) -> ProgressCounts {
	let books_count = i32::try_from(books.len()).unwrap_or(i32::MAX);
	let mut read = 0;
	let mut in_progress = 0;
	let states = books
		.iter()
		.map(|book| {
			let session = sessions.get(&book.id);
			let is_read = session.is_some_and(reading_session::Model::is_complete);
			let is_in_progress = session.is_some_and(|session| !session.is_complete());
			read += i32::from(is_read);
			in_progress += i32::from(is_in_progress);
			(book.number, is_read)
		})
		.collect::<Vec<_>>();
	let (continuous_prefix_count, last_read_continuous_number_sort) =
		continuous_prefix(&states, order_by_number);
	let max_number_sort =
		if books.is_empty() || books.iter().any(|book| !book.source_number) {
			books_count as f32
		} else {
			books
				.iter()
				.filter_map(|book| book.number)
				.fold(0.0, f64::max) as f32
		};
	ProgressCounts {
		books_count,
		books_read_count: read,
		books_unread_count: books_count - read - in_progress,
		books_in_progress_count: in_progress,
		continuous_prefix_count: i32::try_from(continuous_prefix_count)
			.unwrap_or(i32::MAX),
		last_read_continuous_number_sort,
		max_number_sort,
	}
}

pub(crate) async fn counts_for_progress_books(
	conn: &DatabaseConnection,
	user: &AuthUser,
	books: &[ProgressBook],
	order_by_number: bool,
) -> APIResult<ProgressCounts> {
	let ids = books.iter().map(|book| book.id.clone()).collect::<Vec<_>>();
	let sessions = latest_sessions(conn, user, &ids).await?;
	Ok(counts_for_books(books, &sessions, order_by_number))
}

pub(crate) fn finished_progression(pages: i32) -> NormalizedProgression {
	let page = (pages > 0).then_some(pages - 1);
	NormalizedProgression {
		page,
		percentage: page.map(|page| compute_page_based_percentage(page, pages)),
		did_complete: true,
		..Default::default()
	}
}

/// The Komelia book progress endpoints. Authentication is applied by the parent router.
pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::<S>::new().route(
		"/api/v1/books/{id}/read-progress",
		patch(update_read_progress).delete(delete_read_progress),
	)
}

/// Convert a Komga patch into Stump's normalized progression representation.
///
/// Komga pages are zero-based and use a half-open range: for a media item with
/// `pages == n`, valid page values are `0..n`. Media with zero (or negative)
/// pages has no addressable pages, so callers must omit `page` and may still
/// update only the completion state.
fn normalize_patch(
	patch: KomgaBookReadProgressUpdateRequest,
	media_pages: i32,
) -> Result<NormalizedProgression, String> {
	if patch.page.is_none() && patch.completed.is_none() {
		return Err("At least one progress field must be provided".to_string());
	}

	if let Some(page) = patch.page {
		if media_pages <= 0 {
			return Err("This media has no addressable pages".to_string());
		}
		if page < 0 || page >= media_pages {
			return Err(format!(
				"Page {page} is out of bounds (valid range: 0..{media_pages})"
			));
		}
	}

	let percentage = patch
		.page
		.map(|page| compute_page_based_percentage(page, media_pages));

	Ok(NormalizedProgression {
		page: patch.page,
		locator: None,
		percentage,
		elapsed_seconds_delta: None,
		did_complete: patch.completed.unwrap_or(false),
		device_id: None,
		reset_elapsed_seconds: false,
	})
}

async fn update_read_progress(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<KomgaEvents>,
	Json(patch): Json<KomgaBookReadProgressUpdateRequest>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let conn = ctx.conn();

	// Resolve visibility before validating or mutating progress. This keeps
	// missing, hidden, and inaccessible media indistinguishable.
	let book = media::Entity::find_for_user(&user)
		.filter(media::Column::Id.eq(id.clone()))
		.one(conn)
		.await?
		.ok_or_else(|| APIError::NotFound("Book not found".to_string()))?;
	let progression = normalize_patch(patch, book.pages).map_err(APIError::BadRequest)?;
	let parent_series = if let Some(series_id) = book.series_id.as_deref() {
		series::Entity::find_for_user(&user)
			.filter(series::Column::Id.eq(series_id))
			.one(conn)
			.await?
	} else {
		None
	};
	let event_book_id = book.id.clone();
	let event_series_id = book.series_id.clone().unwrap_or_default();
	let event_library_id = parent_series
		.and_then(|series| series.library_id)
		.unwrap_or_default();
	let txn = conn.begin().await?;
	upsert_reading_session(&txn, &user, &id, progression).await?;
	txn.commit().await?;
	events.send(KomgaEvent::ReadProgressChanged {
		book_id: event_book_id.clone().into(),
		user_id: user.id.clone().into(),
	});
	events.send(KomgaEvent::ReadProgressSeriesChanged {
		series_id: event_series_id.clone().into(),
		user_id: user.id.clone().into(),
	});
	events.send(KomgaEvent::BookChanged {
		book_id: event_book_id.into(),
		series_id: event_series_id.clone().into(),
		library_id: event_library_id.clone().into(),
	});
	events.send(KomgaEvent::SeriesChanged {
		series_id: event_series_id.into(),
		library_id: event_library_id.into(),
	});
	Ok(StatusCode::NO_CONTENT)
}

async fn delete_read_progress(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<KomgaEvents>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let conn = ctx.conn();

	// Check visibility first, even when no session exists, so hidden media is
	// never distinguishable from a missing media item and cannot be mutated.
	let book = media::Entity::find_for_user(&user)
		.filter(media::Column::Id.eq(id.clone()))
		.one(conn)
		.await?
		.ok_or_else(|| APIError::NotFound("Book not found".to_string()))?;
	let parent_series = if let Some(series_id) = book.series_id.as_deref() {
		series::Entity::find_for_user(&user)
			.filter(series::Column::Id.eq(series_id))
			.one(conn)
			.await?
	} else {
		None
	};
	let event_book_id = book.id;
	let event_series_id = book.series_id.unwrap_or_default();
	let event_library_id = parent_series
		.and_then(|series| series.library_id)
		.unwrap_or_default();

	reading_session::Entity::delete_many()
		.filter(reading_session::Column::UserId.eq(user.id.clone()))
		.filter(reading_session::Column::MediaId.eq(id))
		.exec(conn)
		.await?;
	events.send(KomgaEvent::ReadProgressDeleted {
		book_id: event_book_id.clone().into(),
		user_id: user.id.clone().into(),
	});
	events.send(KomgaEvent::ReadProgressSeriesDeleted {
		series_id: event_series_id.clone().into(),
		user_id: user.id.clone().into(),
	});
	events.send(KomgaEvent::BookChanged {
		book_id: event_book_id.into(),
		series_id: event_series_id.clone().into(),
		library_id: event_library_id.clone().into(),
	});
	events.send(KomgaEvent::SeriesChanged {
		series_id: event_series_id.into(),
		library_id: event_library_id.into(),
	});
	Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn empty_patch_is_rejected() {
		let patch = KomgaBookReadProgressUpdateRequest {
			page: None,
			completed: None,
		};
		assert!(normalize_patch(patch, 10).is_err());
	}

	#[test]
	fn page_is_zero_based_and_percentage_is_a_fraction() {
		let patch = KomgaBookReadProgressUpdateRequest {
			page: Some(2),
			completed: Some(false),
		};
		let progression = normalize_patch(patch, 10).unwrap();
		assert_eq!(progression.page, Some(2));
		assert_eq!(
			progression.percentage.unwrap(),
			sea_orm::prelude::Decimal::new(2, 1)
		);
		assert!(!progression.did_complete);
	}

	#[test]
	fn zero_and_last_pages_are_valid() {
		for page in [0, 9] {
			let patch = KomgaBookReadProgressUpdateRequest {
				page: Some(page),
				completed: None,
			};
			assert!(normalize_patch(patch, 10).is_ok());
		}
	}

	#[test]
	fn negative_and_upper_bound_pages_are_rejected() {
		for page in [-1, 10] {
			let patch = KomgaBookReadProgressUpdateRequest {
				page: Some(page),
				completed: None,
			};
			assert!(normalize_patch(patch, 10).is_err());
		}
	}

	#[test]
	fn zero_page_media_accepts_completion_without_page_but_not_page_zero() {
		let completion = KomgaBookReadProgressUpdateRequest {
			page: None,
			completed: Some(true),
		};
		assert!(normalize_patch(completion, 0).unwrap().did_complete);

		let page = KomgaBookReadProgressUpdateRequest {
			page: Some(0),
			completed: None,
		};
		assert!(normalize_patch(page, 0).is_err());
	}

	#[test]
	fn omitted_page_is_preserved_and_completion_controls_status() {
		let patch = KomgaBookReadProgressUpdateRequest {
			page: None,
			completed: Some(true),
		};
		let progression = normalize_patch(patch, 10).unwrap();
		assert_eq!(progression.page, None);
		assert_eq!(progression.percentage, None);
		assert!(progression.did_complete);
	}
	#[test]
	fn continuous_prefix_stops_at_a_gap() {
		assert_eq!(
			continuous_prefix(
				&[(Some(1.0), true), (Some(2.0), false), (Some(3.0), true)],
				true
			),
			(1, 1.0)
		);
	}

	#[test]
	fn continuous_prefix_handles_all_and_none_read() {
		assert_eq!(
			continuous_prefix(&[(Some(1.0), true), (Some(2.0), true)], true),
			(2, 2.0)
		);
		assert_eq!(
			continuous_prefix(&[(Some(1.0), false), (Some(2.0), false)], true),
			(0, 0.0)
		);
	}

	#[test]
	fn continuous_prefix_handles_missing_numbers() {
		assert_eq!(
			continuous_prefix(
				&[(None, true), (Some(2.0), true), (Some(3.0), false)],
				true
			),
			(2, 2.0)
		);
	}

	#[test]
	fn tachiyomi_dto_uses_exact_wire_names() {
		let series = serde_json::to_value(KomgaSeriesReadProgressDto {
			books_count: 3,
			books_read_count: 1,
			books_unread_count: 1,
			books_in_progress_count: 1,
			last_read_continuous_number_sort: 1.0,
			max_number_sort: 3.0,
		})
		.unwrap();
		assert_eq!(series["booksCount"], 3);
		assert_eq!(series["lastReadContinuousNumberSort"], 1.0);
		assert_eq!(series["maxNumberSort"], 3.0);
		let readlist_response = serde_json::to_value(KomgaReadProgressDto {
			books_count: 2,
			books_read_count: 1,
			books_unread_count: 1,
			books_in_progress_count: 0,
			last_read_continuous_index: 1,
		})
		.unwrap();
		assert_eq!(readlist_response["lastReadContinuousIndex"], 1);
		let readlist =
			serde_json::to_value(KomgaReadProgressUpdateDto { last_book_read: 2 })
				.unwrap();
		assert_eq!(readlist, serde_json::json!({"lastBookRead": 2}));
	}
}
