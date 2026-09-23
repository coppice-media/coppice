use chrono::{Datelike, Utc};
use std::{
	cmp::Ordering as CmpOrdering, collections::BTreeSet, convert::TryFrom, sync::Arc,
};

use crate::{
	book::{KomgaMediaStatus, KomgaReadStatus},
	extensions_for_media_profile,
	search::{
		AuthorMatch, BookCondition, BooleanOperator, DateOperator, Equality,
		EqualityNullable, SeriesCondition,
	},
	KomgaAuthor, KomgaBook, KomgaBookSearch, KomgaLibrary, KomgaSeries,
	KomgaSeriesSearch, KomgaSeriesStatus, Page, SUPPORTED_MEDIA_EXTENSIONS,
};
use axum::{
	body::Body,
	extract::Path,
	http::HeaderMap,
	response::Response,
	routing::{get, post},
	Extension, Json, Router,
};
use axum_extra::extract::Query;
use models::entity::{
	library, library_config, media, media_metadata, media_tag, reading_head,
	reading_list, reading_list_item, series, series_metadata, series_tag, tag,
	user::AuthUser,
};
use models::shared::enums::FileStatus;
use sea_orm::{
	prelude::*,
	sea_query::{
		Condition, Expr, Func, NullOrdering, Order, SelectStatement, SimpleExpr, Value,
	},
	ConnectionTrait, FromQueryResult, QueryOrder, QuerySelect, QueryTrait, Statement,
};
use serde::Deserialize;
use stump_auth::AuthContext;

use super::mapper::{map_books, map_library, map_series};
use super::KomgaBackend;
use crate::{
	errors::{APIError, APIResult},
	routes::response::cached_json,
};

const DEFAULT_PAGE_SIZE: i32 = 20;
const MAX_PAGE_SIZE: i32 = 200;
const READING_LIST_READER_ROLE: i32 = 1;

/// The catalog and referential endpoints used by Komelia. Authentication is installed by the
/// parent router; every handler receives the authenticated context as an extension.
pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::<S>::new()
		.route("/api/v1/libraries", get(get_libraries))
		.route("/api/v1/libraries/{id}", get(get_library))
		.route("/api/v1/books/list", post(get_books))
		.route("/api/v1/series", get(get_series_legacy))
		.route("/api/v1/series/list", post(get_series))
		.route("/api/v1/series/{id}/books", get(get_series_books))
		.route("/api/v1/books/ondeck", get(get_books_on_deck))
		.route("/api/v1/series/new", get(get_series_new))
		.route("/api/v1/series/updated", get(get_series_updated))
		.route("/api/v1/books/{id}", get(get_book))
		.route("/api/v1/series/{id}", get(get_series_by_id))
		.route("/api/v1/books/{id}/previous", get(get_previous_book))
		.route("/api/v1/books/{id}/next", get(get_next_book))
		.route("/api/v1/tags/book", get(get_book_tags))
		.route("/api/v1/tags/series", get(get_series_tags))
		.route("/api/v1/tags", get(get_tags))
		.route("/api/v1/authors", get(get_authors_legacy))
		.route("/api/v2/authors", get(get_authors))
		.route("/api/v1/genres", get(get_genres))
		.route("/api/v1/series/release-dates", get(get_release_dates))
		.route("/api/v1/age-ratings", get(get_age_ratings))
		.route("/api/v1/publishers", get(get_publishers))
		.route("/api/v1/sharing-labels", get(get_sharing_labels))
		.route("/api/v1/languages", get(get_languages))
}

pub(crate) fn legacy_books<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::<S>::new().route("/api/v1/books", get(get_books_legacy))
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PaginationQuery {
	page: i32,
	size: i32,
	unpaged: bool,
	sort: Vec<String>,
}

impl Default for PaginationQuery {
	fn default() -> Self {
		Self {
			page: 0,
			size: DEFAULT_PAGE_SIZE,
			unpaged: false,
			sort: Vec::new(),
		}
	}
}

#[derive(Debug, Clone, Copy)]
struct Pagination {
	page: i32,
	size: i32,
	unpaged: bool,
}

impl PaginationQuery {
	fn validate(self) -> APIResult<(Pagination, Vec<SortSpec>)> {
		if self.page < 0 {
			return Err(APIError::BadRequest(
				"page must be zero-based and non-negative".to_owned(),
			));
		}
		// size=0 is Komga's count-only idiom: the page envelope carries
		// totalElements while content stays empty.
		if self.size < 0 {
			return Err(APIError::BadRequest("size must be non-negative".to_owned()));
		}
		let size = if self.size > MAX_PAGE_SIZE {
			tracing::warn!(
				requested = self.size,
				clamped = MAX_PAGE_SIZE,
				"Clamping oversized Komga page size"
			);
			MAX_PAGE_SIZE
		} else {
			self.size
		};
		let sorts = parse_sort_specs(&self.sort);
		Ok((
			Pagination {
				page: self.page,
				size,
				unpaged: self.unpaged,
			},
			sorts,
		))
	}
}

impl Pagination {
	fn offset(self) -> u64 {
		(self.page as u64).saturating_mul(self.size as u64)
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortDirection {
	Asc,
	Desc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SortSpec {
	field: String,
	direction: SortDirection,
}

fn parse_sort_specs(values: &[String]) -> Vec<SortSpec> {
	let mut specs = Vec::with_capacity(values.len());
	for value in values {
		let mut parts = value.split(',');
		let field = parts.next().unwrap_or_default().trim();
		let direction = parts.next().unwrap_or("asc").trim();
		if field.is_empty() || parts.next().is_some() {
			tracing::warn!(
				sort = %value,
				"Malformed Komga sort; using the endpoint default ordering"
			);
			return Vec::new();
		}
		let direction = match direction.to_ascii_lowercase().as_str() {
			"asc" => SortDirection::Asc,
			"desc" => SortDirection::Desc,
			_ => {
				tracing::warn!(
					sort = %value,
					"Invalid Komga sort direction; using the endpoint default ordering"
				);
				return Vec::new();
			},
		};
		specs.push(SortSpec {
			field: field.to_ascii_lowercase(),
			direction,
		});
	}
	specs
}

fn total_as_i32(total: u64) -> APIResult<i32> {
	i32::try_from(total).map_err(|error| {
		APIError::InternalServerError(format!("page total is too large: {error}"))
	})
}

fn unsupported_filter(name: &str) -> APIError {
	APIError::BadRequest(format!(
		"filter {name} is not supported by this Komga profile"
	))
}

fn csv_values(values: &[String]) -> Vec<String> {
	values
		.iter()
		.flat_map(|value| value.split(','))
		.map(str::trim)
		.filter(|value| !value.is_empty())
		.map(ToOwned::to_owned)
		.collect()
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct LegacyCatalogQuery {
	page: i32,
	size: i32,
	unpaged: bool,
	search: String,
	deleted: bool,
	#[serde(rename = "library_id", alias = "libraryId")]
	library_id: Vec<String>,
	read_status: Vec<String>,
	author: Vec<String>,
	status: Vec<String>,
	genre: Vec<String>,
	tag: Vec<String>,
	publisher: Vec<String>,
	media_status: Vec<String>,
	sort: Vec<String>,
}

impl Default for LegacyCatalogQuery {
	fn default() -> Self {
		Self {
			page: 0,
			size: DEFAULT_PAGE_SIZE,
			unpaged: false,
			search: String::new(),
			deleted: false,
			library_id: Vec::new(),
			read_status: Vec::new(),
			author: Vec::new(),
			status: Vec::new(),
			genre: Vec::new(),
			tag: Vec::new(),
			publisher: Vec::new(),
			media_status: Vec::new(),
			sort: Vec::new(),
		}
	}
}

impl LegacyCatalogQuery {
	fn pagination(&self) -> APIResult<(Pagination, Vec<SortSpec>)> {
		PaginationQuery {
			page: self.page,
			size: self.size,
			unpaged: self.unpaged,
			sort: self.sort.clone(),
		}
		.validate()
	}

	fn book_search(&self, series_id: Option<&str>) -> APIResult<KomgaBookSearch> {
		let mut conditions = Vec::new();
		let library_conditions = csv_values(&self.library_id)
			.into_iter()
			.map(|value| BookCondition::LibraryId {
				operator: Equality::Is {
					value: value.into(),
				},
			})
			.collect::<Vec<_>>();
		if let Some(condition) = any_book_condition(library_conditions) {
			conditions.push(condition);
		}

		if let Some(series_id) = series_id.filter(|id| !id.trim().is_empty()) {
			conditions.push(BookCondition::SeriesId {
				operator: Equality::Is {
					value: series_id.into(),
				},
			});
		}

		let read_statuses =
			parse_query_values::<KomgaReadStatus>(&self.read_status, "read_status")?;
		let read_conditions = read_statuses
			.into_iter()
			.map(|value| BookCondition::ReadStatus {
				operator: Equality::Is { value },
			})
			.collect::<Vec<_>>();
		if let Some(condition) = any_book_condition(read_conditions) {
			conditions.push(condition);
		}

		let media_statuses =
			parse_query_values::<KomgaMediaStatus>(&self.media_status, "media_status")?;
		let media_conditions = media_statuses
			.into_iter()
			.map(|value| BookCondition::MediaStatus {
				operator: Equality::Is { value },
			})
			.collect::<Vec<_>>();
		if let Some(condition) = any_book_condition(media_conditions) {
			conditions.push(condition);
		}

		let tag_conditions = csv_values(&self.tag)
			.into_iter()
			.map(|value| BookCondition::Tag {
				operator: EqualityNullable::Is { value },
			})
			.collect::<Vec<_>>();
		if let Some(condition) = any_book_condition(tag_conditions) {
			conditions.push(condition);
		}

		let author_conditions = self
			.author
			.iter()
			.map(|value| parse_author_match(value))
			.map(|value| {
				value.map(|value| BookCondition::Author {
					operator: Equality::Is { value },
				})
			})
			.collect::<APIResult<Vec<_>>>()?;
		if let Some(condition) = any_book_condition(author_conditions) {
			conditions.push(condition);
		}

		Ok(KomgaBookSearch {
			condition: all_book_condition(conditions),
			full_text_search: nonempty_query(&self.search),
		})
	}

	fn series_search(&self) -> APIResult<KomgaSeriesSearch> {
		let mut conditions = Vec::new();
		let library_conditions = csv_values(&self.library_id)
			.into_iter()
			.map(|value| SeriesCondition::LibraryId {
				operator: Equality::Is {
					value: value.into(),
				},
			})
			.collect::<Vec<_>>();
		if let Some(condition) = any_series_condition(library_conditions) {
			conditions.push(condition);
		}

		let statuses = parse_query_values::<KomgaSeriesStatus>(&self.status, "status")?;
		let status_conditions = statuses
			.into_iter()
			.map(|value| SeriesCondition::SeriesStatus {
				operator: Equality::Is { value },
			})
			.collect::<Vec<_>>();
		if let Some(condition) = any_series_condition(status_conditions) {
			conditions.push(condition);
		}

		let read_statuses =
			parse_query_values::<KomgaReadStatus>(&self.read_status, "read_status")?;
		let read_conditions = read_statuses
			.into_iter()
			.map(|value| SeriesCondition::ReadStatus {
				operator: Equality::Is { value },
			})
			.collect::<Vec<_>>();
		if let Some(condition) = any_series_condition(read_conditions) {
			conditions.push(condition);
		}

		let genre_conditions = csv_values(&self.genre)
			.into_iter()
			.map(|value| SeriesCondition::Genre {
				operator: EqualityNullable::Is { value },
			})
			.collect::<Vec<_>>();
		if let Some(condition) = any_series_condition(genre_conditions) {
			conditions.push(condition);
		}

		let tag_conditions = csv_values(&self.tag)
			.into_iter()
			.map(|value| SeriesCondition::Tag {
				operator: EqualityNullable::Is { value },
			})
			.collect::<Vec<_>>();
		if let Some(condition) = any_series_condition(tag_conditions) {
			conditions.push(condition);
		}

		let publisher_conditions = csv_values(&self.publisher)
			.into_iter()
			.map(|value| SeriesCondition::Publisher {
				operator: Equality::Is { value },
			})
			.collect::<Vec<_>>();
		if let Some(condition) = any_series_condition(publisher_conditions) {
			conditions.push(condition);
		}

		let author_conditions = self
			.author
			.iter()
			.map(|value| parse_author_match(value))
			.map(|value| {
				value.map(|value| SeriesCondition::Author {
					operator: Equality::Is { value },
				})
			})
			.collect::<APIResult<Vec<_>>>()?;
		if let Some(condition) = any_series_condition(author_conditions) {
			conditions.push(condition);
		}

		Ok(KomgaSeriesSearch {
			condition: all_series_condition(conditions),
			full_text_search: nonempty_query(&self.search),
		})
	}
}

fn nonempty_query(value: &str) -> Option<String> {
	let value = value.trim();
	(!value.is_empty()).then(|| value.to_owned())
}

fn parse_query_values<T>(values: &[String], field: &str) -> APIResult<Vec<T>>
where
	T: serde::de::DeserializeOwned,
{
	csv_values(values)
		.into_iter()
		.map(|value| {
			serde_json::from_value(serde_json::Value::String(value.to_ascii_uppercase()))
				.map_err(|_| {
					APIError::BadRequest(format!("invalid {field} value: {value}"))
				})
		})
		.collect()
}

fn parse_author_match(value: &str) -> APIResult<AuthorMatch> {
	let Some((name, role)) = value.rsplit_once(',') else {
		return Err(APIError::BadRequest(
			"author filters must use name,role".to_owned(),
		));
	};
	let name = name.trim();
	let role = role.trim();
	if name.is_empty() || role.is_empty() {
		return Err(APIError::BadRequest(
			"author filters must use a non-empty name and role".to_owned(),
		));
	}
	Ok(AuthorMatch {
		name: Some(name.to_owned()),
		role: Some(role.to_ascii_lowercase()),
	})
}

fn any_book_condition(mut conditions: Vec<BookCondition>) -> Option<BookCondition> {
	match conditions.len() {
		0 => None,
		1 => Some(conditions.pop().expect("one condition exists")),
		_ => Some(BookCondition::AnyOfBook { conditions }),
	}
}

fn all_book_condition(mut conditions: Vec<BookCondition>) -> Option<BookCondition> {
	match conditions.len() {
		0 => None,
		1 => Some(conditions.pop().expect("one condition exists")),
		_ => Some(BookCondition::AllOfBook { conditions }),
	}
}

fn any_series_condition(mut conditions: Vec<SeriesCondition>) -> Option<SeriesCondition> {
	match conditions.len() {
		0 => None,
		1 => Some(conditions.pop().expect("one condition exists")),
		_ => Some(SeriesCondition::AnyOfSeries { conditions }),
	}
}

fn all_series_condition(mut conditions: Vec<SeriesCondition>) -> Option<SeriesCondition> {
	match conditions.len() {
		0 => None,
		1 => Some(conditions.pop().expect("one condition exists")),
		_ => Some(SeriesCondition::AllOfSeries { conditions }),
	}
}

fn series_status_name(status: KomgaSeriesStatus) -> &'static str {
	match status {
		KomgaSeriesStatus::Ended => "ENDED",
		KomgaSeriesStatus::Ongoing => "ONGOING",
		KomgaSeriesStatus::Abandoned => "ABANDONED",
		KomgaSeriesStatus::Hiatus => "HIATUS",
	}
}

fn apply_legacy_book_metadata_filters(
	mut query: Select<media::Entity>,
	legacy: &LegacyCatalogQuery,
) -> APIResult<Select<media::Entity>> {
	let statuses = parse_query_values::<KomgaSeriesStatus>(&legacy.status, "status")?;
	if !statuses.is_empty() {
		let statuses = statuses
			.into_iter()
			.map(series_status_name)
			.collect::<Vec<_>>();
		query = query.filter(series_metadata::Column::Status.is_in(statuses));
	}

	let genres = csv_values(&legacy.genre);
	if !genres.is_empty() {
		let mut condition = Condition::any();
		for genre in genres {
			condition = condition.add(series_metadata::Column::Genres.contains(genre));
		}
		query = query.filter(condition);
	}

	let publishers = csv_values(&legacy.publisher);
	if !publishers.is_empty() {
		let mut condition = Condition::any();
		for publisher in publishers {
			condition = condition.add(series_metadata::Column::Publisher.eq(publisher));
		}
		query = query.filter(condition);
	}

	Ok(query)
}

fn visible_media_ids_subquery(
	user: &AuthUser,
	library_ids: Option<&[String]>,
	series_id: Option<&str>,
	read_list_ids: Option<&[String]>,
) -> SelectStatement {
	let series_ids = series_id.map(|series_id| vec![series_id.to_owned()]);
	visible_media_ids_for_series_ids_subquery(
		user,
		library_ids,
		series_ids.as_deref(),
		read_list_ids,
	)
}

fn visible_media_ids_for_series_ids_subquery(
	user: &AuthUser,
	library_ids: Option<&[String]>,
	series_ids: Option<&[String]>,
	read_list_ids: Option<&[String]>,
) -> SelectStatement {
	let mut query = media::Entity::find_for_user(user)
		.filter(media::Column::DeletedAt.is_null())
		.filter(
			media::Column::Extension.is_in(SUPPORTED_MEDIA_EXTENSIONS.iter().copied()),
		)
		.select_only()
		.column(media::Column::Id);
	if let Some(library_ids) = library_ids.filter(|ids| !ids.is_empty()) {
		query = query.filter(series::Column::LibraryId.is_in(library_ids.to_vec()));
	}
	if let Some(series_ids) = series_ids.filter(|ids| !ids.is_empty()) {
		query = query.filter(media::Column::SeriesId.is_in(series_ids.to_vec()));
	}
	if let Some(read_list_ids) = read_list_ids.filter(|ids| !ids.is_empty()) {
		let visible_lists =
			reading_list::Entity::find_for_user(user, READING_LIST_READER_ROLE)
				.filter(reading_list::Column::Id.is_in(read_list_ids.to_vec()))
				.select_only()
				.column(reading_list::Column::Id)
				.into_query();
		let list_media = reading_list_item::Entity::find()
			.filter(reading_list_item::Column::ReadingListId.in_subquery(visible_lists))
			.select_only()
			.column(reading_list_item::Column::MediaId)
			.into_query();
		query = query.filter(media::Column::Id.in_subquery(list_media));
	}
	query.into_query()
}

fn visible_series_ids_subquery(
	user: &AuthUser,
	library_ids: Option<&[String]>,
) -> SelectStatement {
	visible_series_ids_for_series_ids_subquery(user, library_ids, None)
}

fn visible_series_ids_for_series_ids_subquery(
	user: &AuthUser,
	library_ids: Option<&[String]>,
	series_ids: Option<&[String]>,
) -> SelectStatement {
	let supported_series = media::Entity::find()
		.filter(
			media::Column::Extension.is_in(SUPPORTED_MEDIA_EXTENSIONS.iter().copied()),
		)
		.filter(media::Column::SeriesId.is_not_null())
		.select_only()
		.column(media::Column::SeriesId)
		.into_query();
	let mut query = series::Entity::find_for_user(user)
		.filter(series::Column::Id.in_subquery(supported_series))
		.select_only()
		.column(series::Column::Id);
	if let Some(library_ids) = library_ids.filter(|ids| !ids.is_empty()) {
		query = query.filter(series::Column::LibraryId.is_in(library_ids.to_vec()));
	}
	if let Some(series_ids) = series_ids.filter(|ids| !ids.is_empty()) {
		query = query.filter(series::Column::Id.is_in(series_ids.to_vec()));
	}
	query.into_query()
}

fn visible_series_ids_for_media_series_ids_subquery(
	user: &AuthUser,
	library_ids: Option<&[String]>,
	series_ids: Option<&[String]>,
	read_list_ids: Option<&[String]>,
) -> SelectStatement {
	let visible_media = visible_media_ids_for_series_ids_subquery(
		user,
		library_ids,
		series_ids,
		read_list_ids,
	);
	media::Entity::find()
		.select_only()
		.column(media::Column::SeriesId)
		.filter(media::Column::Id.in_subquery(visible_media))
		.filter(media::Column::SeriesId.is_not_null())
		.into_query()
}

/// Media ids the user has a reading head for, optionally narrowed to
/// completed (`Some(true)`) or in-progress (`Some(false)`) heads.
fn visible_head_media_ids_subquery(
	user: &AuthUser,
	completed: Option<bool>,
) -> SelectStatement {
	let mut query = reading_head::Entity::find_for_user(&user.id)
		.select_only()
		.column(reading_head::Column::MediaId)
		.filter(
			reading_head::Column::MediaId
				.in_subquery(visible_media_ids_subquery(user, None, None, None)),
		);
	if let Some(completed) = completed {
		query = query.filter(reading_head::Column::Completed.eq(completed));
	}
	query.into_query()
}

fn release_date_condition(duration: &std::time::Duration) -> APIResult<Condition> {
	let elapsed = chrono::Duration::from_std(*duration).map_err(|_| {
		APIError::BadRequest("release date duration is too large".to_owned())
	})?;
	let cutoff = Utc::now()
		.checked_sub_signed(elapsed)
		.ok_or_else(|| {
			APIError::BadRequest("release date duration is too large".to_owned())
		})?
		.date_naive();
	let cutoff =
		cutoff.year() * 10_000 + cutoff.month() as i32 * 100 + cutoff.day() as i32;
	let release_date = Expr::col((media_metadata::Entity, media_metadata::Column::Year))
		.mul(10_000)
		.add(Expr::col((media_metadata::Entity, media_metadata::Column::Month)).mul(100))
		.add(Expr::col((
			media_metadata::Entity,
			media_metadata::Column::Day,
		)));
	Ok(Condition::all()
		.add(
			Expr::col((media_metadata::Entity, media_metadata::Column::Year))
				.is_not_null(),
		)
		.add(release_date.binary(
			sea_orm::sea_query::BinOper::GreaterThanOrEqual,
			Expr::val(cutoff),
		)))
}

fn series_ids_for_read_status(
	user: &AuthUser,
	status: KomgaReadStatus,
) -> SelectStatement {
	let mut media_query = media::Entity::find()
		.select_only()
		.column(media::Column::SeriesId)
		.filter(media::Column::SeriesId.is_not_null())
		.filter(
			media::Column::Id
				.in_subquery(visible_media_ids_subquery(user, None, None, None)),
		);
	match status {
		KomgaReadStatus::Read => {
			media_query = media_query.filter(
				media::Column::Id
					.in_subquery(visible_head_media_ids_subquery(user, Some(true))),
			);
		},
		KomgaReadStatus::InProgress => {
			media_query = media_query.filter(
				media::Column::Id
					.in_subquery(visible_head_media_ids_subquery(user, Some(false))),
			);
		},
		KomgaReadStatus::Unread => {
			media_query = media_query.filter(
				media::Column::Id
					.not_in_subquery(visible_head_media_ids_subquery(user, None)),
			);
		},
	}
	media_query.into_query()
}

fn book_condition_filter(
	condition: &BookCondition,
	user: &AuthUser,
) -> APIResult<Option<Condition>> {
	match condition {
		BookCondition::AllOfBook { conditions } => {
			if conditions.is_empty() {
				return Ok(None);
			}
			let mut result = Condition::all();
			for condition in conditions {
				if let Some(filter) = book_condition_filter(condition, user)? {
					result = result.add(filter);
				}
			}
			Ok(Some(result))
		},
		BookCondition::AnyOfBook { conditions } => {
			if conditions.is_empty() {
				return Ok(Some(Condition::any()));
			}
			let mut result = Condition::any();
			for condition in conditions {
				match book_condition_filter(condition, user)? {
					Some(filter) => result = result.add(filter),
					None => return Ok(None),
				}
			}
			Ok(Some(result))
		},
		BookCondition::LibraryId {
			operator: Equality::Is { value },
		} => Ok(Some(
			Condition::all().add(series::Column::LibraryId.eq(value.0.clone())),
		)),
		BookCondition::SeriesId {
			operator: Equality::Is { value },
		} => Ok(Some(
			Condition::all().add(media::Column::SeriesId.eq(value.0.clone())),
		)),
		BookCondition::Deleted {
			operator: BooleanOperator::IsFalse,
		} => Ok(Some(
			Condition::all().add(media::Column::DeletedAt.is_null()),
		)),
		BookCondition::Deleted {
			operator: BooleanOperator::IsTrue,
		} => Ok(Some(
			Condition::all().add(media::Column::DeletedAt.is_not_null()),
		)),
		BookCondition::ReadStatus {
			operator: Equality::Is {
				value: KomgaReadStatus::Read,
			},
		} => Ok(Some(Condition::all().add(media::Column::Id.in_subquery(
			visible_head_media_ids_subquery(user, Some(true)),
		)))),
		BookCondition::ReadStatus {
			operator: Equality::Is {
				value: KomgaReadStatus::InProgress,
			},
		} => Ok(Some(Condition::all().add(media::Column::Id.in_subquery(
			visible_head_media_ids_subquery(user, Some(false)),
		)))),
		BookCondition::ReadStatus {
			operator: Equality::Is {
				value: KomgaReadStatus::Unread,
			},
		} => Ok(Some(
			Condition::all().add(
				media::Column::Id
					.not_in_subquery(visible_head_media_ids_subquery(user, None)),
			),
		)),
		BookCondition::Author {
			operator: Equality::Is { value },
		} => {
			let role = value.role.as_deref().map(str::to_ascii_lowercase);
			let columns = match role.as_deref() {
				None => vec![
					media_metadata::Column::Writers,
					media_metadata::Column::Pencillers,
					media_metadata::Column::Inkers,
					media_metadata::Column::Colorists,
					media_metadata::Column::Letterers,
					media_metadata::Column::CoverArtists,
					media_metadata::Column::Editors,
				],
				Some("writer") => vec![media_metadata::Column::Writers],
				Some("penciller") => vec![media_metadata::Column::Pencillers],
				Some("inker") => vec![media_metadata::Column::Inkers],
				Some("colorist") => vec![media_metadata::Column::Colorists],
				Some("letterer") => vec![media_metadata::Column::Letterers],
				Some("cover") | Some("cover_artist") => {
					vec![media_metadata::Column::CoverArtists]
				},
				Some("editor") => vec![media_metadata::Column::Editors],
				Some(_) => return Err(unsupported_filter("author role")),
			};
			let mut authors = Condition::any();
			for column in columns {
				authors = authors.add(match value.name.as_deref() {
					Some(name) => column.contains(name),
					None => column.is_not_null(),
				});
			}
			Ok(Some(authors))
		},
		BookCondition::Tag {
			operator: EqualityNullable::Is { value },
		} => {
			let matching_tags = tag::Entity::find()
				.select_only()
				.column(tag::Column::Id)
				.filter(tag::Column::Name.eq(value.clone()))
				.into_query();
			let matching_media = media_tag::Entity::find()
				.select_only()
				.column(media_tag::Column::MediaId)
				.filter(media_tag::Column::TagId.in_subquery(matching_tags))
				.into_query();
			Ok(Some(
				Condition::all().add(media::Column::Id.in_subquery(matching_media)),
			))
		},
		BookCondition::Tag {
			operator: EqualityNullable::IsNot { value },
		} => {
			let matching_tags = tag::Entity::find()
				.select_only()
				.column(tag::Column::Id)
				.filter(tag::Column::Name.eq(value.clone()))
				.into_query();
			let matching_media = media_tag::Entity::find()
				.select_only()
				.column(media_tag::Column::MediaId)
				.filter(media_tag::Column::TagId.in_subquery(matching_tags))
				.into_query();
			Ok(Some(
				Condition::all().add(media::Column::Id.not_in_subquery(matching_media)),
			))
		},
		BookCondition::Tag {
			operator: EqualityNullable::IsNull,
		} => Ok(Some(
			Condition::all().add(
				media::Column::Id.not_in_subquery(
					media_tag::Entity::find()
						.select_only()
						.column(media_tag::Column::MediaId)
						.into_query(),
				),
			),
		)),
		BookCondition::Tag {
			operator: EqualityNullable::IsNotNull,
		} => Ok(Some(
			Condition::all().add(
				media::Column::Id.in_subquery(
					media_tag::Entity::find()
						.select_only()
						.column(media_tag::Column::MediaId)
						.into_query(),
				),
			),
		)),
		BookCondition::MediaStatus {
			operator: Equality::Is { value },
		} => match value {
			KomgaMediaStatus::Ready => Ok(Some(
				Condition::all().add(media::Column::Status.eq(FileStatus::Ready)),
			)),
			KomgaMediaStatus::Unknown => Ok(Some(
				Condition::any()
					.add(media::Column::Status.eq(FileStatus::Unknown))
					.add(media::Column::Status.eq(FileStatus::Missing)),
			)),
			KomgaMediaStatus::Error => Ok(Some(
				Condition::all().add(media::Column::Status.eq(FileStatus::Error)),
			)),
			KomgaMediaStatus::Unsupported => Ok(Some(
				Condition::all().add(media::Column::Status.eq(FileStatus::Unsupported)),
			)),
			// Stump has no source status corresponding to Komga's OUTDATED value.
			KomgaMediaStatus::Outdated => {
				Ok(Some(Condition::all().add(Expr::val(false).eq(true))))
			},
		},
		BookCondition::MediaProfile {
			operator: Equality::Is { value },
		} => Ok(Some(
			Condition::all().add(
				Expr::expr(Func::lower(Expr::col((
					media::Entity,
					media::Column::Extension,
				))))
				.is_in(extensions_for_media_profile(*value).iter().copied()),
			),
		)),
		BookCondition::ReleaseDate {
			operator: DateOperator::IsInTheLast { duration },
		} => Ok(Some(release_date_condition(duration)?)),
		other => {
			tracing::warn!(
				condition = %serde_json::to_string(other).unwrap_or_default(),
				"Rejecting unsupported Komga book condition"
			);
			Err(unsupported_filter("book condition"))
		},
	}
}

fn series_condition_filter(
	condition: &SeriesCondition,
	user: &AuthUser,
) -> APIResult<Option<Condition>> {
	match condition {
		SeriesCondition::AllOfSeries { conditions } => {
			if conditions.is_empty() {
				return Ok(None);
			}
			let mut result = Condition::all();
			for condition in conditions {
				if let Some(filter) = series_condition_filter(condition, user)? {
					result = result.add(filter);
				}
			}
			Ok(Some(result))
		},
		SeriesCondition::AnyOfSeries { conditions } => {
			if conditions.is_empty() {
				return Ok(Some(Condition::any()));
			}
			let mut result = Condition::any();
			for condition in conditions {
				match series_condition_filter(condition, user)? {
					Some(filter) => result = result.add(filter),
					None => return Ok(None),
				}
			}
			Ok(Some(result))
		},
		SeriesCondition::LibraryId {
			operator: Equality::Is { value },
		} => Ok(Some(
			Condition::all().add(series::Column::LibraryId.eq(value.0.clone())),
		)),
		SeriesCondition::Deleted {
			operator: BooleanOperator::IsFalse,
		} => Ok(Some(
			Condition::all().add(series::Column::DeletedAt.is_null()),
		)),
		SeriesCondition::Deleted {
			operator: BooleanOperator::IsTrue,
		} => Ok(Some(
			Condition::all().add(series::Column::DeletedAt.is_not_null()),
		)),
		SeriesCondition::ReadStatus {
			operator: Equality::Is { value },
		} => Ok(Some(Condition::all().add(
			series::Column::Id.in_subquery(series_ids_for_read_status(user, *value)),
		))),
		SeriesCondition::SeriesStatus {
			operator: Equality::Is { value },
		} => Ok(Some(Condition::all().add(
			series_metadata::Column::Status.eq(series_status_name(*value)),
		))),
		SeriesCondition::Genre {
			operator: EqualityNullable::Is { value },
		} => Ok(Some(
			Condition::all().add(series_metadata::Column::Genres.contains(value)),
		)),
		SeriesCondition::Publisher {
			operator: Equality::Is { value },
		} => Ok(Some(
			Condition::all().add(series_metadata::Column::Publisher.eq(value.clone())),
		)),
		SeriesCondition::Tag {
			operator: EqualityNullable::Is { value },
		} => {
			let matching_tags = tag::Entity::find()
				.select_only()
				.column(tag::Column::Id)
				.filter(tag::Column::Name.eq(value.clone()))
				.into_query();
			let matching_series = series_tag::Entity::find()
				.select_only()
				.column(series_tag::Column::SeriesId)
				.filter(series_tag::Column::TagId.in_subquery(matching_tags))
				.into_query();
			Ok(Some(
				Condition::all().add(series::Column::Id.in_subquery(matching_series)),
			))
		},
		SeriesCondition::Author {
			operator: Equality::Is { value },
		} => {
			let role = value.role.as_deref().map(str::to_ascii_lowercase);
			if role.as_deref().is_some_and(|role| role != "writer") {
				return Ok(Some(Condition::all().add(Expr::val(false).eq(true))));
			}
			let condition = match value.name.as_deref() {
				Some(name) => series_metadata::Column::Writers.contains(name),
				None => series_metadata::Column::Writers.is_not_null(),
			};
			Ok(Some(Condition::all().add(condition)))
		},
		SeriesCondition::Language {
			operator: Equality::Is { value },
		} => {
			let matching_media = media_metadata::Entity::find()
				.select_only()
				.column(media_metadata::Column::MediaId)
				.filter(
					Expr::expr(Func::lower(Expr::col(media_metadata::Column::Language)))
						.eq(value.to_ascii_lowercase()),
				)
				.filter(
					media_metadata::Column::MediaId
						.in_subquery(visible_media_ids_subquery(user, None, None, None)),
				)
				.into_query();
			let matching_series = media::Entity::find()
				.select_only()
				.column(media::Column::SeriesId)
				.filter(media::Column::Id.in_subquery(matching_media))
				.filter(media::Column::SeriesId.is_not_null())
				.into_query();
			Ok(Some(
				Condition::all().add(series::Column::Id.in_subquery(matching_series)),
			))
		},
		other => {
			tracing::warn!(
				condition = %serde_json::to_string(other).unwrap_or_default(),
				"Rejecting unsupported Komga series condition"
			);
			Err(unsupported_filter("series condition"))
		},
	}
}

fn apply_book_search(
	mut query: Select<media::Entity>,
	search: &crate::KomgaBookSearch,
	user: &AuthUser,
) -> APIResult<Select<media::Entity>> {
	if let Some(term) = search
		.full_text_search
		.as_deref()
		.map(str::trim)
		.filter(|term| !term.is_empty())
	{
		let term = term.to_owned();
		query = query.filter(
			Condition::any()
				.add(media::Column::Name.contains(term.clone()))
				.add(media_metadata::Column::Title.contains(term.clone()))
				.add(media_metadata::Column::Summary.contains(term.clone()))
				.add(media_metadata::Column::Writers.contains(term.clone()))
				.add(series::Column::Name.contains(term.clone()))
				.add(series_metadata::Column::Title.contains(term.clone()))
				.add(series_metadata::Column::Summary.contains(term)),
		);
	}
	if let Some(condition) = search.condition.as_ref() {
		if let Some(filter) = book_condition_filter(condition, user)? {
			query = query.filter(filter);
		}
	}
	Ok(query)
}

fn apply_series_search(
	mut query: Select<series::Entity>,
	search: &crate::KomgaSeriesSearch,
	user: &AuthUser,
) -> APIResult<Select<series::Entity>> {
	if let Some(term) = search
		.full_text_search
		.as_deref()
		.map(str::trim)
		.filter(|term| !term.is_empty())
	{
		let term = term.to_owned();
		query = query.filter(
			Condition::any()
				.add(series::Column::Name.contains(term.clone()))
				.add(series_metadata::Column::Title.contains(term.clone()))
				.add(series_metadata::Column::Summary.contains(term.clone()))
				.add(series_metadata::Column::Writers.contains(term)),
		);
	}
	if let Some(condition) = search.condition.as_ref() {
		if let Some(filter) = series_condition_filter(condition, user)? {
			query = query.filter(filter);
		}
	}
	Ok(query)
}

fn default_book_order(query: Select<media::Entity>) -> Select<media::Entity> {
	query
		.order_by_asc(media::Column::Name)
		.order_by_asc(media::Column::Id)
}

fn default_series_order(query: Select<series::Entity>) -> Select<series::Entity> {
	query
		.order_by_asc(series::Column::Name)
		.order_by_asc(series::Column::Id)
}

fn default_series_new_order(query: Select<series::Entity>) -> Select<series::Entity> {
	query
		.order_by_desc(series::Column::CreatedAt)
		.order_by_asc(series::Column::Id)
}

fn default_series_updated_order(query: Select<series::Entity>) -> Select<series::Entity> {
	query
		.order_by_desc(series::Column::UpdatedAt)
		.order_by_asc(series::Column::Id)
}

fn release_date_sort_expr() -> SimpleExpr {
	Expr::col((media_metadata::Entity, media_metadata::Column::Year))
		.mul(10_000)
		.add(Expr::col((media_metadata::Entity, media_metadata::Column::Month)).mul(100))
		.add(Expr::col((
			media_metadata::Entity,
			media_metadata::Column::Day,
		)))
}

fn latest_read_date_sort_expr(user: &AuthUser) -> SimpleExpr {
	let latest_read_date = reading_head::Entity::find_for_user(&user.id)
		.select_only()
		.column(reading_head::Column::UpdatedAt)
		.filter(
			Expr::col((reading_head::Entity, reading_head::Column::MediaId))
				.equals((media::Entity, media::Column::Id)),
		)
		.into_query();
	SimpleExpr::SubQuery(None, Box::new(latest_read_date.into_sub_query_statement()))
}

fn book_sort_supported(field: &str) -> bool {
	matches!(
		field,
		"name"
			| "lastmodified"
			| "createddate"
			| "metadata.titlesort"
			| "metadata.numbersort"
			| "metadata.releasedate"
			| "readprogress.readdate"
			| "relevance"
			| "random"
	)
}

fn series_sort_supported(field: &str) -> bool {
	matches!(
		field,
		"name"
			| "lastmodified"
			| "createddate"
			| "metadata.titlesort"
			| "relevance"
			| "random"
	)
}

fn first_unsupported_sort(
	sorts: &[SortSpec],
	supported: fn(&str) -> bool,
) -> Option<&SortSpec> {
	sorts.iter().find(|sort| !supported(sort.field.as_str()))
}

fn apply_book_order(
	mut query: Select<media::Entity>,
	sorts: &[SortSpec],
	user: &AuthUser,
) -> Select<media::Entity> {
	if sorts.is_empty() {
		return default_book_order(query);
	}
	if let Some(sort) = first_unsupported_sort(sorts, book_sort_supported) {
		tracing::warn!(
			sort = %sort.field,
			"Unsupported Komga book sort; using the endpoint default ordering"
		);
		return default_book_order(query);
	}
	for sort in sorts {
		let order = if sort.direction == SortDirection::Desc {
			Order::Desc
		} else {
			Order::Asc
		};
		query = match sort.field.as_str() {
			"name" | "relevance" => query.order_by(media::Column::Name, order),
			"random" => query.order_by(Expr::cust("RANDOM()"), order),
			"lastmodified" => query.order_by(
				Expr::expr(Func::coalesce([
					Expr::col((media::Entity, media::Column::UpdatedAt)).into(),
					Expr::col((media::Entity, media::Column::CreatedAt)).into(),
				])),
				order,
			),
			"createddate" => query.order_by(media::Column::CreatedAt, order),
			"metadata.titlesort" => query.order_by(
				Expr::expr(Func::coalesce([
					Expr::col((media_metadata::Entity, media_metadata::Column::Title))
						.into(),
					Expr::col((media::Entity, media::Column::Name)).into(),
				])),
				order,
			),
			"metadata.numbersort" => {
				query.order_by(media_metadata::Column::Number, order)
			},
			"metadata.releasedate" => query.order_by(release_date_sort_expr(), order),
			"readprogress.readdate" => query.order_by_with_nulls(
				latest_read_date_sort_expr(user),
				order,
				NullOrdering::Last,
			),
			_ => unreachable!("unsupported book sort passed validation"),
		};
	}
	query.order_by_asc(media::Column::Id)
}

fn apply_series_order(
	mut query: Select<series::Entity>,
	sorts: &[SortSpec],
) -> Select<series::Entity> {
	if sorts.is_empty() {
		return default_series_order(query);
	}
	if let Some(sort) = first_unsupported_sort(sorts, series_sort_supported) {
		tracing::warn!(
			sort = %sort.field,
			"Unsupported Komga series sort; using the endpoint default ordering"
		);
		return default_series_order(query);
	}
	for sort in sorts {
		let order = if sort.direction == SortDirection::Desc {
			Order::Desc
		} else {
			Order::Asc
		};
		query = match sort.field.as_str() {
			"name" | "relevance" => query.order_by(series::Column::Name, order),
			"random" => query.order_by(Expr::cust("RANDOM()"), order),
			"lastmodified" => query.order_by(
				Expr::expr(Func::coalesce([
					Expr::col((series::Entity, series::Column::UpdatedAt)).into(),
					Expr::col((series::Entity, series::Column::CreatedAt)).into(),
				])),
				order,
			),
			"createddate" => query.order_by(series::Column::CreatedAt, order),
			"metadata.titlesort" => query.order_by(
				Expr::expr(Func::coalesce([
					Expr::col((
						series_metadata::Entity,
						series_metadata::Column::TitleSort,
					))
					.into(),
					Expr::col((series::Entity, series::Column::Name)).into(),
				])),
				order,
			),
			_ => unreachable!("unsupported series sort passed validation"),
		};
	}
	query.order_by_asc(series::Column::Id)
}

fn apply_series_feed_order(
	query: Select<series::Entity>,
	sorts: &[SortSpec],
	default_order: fn(Select<series::Entity>) -> Select<series::Entity>,
	endpoint: &'static str,
) -> Select<series::Entity> {
	if sorts.is_empty() {
		return default_order(query);
	}
	if let Some(sort) = first_unsupported_sort(sorts, series_sort_supported) {
		tracing::warn!(
			sort = %sort.field,
			endpoint,
			"Unsupported Komga feed sort; using the endpoint default ordering"
		);
		return default_order(query);
	}
	apply_series_order(query, sorts)
}

async fn get_libraries(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	let rows = library::Entity::find_for_user(&user)
		.order_by_asc(library::Column::Name)
		.find_also_related(library_config::Entity)
		.all(ctx.conn())
		.await?;
	let libraries: Vec<KomgaLibrary> = rows
		.into_iter()
		.map(|(library, config)| map_library(library, config, &user))
		.collect();
	cached_json(&headers, &libraries)
}

async fn get_library(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	let Some((library, config)) = library::Entity::find_for_user(&user)
		.filter(library::Column::Id.eq(id))
		.find_also_related(library_config::Entity)
		.one(ctx.conn())
		.await?
	else {
		return Err(APIError::NotFound("Library not found".to_owned()));
	};
	cached_json(&headers, &map_library(library, config, &user))
}

#[allow(clippy::too_many_arguments)] // Keeps legacy and JSON list handlers on one query path.
async fn list_books_page(
	ctx: &dyn KomgaBackend,
	user: &AuthUser,
	pagination: Pagination,
	sorts: &[SortSpec],
	search: &KomgaBookSearch,
	deleted: bool,
	legacy: Option<&LegacyCatalogQuery>,
	order_by_number: bool,
) -> APIResult<Page<KomgaBook>> {
	let mut query = media::ModelWithMetadata::find_for_user(user);
	query = query.filter(
		media::Column::Extension.is_in(SUPPORTED_MEDIA_EXTENSIONS.iter().copied()),
	);
	query = query.filter(if deleted {
		media::Column::DeletedAt.is_not_null()
	} else {
		media::Column::DeletedAt.is_null()
	});
	if !deleted {
		// A book whose file the scanner could not find is not browsable: Komga
		// drops such a book out of its lists rather than serving an entry that
		// 404s on read, so `MISSING` maps onto that hidden state.
		query = query.filter(media::Column::Status.ne(FileStatus::Missing));
	}
	query = apply_book_search(query, search, user)?;
	if let Some(legacy) = legacy {
		query = apply_legacy_book_metadata_filters(query, legacy)?;
	}
	query = if order_by_number {
		query
			.order_by_asc(media_metadata::Column::Number)
			.order_by_asc(media::Column::Id)
	} else {
		apply_book_order(query, sorts, user)
	};
	let total = total_as_i32(query.clone().count(ctx.conn()).await?)?;
	let query = if pagination.unpaged {
		query
	} else {
		query
			.offset(pagination.offset())
			.limit(pagination.size as u64)
	};
	let models = query
		.into_model::<media::ModelWithMetadata>()
		.all(ctx.conn())
		.await?;
	let books = map_books(ctx.conn(), user, models).await?;
	Ok(Page::new(
		books,
		pagination.page,
		pagination.size,
		total,
		pagination.unpaged,
	))
}

async fn list_series_page(
	ctx: &dyn KomgaBackend,
	user: &AuthUser,
	pagination: Pagination,
	sorts: &[SortSpec],
	search: &KomgaSeriesSearch,
	deleted: bool,
) -> APIResult<Page<KomgaSeries>> {
	let mut query = series::ModelWithMetadata::find_for_user(user);
	query = query
		.filter(series::Column::Id.in_subquery(visible_series_ids_subquery(user, None)));
	query = query.filter(if deleted {
		series::Column::DeletedAt.is_not_null()
	} else {
		series::Column::DeletedAt.is_null()
	});
	query = apply_series_search(query, search, user)?;
	query = apply_series_order(query, sorts);
	let total = total_as_i32(query.clone().count(ctx.conn()).await?)?;
	let query = if pagination.unpaged {
		query
	} else {
		query
			.offset(pagination.offset())
			.limit(pagination.size as u64)
	};
	let models = query
		.into_model::<series::ModelWithMetadata>()
		.all(ctx.conn())
		.await?;
	let series = map_series(ctx.conn(), user, models).await?;
	Ok(Page::new(
		series,
		pagination.page,
		pagination.size,
		total,
		pagination.unpaged,
	))
}

/// Mode B: when a series-list search is pinned to a virtual library, browse
/// the backing source live. Returns `None` when the search does not target
/// a single virtual library, letting the caller fall through to the
/// database.
async fn virtual_series_page(
	ctx: &dyn KomgaBackend,
	user: &AuthUser,
	search: &KomgaSeriesSearch,
	pagination: &Pagination,
	sorts: &[SortSpec],
) -> Option<APIResult<Page<KomgaSeries>>> {
	let library_id = virtual_library_id_from_search(ctx, search).await?;
	// The user must be allowed to see the library, exactly as for stored rows.
	match library::Entity::find_for_user(user)
		.filter(library::Column::Id.eq(library_id.clone()))
		.one(ctx.conn())
		.await
	{
		Ok(Some(_)) => {},
		Ok(None) => return None,
		Err(error) => return Some(Err(error.into())),
	}
	let sort_fields: Vec<String> = sorts.iter().map(|sort| sort.field.clone()).collect();
	ctx.virtual_series_list(
		user,
		library_id,
		search,
		&sort_fields,
		pagination.page,
		pagination.size,
		pagination.unpaged,
	)
	.await
}

/// The virtual library a search is pinned to, if any. Only a lone
/// `libraryId is` condition (at any nesting depth) routes to live browse;
/// other conditions are answered from the database, which simply has no
/// virtual rows.
async fn virtual_library_id_from_search(
	ctx: &dyn KomgaBackend,
	search: &KomgaSeriesSearch,
) -> Option<String> {
	let condition = search.condition.as_ref()?;
	let mut library_ids = Vec::new();
	collect_library_ids(condition, &mut library_ids);
	let [library_id] = library_ids.as_slice() else {
		return None;
	};
	ctx.virtual_library_source(library_id).await?;
	Some(library_id.clone())
}

fn collect_library_ids(condition: &SeriesCondition, out: &mut Vec<String>) {
	match condition {
		SeriesCondition::LibraryId {
			operator: crate::Equality::Is { value },
		} => out.push(value.0.clone()),
		SeriesCondition::AnyOfSeries { conditions }
		| SeriesCondition::AllOfSeries { conditions } => {
			for condition in conditions {
				collect_library_ids(condition, out);
			}
		},
		_ => {},
	}
}

async fn get_books(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(page_query): Query<PaginationQuery>,
	headers: HeaderMap,
	Json(search): Json<crate::KomgaBookSearch>,
) -> APIResult<Response<Body>> {
	let (pagination, sorts) = page_query.validate()?;
	let user = auth.user();
	let page = list_books_page(
		ctx.as_ref(),
		&user,
		pagination,
		&sorts,
		&search,
		false,
		None,
		false,
	)
	.await?;
	cached_json(&headers, &page)
}

async fn get_books_legacy(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<LegacyCatalogQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let (pagination, sorts) = query.pagination()?;
	let search = query.book_search(None)?;
	let user = auth.user();
	let page = list_books_page(
		ctx.as_ref(),
		&user,
		pagination,
		&sorts,
		&search,
		query.deleted,
		Some(&query),
		false,
	)
	.await?;
	cached_json(&headers, &page)
}

async fn get_series(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(page_query): Query<PaginationQuery>,
	headers: HeaderMap,
	Json(search): Json<crate::KomgaSeriesSearch>,
) -> APIResult<Response<Body>> {
	let (pagination, sorts) = page_query.validate()?;
	let user = auth.user();
	// Mode B: a search pinned to a virtual library browses the source live.
	if let Some(page) =
		virtual_series_page(ctx.as_ref(), &user, &search, &pagination, &sorts).await
	{
		return cached_json(&headers, &page?);
	}
	let page =
		list_series_page(ctx.as_ref(), &user, pagination, &sorts, &search, false).await?;
	cached_json(&headers, &page)
}

async fn get_series_legacy(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<LegacyCatalogQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let (pagination, sorts) = query.pagination()?;
	let search = query.series_search()?;
	let user = auth.user();
	let page = list_series_page(
		ctx.as_ref(),
		&user,
		pagination,
		&sorts,
		&search,
		query.deleted,
	)
	.await?;
	cached_json(&headers, &page)
}

async fn get_series_books(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	Query(query): Query<LegacyCatalogQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let (pagination, sorts) = query.pagination()?;
	let user = auth.user();
	let mut series_query =
		series::Entity::find_for_user(&user).filter(series::Column::Id.eq(id.clone()));
	series_query = series_query.filter(if query.deleted {
		series::Column::DeletedAt.is_not_null()
	} else {
		series::Column::DeletedAt.is_null()
	});
	if series_query.one(ctx.conn()).await?.is_none() {
		// Mode B: the series may be live-only; materialise it, then fall
		// through to the ordinary database path.
		if !ctx.virtual_materialise_series(&user, &id).await? {
			return Err(APIError::NotFound("Series not found".to_owned()));
		}
	}
	let search = query.book_search(Some(&id))?;
	let page = list_books_page(
		ctx.as_ref(),
		&user,
		pagination,
		&sorts,
		&search,
		query.deleted,
		Some(&query),
		true,
	)
	.await?;
	cached_json(&headers, &page)
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeedQuery {
	page: i32,
	size: i32,
	unpaged: bool,
	sort: Vec<String>,
	#[serde(rename = "libraryId", alias = "library_id")]
	library_id: Vec<String>,
	#[serde(default)]
	deleted: Option<bool>,
	#[serde(rename = "oneShot", alias = "oneshot", alias = "one_shot")]
	oneshot: Option<bool>,
}
impl Default for FeedQuery {
	fn default() -> Self {
		Self {
			page: 0,
			size: DEFAULT_PAGE_SIZE,
			unpaged: false,
			sort: Vec::new(),
			library_id: Vec::new(),
			deleted: None,
			oneshot: None,
		}
	}
}

fn feed_pagination(query: &FeedQuery) -> APIResult<(Pagination, Vec<SortSpec>)> {
	PaginationQuery {
		page: query.page,
		size: query.size,
		unpaged: query.unpaged,
		sort: query.sort.clone(),
	}
	.validate()
}

/// Matches the mapper's `is_one_shot`: a series is a one-shot when its metadata
/// booktype, lowercased with hyphens removed, equals `oneshot`.
fn one_shot_condition(is_one_shot: bool) -> Condition {
	let normalized =
		Expr::expr(Func::lower(
			Func::cust(SqlReplace).args([
				Expr::col((series_metadata::Entity, series_metadata::Column::Booktype))
					.into(),
				Expr::val("-").into(),
				Expr::val("").into(),
			]),
		));
	if is_one_shot {
		Condition::all()
			.add(
				Expr::col((series_metadata::Entity, series_metadata::Column::Booktype))
					.is_not_null(),
			)
			.add(normalized.eq("oneshot"))
	} else {
		Condition::any()
			.add(
				Expr::col((series_metadata::Entity, series_metadata::Column::Booktype))
					.is_null(),
			)
			.add(normalized.ne("oneshot"))
	}
}

struct SqlReplace;

impl sea_orm::sea_query::Iden for SqlReplace {
	fn unquoted(&self, s: &mut dyn std::fmt::Write) {
		s.write_str("REPLACE").expect("infallible identifier write");
	}
}

async fn get_series_new(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(feed): Query<FeedQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let (pagination, sorts) = feed_pagination(&feed)?;
	let user = auth.user();
	let library_ids = csv_values(&feed.library_id);
	let mut query = series::ModelWithMetadata::find_for_user(&user);
	if !library_ids.is_empty() {
		query = query.filter(series::Column::LibraryId.is_in(library_ids));
	}
	if let Some(deleted) = feed.deleted {
		query = query.filter(if deleted {
			series::Column::DeletedAt.is_not_null()
		} else {
			series::Column::DeletedAt.is_null()
		});
	} else {
		query = query.filter(series::Column::DeletedAt.is_null());
	}
	if let Some(one_shot) = feed.oneshot {
		query = query.filter(one_shot_condition(one_shot));
	}
	let query =
		apply_series_feed_order(query, &sorts, default_series_new_order, "series/new");
	let total = total_as_i32(query.clone().count(ctx.conn()).await?)?;
	let query = (if pagination.unpaged {
		query
	} else {
		query
			.offset(pagination.offset())
			.limit(pagination.size as u64)
	})
	.into_model::<series::ModelWithMetadata>();
	let models = query.all(ctx.conn()).await?;
	let content = map_series(ctx.conn(), &user, models).await?;
	cached_json(
		&headers,
		&Page::new(
			content,
			pagination.page,
			pagination.size,
			total,
			pagination.unpaged,
		),
	)
}

async fn get_series_updated(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(feed): Query<FeedQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let (pagination, sorts) = feed_pagination(&feed)?;
	let user = auth.user();
	let library_ids = csv_values(&feed.library_id);
	let mut query = series::ModelWithMetadata::find_for_user(&user);
	if !library_ids.is_empty() {
		query = query.filter(series::Column::LibraryId.is_in(library_ids));
	}
	if let Some(deleted) = feed.deleted {
		query = query.filter(if deleted {
			series::Column::DeletedAt.is_not_null()
		} else {
			series::Column::DeletedAt.is_null()
		});
	} else {
		query = query.filter(series::Column::DeletedAt.is_null());
	}
	if let Some(one_shot) = feed.oneshot {
		query = query.filter(one_shot_condition(one_shot));
	}
	let query = apply_series_feed_order(
		query,
		&sorts,
		default_series_updated_order,
		"series/updated",
	);
	let total = total_as_i32(query.clone().count(ctx.conn()).await?)?;
	let query = (if pagination.unpaged {
		query
	} else {
		query
			.offset(pagination.offset())
			.limit(pagination.size as u64)
	})
	.into_model::<series::ModelWithMetadata>();
	let models = query.all(ctx.conn()).await?;
	let content = map_series(ctx.conn(), &user, models).await?;
	cached_json(
		&headers,
		&Page::new(
			content,
			pagination.page,
			pagination.size,
			total,
			pagination.unpaged,
		),
	)
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct OnDeckQuery {
	page: i32,
	size: i32,
	unpaged: bool,
	sort: Vec<String>,
	#[serde(rename = "libraryId", alias = "library_id")]
	library_id: Vec<String>,
}

impl Default for OnDeckQuery {
	fn default() -> Self {
		Self {
			page: 0,
			size: DEFAULT_PAGE_SIZE,
			unpaged: false,
			sort: Vec::new(),
			library_id: Vec::new(),
		}
	}
}

fn on_deck_order(sorts: &[SortSpec]) -> &'static str {
	const DEFAULT: &str = "series_last_read_date DESC, name ASC, id ASC";
	if sorts.is_empty() {
		return DEFAULT;
	}
	if let Some(sort) = sorts.iter().find(|sort| {
		!matches!(
			sort.field.as_str(),
			"created"
				| "createdat"
				| "created_at"
				| "lastmodified"
				| "lastmodifieddate"
				| "last_modified"
				| "name" | "title"
				| "metadata.title"
				| "relevance"
		)
	}) {
		tracing::warn!(
			sort = %sort.field,
			"Unsupported Komga on-deck sort; using the endpoint default ordering"
		);
		return DEFAULT;
	}
	match sorts[0].field.as_str() {
		"created" | "createdat" | "created_at" => {
			if sorts[0].direction == SortDirection::Desc {
				"created_at DESC, id ASC"
			} else {
				"created_at ASC, id ASC"
			}
		},
		"lastmodified" | "lastmodifieddate" | "last_modified" => {
			if sorts[0].direction == SortDirection::Desc {
				"updated_at DESC, id ASC"
			} else {
				"updated_at ASC, id ASC"
			}
		},
		"name" | "title" | "metadata.title" | "relevance" => {
			if sorts[0].direction == SortDirection::Desc {
				"name DESC, id ASC"
			} else {
				"name ASC, id ASC"
			}
		},
		_ => unreachable!("unsupported on-deck sort passed validation"),
	}
}

fn push_sql_value(values: &mut Vec<Value>, value: Value) -> String {
	values.push(value);
	format!("${}", values.len())
}

fn on_deck_cte(
	user: &AuthUser,
	library_ids: &[String],
	limit: Option<u64>,
	offset: Option<u64>,
	sort: &[SortSpec],
) -> (String, Vec<Value>) {
	let mut values: Vec<Value> = vec![user.id.clone().into()];
	let mut library_predicate = String::new();
	if !library_ids.is_empty() {
		let placeholders = library_ids
			.iter()
			.map(|id| push_sql_value(&mut values, id.clone().into()))
			.collect::<Vec<_>>();
		library_predicate = format!(" AND s.library_id IN ({})", placeholders.join(", "));
	}
	let age_condition = if let Some(restriction) = user.age_restriction.as_ref() {
		let placeholder = push_sql_value(&mut values, restriction.age.into());
		if restriction.restrict_on_unset {
			format!(
				"((mm.age_rating IS NOT NULL AND mm.age_rating <= {placeholder}) OR (mm.age_rating IS NULL AND sm.age_rating IS NOT NULL AND sm.age_rating <= {placeholder}))"
			)
		} else {
			format!(
				"(((mm.id IS NULL OR mm.age_rating IS NULL) AND (sm.series_id IS NULL OR sm.age_rating IS NULL OR sm.age_rating <= {placeholder})) OR (mm.id IS NOT NULL AND mm.age_rating IS NOT NULL AND mm.age_rating <= {placeholder}))"
			)
		}
	} else {
		"1 = 1".to_owned()
	};
	let limit_placeholder = limit.map(|limit| push_sql_value(&mut values, limit.into()));
	let offset_placeholder =
		offset.map(|offset| push_sql_value(&mut values, offset.into()));
	let order = on_deck_order(sort);
	let tail = match (limit_placeholder, offset_placeholder) {
		(Some(limit), Some(offset)) => {
			format!(" ORDER BY {order} LIMIT {limit} OFFSET {offset}")
		},
		_ => String::new(),
	};
	let sql = format!(
		r#"
WITH visible_media AS (
    SELECT m.id, m.name, m.series_id, m.created_at, m.updated_at
    FROM media m
    JOIN series s ON s.id = m.series_id
    LEFT JOIN media_metadata mm ON mm.media_id = m.id
    LEFT JOIN series_metadata sm ON sm.series_id = s.id
    WHERE m.deleted_at IS NULL
      AND s.deleted_at IS NULL
      AND s.library_id NOT IN (
          SELECT library_id FROM library_exclusions WHERE user_id = $1
      ){library_predicate}
      AND ({age_condition})
), user_read_series AS (
    SELECT DISTINCT vm.series_id
    FROM visible_media vm
    JOIN reading_heads rh ON rh.media_id = vm.id
    WHERE rh.user_id = $1 AND rh.completed = 1
), user_active_series AS (
    SELECT DISTINCT vm.series_id
    FROM visible_media vm
    JOIN reading_heads rh ON rh.media_id = vm.id
    WHERE rh.user_id = $1 AND rh.completed = 0
), user_read_media AS (
    SELECT DISTINCT rh.media_id
    FROM reading_heads rh
    WHERE rh.user_id = $1
), series_last_read AS (
    SELECT vm.series_id, MAX(rh.updated_at) AS series_last_read_date
    FROM visible_media vm
    JOIN reading_heads rh ON rh.media_id = vm.id
    WHERE rh.user_id = $1 AND rh.completed = 1
    GROUP BY vm.series_id
), next_in_series AS (
    SELECT vm.id, vm.name, vm.created_at, vm.updated_at,
           ROW_NUMBER() OVER (PARTITION BY vm.series_id ORDER BY vm.name, vm.id) AS book_rank,
           COALESCE(slr.series_last_read_date, '1970-01-01') AS series_last_read_date
    FROM visible_media vm
    LEFT JOIN series_last_read slr ON slr.series_id = vm.series_id
    WHERE vm.series_id IN (SELECT series_id FROM user_read_series)
      AND vm.series_id NOT IN (SELECT series_id FROM user_active_series)
      AND vm.id NOT IN (SELECT media_id FROM user_read_media)
)
SELECT id FROM next_in_series WHERE book_rank = 1{tail}
"#,
	);
	(sql, values)
}

async fn get_books_on_deck(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<OnDeckQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let pagination_query = PaginationQuery {
		page: query.page,
		size: query.size,
		unpaged: query.unpaged,
		sort: query.sort.clone(),
	};
	let (pagination, sorts) = pagination_query.validate()?;
	let user = auth.user();
	let library_ids = csv_values(&query.library_id);
	let (count_sql, count_values) = on_deck_cte(&user, &library_ids, None, None, &sorts);
	let count_sql = count_sql.replace(
		"SELECT id FROM next_in_series WHERE book_rank = 1",
		"SELECT COUNT(*) AS count FROM next_in_series WHERE book_rank = 1",
	);
	let count_row = ctx
		.conn()
		.query_one(Statement::from_sql_and_values(
			ctx.conn().get_database_backend(),
			count_sql,
			count_values,
		))
		.await?
		.ok_or_else(|| {
			APIError::InternalServerError("failed to count on-deck books".to_owned())
		})?;
	let total: i64 = count_row.try_get("", "count")?;
	let total = total_as_i32(u64::try_from(total).unwrap_or(0))?;
	let (sql, values) = on_deck_cte(
		&user,
		&library_ids,
		(!pagination.unpaged).then_some(pagination.size as u64),
		(!pagination.unpaged).then_some(pagination.offset()),
		&sorts,
	);
	#[derive(Debug, FromQueryResult)]
	struct OnDeckId {
		id: String,
	}
	let ids = OnDeckId::find_by_statement(Statement::from_sql_and_values(
		ctx.conn().get_database_backend(),
		sql,
		values,
	))
	.all(ctx.conn())
	.await?
	.into_iter()
	.map(|row| row.id)
	.collect::<Vec<_>>();
	let models = if ids.is_empty() {
		Vec::new()
	} else {
		let models = media::ModelWithMetadata::find_for_user(&user)
			.filter(media::Column::DeletedAt.is_null())
			.filter(media::Column::Id.is_in(ids.clone()))
			.into_model::<media::ModelWithMetadata>()
			.all(ctx.conn())
			.await?;
		let mut by_id = models
			.into_iter()
			.map(|model| (model.media.id.clone(), model))
			.collect::<std::collections::HashMap<_, _>>();
		ids.into_iter().filter_map(|id| by_id.remove(&id)).collect()
	};
	let content = map_books(ctx.conn(), &user, models).await?;
	cached_json(
		&headers,
		&Page::new(
			content,
			pagination.page,
			pagination.size,
			total,
			pagination.unpaged,
		),
	)
}

async fn get_book(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	let Some(model) = media::ModelWithMetadata::find_by_id_for_user(id, &user)
		.filter(media::Column::DeletedAt.is_null())
		.filter(
			media::Column::Extension.is_in(SUPPORTED_MEDIA_EXTENSIONS.iter().copied()),
		)
		.into_model::<media::ModelWithMetadata>()
		.one(ctx.conn())
		.await?
	else {
		return Err(APIError::NotFound("Book not found".to_owned()));
	};
	let mut books = map_books(ctx.conn(), &user, vec![model]).await?;
	let book = books
		.pop()
		.ok_or_else(|| APIError::NotFound("Book not found".to_owned()))?;
	cached_json(&headers, &book)
}

async fn get_series_by_id(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	// Mode B: a live-only virtual series is served straight from the source
	// under its deterministic id; materialised series use the database path.
	if let Some(live) = ctx.virtual_series_by_id(&user, &id).await {
		return cached_json(&headers, &live?);
	}
	let Some(model) = series::ModelWithMetadata::find_by_id_for_user(id, &user)
		.filter(series::Column::Id.in_subquery(visible_series_ids_subquery(&user, None)))
		.into_model::<series::ModelWithMetadata>()
		.one(ctx.conn())
		.await?
	else {
		return Err(APIError::NotFound("Series not found".to_owned()));
	};
	let mut series = map_series(ctx.conn(), &user, vec![model]).await?;
	let item = series
		.pop()
		.ok_or_else(|| APIError::NotFound("Series not found".to_owned()))?;
	cached_json(&headers, &item)
}

fn sibling_cmp(
	a: &media::ModelWithMetadata,
	b: &media::ModelWithMetadata,
) -> CmpOrdering {
	let a_number = a.metadata.as_ref().and_then(|metadata| metadata.number);
	let b_number = b.metadata.as_ref().and_then(|metadata| metadata.number);
	let number_order = match (a_number, b_number) {
		(Some(a_number), Some(b_number)) => a_number.cmp(&b_number),
		(Some(_), None) => CmpOrdering::Less,
		(None, Some(_)) => CmpOrdering::Greater,
		(None, None) => CmpOrdering::Equal,
	};
	number_order
		.then_with(|| {
			a.media
				.name
				.to_ascii_lowercase()
				.cmp(&b.media.name.to_ascii_lowercase())
		})
		.then_with(|| a.media.name.cmp(&b.media.name))
		.then_with(|| a.media.id.cmp(&b.media.id))
}

fn choose_sibling(
	models: &mut [media::ModelWithMetadata],
	current_id: &str,
	previous: bool,
) -> Option<media::ModelWithMetadata> {
	models.sort_by(sibling_cmp);
	let index = models
		.iter()
		.position(|model| model.media.id == current_id)?;
	let sibling_index = if previous {
		index.checked_sub(1)?
	} else {
		index.checked_add(1)?
	};
	models.get(sibling_index).cloned()
}

async fn sibling(
	backend: &dyn KomgaBackend,
	user: &AuthUser,
	id: String,
	previous: bool,
) -> APIResult<KomgaBook> {
	let Some(current) = media::ModelWithMetadata::find_by_id_for_user(id.clone(), user)
		.filter(media::Column::DeletedAt.is_null())
		.filter(
			media::Column::Extension.is_in(SUPPORTED_MEDIA_EXTENSIONS.iter().copied()),
		)
		.into_model::<media::ModelWithMetadata>()
		.one(backend.conn())
		.await?
	else {
		return Err(APIError::NotFound("Book not found".to_owned()));
	};
	let series_id = current
		.media
		.series_id
		.clone()
		.ok_or_else(|| APIError::NotFound("Book not found".to_owned()))?;
	let mut models = media::ModelWithMetadata::find_for_user(user)
		.filter(media::Column::DeletedAt.is_null())
		.filter(
			media::Column::Extension.is_in(SUPPORTED_MEDIA_EXTENSIONS.iter().copied()),
		)
		.filter(media::Column::SeriesId.eq(series_id))
		.into_model::<media::ModelWithMetadata>()
		.all(backend.conn())
		.await?;
	let Some(model) = choose_sibling(&mut models, &id, previous) else {
		return Err(APIError::NotFound("Book sibling not found".to_owned()));
	};
	let mut books = map_books(backend.conn(), user, vec![model]).await?;
	books
		.pop()
		.ok_or_else(|| APIError::NotFound("Book sibling not found".to_owned()))
}

async fn get_previous_book(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let book = sibling(ctx.as_ref(), &auth.user(), id, true).await?;
	cached_json(&headers, &book)
}

async fn get_next_book(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let book = sibling(ctx.as_ref(), &auth.user(), id, false).await?;
	cached_json(&headers, &book)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct BookTagsQuery {
	#[serde(rename = "seriesId", alias = "series_id")]
	series_id: Option<String>,
	#[serde(rename = "readListId", alias = "readlistId", alias = "readlist_id")]
	readlist_id: Option<String>,
	#[serde(rename = "libraryId", alias = "library_id")]
	library_id: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SeriesTagsQuery {
	#[serde(rename = "libraryId", alias = "library_id")]
	library_id: Vec<String>,
	#[serde(rename = "collectionId", alias = "collection_id")]
	collection_id: Option<String>,
}

async fn get_book_tags(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<BookTagsQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	let library_ids = csv_values(&query.library_id);
	let series_id = query.series_id.as_deref();
	let readlist_ids = query.readlist_id.as_deref().map(|id| vec![id.to_owned()]);
	let media_ids = visible_media_ids_subquery(
		&user,
		Some(&library_ids),
		series_id,
		readlist_ids.as_deref(),
	);
	let names = tag::Entity::find()
		.select_only()
		.column(tag::Column::Name)
		.distinct()
		.filter(
			tag::Column::Id.in_subquery(
				media_tag::Entity::find()
					.select_only()
					.column(media_tag::Column::TagId)
					.filter(media_tag::Column::MediaId.in_subquery(media_ids))
					.into_query(),
			),
		)
		.order_by_asc(tag::Column::Name)
		.into_tuple::<String>()
		.all(ctx.conn())
		.await?;
	cached_json(&headers, &names)
}

async fn get_series_tags(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesTagsQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	if query.collection_id.is_some() {
		return Err(unsupported_filter("collectionId"));
	}
	let user = auth.user();
	let library_ids = csv_values(&query.library_id);
	let series_ids = visible_series_ids_subquery(&user, Some(&library_ids));
	let names = tag::Entity::find()
		.select_only()
		.column(tag::Column::Name)
		.distinct()
		.filter(
			tag::Column::Id.in_subquery(
				series_tag::Entity::find()
					.select_only()
					.column(series_tag::Column::TagId)
					.filter(series_tag::Column::SeriesId.in_subquery(series_ids))
					.into_query(),
			),
		)
		.order_by_asc(tag::Column::Name)
		.into_tuple::<String>()
		.all(ctx.conn())
		.await?;
	cached_json(&headers, &names)
}
fn merge_tag_names(media_names: &[String], series_names: &[String]) -> Vec<String> {
	let mut names = BTreeSet::new();
	names.extend(media_names.iter().cloned());
	names.extend(series_names.iter().cloned());
	names.into_iter().collect()
}

async fn get_tags(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	let visible_media_ids = media::Entity::find_for_user(&user)
		.filter(media::Column::DeletedAt.is_null())
		.filter(series::Column::DeletedAt.is_null())
		.select_only()
		.column(media::Column::Id)
		.into_query();
	let media_names = tag::Entity::find()
		.select_only()
		.column(tag::Column::Name)
		.distinct()
		.filter(
			tag::Column::Id.in_subquery(
				media_tag::Entity::find()
					.select_only()
					.column(media_tag::Column::TagId)
					.filter(media_tag::Column::MediaId.in_subquery(visible_media_ids))
					.into_query(),
			),
		)
		.into_tuple::<String>()
		.all(ctx.conn())
		.await?;

	let visible_series_ids = series::Entity::find_for_user(&user)
		.select_only()
		.column(series::Column::Id)
		.into_query();
	let series_names = tag::Entity::find()
		.select_only()
		.column(tag::Column::Name)
		.distinct()
		.filter(
			tag::Column::Id.in_subquery(
				series_tag::Entity::find()
					.select_only()
					.column(series_tag::Column::TagId)
					.filter(series_tag::Column::SeriesId.in_subquery(visible_series_ids))
					.into_query(),
			),
		)
		.into_tuple::<String>()
		.all(ctx.conn())
		.await?;

	cached_json(&headers, &merge_tag_names(&media_names, &series_names))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct V1MetadataQuery {
	#[serde(rename = "libraryId", alias = "library_id")]
	library_id: Vec<String>,
	#[serde(rename = "collectionId", alias = "collection_id")]
	collection_id: Option<String>,
}

async fn get_genres(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<V1MetadataQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	if query.collection_id.is_some() {
		return Err(unsupported_filter("collectionId"));
	}
	let values = distinct_series_csv_values(
		ctx.conn(),
		&auth.user(),
		&csv_values(&query.library_id),
		series_metadata::Column::Genres,
	)
	.await?;
	cached_json(&headers, &values)
}

async fn get_publishers(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<V1MetadataQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	if query.collection_id.is_some() {
		return Err(unsupported_filter("collectionId"));
	}
	let values = distinct_series_scalar_values(
		ctx.conn(),
		&auth.user(),
		&csv_values(&query.library_id),
		series_metadata::Column::Publisher,
	)
	.await?;
	cached_json(&headers, &values)
}

async fn get_languages(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<V1MetadataQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	if query.collection_id.is_some() {
		return Err(unsupported_filter("collectionId"));
	}
	let user = auth.user();
	let media_ids = visible_media_ids_subquery(
		&user,
		Some(&csv_values(&query.library_id)),
		None,
		None,
	);
	let values = media_metadata::Entity::find()
		.select_only()
		.column(media_metadata::Column::Language)
		.distinct()
		.filter(media_metadata::Column::MediaId.in_subquery(media_ids))
		.into_tuple::<Option<String>>()
		.all(ctx.conn())
		.await?;
	let mut languages = BTreeSet::new();
	for value in values {
		if let Some(value) = value
			.map(|value| value.trim().to_owned())
			.filter(|value| !value.is_empty())
		{
			languages.insert(value);
		}
	}
	let languages = languages.into_iter().collect::<Vec<_>>();
	cached_json(&headers, &languages)
}
/// Stump has no sharing-label model; report the honest empty referential.
async fn get_sharing_labels(headers: HeaderMap) -> APIResult<Response<Body>> {
	let sharing_labels: Vec<String> = Vec::new();
	cached_json(&headers, &sharing_labels)
}

async fn get_age_ratings(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<V1MetadataQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	if query.collection_id.is_some() {
		return Err(unsupported_filter("collectionId"));
	}
	let user = auth.user();
	let ids = visible_series_ids_subquery(&user, Some(&csv_values(&query.library_id)));
	let values = series_metadata::Entity::find()
		.select_only()
		.column(series_metadata::Column::AgeRating)
		.distinct()
		.filter(series_metadata::Column::SeriesId.in_subquery(ids))
		.order_by_asc(series_metadata::Column::AgeRating)
		.into_tuple::<Option<i32>>()
		.all(ctx.conn())
		.await?;
	let values = values
		.into_iter()
		.map(|value| value.map_or_else(|| "None".to_owned(), |value| value.to_string()))
		.collect::<Vec<_>>();
	cached_json(&headers, &values)
}

async fn get_release_dates(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<V1MetadataQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	if query.collection_id.is_some() {
		return Err(unsupported_filter("collectionId"));
	}
	let user = auth.user();
	let ids = visible_media_ids_subquery(
		&user,
		Some(&csv_values(&query.library_id)),
		None,
		None,
	);
	let years = media_metadata::Entity::find()
		.select_only()
		.column(media_metadata::Column::Year)
		.distinct()
		.filter(media_metadata::Column::MediaId.in_subquery(ids))
		.filter(media_metadata::Column::Year.is_not_null())
		.order_by_desc(media_metadata::Column::Year)
		.into_tuple::<Option<i32>>()
		.all(ctx.conn())
		.await?;
	let years = years
		.into_iter()
		.flatten()
		.map(|year| year.to_string())
		.collect::<Vec<_>>();
	cached_json(&headers, &years)
}

async fn distinct_series_scalar_values(
	conn: &DatabaseConnection,
	user: &AuthUser,
	library_ids: &[String],
	column: series_metadata::Column,
) -> APIResult<Vec<String>> {
	let ids = visible_series_ids_subquery(user, Some(library_ids));
	let values = series_metadata::Entity::find()
		.select_only()
		.column(column)
		.distinct()
		.filter(series_metadata::Column::SeriesId.in_subquery(ids))
		.into_tuple::<Option<String>>()
		.all(conn)
		.await?;
	let mut result = BTreeSet::new();
	for value in values {
		if let Some(value) = value
			.map(|value| value.trim().to_owned())
			.filter(|value| !value.is_empty())
		{
			result.insert(value);
		}
	}
	Ok(result.into_iter().collect())
}

async fn distinct_series_csv_values(
	conn: &DatabaseConnection,
	user: &AuthUser,
	library_ids: &[String],
	column: series_metadata::Column,
) -> APIResult<Vec<String>> {
	let ids = visible_series_ids_subquery(user, Some(library_ids));
	let values = series_metadata::Entity::find()
		.select_only()
		.column(column)
		.distinct()
		.filter(series_metadata::Column::SeriesId.in_subquery(ids))
		.into_tuple::<Option<String>>()
		.all(conn)
		.await?;
	let mut result = BTreeSet::new();
	for value in values.into_iter().flatten() {
		for item in value
			.split(',')
			.map(str::trim)
			.filter(|item| !item.is_empty())
		{
			result.insert(item.to_owned());
		}
	}
	Ok(result.into_iter().collect())
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AuthorsQuery {
	page: i32,
	size: i32,
	unpaged: bool,
	search: Option<String>,
	role: Option<String>,
	#[serde(rename = "libraryId", alias = "library_id")]
	library_id: Vec<String>,
	#[serde(rename = "seriesId", alias = "series_id")]
	series_id: Vec<String>,
	#[serde(rename = "readListId", alias = "readlistId", alias = "readlist_id")]
	readlist_id: Vec<String>,
	#[serde(rename = "collectionId", alias = "collection_id")]
	collection_id: Vec<String>,
}

impl Default for AuthorsQuery {
	fn default() -> Self {
		Self {
			page: 0,
			size: DEFAULT_PAGE_SIZE,
			unpaged: false,
			search: None,
			role: None,
			library_id: Vec::new(),
			series_id: Vec::new(),
			readlist_id: Vec::new(),
			collection_id: Vec::new(),
		}
	}
}

#[derive(Debug, FromQueryResult)]
struct MediaAuthorsRow {
	writers: Option<String>,
	pencillers: Option<String>,
	inkers: Option<String>,
	colorists: Option<String>,
	letterers: Option<String>,
	cover_artists: Option<String>,
	editors: Option<String>,
}

#[derive(Debug, FromQueryResult)]
struct SeriesAuthorsRow {
	writers: Option<String>,
}

fn add_authors(
	set: &mut BTreeSet<(String, String, String)>,
	value: Option<&str>,
	role: &str,
) {
	if let Some(value) = value {
		for name in value
			.split(',')
			.map(str::trim)
			.filter(|name| !name.is_empty())
		{
			set.insert((name.to_ascii_lowercase(), role.to_owned(), name.to_owned()));
		}
	}
}

async fn collect_authors(
	ctx: &dyn KomgaBackend,
	user: &AuthUser,
	query: &AuthorsQuery,
) -> APIResult<Vec<KomgaAuthor>> {
	if !query.collection_id.is_empty() {
		return Err(unsupported_filter("collectionId"));
	}
	let library_ids = csv_values(&query.library_id);
	let series_ids = csv_values(&query.series_id);
	let readlist_ids = csv_values(&query.readlist_id);
	let series_filter = (!series_ids.is_empty()).then_some(series_ids.as_slice());
	let readlists = (!readlist_ids.is_empty()).then_some(readlist_ids.as_slice());
	let visible_media = visible_media_ids_for_series_ids_subquery(
		user,
		Some(&library_ids),
		series_filter,
		readlists,
	);
	let media_rows = media_metadata::Entity::find()
		.select_only()
		.columns([
			media_metadata::Column::Writers,
			media_metadata::Column::Pencillers,
			media_metadata::Column::Inkers,
			media_metadata::Column::Colorists,
			media_metadata::Column::Letterers,
			media_metadata::Column::CoverArtists,
			media_metadata::Column::Editors,
		])
		.filter(media_metadata::Column::MediaId.in_subquery(visible_media))
		.into_model::<MediaAuthorsRow>()
		.all(ctx.conn())
		.await?;
	let visible_series = if !readlist_ids.is_empty() {
		visible_series_ids_for_media_series_ids_subquery(
			user,
			Some(&library_ids),
			series_filter,
			readlists,
		)
	} else {
		visible_series_ids_for_series_ids_subquery(
			user,
			Some(&library_ids),
			series_filter,
		)
	};
	let series_rows = series_metadata::Entity::find()
		.select_only()
		.column(series_metadata::Column::Writers)
		.filter(series_metadata::Column::SeriesId.in_subquery(visible_series))
		.into_model::<SeriesAuthorsRow>()
		.all(ctx.conn())
		.await?;
	let mut authors = BTreeSet::new();
	for row in &media_rows {
		add_authors(&mut authors, row.writers.as_deref(), "writer");
		add_authors(&mut authors, row.pencillers.as_deref(), "penciller");
		add_authors(&mut authors, row.inkers.as_deref(), "inker");
		add_authors(&mut authors, row.colorists.as_deref(), "colorist");
		add_authors(&mut authors, row.letterers.as_deref(), "letterer");
		add_authors(&mut authors, row.cover_artists.as_deref(), "cover");
		add_authors(&mut authors, row.editors.as_deref(), "editor");
	}
	for row in &series_rows {
		add_authors(&mut authors, row.writers.as_deref(), "writer");
	}
	let search = query
		.search
		.as_deref()
		.unwrap_or_default()
		.trim()
		.to_ascii_lowercase();
	let role = query
		.role
		.as_deref()
		.map(str::trim)
		.filter(|role| !role.is_empty())
		.map(str::to_ascii_lowercase);
	Ok(authors
		.into_iter()
		.filter(|(name_lower, _, _)| search.is_empty() || name_lower.contains(&search))
		.filter(|(_, author_role, _)| {
			role.as_deref().is_none_or(|role| role == author_role)
		})
		.map(|(_, role, name)| KomgaAuthor { name, role })
		.collect())
}

async fn get_authors(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<AuthorsQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let pagination = PaginationQuery {
		page: query.page,
		size: query.size,
		unpaged: query.unpaged,
		sort: Vec::new(),
	}
	.validate()?
	.0;
	let user = auth.user();
	let authors = collect_authors(ctx.as_ref(), &user, &query).await?;
	let total = total_as_i32(authors.len() as u64)?;
	let content = if pagination.unpaged {
		authors
	} else {
		let start = usize::try_from(pagination.offset()).unwrap_or(usize::MAX);
		authors
			.into_iter()
			.skip(start)
			.take(pagination.size as usize)
			.collect()
	};
	cached_json(
		&headers,
		&Page::new(
			content,
			pagination.page,
			pagination.size,
			total,
			pagination.unpaged,
		),
	)
}

async fn get_authors_legacy(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<AuthorsQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	let authors = collect_authors(ctx.as_ref(), &user, &query).await?;
	cached_json(&headers, &authors)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn pagination_is_zero_based_and_bounded() {
		let (pagination, _) = PaginationQuery::default().validate().unwrap();
		assert_eq!(pagination.page, 0);
		assert_eq!(pagination.size, 20);
		assert_eq!(pagination.offset(), 0);
		assert!(PaginationQuery {
			page: -1,
			..Default::default()
		}
		.validate()
		.is_err());
		// size=0 is Komga's count-only idiom and stays valid.
		let (pagination, _) = PaginationQuery {
			size: 0,
			..Default::default()
		}
		.validate()
		.expect("count-only size is valid");
		assert_eq!(pagination.size, 0);
		assert!(PaginationQuery {
			size: -1,
			..Default::default()
		}
		.validate()
		.is_err());
		// Oversized pages clamp to the maximum instead of failing the request.
		let (pagination, _) = PaginationQuery {
			size: 201,
			..Default::default()
		}
		.validate()
		.expect("oversized size clamps");
		assert_eq!(pagination.size, MAX_PAGE_SIZE);
	}

	#[test]
	fn sort_parser_defaults_direction_and_accepts_repeated_values() {
		assert_eq!(
			parse_sort_specs(&[
				"metadata.titleSort".to_owned(),
				"createdDate,desc".to_owned(),
				"readProgress.readDate".to_owned(),
			]),
			vec![
				SortSpec {
					field: "metadata.titlesort".to_owned(),
					direction: SortDirection::Asc,
				},
				SortSpec {
					field: "createddate".to_owned(),
					direction: SortDirection::Desc,
				},
				SortSpec {
					field: "readprogress.readdate".to_owned(),
					direction: SortDirection::Asc,
				},
			]
		);
		assert!(parse_sort_specs(&["metadata.titleSort,sideways".to_owned()]).is_empty());
		assert!(
			parse_sort_specs(&["metadata.titleSort,asc,extra".to_owned()]).is_empty()
		);
	}

	#[test]
	fn series_title_sort_uses_persisted_override_with_name_fallback() {
		let query = apply_series_order(
			series::ModelWithMetadata::find(),
			&[SortSpec {
				field: "metadata.titlesort".to_owned(),
				direction: SortDirection::Asc,
			}],
		);
		let sql = query
			.into_query()
			.to_string(sea_orm::sea_query::SqliteQueryBuilder);
		assert!(sql.contains("COALESCE"));
		assert!(sql.contains("\"series_metadata\".\"title_sort\""));
		assert!(sql.contains("\"series\".\"name\""));
	}

	#[test]
	fn csv_values_are_trimmed_and_split() {
		assert_eq!(
			csv_values(&[" a,b ".to_owned(), "c".to_owned()]),
			vec!["a", "b", "c"]
		);
	}

	#[test]
	fn empty_all_of_is_a_noop_and_empty_any_of_is_false() {
		let user = AuthUser::default();
		let all = BookCondition::AllOfBook { conditions: vec![] };
		assert!(book_condition_filter(&all, &user).unwrap().is_none());
		let any = BookCondition::AnyOfBook { conditions: vec![] };
		assert!(book_condition_filter(&any, &user).unwrap().is_some());
		let series = SeriesCondition::AllOfSeries { conditions: vec![] };
		assert!(series_condition_filter(&series, &user).unwrap().is_none());
		let any_series = SeriesCondition::AnyOfSeries { conditions: vec![] };
		assert!(series_condition_filter(&any_series, &user)
			.unwrap()
			.is_some());
	}
	#[test]
	fn tag_names_merge_distinct_values_in_sorted_order() {
		let media = vec!["zeta".to_owned(), "shared".to_owned()];
		let series = vec!["alpha".to_owned(), "shared".to_owned()];
		assert_eq!(
			merge_tag_names(&media, &series),
			vec!["alpha", "shared", "zeta"]
		);
	}
	#[test]
	fn legacy_query_translates_library_csv_read_status_and_sort() {
		let query = LegacyCatalogQuery {
			library_id: vec!["library-a, library-b".to_owned()],
			read_status: vec!["UNREAD".to_owned(), "IN_PROGRESS".to_owned()],
			sort: vec!["metadata.titleSort,asc".to_owned()],
			..Default::default()
		};
		let search = query.book_search(None).expect("legacy query translates");
		let condition =
			serde_json::to_value(search.condition).expect("condition serializes");
		let all = condition["allOf"].as_array().expect("combined conditions");
		assert_eq!(all.len(), 2);
		assert_eq!(all[0]["type"], "AnyOfBook");
		let library_values = all[0]["anyOf"]
			.as_array()
			.expect("library alternatives")
			.iter()
			.map(|condition| condition["libraryId"]["value"].as_str().unwrap())
			.collect::<Vec<_>>();
		assert_eq!(library_values, ["library-a", "library-b"]);
		assert_eq!(all[1]["type"], "AnyOfBook");
		let read_statuses = all[1]["anyOf"]
			.as_array()
			.expect("read-status alternatives")
			.iter()
			.map(|condition| condition["readStatus"]["value"].as_str().unwrap())
			.collect::<Vec<_>>();
		assert_eq!(read_statuses, ["UNREAD", "IN_PROGRESS"]);

		let (pagination, sorts) = query.pagination().expect("pagination translates");
		assert_eq!(pagination.size, DEFAULT_PAGE_SIZE);
		assert_eq!(
			sorts,
			vec![SortSpec {
				field: "metadata.titlesort".to_owned(),
				direction: SortDirection::Asc,
			}]
		);
	}
}
