use std::{collections::HashMap, convert::TryFrom, sync::Arc};

use crate::{
	book::{KomgaBookId, KomgaMediaStatus},
	collection::{
		KomgaCollection, KomgaCollectionCreateRequest, KomgaCollectionId,
		KomgaCollectionQuery, KomgaCollectionUpdateRequest,
	},
	read_list::{
		KomgaReadList, KomgaReadListCreateRequest, KomgaReadListId, KomgaReadListQuery,
		KomgaReadListUpdateRequest,
	},
	routes::progress::{
		counts_for_progress_books, mark_book_read, progress_books, KomgaReadProgressDto,
		KomgaReadProgressUpdateDto,
	},
	series::KomgaSeriesId,
	sse::KomgaEvent,
	Page, SUPPORTED_MEDIA_EXTENSIONS,
};
use axum::{
	body::Body,
	extract::Path,
	http::{HeaderMap, StatusCode},
	response::Response,
	routing::get,
	Extension, Json, Router,
};
use axum_extra::extract::Query;
use chrono::{DateTime, Utc};
use models::entity::{
	collection, collection_series, media, reading_list, reading_list_item, series,
	user::AuthUser,
};
use models::txn::begin_write;
use sea_orm::{
	prelude::*,
	sea_query::{Condition, Order, SelectStatement},
	QueryOrder, QuerySelect, QueryTrait,
};
use serde::Deserialize;
use stump_auth::AuthContext;

use super::{KomgaBackend, KomgaEvents};
use crate::errors::{APIError, APIResult};
use crate::routes::{
	mapper::{map_books, map_series},
	response::cached_json,
};

const DEFAULT_PAGE_SIZE: i32 = 20;
const MAX_PAGE_SIZE: i32 = 200;
const READING_LIST_READER_ROLE: i32 = 1;

/// The Komga compatibility routes for persisted reading lists and collections.
///
/// Authentication is deliberately not installed here. The top-level Komga router applies the
/// auth middleware once around all of these routes, and every handler below receives the resulting
/// `AuthContext` extension.
pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::<S>::new()
		.route(
			"/api/v1/readlists",
			get(get_readlists).post(create_readlist),
		)
		.route(
			"/api/v1/readlists/{id}",
			get(get_readlist)
				.patch(patch_readlist)
				.delete(delete_readlist),
		)
		.route(
			"/api/v1/readlists/{id}/thumbnail",
			get(get_readlist_thumbnail),
		)
		.route("/api/v1/readlists/{id}/books", get(get_readlist_books))
		.route(
			"/api/v1/readlists/{id}/read-progress/tachiyomi",
			get(get_tachiyomi_readlist_progress).put(update_tachiyomi_readlist_progress),
		)
		.route("/api/v1/books/{id}/readlists", get(get_book_readlists))
		.route(
			"/api/v1/collections",
			get(get_collections).post(create_collection),
		)
		.route(
			"/api/v1/collections/{id}",
			get(get_collection)
				.patch(patch_collection)
				.delete(delete_collection),
		)
		.route(
			"/api/v1/collections/{id}/thumbnail",
			get(get_collection_thumbnail),
		)
		.route(
			"/api/v1/collections/{id}/series",
			get(get_collection_series),
		)
		.route(
			"/api/v1/series/{id}/collections",
			get(get_series_collections),
		)
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
	#[serde(default)]
	search: Option<String>,
	#[serde(default)]
	condition: Option<String>,
	#[serde(rename = "fullTextSearch", default)]
	full_text_search: Option<String>,
	#[serde(rename = "full_text_search", default)]
	legacy_full_text_search: Option<String>,
}

fn nonempty(value: Option<&str>) -> bool {
	value.is_some_and(|value| !value.trim().is_empty())
}

fn validate_search(query: &SearchQuery) -> APIResult<()> {
	if nonempty(query.search.as_deref()) {
		return Err(unsupported_filter("search"));
	}
	if nonempty(query.condition.as_deref()) {
		return Err(unsupported_filter("condition"));
	}
	if nonempty(query.full_text_search.as_deref())
		|| nonempty(query.legacy_full_text_search.as_deref())
	{
		return Err(unsupported_filter("fullTextSearch"));
	}
	Ok(())
}

#[derive(Debug, Deserialize)]
struct LibraryFilterQuery {
	// serde_html_form only applies repeated-key/scalar collection to plain `Vec`
	// fields; `Option<Vec<_>>` would reject Komga's `library_id=<id>` form.
	#[serde(rename = "libraryIds", default)]
	library_ids: Vec<crate::KomgaLibraryId>,
	#[serde(rename = "library_id", default)]
	legacy_library_ids: Vec<crate::KomgaLibraryId>,
}

impl LibraryFilterQuery {
	fn ids(&self) -> Option<Vec<String>> {
		let mut ids = self
			.library_ids
			.iter()
			.map(|id| id.0.clone())
			.collect::<Vec<_>>();
		ids.extend(self.legacy_library_ids.iter().map(|id| id.0.clone()));
		(!ids.is_empty()).then_some(ids)
	}
}

#[derive(Debug, Deserialize)]
struct PaginationQuery {
	#[serde(default)]
	page: i32,
	#[serde(default = "default_page_size")]
	size: i32,
	#[serde(default)]
	unpaged: bool,
	#[serde(default)]
	sort: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct Pagination {
	page: i32,
	size: i32,
	unpaged: bool,
}

impl PaginationQuery {
	fn validate(self) -> APIResult<Pagination> {
		if self.page < 0 {
			return Err(APIError::BadRequest(
				"page must be zero-based and non-negative".to_owned(),
			));
		}
		if self.size < 0 {
			return Err(APIError::BadRequest("size must be non-negative".to_owned()));
		}
		let size = self.size.min(MAX_PAGE_SIZE);

		Ok(Pagination {
			page: self.page,
			size,
			unpaged: self.unpaged,
		})
	}
}

fn default_page_size() -> i32 {
	DEFAULT_PAGE_SIZE
}

impl Pagination {
	fn offset(self) -> u64 {
		(self.page as u64).saturating_mul(self.size as u64)
	}
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

fn invalid_sort(value: &str) -> APIError {
	APIError::BadRequest(format!(
		"sort '{value}' is not supported; expected field(,asc|desc)"
	))
}

fn parse_sort_specs(values: &[String]) -> APIResult<Vec<SortSpec>> {
	let mut specs = Vec::with_capacity(values.len());
	for value in values {
		let mut parts = value.split(',');
		let field = parts.next().unwrap_or_default().trim();
		let direction = parts.next().unwrap_or("asc").trim();
		if field.is_empty() || parts.next().is_some() {
			return Err(invalid_sort(value));
		}

		let field = field.to_ascii_lowercase();
		if !matches!(field.as_str(), "name" | "createddate" | "lastmodifieddate") {
			return Err(invalid_sort(value));
		}

		let direction = match direction.to_ascii_lowercase().as_str() {
			"asc" => SortDirection::Asc,
			"desc" => SortDirection::Desc,
			_ => return Err(invalid_sort(value)),
		};
		specs.push(SortSpec { field, direction });
	}
	Ok(specs)
}

fn order_for(direction: SortDirection) -> Order {
	match direction {
		SortDirection::Asc => Order::Asc,
		SortDirection::Desc => Order::Desc,
	}
}

fn apply_read_list_order(
	mut query: Select<reading_list::Entity>,
	sorts: &[SortSpec],
	search_present: bool,
) -> Select<reading_list::Entity> {
	// Upstream uses relevance for an un-sorted search. This profile preserves
	// the existing search order (the RBAC query's stable ID order) because it
	// has no relevance index; the default name order applies when no search is
	// present.
	if sorts.is_empty() && search_present {
		return query;
	}
	// `find_for_user` supplies an ID order for callers that do not add one.
	// Replace it here so an explicit Komga sort is the primary ordering.
	QueryTrait::query(&mut query).clear_order_by();
	if sorts.is_empty() {
		query = query.order_by_asc(reading_list::Column::Name);
	} else {
		for sort in sorts {
			query = match sort.field.as_str() {
				"name" => {
					query.order_by(reading_list::Column::Name, order_for(sort.direction))
				},
				"createddate" | "lastmodifieddate" => query
					.order_by(reading_list::Column::UpdatedAt, order_for(sort.direction)),
				_ => unreachable!("read-list sort passed validation"),
			};
		}
	}
	query.order_by_asc(reading_list::Column::Id)
}

fn apply_collection_order(
	mut query: Select<collection::Entity>,
	sorts: &[SortSpec],
) -> Select<collection::Entity> {
	QueryTrait::query(&mut query).clear_order_by();
	if sorts.is_empty() {
		query = query.order_by_asc(collection::Column::Name);
	} else {
		for sort in sorts {
			query = match sort.field.as_str() {
				"name" => {
					query.order_by(collection::Column::Name, order_for(sort.direction))
				},
				"createddate" | "lastmodifieddate" => query
					.order_by(collection::Column::UpdatedAt, order_for(sort.direction)),
				_ => unreachable!("collection sort passed validation"),
			};
		}
	}
	query.order_by_asc(collection::Column::Id)
}

fn validate_read_list_filters(query: &KomgaReadListQuery) -> APIResult<()> {
	if query
		.read_status
		.as_ref()
		.is_some_and(|values| !values.is_empty())
	{
		return Err(unsupported_filter("readStatus"));
	}
	if query.tags.as_ref().is_some_and(|values| !values.is_empty()) {
		return Err(unsupported_filter("tags"));
	}
	if query
		.media_status
		.as_ref()
		.is_some_and(|values| !values.is_empty())
	{
		return Err(unsupported_filter("mediaStatus"));
	}
	if query.deleted.is_some() {
		return Err(unsupported_filter("deleted"));
	}
	if query
		.authors
		.as_ref()
		.is_some_and(|values| !values.is_empty())
	{
		return Err(unsupported_filter("authors"));
	}
	Ok(())
}

fn validate_collection_filters(query: &KomgaCollectionQuery) -> APIResult<()> {
	if query
		.status
		.as_ref()
		.is_some_and(|values| !values.is_empty())
	{
		return Err(unsupported_filter("status"));
	}
	if query
		.read_status
		.as_ref()
		.is_some_and(|values| !values.is_empty())
	{
		return Err(unsupported_filter("readStatus"));
	}
	if query
		.publishers
		.as_ref()
		.is_some_and(|values| !values.is_empty())
	{
		return Err(unsupported_filter("publishers"));
	}
	if query
		.languages
		.as_ref()
		.is_some_and(|values| !values.is_empty())
	{
		return Err(unsupported_filter("languages"));
	}
	if query
		.genres
		.as_ref()
		.is_some_and(|values| !values.is_empty())
	{
		return Err(unsupported_filter("genres"));
	}
	if query.tags.as_ref().is_some_and(|values| !values.is_empty()) {
		return Err(unsupported_filter("tags"));
	}
	if query
		.age_ratings
		.as_ref()
		.is_some_and(|values| !values.is_empty())
	{
		return Err(unsupported_filter("ageRatings"));
	}
	if query
		.release_years
		.as_ref()
		.is_some_and(|values| !values.is_empty())
	{
		return Err(unsupported_filter("releaseYears"));
	}
	if query
		.authors
		.as_ref()
		.is_some_and(|values| !values.is_empty())
	{
		return Err(unsupported_filter("authors"));
	}
	if query.deleted.is_some() {
		return Err(unsupported_filter("deleted"));
	}
	if query.complete.is_some() {
		return Err(unsupported_filter("complete"));
	}
	Ok(())
}

fn visible_media_ids_subquery(
	user: &AuthUser,
	library_ids: Option<&[String]>,
) -> SelectStatement {
	let mut query = media::Entity::find_for_user(user)
		.filter(
			media::Column::Extension.is_in(SUPPORTED_MEDIA_EXTENSIONS.iter().copied()),
		)
		.select_only()
		.column(media::Column::Id);
	if let Some(library_ids) = library_ids {
		query = query.filter(series::Column::LibraryId.is_in(library_ids.to_vec()));
	}
	query.into_query()
}

fn visible_series_ids_subquery(
	user: &AuthUser,
	library_ids: Option<&[String]>,
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
	if let Some(library_ids) = library_ids {
		query = query.filter(series::Column::LibraryId.is_in(library_ids.to_vec()));
	}
	query.into_query()
}

fn visible_reading_items_query(
	user: &AuthUser,
	list_ids: &[String],
	library_ids: Option<&[String]>,
) -> Select<reading_list_item::Entity> {
	reading_list_item::Entity::find()
		.filter(reading_list_item::Column::ReadingListId.is_in(list_ids.to_vec()))
		.filter(
			reading_list_item::Column::MediaId
				.in_subquery(visible_media_ids_subquery(user, library_ids)),
		)
		.order_by_asc(reading_list_item::Column::ReadingListId)
		.order_by_asc(reading_list_item::Column::DisplayOrder)
		.order_by_asc(reading_list_item::Column::Id)
}

fn visible_collection_series_query(
	user: &AuthUser,
	collection_ids: &[String],
	library_ids: Option<&[String]>,
) -> Select<collection_series::Entity> {
	collection_series::Entity::find()
		.filter(collection_series::Column::CollectionId.is_in(collection_ids.to_vec()))
		.filter(
			collection_series::Column::SeriesId
				.in_subquery(visible_series_ids_subquery(user, library_ids)),
		)
		.order_by_asc(collection_series::Column::CollectionId)
		.order_by_asc(collection_series::Column::DisplayOrder)
		.order_by_asc(collection_series::Column::Id)
}

async fn visible_reading_items(
	conn: &DatabaseConnection,
	user: &AuthUser,
	list_ids: &[String],
	library_ids: Option<&[String]>,
) -> APIResult<Vec<reading_list_item::Model>> {
	if list_ids.is_empty() {
		return Ok(Vec::new());
	}
	Ok(visible_reading_items_query(user, list_ids, library_ids)
		.all(conn)
		.await?)
}

async fn visible_collection_series(
	conn: &DatabaseConnection,
	user: &AuthUser,
	collection_ids: &[String],
	library_ids: Option<&[String]>,
) -> APIResult<Vec<collection_series::Model>> {
	if collection_ids.is_empty() {
		return Ok(Vec::new());
	}
	Ok(
		visible_collection_series_query(user, collection_ids, library_ids)
			.all(conn)
			.await?,
	)
}

fn group_reading_items(
	items: Vec<reading_list_item::Model>,
) -> HashMap<String, Vec<String>> {
	let mut grouped = HashMap::new();
	for item in items {
		grouped
			.entry(item.reading_list_id)
			.or_insert_with(Vec::new)
			.push(item.media_id);
	}
	grouped
}

fn group_collection_series(
	members: Vec<collection_series::Model>,
) -> HashMap<String, Vec<String>> {
	let mut grouped = HashMap::new();
	for member in members {
		grouped
			.entry(member.collection_id)
			.or_insert_with(Vec::new)
			.push(member.series_id);
	}
	grouped
}

fn to_utc(timestamp: DateTimeWithTimeZone) -> DateTime<Utc> {
	timestamp.with_timezone(&Utc)
}

fn map_read_list(model: reading_list::Model, book_ids: Vec<String>) -> KomgaReadList {
	let updated_at = to_utc(model.updated_at);
	KomgaReadList {
		id: KomgaReadListId::from(model.id),
		name: model.name,
		summary: model.description.unwrap_or_default(),
		ordered: model.ordering.eq_ignore_ascii_case("MANUAL"),
		book_ids: book_ids.into_iter().map(KomgaBookId::from).collect(),
		created_date: updated_at,
		last_modified_date: updated_at,
		filtered: false,
	}
}

fn map_collection(model: collection::Model, series_ids: Vec<String>) -> KomgaCollection {
	let updated_at = to_utc(model.updated_at);
	KomgaCollection {
		id: KomgaCollectionId::from(model.id),
		name: model.name,
		ordered: model.ordered,
		series_ids: series_ids.into_iter().map(KomgaSeriesId::from).collect(),
		created_date: updated_at,
		last_modified_date: updated_at,
		filtered: false,
	}
}

async fn map_visible_books(
	conn: &DatabaseConnection,
	user: &AuthUser,
	book_ids: Vec<String>,
) -> APIResult<Vec<crate::KomgaBook>> {
	if book_ids.is_empty() {
		return Ok(Vec::new());
	}

	let models = media::ModelWithMetadata::find_for_user(user)
		.filter(media::Column::Id.is_in(book_ids.clone()))
		.into_model::<media::ModelWithMetadata>()
		.all(conn)
		.await?;
	let mut by_id = models
		.into_iter()
		.map(|model| (model.media.id.clone(), model))
		.collect::<HashMap<_, _>>();
	let ordered_models = book_ids
		.into_iter()
		.filter_map(|book_id| by_id.remove(&book_id))
		.collect();
	map_books(conn, user, ordered_models).await
}

async fn map_visible_series(
	conn: &DatabaseConnection,
	user: &AuthUser,
	series_ids: Vec<String>,
) -> APIResult<Vec<crate::KomgaSeries>> {
	if series_ids.is_empty() {
		return Ok(Vec::new());
	}

	let models = series::ModelWithMetadata::find_for_user(user)
		.filter(series::Column::Id.is_in(series_ids.clone()))
		.into_model::<series::ModelWithMetadata>()
		.all(conn)
		.await?;
	let mut by_id = models
		.into_iter()
		.map(|model| (model.series.id.clone(), model))
		.collect::<HashMap<_, _>>();
	let ordered_models = series_ids
		.into_iter()
		.filter_map(|series_id| by_id.remove(&series_id))
		.collect();
	map_series(conn, user, ordered_models).await
}

async fn get_readlists(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(pagination): Query<PaginationQuery>,
	Query(search): Query<SearchQuery>,
	Query(filters): Query<KomgaReadListQuery>,
	Query(library_filter): Query<LibraryFilterQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let sorts = parse_sort_specs(&pagination.sort)?;
	let pagination = pagination.validate()?;
	let mut filters_to_validate = filters.clone();
	if filters_to_validate.deleted == Some(false) {
		filters_to_validate.deleted = None;
	}
	validate_read_list_filters(&filters_to_validate)?;
	if nonempty(search.condition.as_deref()) {
		return Err(unsupported_filter("condition"));
	}
	if nonempty(search.full_text_search.as_deref())
		|| nonempty(search.legacy_full_text_search.as_deref())
	{
		return Err(unsupported_filter("fullTextSearch"));
	}
	let user = auth.user();
	let search = search
		.search
		.as_deref()
		.map(str::trim)
		.filter(|term| !term.is_empty())
		.map(ToOwned::to_owned);
	let search_present = search.is_some();
	let library_ids = library_filter.ids();

	let mut query =
		reading_list::Entity::find_for_user(&user, READING_LIST_READER_ROLE).distinct();
	if let Some(term) = search {
		query = query.filter(
			Condition::any()
				.add(reading_list::Column::Name.contains(term.clone()))
				.add(reading_list::Column::Description.contains(term)),
		);
	}
	if let Some(library_ids) = library_ids.as_deref() {
		let list_ids = reading_list_item::Entity::find()
			.filter(
				reading_list_item::Column::MediaId
					.in_subquery(visible_media_ids_subquery(&user, Some(library_ids))),
			)
			.select_only()
			.column(reading_list_item::Column::ReadingListId)
			.into_query();
		query = query.filter(reading_list::Column::Id.in_subquery(list_ids));
	}
	query = apply_read_list_order(query, &sorts, search_present);

	let total = total_as_i32(query.clone().count(ctx.conn()).await?)?;
	let query = if pagination.unpaged {
		query
	} else {
		query
			.offset(pagination.offset())
			.limit(pagination.size as u64)
	};
	let models = query.all(ctx.conn()).await?;
	let list_ids = models
		.iter()
		.map(|model| model.id.clone())
		.collect::<Vec<_>>();
	let items =
		visible_reading_items(ctx.conn(), &user, &list_ids, library_ids.as_deref())
			.await?;
	let mut grouped = group_reading_items(items);
	let content = models
		.into_iter()
		.map(|model| {
			let book_ids = grouped.remove(&model.id).unwrap_or_default();
			map_read_list(model, book_ids)
		})
		.collect::<Vec<_>>();

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
async fn tachiyomi_readlist_books(
	conn: &DatabaseConnection,
	user: &AuthUser,
	id: &str,
) -> APIResult<Vec<crate::routes::progress::ProgressBook>> {
	reading_list::Entity::find_for_user_and_id(user, READING_LIST_READER_ROLE, id)
		.distinct()
		.one(conn)
		.await?
		.ok_or_else(|| APIError::NotFound("Reading list not found".to_owned()))?;
	let items =
		visible_reading_items(conn, user, std::slice::from_ref(&id.to_owned()), None)
			.await?;
	let ids = items
		.iter()
		.map(|item| item.media_id.clone())
		.collect::<Vec<_>>();
	if ids.is_empty() {
		return Ok(Vec::new());
	}
	let rows = media::ModelWithMetadata::find_for_user(user)
		.filter(media::Column::Id.is_in(ids.clone()))
		.filter(media::Column::DeletedAt.is_null())
		.into_model::<media::ModelWithMetadata>()
		.all(conn)
		.await?;
	let mut by_id = rows
		.into_iter()
		.map(|book| (book.media.id.clone(), book))
		.collect::<HashMap<_, _>>();
	Ok(items
		.into_iter()
		.filter_map(|item| by_id.remove(&item.media_id))
		.map(|book| progress_books([book]).remove(0))
		.collect())
}

async fn get_tachiyomi_readlist_progress(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<Json<KomgaReadProgressDto>> {
	let user = auth.user();
	let books = tachiyomi_readlist_books(ctx.conn(), &user, &id).await?;
	let counts = counts_for_progress_books(ctx.conn(), &user, &books, false).await?;
	Ok(Json(KomgaReadProgressDto {
		books_count: counts.books_count,
		books_read_count: counts.books_read_count,
		books_unread_count: counts.books_unread_count,
		books_in_progress_count: counts.books_in_progress_count,
		last_read_continuous_index: counts.continuous_prefix_count,
	}))
}

async fn update_tachiyomi_readlist_progress(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<KomgaEvents>,
	Json(update): Json<KomgaReadProgressUpdateDto>,
) -> APIResult<StatusCode> {
	if update.last_book_read < 0 {
		return Err(APIError::BadRequest(
			"lastBookRead must be a non-negative index".to_owned(),
		));
	}
	let user = auth.user();
	let books = tachiyomi_readlist_books(ctx.conn(), &user, &id).await?;
	let to_mark = books
		.into_iter()
		.take(update.last_book_read as usize)
		.collect::<Vec<_>>();
	if to_mark.is_empty() {
		return Ok(StatusCode::NO_CONTENT);
	}
	let series_ids = to_mark
		.iter()
		.filter_map(|book| book.series_id.clone())
		.collect::<Vec<_>>();
	let parent_series = series::Entity::find_for_user(&user)
		.filter(series::Column::Id.is_in(series_ids))
		.all(ctx.conn())
		.await?
		.into_iter()
		.map(|series| (series.id, series.library_id.unwrap_or_default()))
		.collect::<HashMap<_, _>>();
	let txn = begin_write(ctx.conn()).await?;
	for book in &to_mark {
		mark_book_read(&txn, &user, &book.id, book.pages).await?;
	}
	txn.commit().await?;
	for book in &to_mark {
		let series_id = book.series_id.clone().unwrap_or_default();
		let library_id = parent_series.get(&series_id).cloned().unwrap_or_default();
		tracing::debug!(
			book_id = %book.id,
			series_id = %series_id,
			readlist_id = %id,
			"Marked book read from Tachiyomi readlist progress"
		);
		events.send(KomgaEvent::ReadProgressChanged {
			book_id: book.id.clone().into(),
			user_id: user.id.clone().into(),
		});
		events.send(KomgaEvent::ReadProgressSeriesChanged {
			series_id: series_id.clone().into(),
			user_id: user.id.clone().into(),
		});
		events.send(KomgaEvent::BookChanged {
			book_id: book.id.clone().into(),
			series_id: series_id.clone().into(),
			library_id: library_id.clone().into(),
		});
		events.send(KomgaEvent::SeriesChanged {
			series_id: series_id.into(),
			library_id: library_id.into(),
		});
	}
	Ok(StatusCode::NO_CONTENT)
}

async fn get_readlist(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	let model =
		reading_list::Entity::find_for_user_and_id(&user, READING_LIST_READER_ROLE, &id)
			.distinct()
			.one(ctx.conn())
			.await?
			.ok_or_else(|| APIError::NotFound("Reading list not found".to_owned()))?;
	let items =
		visible_reading_items(ctx.conn(), &user, std::slice::from_ref(&id), None).await?;
	let book_ids = items.into_iter().map(|item| item.media_id).collect();

	cached_json(&headers, &map_read_list(model, book_ids))
}

async fn get_readlist_thumbnail(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	reading_list::Entity::find_for_user_and_id(&user, READING_LIST_READER_ROLE, &id)
		.distinct()
		.one(ctx.conn())
		.await?
		.ok_or_else(|| APIError::NotFound("Reading list not found".to_owned()))?;

	let items =
		visible_reading_items(ctx.conn(), &user, std::slice::from_ref(&id), None).await?;
	for item in items {
		let book_id = item.media_id;
		let visible = media::Entity::find_for_user(&user)
			.filter(media::Column::DeletedAt.is_null())
			.filter(series::Column::DeletedAt.is_null())
			.filter(media::Column::Id.eq(book_id.clone()))
			.select_only()
			.column(media::Column::Id)
			.into_tuple::<String>()
			.one(ctx.conn())
			.await?;
		if visible.is_some() {
			let image = ctx.book_thumbnail(&user, book_id).await?;
			return super::media::cache_image(&headers, image);
		}
	}

	Err(APIError::NotFound(
		"Reading list has no visible books with a thumbnail".to_owned(),
	))
}

async fn get_collection_thumbnail(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	collection::Entity::find_by_id(id.clone())
		.one(ctx.conn())
		.await?
		.ok_or_else(|| APIError::NotFound("Collection not found".to_owned()))?;

	let member =
		visible_collection_series(ctx.conn(), &user, std::slice::from_ref(&id), None)
			.await?
			.into_iter()
			.next()
			.ok_or_else(|| {
				APIError::NotFound(
					"Collection has no visible series with a thumbnail".to_owned(),
				)
			})?;
	let image = ctx.series_thumbnail(&user, &member.series_id).await?;
	super::media::cache_image(&headers, image)
}

#[allow(clippy::too_many_arguments)] // Mirrors the Komga DTO field set.
async fn get_readlist_books(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	Query(pagination): Query<PaginationQuery>,
	Query(search): Query<SearchQuery>,
	Query(filters): Query<KomgaReadListQuery>,
	Query(library_filter): Query<LibraryFilterQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let pagination = pagination.validate()?;
	let mut filters_to_validate = filters.clone();
	if filters_to_validate.deleted == Some(false) {
		filters_to_validate.deleted = None;
	}
	if filters_to_validate.media_status == Some(vec![KomgaMediaStatus::Ready]) {
		filters_to_validate.media_status = None;
	}
	validate_search(&search)?;
	validate_read_list_filters(&filters_to_validate)?;
	let user = auth.user();
	let library_ids = library_filter.ids();
	reading_list::Entity::find_for_user_and_id(&user, READING_LIST_READER_ROLE, &id)
		.distinct()
		.one(ctx.conn())
		.await?
		.ok_or_else(|| APIError::NotFound("Reading list not found".to_owned()))?;

	let query = visible_reading_items_query(
		&user,
		std::slice::from_ref(&id),
		library_ids.as_deref(),
	);
	let total = total_as_i32(query.clone().count(ctx.conn()).await?)?;
	let query = if pagination.unpaged {
		query
	} else {
		query
			.offset(pagination.offset())
			.limit(pagination.size as u64)
	};
	let items = query.all(ctx.conn()).await?;
	let book_ids = items.into_iter().map(|item| item.media_id).collect();
	let books = map_visible_books(ctx.conn(), &user, book_ids).await?;

	cached_json(
		&headers,
		&Page::new(
			books,
			pagination.page,
			pagination.size,
			total,
			pagination.unpaged,
		),
	)
}

/// Komga returns a plain array here (`getAllReadListsByBook` in the pinned
/// client), not a Spring page.
async fn get_book_readlists(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	let visible_book = media::Entity::find_for_user(&user)
		.filter(media::Column::Id.eq(id.clone()))
		.one(ctx.conn())
		.await?;
	if visible_book.is_none() {
		return Err(APIError::NotFound("Book not found".to_owned()));
	}

	let list_ids = reading_list_item::Entity::find()
		.filter(reading_list_item::Column::MediaId.eq(id))
		.select_only()
		.column(reading_list_item::Column::ReadingListId)
		.into_query();
	let models = reading_list::Entity::find_for_user(&user, READING_LIST_READER_ROLE)
		.distinct()
		.filter(reading_list::Column::Id.in_subquery(list_ids))
		.order_by_asc(reading_list::Column::Id)
		.all(ctx.conn())
		.await?;
	let ids = models
		.iter()
		.map(|model| model.id.clone())
		.collect::<Vec<_>>();
	let items = visible_reading_items(ctx.conn(), &user, &ids, None).await?;
	let mut grouped = group_reading_items(items);
	let content = models
		.into_iter()
		.map(|model| {
			let book_ids = grouped.remove(&model.id).unwrap_or_default();
			map_read_list(model, book_ids)
		})
		.collect::<Vec<_>>();

	cached_json(&headers, &content)
}

async fn get_collections(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(pagination): Query<PaginationQuery>,
	Query(search): Query<SearchQuery>,
	Query(filters): Query<KomgaCollectionQuery>,
	Query(library_filter): Query<LibraryFilterQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let sorts = parse_sort_specs(&pagination.sort)?;
	let pagination = pagination.validate()?;
	validate_search(&search)?;
	validate_collection_filters(&filters)?;
	let user = auth.user();
	let library_ids = library_filter.ids();
	let mut query = collection::Entity::find();
	if let Some(library_ids) = library_ids.as_deref() {
		let collection_ids = collection_series::Entity::find()
			.filter(
				collection_series::Column::SeriesId
					.in_subquery(visible_series_ids_subquery(&user, Some(library_ids))),
			)
			.select_only()
			.column(collection_series::Column::CollectionId)
			.into_query();
		query = query.filter(collection::Column::Id.in_subquery(collection_ids));
	}
	query = apply_collection_order(query, &sorts);

	let total = total_as_i32(query.clone().count(ctx.conn()).await?)?;
	let query = if pagination.unpaged {
		query
	} else {
		query
			.offset(pagination.offset())
			.limit(pagination.size as u64)
	};
	let models = query.all(ctx.conn()).await?;
	let collection_ids = models
		.iter()
		.map(|model| model.id.clone())
		.collect::<Vec<_>>();
	let members = visible_collection_series(
		ctx.conn(),
		&user,
		&collection_ids,
		library_ids.as_deref(),
	)
	.await?;
	let mut grouped = group_collection_series(members);
	let content = models
		.into_iter()
		.map(|model| {
			let series_ids = grouped.remove(&model.id).unwrap_or_default();
			map_collection(model, series_ids)
		})
		.collect::<Vec<_>>();

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

async fn get_collection(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	let model = collection::Entity::find_by_id(id.clone())
		.one(ctx.conn())
		.await?
		.ok_or_else(|| APIError::NotFound("Collection not found".to_owned()))?;
	let members =
		visible_collection_series(ctx.conn(), &user, std::slice::from_ref(&id), None)
			.await?;
	let series_ids = members.into_iter().map(|member| member.series_id).collect();

	cached_json(&headers, &map_collection(model, series_ids))
}

#[allow(clippy::too_many_arguments)] // Mirrors the Komga DTO field set.
async fn get_collection_series(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	Query(pagination): Query<PaginationQuery>,
	Query(search): Query<SearchQuery>,
	Query(filters): Query<KomgaCollectionQuery>,
	Query(library_filter): Query<LibraryFilterQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let pagination = pagination.validate()?;
	let mut filters_to_validate = filters.clone();
	if filters_to_validate.deleted == Some(false) {
		filters_to_validate.deleted = None;
	}
	validate_search(&search)?;
	validate_collection_filters(&filters_to_validate)?;
	let user = auth.user();
	let library_ids = library_filter.ids();
	collection::Entity::find_by_id(id.clone())
		.one(ctx.conn())
		.await?
		.ok_or_else(|| APIError::NotFound("Collection not found".to_owned()))?;

	let query = visible_collection_series_query(
		&user,
		std::slice::from_ref(&id),
		library_ids.as_deref(),
	);
	let total = total_as_i32(query.clone().count(ctx.conn()).await?)?;
	let query = if pagination.unpaged {
		query
	} else {
		query
			.offset(pagination.offset())
			.limit(pagination.size as u64)
	};
	let members = query.all(ctx.conn()).await?;
	let ids = members
		.into_iter()
		.map(|member| member.series_id)
		.collect::<Vec<_>>();
	let series = map_visible_series(ctx.conn(), &user, ids).await?;

	cached_json(
		&headers,
		&Page::new(
			series,
			pagination.page,
			pagination.size,
			total,
			pagination.unpaged,
		),
	)
}

/// Komga returns a plain array here (`getAllCollectionsBySeries` in the pinned
/// client), not a Spring page.
async fn get_series_collections(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	let visible_series = series::Entity::find_for_user(&user)
		.filter(series::Column::Id.eq(id.clone()))
		.one(ctx.conn())
		.await?;
	if visible_series.is_none() {
		return Err(APIError::NotFound("Series not found".to_owned()));
	}

	let collection_ids = collection_series::Entity::find()
		.filter(collection_series::Column::SeriesId.eq(id))
		.select_only()
		.column(collection_series::Column::CollectionId)
		.into_query();
	let models = collection::Entity::find()
		.filter(collection::Column::Id.in_subquery(collection_ids))
		.order_by_asc(collection::Column::Id)
		.all(ctx.conn())
		.await?;
	let ids = models
		.iter()
		.map(|model| model.id.clone())
		.collect::<Vec<_>>();
	let members = visible_collection_series(ctx.conn(), &user, &ids, None).await?;
	let mut grouped = group_collection_series(members);
	let content = models
		.into_iter()
		.map(|model| {
			let series_ids = grouped.remove(&model.id).unwrap_or_default();
			map_collection(model, series_ids)
		})
		.collect::<Vec<_>>();

	cached_json(&headers, &content)
}

/// Creates a collection (Komga `POST /api/v1/collections`). Validation and
/// the change event live in the canonical container service; the route
/// renders the DTO from the created row plus the requested membership.
async fn create_collection(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(request): Json<KomgaCollectionCreateRequest>,
) -> APIResult<Json<KomgaCollection>> {
	let user = auth.user();
	let series_ids: Vec<String> =
		request.series_ids.iter().map(|id| id.0.clone()).collect();
	let model = ctx
		.create_collection(&user, request.name, request.ordered, series_ids.clone())
		.await?;
	Ok(Json(map_collection(model, series_ids)))
}

fn collection_patch_fields(
	patch: &KomgaCollectionUpdateRequest,
) -> (Option<String>, Option<bool>, Option<Vec<String>>) {
	let name = match &patch.name {
		crate::PatchValue::Some(name) => Some(name.clone()),
		_ => None,
	};
	let ordered = match &patch.ordered {
		crate::PatchValue::Some(ordered) => Some(*ordered),
		_ => None,
	};
	let series_ids = match &patch.series_ids {
		crate::PatchValue::Some(ids) => Some(ids.iter().map(|id| id.0.clone()).collect()),
		_ => None,
	};
	(name, ordered, series_ids)
}

async fn patch_collection(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	Json(patch): Json<KomgaCollectionUpdateRequest>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let (name, ordered, series_ids) = collection_patch_fields(&patch);
	ctx.update_collection(&user, &id, name, ordered, series_ids)
		.await?;
	Ok(StatusCode::NO_CONTENT)
}

async fn delete_collection(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
) -> APIResult<StatusCode> {
	ctx.delete_collection(&auth.user(), &id).await?;
	Ok(StatusCode::NO_CONTENT)
}

/// Creates a reading list (Komga `POST /api/v1/readlists`).
async fn create_readlist(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(request): Json<KomgaReadListCreateRequest>,
) -> APIResult<Json<KomgaReadList>> {
	let user = auth.user();
	let book_ids: Vec<String> = request.book_ids.iter().map(|id| id.0.clone()).collect();
	let model = ctx
		.create_read_list(
			&user,
			request.name,
			Some(request.summary),
			request.ordered,
			book_ids.clone(),
		)
		.await?;
	Ok(Json(map_read_list(model, book_ids)))
}

fn read_list_patch_fields(
	patch: &KomgaReadListUpdateRequest,
) -> (
	Option<String>,
	Option<Option<String>>,
	Option<bool>,
	Option<Vec<String>>,
) {
	let name = match &patch.name {
		crate::PatchValue::Some(name) => Some(name.clone()),
		_ => None,
	};
	let summary = match &patch.summary {
		crate::PatchValue::Some(summary) => Some(Some(summary.clone())),
		crate::PatchValue::None => Some(None),
		crate::PatchValue::Unset => None,
	};
	let ordered = match &patch.ordered {
		crate::PatchValue::Some(ordered) => Some(*ordered),
		_ => None,
	};
	let book_ids = match &patch.book_ids {
		crate::PatchValue::Some(ids) => Some(ids.iter().map(|id| id.0.clone()).collect()),
		_ => None,
	};
	(name, summary, ordered, book_ids)
}

async fn patch_readlist(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	Json(patch): Json<KomgaReadListUpdateRequest>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let (name, summary, ordered, book_ids) = read_list_patch_fields(&patch);
	ctx.update_read_list(&user, &id, name, summary, ordered, book_ids)
		.await?;
	Ok(StatusCode::NO_CONTENT)
}

async fn delete_readlist(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
) -> APIResult<StatusCode> {
	ctx.delete_read_list(&auth.user(), &id).await?;
	Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::PatchValue;

	#[test]
	fn pagination_defaults_to_the_komga_page_shape() {
		let pagination = PaginationQuery {
			page: 0,
			size: default_page_size(),
			unpaged: false,
			sort: Vec::new(),
		}
		.validate()
		.unwrap();
		assert_eq!(pagination.page, 0);
		assert_eq!(pagination.size, 20);
		assert!(!pagination.unpaged);
	}

	#[test]
	fn pagination_rejects_values_outside_the_contract() {
		for (page, size) in [(0, -1), (-1, 20)] {
			assert!(PaginationQuery {
				page,
				size,
				unpaged: false,
				sort: Vec::new(),
			}
			.validate()
			.is_err());
		}
		// size=0 is Komga's count-only idiom and stays valid.
		let pagination = PaginationQuery {
			page: 0,
			size: 0,
			unpaged: false,
			sort: Vec::new(),
		}
		.validate()
		.expect("count-only size is valid");
		assert_eq!(pagination.size, 0);
		// Oversized pages clamp instead of failing.
		let pagination = PaginationQuery {
			page: 0,
			size: 201,
			unpaged: false,
			sort: Vec::new(),
		}
		.validate()
		.expect("oversized size clamps");
		assert_eq!(pagination.size, MAX_PAGE_SIZE);
	}

	#[test]
	fn list_sort_parser_accepts_repeated_fields_and_default_direction() {
		assert_eq!(
			parse_sort_specs(&[
				"name".to_owned(),
				"createdDate,desc".to_owned(),
				"lastModifiedDate".to_owned(),
			])
			.unwrap(),
			vec![
				SortSpec {
					field: "name".to_owned(),
					direction: SortDirection::Asc,
				},
				SortSpec {
					field: "createddate".to_owned(),
					direction: SortDirection::Desc,
				},
				SortSpec {
					field: "lastmodifieddate".to_owned(),
					direction: SortDirection::Asc,
				},
			]
		);
	}

	#[test]
	fn list_sort_parser_rejects_unknown_fields_directions_and_shapes() {
		for value in ["rank,asc", "name,sideways", "name,asc,extra", ",asc"] {
			assert!(matches!(
				parse_sort_specs(&[value.to_owned()]),
				Err(APIError::BadRequest(message)) if message.contains("sort")
			));
		}
	}

	#[test]
	fn collection_patch_maps_dto_fields_onto_the_service_call() {
		let patch = KomgaCollectionUpdateRequest {
			name: PatchValue::Some("Renamed".to_owned()),
			ordered: PatchValue::Some(true),
			series_ids: PatchValue::Some(vec![
				KomgaSeriesId::from("series-1"),
				KomgaSeriesId::from("series-2"),
			]),
		};
		let (name, ordered, series_ids) = collection_patch_fields(&patch);
		assert_eq!(name.as_deref(), Some("Renamed"));
		assert_eq!(ordered, Some(true));
		assert_eq!(
			series_ids,
			Some(vec!["series-1".to_owned(), "series-2".to_owned()])
		);

		// Unset fields stay None so the service leaves them untouched, and an
		// explicit empty membership clears the container.
		let patch = KomgaCollectionUpdateRequest {
			name: PatchValue::Unset,
			ordered: PatchValue::Unset,
			series_ids: PatchValue::Some(Vec::new()),
		};
		let (name, ordered, series_ids) = collection_patch_fields(&patch);
		assert!(name.is_none() && ordered.is_none());
		assert_eq!(series_ids, Some(Vec::new()));
	}

	#[test]
	fn read_list_patch_distinguishes_null_summary_from_unset() {
		let patch = KomgaReadListUpdateRequest {
			name: PatchValue::Unset,
			summary: PatchValue::None,
			ordered: PatchValue::Unset,
			book_ids: PatchValue::Some(vec![KomgaBookId::from("book-1")]),
		};
		let (name, summary, ordered, book_ids) = read_list_patch_fields(&patch);
		assert!(name.is_none() && ordered.is_none());
		assert_eq!(summary, Some(None));
		assert_eq!(book_ids, Some(vec!["book-1".to_owned()]));
	}

	#[test]
	fn unsupported_nonempty_filters_are_not_ignored() {
		let query = KomgaReadListQuery {
			library_ids: None,
			read_status: Some(vec![crate::KomgaReadStatus::Unread]),
			tags: None,
			media_status: None,
			deleted: None,
			authors: None,
		};
		assert!(matches!(
			validate_read_list_filters(&query),
			Err(APIError::BadRequest(message)) if message.contains("readStatus")
		));
	}
}
