use std::{
	collections::{HashMap, HashSet},
	sync::Arc,
};

use crate::{
	routes::progress::{
		clear_books_progress, counts_for_progress_books, mark_book_read, progress_books,
		KomgaSeriesReadProgressDto, KomgaSeriesReadProgressUpdateDto,
	},
	sse::KomgaEvent,
	KomgaSeriesMetadataUpdateRequest, KomgaSeriesStatus, KomgaWebLink, PatchValue,
};
use axum::{
	extract::Path,
	http::StatusCode,
	routing::{get, patch, post},
	Extension, Json, Router,
};
use models::txn::begin_write;
use models::{
	entity::{media, series, series_metadata, series_tag, tag, user::AuthUser},
	shared::enums::UserPermission,
};
use sea_orm::{
	prelude::*, ActiveValue, ActiveValue::Set, DatabaseTransaction, IntoActiveModel,
	QueryOrder,
};
use stump_auth::AuthContext;

use super::{KomgaBackend, KomgaEvents};
use crate::errors::{APIError, APIResult};

/// Komga series metadata and series-wide reading-progress compatibility routes.
/// Authentication is applied by the parent Komga router.
pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::<S>::new()
		.route("/api/v1/series/{id}/metadata", patch(patch_series_metadata))
		.route("/api/v1/series/{id}/analyze", post(analyze_series))
		.route(
			"/api/v1/series/{id}/read-progress",
			post(mark_series_read).delete(delete_series_read_progress),
		)
		.route(
			"/api/v2/series/{id}/read-progress/tachiyomi",
			get(get_tachiyomi_series_progress).put(update_tachiyomi_series_progress),
		)
}

/// Komf requests series-level analysis after metadata writes. Komga fans the
/// request out to one analysis task per book of the series; Stump's analysis
/// job accepts a series scope directly (upstream: `202 Accepted`).
async fn analyze_series(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<StatusCode> {
	enforce_manage_library(&auth)?;
	let user = auth.user();
	let series = find_visible_series(ctx.conn(), &user, &id).await?;
	ctx.enqueue_series_analysis(series.id).await?;
	Ok(StatusCode::ACCEPTED)
}
fn enforce_manage_library(auth: &AuthContext) -> APIResult<()> {
	auth.enforce_permissions(&[UserPermission::ManageLibrary])
		.map_err(|_| APIError::forbidden_discreet())
}
async fn find_visible_series(
	conn: &DatabaseConnection,
	user: &AuthUser,
	id: &str,
) -> APIResult<series::Model> {
	series::Entity::find_for_user(user)
		.filter(series::Column::Id.eq(id.to_owned()))
		.one(conn)
		.await?
		.ok_or_else(|| APIError::NotFound("Series not found".to_owned()))
}

fn unsupported_metadata_fields(
	patch: &KomgaSeriesMetadataUpdateRequest,
) -> Vec<&'static str> {
	let mut fields = Vec::new();
	macro_rules! unsupported {
		($field:ident, $wire_name:literal) => {
			if !patch.$field.is_unset() {
				fields.push($wire_name);
			}
		};
	}

	// Stump persists all of the Komga series metadata fields below except sharing labels.
	// Sharing labels have no Stump equivalent, so a present value (including null) is
	// rejected instead of silently dropping the write.
	unsupported!(sharing_labels, "sharingLabels");
	unsupported!(sharing_labels_lock, "sharingLabelsLock");
	fields
}

fn validate_metadata_patch(patch: &KomgaSeriesMetadataUpdateRequest) -> APIResult<()> {
	let fields = unsupported_metadata_fields(patch);
	if fields.is_empty() {
		Ok(())
	} else {
		Err(APIError::BadRequest(format!(
			"Unsupported series metadata fields: {}",
			fields.join(", ")
		)))
	}
}

fn status_name(status: KomgaSeriesStatus) -> String {
	match status {
		KomgaSeriesStatus::Ended => "ENDED",
		KomgaSeriesStatus::Ongoing => "ONGOING",
		KomgaSeriesStatus::Abandoned => "ABANDONED",
		KomgaSeriesStatus::Hiatus => "HIATUS",
	}
	.to_owned()
}

fn csv_value(values: &[String]) -> Option<String> {
	(!values.is_empty()).then(|| values.join(", "))
}

fn links_csv_value(values: &[KomgaWebLink]) -> Option<String> {
	(!values.is_empty()).then(|| {
		values
			.iter()
			.map(|link| link.url.clone())
			.collect::<Vec<_>>()
			.join(", ")
	})
}

fn metadata_patch_has_changes(patch: &KomgaSeriesMetadataUpdateRequest) -> bool {
	!patch.status.is_unset()
		|| !patch.status_lock.is_unset()
		|| !patch.title.is_unset()
		|| !patch.title_lock.is_unset()
		|| !patch.title_sort.is_unset()
		|| !patch.title_sort_lock.is_unset()
		|| !patch.summary.is_unset()
		|| !patch.summary_lock.is_unset()
		|| !patch.publisher.is_unset()
		|| !patch.publisher_lock.is_unset()
		|| !patch.reading_direction.is_unset()
		|| !patch.reading_direction_lock.is_unset()
		|| !patch.age_rating.is_unset()
		|| !patch.age_rating_lock.is_unset()
		|| !patch.language.is_unset()
		|| !patch.language_lock.is_unset()
		|| !patch.alternate_titles.is_unset()
		|| !patch.alternate_titles_lock.is_unset()
		|| !patch.genres.is_unset()
		|| !patch.genres_lock.is_unset()
		|| !patch.tags_lock.is_unset()
		|| !patch.total_book_count.is_unset()
		|| !patch.total_book_count_lock.is_unset()
		|| !patch.links.is_unset()
		|| !patch.links_lock.is_unset()
}

fn reading_direction_name(direction: crate::KomgaReadingDirection) -> &'static str {
	match direction {
		crate::KomgaReadingDirection::LeftToRight => "LEFT_TO_RIGHT",
		crate::KomgaReadingDirection::RightToLeft => "RIGHT_TO_LEFT",
		crate::KomgaReadingDirection::Vertical => "VERTICAL",
		crate::KomgaReadingDirection::Webtoon => "WEBTOON",
	}
}

fn apply_bool_patch(active: &mut ActiveValue<bool>, patch: &PatchValue<bool>) {
	match patch {
		PatchValue::Unset => {},
		PatchValue::None => *active = Set(false),
		PatchValue::Some(value) => *active = Set(*value),
	}
}

fn apply_komga_series_metadata_patch(
	active: &mut series_metadata::ActiveModel,
	patch: &KomgaSeriesMetadataUpdateRequest,
) -> APIResult<()> {
	match &patch.title_sort {
		PatchValue::Unset => {},
		PatchValue::None => active.title_sort = Set(None),
		PatchValue::Some(value) => active.title_sort = Set(Some(value.clone())),
	}
	match &patch.reading_direction {
		PatchValue::Unset => {},
		PatchValue::None => active.reading_direction = Set(None),
		PatchValue::Some(value) => {
			active.reading_direction =
				Set(Some(reading_direction_name(*value).to_owned()))
		},
	}
	match &patch.language {
		PatchValue::Unset => {},
		PatchValue::None => active.language = Set(None),
		PatchValue::Some(value) => active.language = Set(Some(value.clone())),
	}
	match &patch.alternate_titles {
		PatchValue::Unset => {},
		PatchValue::None => active.alternate_titles = Set(None),
		PatchValue::Some(value) => {
			active.alternate_titles = Set(Some(serde_json::to_string(value)?))
		},
	}

	apply_bool_patch(&mut active.title_sort_lock, &patch.title_sort_lock);
	apply_bool_patch(
		&mut active.reading_direction_lock,
		&patch.reading_direction_lock,
	);
	apply_bool_patch(&mut active.language_lock, &patch.language_lock);
	apply_bool_patch(
		&mut active.alternate_titles_lock,
		&patch.alternate_titles_lock,
	);
	Ok(())
}

fn lock_patch_requested(patch: &KomgaSeriesMetadataUpdateRequest) -> bool {
	!patch.status_lock.is_unset()
		|| !patch.title_lock.is_unset()
		|| !patch.summary_lock.is_unset()
		|| !patch.publisher_lock.is_unset()
		|| !patch.age_rating_lock.is_unset()
		|| !patch.genres_lock.is_unset()
		|| !patch.tags_lock.is_unset()
		|| !patch.total_book_count_lock.is_unset()
		|| !patch.links_lock.is_unset()
}

fn lock_matches(value: &serde_json::Value, field: &str) -> bool {
	value
		.as_str()
		.is_some_and(|value| value.eq_ignore_ascii_case(field))
}

fn apply_lock_patch(
	values: &mut Vec<serde_json::Value>,
	field: &str,
	patch: &PatchValue<bool>,
) {
	match patch {
		PatchValue::Unset => {},
		PatchValue::Some(true) => {
			if !values.iter().any(|value| lock_matches(value, field)) {
				values.push(serde_json::Value::String(field.to_owned()));
			}
		},
		PatchValue::Some(false) | PatchValue::None => {
			values.retain(|value| !lock_matches(value, field));
		},
	}
}

fn lock_values(metadata: Option<&series_metadata::Model>) -> Vec<serde_json::Value> {
	metadata
		.and_then(|metadata| metadata.locked_fields.as_ref())
		.and_then(serde_json::Value::as_array)
		.cloned()
		.unwrap_or_default()
}

fn unique_tag_names(values: &[String]) -> Vec<String> {
	let mut seen = HashSet::with_capacity(values.len());
	values
		.iter()
		.filter(|value| seen.insert((*value).clone()))
		.cloned()
		.collect()
}

async fn replace_series_tags(
	txn: &DatabaseTransaction,
	series_id: &str,
	desired: &[String],
) -> APIResult<()> {
	let desired = unique_tag_names(desired);
	let existing = if desired.is_empty() {
		Vec::new()
	} else {
		tag::Entity::find()
			.filter(tag::Column::Name.is_in(desired.clone()))
			.all(txn)
			.await?
	};
	let mut tag_ids = existing
		.into_iter()
		.map(|model| (model.name, model.id))
		.collect::<HashMap<_, _>>();

	let missing = desired
		.iter()
		.filter(|name| !tag_ids.contains_key(*name))
		.map(|name| tag::ActiveModel {
			name: Set((*name).clone()),
			..Default::default()
		})
		.collect::<Vec<_>>();
	if !missing.is_empty() {
		for model in tag::Entity::insert_many(missing)
			.exec_with_returning_many(txn)
			.await?
		{
			tag_ids.insert(model.name, model.id);
		}
	}

	series_tag::Entity::delete_many()
		.filter(series_tag::Column::SeriesId.eq(series_id.to_owned()))
		.exec(txn)
		.await?;
	if !desired.is_empty() {
		let links = desired
			.iter()
			.filter_map(|name| tag_ids.get(name).copied())
			.map(|tag_id| series_tag::ActiveModel {
				series_id: Set(series_id.to_owned()),
				tag_id: Set(tag_id),
				..Default::default()
			})
			.collect::<Vec<_>>();
		if !links.is_empty() {
			series_tag::Entity::insert_many(links).exec(txn).await?;
		}
	}
	Ok(())
}

async fn patch_series_metadata(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<KomgaEvents>,
	Json(patch): Json<KomgaSeriesMetadataUpdateRequest>,
) -> APIResult<StatusCode> {
	enforce_manage_library(&auth)?;
	validate_metadata_patch(&patch)?;
	let user = auth.user();
	let series = find_visible_series(ctx.conn(), &user, &id).await?;
	let txn = begin_write(ctx.conn()).await?;
	let existing_metadata = series_metadata::Entity::find_by_id(series.id.clone())
		.one(&txn)
		.await?;
	let had_metadata = existing_metadata.is_some();
	let mut active = match existing_metadata.as_ref() {
		Some(metadata) => metadata.clone().into_active_model(),
		None => series_metadata::ActiveModel {
			series_id: Set(series.id.clone()),
			..Default::default()
		},
	};
	apply_komga_series_metadata_patch(&mut active, &patch)?;

	match &patch.status {
		PatchValue::Unset => {},
		PatchValue::None => active.status = Set(None),
		PatchValue::Some(value) => active.status = Set(Some(status_name(*value))),
	}
	match &patch.title {
		PatchValue::Unset => {},
		PatchValue::None => active.title = Set(None),
		PatchValue::Some(value) => active.title = Set(Some(value.clone())),
	}
	match &patch.summary {
		PatchValue::Unset => {},
		PatchValue::None => active.summary = Set(None),
		PatchValue::Some(value) => active.summary = Set(Some(value.clone())),
	}
	match &patch.publisher {
		PatchValue::Unset => {},
		PatchValue::None => active.publisher = Set(None),
		PatchValue::Some(value) => active.publisher = Set(Some(value.clone())),
	}
	match &patch.age_rating {
		PatchValue::Unset => {},
		PatchValue::None => active.age_rating = Set(None),
		PatchValue::Some(value) => active.age_rating = Set(Some(*value)),
	}
	match &patch.genres {
		PatchValue::Unset => {},
		PatchValue::None => active.genres = Set(None),
		PatchValue::Some(value) => active.genres = Set(csv_value(value)),
	}
	match &patch.total_book_count {
		PatchValue::Unset => {},
		PatchValue::None => active.total_issues = Set(None),
		PatchValue::Some(value) => active.total_issues = Set(Some(*value)),
	}
	match &patch.links {
		PatchValue::Unset => {},
		PatchValue::None => active.links = Set(None),
		PatchValue::Some(value) => active.links = Set(links_csv_value(value)),
	}

	if lock_patch_requested(&patch) {
		let mut values = lock_values(existing_metadata.as_ref());
		apply_lock_patch(&mut values, "STATUS", &patch.status_lock);
		apply_lock_patch(&mut values, "TITLE", &patch.title_lock);
		apply_lock_patch(&mut values, "SUMMARY", &patch.summary_lock);
		apply_lock_patch(&mut values, "PUBLISHER", &patch.publisher_lock);
		apply_lock_patch(&mut values, "AGE_RATING", &patch.age_rating_lock);
		apply_lock_patch(&mut values, "GENRES", &patch.genres_lock);
		apply_lock_patch(&mut values, "TAGS", &patch.tags_lock);
		apply_lock_patch(&mut values, "VOLUME_COUNT", &patch.total_book_count_lock);
		apply_lock_patch(&mut values, "LINKS", &patch.links_lock);
		active.locked_fields = Set(Some(serde_json::Value::Array(values)));
	}

	if had_metadata || metadata_patch_has_changes(&patch) {
		if had_metadata {
			active.update(&txn).await?;
		} else {
			active.insert(&txn).await?;
		}
	}

	if let Some(desired) = match &patch.tags {
		PatchValue::Unset => None,
		PatchValue::None => Some(Vec::new()),
		PatchValue::Some(values) => Some(values.clone()),
	} {
		replace_series_tags(&txn, &series.id, &desired).await?;
	}

	txn.commit().await?;
	events.send(KomgaEvent::SeriesChanged {
		series_id: series.id.clone().into(),
		library_id: series.library_id.clone().unwrap_or_default().into(),
	});
	Ok(StatusCode::NO_CONTENT)
}
async fn tracker_series_books(
	conn: &DatabaseConnection,
	user: &AuthUser,
	series_id: &str,
) -> APIResult<Vec<crate::routes::progress::ProgressBook>> {
	let books = media::ModelWithMetadata::find_for_user(user)
		.filter(media::Column::SeriesId.eq(series_id.to_owned()))
		.filter(media::Column::DeletedAt.is_null())
		.order_by_asc(media::Column::Name)
		.into_model::<media::ModelWithMetadata>()
		.all(conn)
		.await?;
	let mut books = progress_books(books);
	for (index, book) in books.iter_mut().enumerate() {
		if book.number.is_none() {
			book.number = Some((index + 1) as f64);
		}
	}
	Ok(books)
}

async fn get_tachiyomi_series_progress(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<Json<KomgaSeriesReadProgressDto>> {
	let user = auth.user();
	let series = find_visible_series(ctx.conn(), &user, &id).await?;
	let books = tracker_series_books(ctx.conn(), &user, &series.id).await?;
	let counts = counts_for_progress_books(ctx.conn(), &user, &books, true).await?;
	Ok(Json(KomgaSeriesReadProgressDto {
		books_count: counts.books_count,
		books_read_count: counts.books_read_count,
		books_unread_count: counts.books_unread_count,
		books_in_progress_count: counts.books_in_progress_count,
		last_read_continuous_number_sort: counts.last_read_continuous_number_sort,
		max_number_sort: counts.max_number_sort,
	}))
}

async fn update_tachiyomi_series_progress(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<KomgaEvents>,
	Json(update): Json<KomgaSeriesReadProgressUpdateDto>,
) -> APIResult<StatusCode> {
	if !update.last_book_number_sort_read.is_finite()
		|| update.last_book_number_sort_read < 0.0
	{
		return Err(APIError::BadRequest(
			"lastBookNumberSortRead must be a non-negative finite number".to_owned(),
		));
	}
	let user = auth.user();
	let series = find_visible_series(ctx.conn(), &user, &id).await?;
	let books = tracker_series_books(ctx.conn(), &user, &series.id).await?;
	let to_mark = books
		.into_iter()
		.filter(|book| {
			book.number
				.is_some_and(|number| number <= update.last_book_number_sort_read)
		})
		.collect::<Vec<_>>();
	if to_mark.is_empty() {
		return Ok(StatusCode::NO_CONTENT);
	}
	let txn = begin_write(ctx.conn()).await?;
	for book in &to_mark {
		mark_book_read(&txn, &user, &book.id, book.pages).await?;
	}
	txn.commit().await?;
	for book in &to_mark {
		tracing::debug!(
			book_id = %book.id,
			number = ?book.number,
			series_id = %series.id,
			"Marked book read from Tachiyomi series progress"
		);
		events.send(KomgaEvent::ReadProgressChanged {
			book_id: book.id.clone().into(),
			user_id: user.id.clone().into(),
		});
		events.send(KomgaEvent::ReadProgressSeriesChanged {
			series_id: series.id.clone().into(),
			user_id: user.id.clone().into(),
		});
		events.send(KomgaEvent::BookChanged {
			book_id: book.id.clone().into(),
			series_id: series.id.clone().into(),
			library_id: series.library_id.clone().unwrap_or_default().into(),
		});
		events.send(KomgaEvent::SeriesChanged {
			series_id: series.id.clone().into(),
			library_id: series.library_id.clone().unwrap_or_default().into(),
		});
	}
	Ok(StatusCode::NO_CONTENT)
}

async fn series_books(
	conn: &DatabaseConnection,
	user: &AuthUser,
	series_id: &str,
) -> APIResult<Vec<media::Model>> {
	Ok(media::Entity::find_for_user(user)
		.filter(media::Column::SeriesId.eq(series_id.to_owned()))
		.filter(media::Column::DeletedAt.is_null())
		.filter(series::Column::DeletedAt.is_null())
		.order_by_asc(media::Column::Name)
		.all(conn)
		.await?)
}

async fn mark_series_read(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<KomgaEvents>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let series = find_visible_series(ctx.conn(), &user, &id).await?;
	let books = series_books(ctx.conn(), &user, &series.id).await?;
	if books.is_empty() {
		return Ok(StatusCode::NO_CONTENT);
	}

	let txn = begin_write(ctx.conn()).await?;
	for book in &books {
		mark_book_read(&txn, &user, &book.id, book.pages).await?;
	}
	txn.commit().await?;
	events.send(KomgaEvent::ReadProgressSeriesChanged {
		series_id: series.id.clone().into(),
		user_id: user.id.clone().into(),
	});
	events.send(KomgaEvent::SeriesChanged {
		series_id: series.id.clone().into(),
		library_id: series.library_id.clone().unwrap_or_default().into(),
	});
	for book in &books {
		events.send(KomgaEvent::BookChanged {
			book_id: book.id.clone().into(),
			series_id: series.id.clone().into(),
			library_id: series.library_id.clone().unwrap_or_default().into(),
		});
	}
	Ok(StatusCode::NO_CONTENT)
}

async fn delete_series_read_progress(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<KomgaEvents>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let series = find_visible_series(ctx.conn(), &user, &id).await?;
	let books = series_books(ctx.conn(), &user, &series.id).await?;
	let book_ids = books.into_iter().map(|book| book.id).collect::<Vec<_>>();
	if book_ids.is_empty() {
		return Ok(StatusCode::NO_CONTENT);
	}

	let txn = begin_write(ctx.conn()).await?;
	clear_books_progress(&txn, &user, &book_ids).await?;
	txn.commit().await?;
	events.send(KomgaEvent::ReadProgressSeriesDeleted {
		series_id: series.id.clone().into(),
		user_id: user.id.clone().into(),
	});
	events.send(KomgaEvent::SeriesChanged {
		series_id: series.id.clone().into(),
		library_id: series.library_id.clone().unwrap_or_default().into(),
	});
	for book_id in &book_ids {
		events.send(KomgaEvent::BookChanged {
			book_id: book_id.clone().into(),
			series_id: series.id.clone().into(),
			library_id: series.library_id.clone().unwrap_or_default().into(),
		});
	}
	Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn metadata_patch_rejects_only_sharing_labels() {
		let patch: KomgaSeriesMetadataUpdateRequest = serde_json::from_value(json!({
			"titleSort": "A",
			"language": "en",
			"readingDirection": "LEFT_TO_RIGHT",
			"alternateTitles": [],
			"sharingLabels": []
		}))
		.unwrap();
		let error = validate_metadata_patch(&patch).unwrap_err();
		assert!(matches!(
			error,
			APIError::BadRequest(message) if message.contains("sharingLabels")
		));
	}

	#[test]
	fn metadata_patch_applies_new_fields_when_set() {
		let patch: KomgaSeriesMetadataUpdateRequest = serde_json::from_value(json!({
			"titleSort": "Sorted title",
			"titleSortLock": true,
			"readingDirection": "LEFT_TO_RIGHT",
			"readingDirectionLock": true,
			"language": "en",
			"languageLock": true,
			"alternateTitles": [{"label": "native", "title": "Original"}],
			"alternateTitlesLock": true
		}))
		.unwrap();
		let mut active = series_metadata::ActiveModel {
			series_id: Set("series".to_owned()),
			..Default::default()
		};

		apply_komga_series_metadata_patch(&mut active, &patch).unwrap();

		assert!(matches!(
			&active.title_sort,
			ActiveValue::Set(Some(value)) if value == "Sorted title"
		));
		assert!(matches!(
			&active.reading_direction,
			ActiveValue::Set(Some(value)) if value == "LEFT_TO_RIGHT"
		));
		assert!(matches!(
			&active.language,
			ActiveValue::Set(Some(value)) if value == "en"
		));
		assert!(matches!(
			&active.alternate_titles,
			ActiveValue::Set(Some(value))
				if value == r#"[{"label":"native","title":"Original"}]"#
		));
		assert!(matches!(&active.title_sort_lock, ActiveValue::Set(true)));
		assert!(matches!(
			&active.reading_direction_lock,
			ActiveValue::Set(true)
		));
		assert!(matches!(&active.language_lock, ActiveValue::Set(true)));
		assert!(matches!(
			&active.alternate_titles_lock,
			ActiveValue::Set(true)
		));
	}

	#[test]
	fn metadata_patch_clears_new_fields_and_locks_with_null() {
		let patch: KomgaSeriesMetadataUpdateRequest = serde_json::from_value(json!({
			"titleSort": null,
			"titleSortLock": null,
			"readingDirection": null,
			"readingDirectionLock": null,
			"language": null,
			"languageLock": null,
			"alternateTitles": null,
			"alternateTitlesLock": null
		}))
		.unwrap();
		let mut active = series_metadata::ActiveModel {
			series_id: Set("series".to_owned()),
			title_sort: Set(Some("Sorted title".to_owned())),
			title_sort_lock: Set(true),
			reading_direction: Set(Some("RIGHT_TO_LEFT".to_owned())),
			reading_direction_lock: Set(true),
			language: Set(Some("fr".to_owned())),
			language_lock: Set(true),
			alternate_titles: Set(Some("[]".to_owned())),
			alternate_titles_lock: Set(true),
			..Default::default()
		};

		apply_komga_series_metadata_patch(&mut active, &patch).unwrap();

		assert!(matches!(&active.title_sort, ActiveValue::Set(None)));
		assert!(matches!(&active.reading_direction, ActiveValue::Set(None)));
		assert!(matches!(&active.language, ActiveValue::Set(None)));
		assert!(matches!(&active.alternate_titles, ActiveValue::Set(None)));
		assert!(matches!(&active.title_sort_lock, ActiveValue::Set(false)));
		assert!(matches!(
			&active.reading_direction_lock,
			ActiveValue::Set(false)
		));
		assert!(matches!(&active.language_lock, ActiveValue::Set(false)));
		assert!(matches!(
			&active.alternate_titles_lock,
			ActiveValue::Set(false)
		));
	}

	#[test]
	fn metadata_patch_leaves_new_fields_and_locks_unchanged_when_unset() {
		let patch: KomgaSeriesMetadataUpdateRequest =
			serde_json::from_value(json!({})).unwrap();
		let mut active = series_metadata::ActiveModel {
			series_id: Set("series".to_owned()),
			title_sort: Set(Some("Sorted title".to_owned())),
			title_sort_lock: Set(true),
			reading_direction: Set(Some("VERTICAL".to_owned())),
			reading_direction_lock: Set(true),
			language: Set(Some("ja".to_owned())),
			language_lock: Set(true),
			alternate_titles: Set(Some(
				r#"[{"label":"native","title":"Original"}]"#.to_owned(),
			)),
			alternate_titles_lock: Set(true),
			..Default::default()
		};

		apply_komga_series_metadata_patch(&mut active, &patch).unwrap();

		assert!(matches!(
			&active.title_sort,
			ActiveValue::Set(Some(value)) if value == "Sorted title"
		));
		assert!(matches!(
			&active.reading_direction,
			ActiveValue::Set(Some(value)) if value == "VERTICAL"
		));
		assert!(matches!(
			&active.language,
			ActiveValue::Set(Some(value)) if value == "ja"
		));
		assert!(matches!(
			&active.alternate_titles,
			ActiveValue::Set(Some(value))
				if value == r#"[{"label":"native","title":"Original"}]"#
		));
		assert!(matches!(&active.title_sort_lock, ActiveValue::Set(true)));
		assert!(matches!(
			&active.reading_direction_lock,
			ActiveValue::Set(true)
		));
		assert!(matches!(&active.language_lock, ActiveValue::Set(true)));
		assert!(matches!(
			&active.alternate_titles_lock,
			ActiveValue::Set(true)
		));
	}

	#[test]
	fn metadata_patch_accepts_persisted_fields() {
		let patch: KomgaSeriesMetadataUpdateRequest = serde_json::from_value(json!({
			"title": "Updated", "genres": ["one"], "tags": ["tag"]
		}))
		.unwrap();
		assert!(validate_metadata_patch(&patch).is_ok());
	}
}
