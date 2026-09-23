//! `SeriesController`, `VolumeController` and `ChapterController` reads.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
	extract::{Path, Query},
	http::{header, HeaderValue, StatusCode},
	response::{IntoResponse, Response},
	routing::{get, post},
	Extension, Json, Router,
};
use models::entity::{
	kavita_on_deck_removal, library_config, media, series, user::AuthUser,
};
use sea_orm::{prelude::*, Order, QueryOrder, QuerySelect};
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::{
	dto::{
		ChapterDto, ChapterMetadataUpdateDto, GroupedSeriesDto, PaginationHeader,
		RefreshSeriesDto, SeriesByIdsDto, SeriesDetailDto, SeriesDto, SeriesMetadataDto,
		SeriesMetadataUpdateRequestDto, SeriesUpdateDto, VolumeDto,
	},
	errors::{APIError, APIResult},
	filter::SeriesFilterV2Dto,
	ids::{IdKind, KavitaIds, LOOKUP_CHUNK},
	mapper::{
		library_type, map_chapter, map_series, map_series_detail, map_series_metadata,
		map_volume, SeriesInput,
	},
};

use super::{
	query::{
		book_library_ids, find_media, find_series_input, library_for_series,
		load_by_keys, load_kavita_series, on_deck_removals, resolve_series_key,
		restrict_plan, select_series_keys, SeriesKey,
	},
	route_ci,
	series_filter::{plan, FilterPlan, ProgressSort, SortKey},
	KavitaBackend, KavitaSeriesTarget,
};

/// `UserParams`: `PageNumber` defaults to 1, `PageSize` to "everything".
/// Query names are matched case-insensitively (the extension sends
/// `pageNumber`, Turnleaf `PageNumber`).
///
/// Kavita's `PagedList` echoes the requested `PageNumber` verbatim in the
/// `Pagination` header (Kamigura's dashboard sends `PageNumber=0`), while the
/// skip math clamps it like SQLite clamps a negative `OFFSET`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UserParams {
	/// Clamped to `>= 1`; drives the skip offset.
	pub page_number: i32,
	/// The requested value verbatim (`1` when absent); echoed in the header.
	pub requested_page_number: i32,
	pub page_size: i32,
}

impl UserParams {
	pub const MAX_PAGE_SIZE: i32 = i32::MAX;

	pub fn parse(query: &str) -> Self {
		let mut page_number = 1;
		let mut page_size = Self::MAX_PAGE_SIZE;
		for (key, value) in query
			.split('&')
			.filter_map(|pair| pair.split_once('=').or(Some((pair, ""))))
		{
			let value = value.trim().parse::<i32>().ok();
			match key.trim().to_ascii_lowercase().as_str() {
				"pagenumber" => page_number = value.unwrap_or(1),
				"pagesize" => {
					page_size = match value {
						Some(0) | None => Self::MAX_PAGE_SIZE,
						Some(size) if size < 0 => Self::MAX_PAGE_SIZE,
						Some(size) => size,
					}
				},
				_ => {},
			}
		}
		Self {
			page_number: page_number.max(1),
			requested_page_number: page_number,
			page_size,
		}
	}

	pub(crate) fn offset(&self) -> usize {
		if self.page_size == Self::MAX_PAGE_SIZE {
			0
		} else {
			usize::try_from(self.page_number - 1).unwrap_or(0)
				* usize::try_from(self.page_size).unwrap_or(0)
		}
	}
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeriesIdQuery {
	#[serde(default)]
	series_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VolumeIdQuery {
	#[serde(default)]
	volume_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterIdQuery {
	#[serde(default)]
	chapter_id: Option<i32>,
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(router, "/api/Series/all-v2", post(series_all_v2));
	let router = route_ci(router, "/api/Series/update", post(series_update));
	let router = route_ci(router, "/api/Series/metadata", post(series_metadata_update));
	let router = route_ci(router, "/api/Chapter/update", post(chapter_update));
	let router = route_ci(router, "/api/Series/v2", post(series_v2));
	let router = route_ci(router, "/api/Series/on-deck", post(series_on_deck));
	let router = route_ci(
		router,
		"/api/Series/recently-added-v2",
		post(series_recently_added_v2),
	);
	let router = route_ci(
		router,
		"/api/Series/recently-updated-series",
		post(series_recently_updated),
	);
	let router = route_ci(
		router,
		"/api/Series/remove-from-on-deck",
		post(series_remove_from_on_deck),
	);
	let router = route_ci(router, "/api/Series/volumes", get(series_volumes));
	let router = route_ci(router, "/api/Series/volume", get(volume_by_query));
	let router = route_ci(router, "/api/Series/metadata", get(series_metadata));
	let router = route_ci(router, "/api/Series/series-detail", get(series_detail));
	let router = route_ci(router, "/api/Series/chapter", get(chapter_by_query));
	let router = route_ci(router, "/api/Series/scan", post(series_scan));
	let router = route_ci(router, "/api/Series/analyze", post(series_analyze));
	let router = route_ci(router, "/api/Series/series-by-ids", post(series_by_ids));
	let router = route_ci(
		router,
		"/api/Series/refresh-metadata",
		post(series_refresh_metadata),
	);
	let router = route_ci(router, "/api/Series/{seriesId}", get(series_by_id));
	let router = route_ci(router, "/api/Volume", get(volume_by_query));
	let router = route_ci(router, "/api/Volume/{volumeId}", get(volume_by_path));
	route_ci(router, "/api/Chapter", get(chapter_by_query))
}
pub(super) fn target_for_input(input: &SeriesInput) -> KavitaSeriesTarget {
	match input.key() {
		SeriesKey::Series(id) => KavitaSeriesTarget::Series(id),
		SeriesKey::Book(id) => KavitaSeriesTarget::Book(id),
	}
}

async fn series_update(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(update): Json<SeriesUpdateDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let Some(input) = find_series_input(ctx.as_ref(), &user, update.id).await? else {
		return Err(APIError::NotFound("Series does not exist".to_owned()));
	};
	ctx.update_series(&user, target_for_input(&input), update)
		.await?;
	Ok(StatusCode::OK)
}

async fn series_metadata_update(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(update): Json<SeriesMetadataUpdateRequestDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let series_id = update.series_metadata.series_id;
	let Some(input) = find_series_input(ctx.as_ref(), &user, series_id).await? else {
		return Err(APIError::NotFound("Series does not exist".to_owned()));
	};
	ctx.update_series_metadata(&user, target_for_input(&input), update)
		.await?;
	Ok(StatusCode::OK)
}

async fn chapter_update(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(update): Json<ChapterMetadataUpdateDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let Some((input, index)) = find_media(ctx.as_ref(), &user, update.id).await? else {
		return Err(APIError::NotFound("Chapter does not exist".to_owned()));
	};
	let media_id = input
		.media
		.get(index)
		.map(|media| media.media.id.clone())
		.ok_or_else(|| APIError::NotFound("Chapter does not exist".to_owned()))?;
	ctx.update_chapter_metadata(&user, media_id, update).await?;
	Ok(StatusCode::OK)
}

pub(crate) fn pagination_response<T: serde::Serialize>(
	items: Vec<T>,
	header: PaginationHeader,
) -> APIResult<Response> {
	let mut response = Json(items).into_response();
	let value = serde_json::to_string(&header)?;
	response.headers_mut().insert(
		PaginationHeader::NAME,
		HeaderValue::from_str(&value)
			.map_err(|error| APIError::InternalServerError(error.to_string()))?,
	);
	Ok(response)
}
/// A page of series plus the `Pagination` header Kavita attaches. When
/// `primary_order` is set it becomes the leading sort, ahead of any sort the
/// filter carries (Kavita's `GetRecentlyAddedAsync` orders by `Created`
/// descending after the filter pipeline).
///
/// Series rows of Manga/Comic libraries and media rows of Book libraries are
/// selected, sorted, counted and paged in one SQL pass over their keys; only
/// the page is then loaded. Filters that need reading progress load every
/// match and page in memory.
/// `scope`, when set, additionally narrows the listing to those Kavita series
/// keys — the `want-to-read` feeds run the same filter pipeline over a
/// membership set.
pub(crate) async fn list_series(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	filter: &SeriesFilterV2Dto,
	params: UserParams,
	primary_order: Option<SortKey>,
	scope: Option<&[SeriesKey]>,
) -> APIResult<(Vec<SeriesDto>, PaginationHeader)> {
	let mut plan = plan(ctx.conn(), user, filter).await?;
	if let Some(scope) = scope {
		restrict_plan(&mut plan, scope);
	}
	let mut sort = plan.order.clone();
	if let Some(primary) = primary_order {
		sort.insert(0, primary);
	}
	let book_libraries = book_library_ids(ctx).await?;

	if plan.needs_memory_pass() {
		let (keys, _) =
			select_series_keys(ctx, user, &plan, &sort, &book_libraries, None).await?;
		let inputs = load_by_keys(ctx, user, &keys).await?;
		let (items, total) = paginate_in_memory(inputs, &plan, params);
		return Ok((
			items,
			PaginationHeader::new(params.requested_page_number, params.page_size, total),
		));
	}

	let page = (params.page_size != UserParams::MAX_PAGE_SIZE)
		.then(|| {
			Ok::<_, APIError>((
				u64::try_from(params.offset())?,
				u64::try_from(params.page_size)?,
			))
		})
		.transpose()?;
	let (keys, total) =
		select_series_keys(ctx, user, &plan, &sort, &book_libraries, page).await?;
	let inputs = load_by_keys(ctx, user, &keys).await?;
	let items = inputs.iter().map(map_series).collect();
	Ok((
		items,
		PaginationHeader::new(params.requested_page_number, params.page_size, total),
	))
}

fn progress_percentage(input: &SeriesInput) -> f32 {
	let pages = input.pages();
	if pages <= 0 {
		return 0.0;
	}
	input
		.media
		.iter()
		.map(|media| media.pages_read() as f32 / pages as f32)
		.sum::<f32>()
		* 100.0
}

fn unread_count(input: &SeriesInput) -> i32 {
	input
		.media
		.iter()
		.filter(|media| media.pages_read() < media.pages() || media.pages() == 0)
		.count() as i32
}

fn paginate_in_memory(
	inputs: Vec<SeriesInput>,
	plan: &FilterPlan,
	params: UserParams,
) -> (Vec<SeriesDto>, i32) {
	let mut inputs = inputs
		.into_iter()
		.filter(|input| {
			plan.progress
				.matches(progress_percentage(input), plan.combination)
		})
		.collect::<Vec<_>>();
	match plan.sort_by_progress {
		Some(ProgressSort::ReadProgress { ascending }) => {
			inputs.sort_by(|left, right| {
				let ordering = progress_percentage(left)
					.partial_cmp(&progress_percentage(right))
					.unwrap_or(std::cmp::Ordering::Equal);
				if ascending {
					ordering
				} else {
					ordering.reverse()
				}
			});
		},
		Some(ProgressSort::UnreadCount { ascending }) => {
			inputs.sort_by(|left, right| {
				let ordering = unread_count(left).cmp(&unread_count(right));
				if ascending {
					ordering
				} else {
					ordering.reverse()
				}
			});
		},
		None => {},
	}
	if plan.limit_to > 0 {
		inputs.truncate(usize::try_from(plan.limit_to).unwrap_or(usize::MAX));
	}
	let total = i32::try_from(inputs.len()).unwrap_or(i32::MAX);
	let page = if params.page_size == UserParams::MAX_PAGE_SIZE {
		inputs
	} else {
		inputs
			.into_iter()
			.skip(params.offset())
			.take(usize::try_from(params.page_size).unwrap_or(0))
			.collect()
	};
	(page.iter().map(map_series).collect(), total)
}

pub(crate) fn user_params(uri: &axum::http::Uri) -> UserParams {
	UserParams::parse(uri.query().unwrap_or_default())
}

/// The `libraryId` query parameter (Kavita id, `0` meaning all libraries).
fn library_id_param(uri: &axum::http::Uri) -> i32 {
	uri.query()
		.unwrap_or_default()
		.split('&')
		.find_map(|pair| {
			let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
			(key.eq_ignore_ascii_case("libraryId"))
				.then_some(())
				.and_then(|()| value.trim().parse::<i32>().ok())
		})
		.unwrap_or(0)
}

async fn series_all_v2(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	uri: axum::http::Uri,
	Json(filter): Json<SeriesFilterV2Dto>,
) -> APIResult<Response> {
	let user = auth.user();
	let (items, header) =
		list_series(ctx.as_ref(), &user, &filter, user_params(&uri), None, None).await?;
	pagination_response(items, header)
}

async fn series_v2(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	uri: axum::http::Uri,
	Json(filter): Json<SeriesFilterV2Dto>,
) -> APIResult<Response> {
	let user = auth.user();
	let (items, header) =
		list_series(ctx.as_ref(), &user, &filter, user_params(&uri), None, None).await?;
	pagination_response(items, header)
}

/// `SeriesController.GetOnDeck`: the user's in-progress series (`pagesRead > 0`
/// and `< pages`), newest progress first, then most recently added chapter.
/// Kavita additionally restricts this to recent activity (30 days of progress,
/// 7 days of chapter adds) and hides series removed via
/// `remove-from-on-deck`; Stump keeps the series as long as it is in progress
/// and honours the removals.
async fn series_on_deck(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	uri: axum::http::Uri,
) -> APIResult<Response> {
	let user = auth.user();
	let (items, header) = list_on_deck(
		ctx.as_ref(),
		&user,
		library_id_param(&uri),
		user_params(&uri),
	)
	.await?;
	pagination_response(items, header)
}

/// `SeriesController.GetRecentlyAddedV2`: the filter pipeline with `Created`
/// descending as the primary sort.
async fn series_recently_added_v2(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	uri: axum::http::Uri,
	Json(filter): Json<SeriesFilterV2Dto>,
) -> APIResult<Response> {
	let user = auth.user();
	let (items, header) = list_series(
		ctx.as_ref(),
		&user,
		&filter,
		user_params(&uri),
		Some(SortKey::created(Order::Desc)),
		None,
	)
	.await?;
	pagination_response(items, header)
}

/// `SeriesController.GetRecentlyAddedChapters`: chapters created within the
/// last 12 days, grouped per series with an added-chapter count, series in
/// creation order of their newest chapter.
async fn series_recently_updated(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	uri: axum::http::Uri,
) -> APIResult<Response> {
	let user = auth.user();
	let items = list_recently_updated(ctx.as_ref(), &user, user_params(&uri)).await?;
	Ok(Json(items).into_response())
}

/// `SeriesController.RemoveFromOnDeck`: hide a series from on-deck until the
/// next read event on it.
async fn series_remove_from_on_deck(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesIdQuery>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let Some(input) =
		find_series_input(ctx.as_ref(), &user, query.series_id.unwrap_or_default())
			.await?
	else {
		return Err(APIError::BadRequest("Series does not exist".to_owned()));
	};
	remove_from_on_deck(ctx.as_ref(), &user.id, &input.key()).await?;
	Ok(StatusCode::OK)
}

/// Record a Kavita series as removed from the user's on-deck.
pub(crate) async fn remove_from_on_deck(
	ctx: &dyn KavitaBackend,
	user_id: &str,
	key: &SeriesKey,
) -> APIResult<()> {
	let (target_kind, target_id) = key.target();
	kavita_on_deck_removal::Entity::insert(kavita_on_deck_removal::ActiveModel {
		user_id: sea_orm::Set(user_id.to_owned()),
		target_kind: sea_orm::Set(target_kind.to_owned()),
		target_id: sea_orm::Set(target_id.to_owned()),
		created_at: sea_orm::Set(chrono::Utc::now().into()),
	})
	.on_conflict(
		sea_orm::sea_query::OnConflict::columns([
			kavita_on_deck_removal::Column::UserId,
			kavita_on_deck_removal::Column::TargetKind,
			kavita_on_deck_removal::Column::TargetId,
		])
		.do_nothing()
		.to_owned(),
	)
	.do_nothing()
	.exec(ctx.conn())
	.await?;
	Ok(())
}

/// The on-deck listing behind [`series_on_deck`].
pub(crate) async fn list_on_deck(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	library_id: i32,
	params: UserParams,
) -> APIResult<(Vec<SeriesDto>, PaginationHeader)> {
	let mut query = series::ModelWithMetadata::find_for_user(user);
	if library_id > 0 {
		let Some(stump_library_id) =
			KavitaIds::lookup(ctx.conn(), IdKind::Library, library_id).await?
		else {
			return Ok((
				Vec::new(),
				PaginationHeader::new(params.requested_page_number, params.page_size, 0),
			));
		};
		query = query.filter(series::Column::LibraryId.eq(stump_library_id));
	}
	let rows = query
		.into_model::<series::ModelWithMetadata>()
		.all(ctx.conn())
		.await?;
	let mut inputs = load_kavita_series(ctx, user, rows).await?;

	// Series the user removed from on-deck stay hidden until the next read
	// event on them (`SeriesRepository.GetOnDeckAsync`).
	let removed = on_deck_removals(ctx, &user.id).await?;
	inputs.retain(|input| {
		let (pages, read) = (input.pages(), input.pages_read());
		read > 0 && read < pages && !removed.contains(&input.key())
	});
	inputs.sort_by(|left, right| {
		right
			.latest_read_at()
			.cmp(&left.latest_read_at())
			.then_with(|| {
				right
					.last_chapter_added_at()
					.cmp(&left.last_chapter_added_at())
			})
			.then_with(|| left.id.cmp(&right.id))
	});

	let total = i32::try_from(inputs.len())?;
	let page = if params.page_size == UserParams::MAX_PAGE_SIZE {
		inputs.iter().map(map_series).collect()
	} else {
		inputs
			.iter()
			.skip(params.offset())
			.take(usize::try_from(params.page_size).unwrap_or(0))
			.map(map_series)
			.collect()
	};
	Ok((
		page,
		PaginationHeader::new(params.requested_page_number, params.page_size, total),
	))
}

/// `SeriesRepository.ClearOnDeckRemovalAsync`: a read event on a series puts
/// it back on deck. Kavita fires this from `SaveReadingProgress` and the
/// mark-read handlers.
pub(crate) async fn clear_on_deck_removal(
	ctx: &dyn KavitaBackend,
	user_id: &str,
	key: &SeriesKey,
) -> APIResult<()> {
	let (target_kind, target_id) = key.target();
	kavita_on_deck_removal::Entity::delete_many()
		.filter(kavita_on_deck_removal::Column::UserId.eq(user_id))
		.filter(kavita_on_deck_removal::Column::TargetKind.eq(target_kind))
		.filter(kavita_on_deck_removal::Column::TargetId.eq(target_id))
		.exec(ctx.conn())
		.await?;
	Ok(())
}

/// The recently-updated listing behind [`series_recently_updated`], following
/// `SeriesRepository.GetRecentlyUpdatedSeriesAsync`: chapters created within
/// the last 12 days, newest first, grouped per Kavita series with an
/// added-chapter count (a book is its own group of one), paginated by series
/// with a 0-based group index per request.
pub(crate) async fn list_recently_updated(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	params: UserParams,
) -> APIResult<Vec<GroupedSeriesDto>> {
	// One Stump media item is one Kavita chapter (`VolumeExtensions`).
	let window_start = chrono::Utc::now() - chrono::Duration::days(12);
	let chapters = media::Entity::find_for_user(user)
		.filter(media::Column::DeletedAt.is_null())
		.filter(media::Column::SeriesId.is_not_null())
		.filter(media::Column::CreatedAt.gte(window_start))
		.order_by_desc(media::Column::CreatedAt)
		.order_by_desc(media::Column::Id)
		.all(ctx.conn())
		.await?;

	// A chapter of a Book library is a series of its own.
	let book_libraries = book_library_ids(ctx).await?;
	let mut series_ids = chapters
		.iter()
		.filter_map(|chapter| chapter.series_id.clone())
		.collect::<Vec<_>>();
	series_ids.sort();
	series_ids.dedup();
	let mut library_of_series: HashMap<String, String> =
		HashMap::with_capacity(series_ids.len());
	for chunk in series_ids.chunks(LOOKUP_CHUNK) {
		let rows = series::Entity::find()
			.select_only()
			.column(series::Column::Id)
			.column(series::Column::LibraryId)
			.filter(series::Column::Id.is_in(chunk.to_vec()))
			.into_tuple::<(String, Option<String>)>()
			.all(ctx.conn())
			.await?;
		library_of_series.extend(rows.into_iter().filter_map(|(id, library_id)| {
			library_id.map(|library_id| (id, library_id))
		}));
	}

	let mut order: Vec<SeriesKey> = Vec::new();
	let mut counts: HashMap<SeriesKey, i32> = HashMap::new();
	let mut created: HashMap<SeriesKey, DateTimeWithTimeZone> = HashMap::new();
	for chapter in &chapters {
		let Some(series_id) = chapter.series_id.clone() else {
			continue;
		};
		let is_book = library_of_series
			.get(&series_id)
			.is_some_and(|library_id| book_libraries.contains(library_id));
		let key = if is_book {
			SeriesKey::Book(chapter.id.clone())
		} else {
			SeriesKey::Series(series_id)
		};
		counts
			.entry(key.clone())
			.and_modify(|count| *count += 1)
			.or_insert_with(|| {
				order.push(key.clone());
				created.insert(key, chapter.created_at);
				1
			});
	}

	let take = usize::try_from(params.page_size).unwrap_or(0);
	let page_keys: Vec<SeriesKey> = order
		.iter()
		.skip(params.offset())
		.take(take)
		.cloned()
		.collect();
	if page_keys.is_empty() {
		return Ok(Vec::new());
	}

	// Series-level facts come from the same loaders as the other listings, so
	// visibility, ids, formats and library mapping stay consistent.
	let inputs = load_by_keys(ctx, user, &page_keys).await?;
	let by_key: HashMap<SeriesKey, &SeriesInput> =
		inputs.iter().map(|input| (input.key(), input)).collect();

	let library_ids: Vec<String> = inputs
		.iter()
		.filter_map(|input| input.series.library_id.clone())
		.collect();
	let configs: HashMap<String, library_config::Model> = library_config::Entity::find()
		.filter(library_config::Column::LibraryId.is_in(library_ids.clone()))
		.all(ctx.conn())
		.await?
		.into_iter()
		.filter_map(|config| {
			config
				.library_id
				.clone()
				.map(|library_id| (library_id, config))
		})
		.collect();

	let mut groups = Vec::with_capacity(page_keys.len());
	for (index, key) in page_keys.iter().enumerate() {
		let Some(input) = by_key.get(key) else {
			continue;
		};
		let config = input
			.series
			.library_id
			.as_deref()
			.and_then(|library_id| configs.get(library_id));
		groups.push(GroupedSeriesDto {
			series_name: match input.book() {
				Some(_) => input.name(),
				None => input.series.name.clone(),
			},
			localized_series_name: String::new(),
			series_id: input.id,
			library_id: input.library_id,
			library_type: library_type(config),
			created: created[key].into(),
			chapter_id: 0,
			volume_id: 0,
			id: index as i32,
			format: input.format(),
			count: counts[key],
		});
	}
	Ok(groups)
}

/// The Stump series a `RefreshSeriesDto` names, and its folder.
///
/// Kavita answers `200` for a series that does not exist (`ScanSeries` logs
/// and returns), so an unresolvable body is a no-op here too rather than an
/// error — that is what Kamigura's pull-to-refresh relies on.
async fn refresh_target(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	body: &RefreshSeriesDto,
) -> APIResult<Option<(String, String)>> {
	let Some(input) = find_series_input(ctx, user, body.series_id).await? else {
		return Ok(None);
	};
	// A Kavita series in a Book library is one file; rescanning it means
	// rescanning the Stump series that holds it.
	Ok(Some((input.series.id.clone(), input.series.path.clone())))
}

/// `SeriesController.ScanSeries`: rescan one series' folder. Stump's
/// series-scan job is the same job the native API and Komga's library scan
/// enqueue, scoped to this series.
async fn series_scan(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<RefreshSeriesDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	if let Some((id, path)) = refresh_target(ctx.as_ref(), &user, &body).await? {
		ctx.enqueue_series_scan(id, path, body.force_update).await?;
	}
	Ok(StatusCode::OK)
}

/// `SeriesController.Analyze`: re-read every file of the series. Stump's
/// media-analysis job scoped to the series, the same job Komga's
/// `/api/v1/series/{id}/analyze` enqueues.
async fn series_analyze(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<RefreshSeriesDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	if let Some((id, _)) = refresh_target(ctx.as_ref(), &user, &body).await? {
		ctx.enqueue_series_analysis(id).await?;
	}
	Ok(StatusCode::OK)
}

/// `SeriesController.RefreshSeriesMetadata`: re-read the series' metadata
/// from disk. Stump reads metadata during a scan, so this is a forced
/// series scan — the same mapping Komga's `refresh-metadata` uses.
async fn series_refresh_metadata(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<RefreshSeriesDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	if let Some((id, path)) = refresh_target(ctx.as_ref(), &user, &body).await? {
		ctx.enqueue_series_scan(id, path, true).await?;
	}
	Ok(StatusCode::OK)
}

async fn series_by_id(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(series_id): Path<i32>,
) -> APIResult<Json<SeriesDto>> {
	let user = auth.user();
	let input = find_series_input(ctx.as_ref(), &user, series_id)
		.await?
		.ok_or_else(|| APIError::NotFound("Series does not exist".to_owned()))?;
	Ok(Json(map_series(&input)))
}

/// `SeriesController.GetAllSeriesById`: the visible series among `seriesIds`,
/// sorted by `sortName` like `GetSeriesDtoForIdsAsync`. Kamigura opens a
/// reading list by mapping its items onto series ids and asking for them in
/// one call (`library/SearchScreens.kt:637-639`), so without this route the
/// list opens empty. Unknown and invisible ids are skipped rather than
/// refused; a body without `seriesIds` is `400 "Invalid payload"`, both as
/// `kavita-ref` answers.
pub(crate) async fn series_by_ids_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	series_ids: &[i32],
) -> APIResult<Vec<SeriesDto>> {
	let mut keys = Vec::with_capacity(series_ids.len());
	for id in series_ids {
		if let Some(key) = resolve_series_key(ctx, *id).await? {
			keys.push(key);
		}
	}
	if keys.is_empty() {
		return Ok(Vec::new());
	}
	let mut items = load_by_keys(ctx, user, &keys)
		.await?
		.iter()
		.map(map_series)
		.collect::<Vec<_>>();
	items.sort_by(|left, right| {
		left.sort_name
			.to_lowercase()
			.cmp(&right.sort_name.to_lowercase())
			.then_with(|| left.id.cmp(&right.id))
	});
	Ok(items)
}

async fn series_by_ids(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<SeriesByIdsDto>,
) -> APIResult<Json<Vec<SeriesDto>>> {
	let user = auth.user();
	let series_ids = body
		.series_ids
		.ok_or_else(|| APIError::BadRequest("Invalid payload".to_owned()))?;
	Ok(Json(
		series_by_ids_for(ctx.as_ref(), &user, &series_ids).await?,
	))
}

/// Kavita returns an empty list for an unknown series.
async fn series_volumes(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesIdQuery>,
) -> APIResult<Json<Vec<VolumeDto>>> {
	let user = auth.user();
	Ok(Json(
		list_volumes(ctx.as_ref(), &user, query.series_id.unwrap_or_default()).await?,
	))
}

/// `VolumeRepository.GetVolumesDtoAsync`: the volumes behind
/// `GET /api/Series/volumes`; a book has exactly one.
pub(crate) async fn list_volumes(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	series_id: i32,
) -> APIResult<Vec<VolumeDto>> {
	let Some(input) = find_series_input(ctx, user, series_id).await? else {
		return Ok(Vec::new());
	};
	Ok(input
		.media
		.iter()
		.map(|media| map_volume(input.id, media))
		.collect())
}

/// Kavita answers `204 No Content` for an unknown series here.
async fn series_metadata(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesIdQuery>,
) -> APIResult<Response> {
	let user = auth.user();
	let Some(input) =
		find_series_input(ctx.as_ref(), &user, query.series_id.unwrap_or_default())
			.await?
	else {
		return Ok(StatusCode::NO_CONTENT.into_response());
	};
	let dto: SeriesMetadataDto = map_series_metadata(&input);
	Ok(Json(dto).into_response())
}

/// Kavita answers `400 "Series does not exist"` (plain text) here.
async fn series_detail(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesIdQuery>,
) -> APIResult<Response> {
	let user = auth.user();
	let Some(dto) =
		series_detail_for(ctx.as_ref(), &user, query.series_id.unwrap_or_default())
			.await?
	else {
		return Ok((
			StatusCode::BAD_REQUEST,
			[(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
			"Series does not exist",
		)
			.into_response());
	};
	Ok(Json(dto).into_response())
}

/// `SeriesService.GetSeriesDetail` for a Kavita series id of either kind.
pub(crate) async fn series_detail_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	series_id: i32,
) -> APIResult<Option<SeriesDetailDto>> {
	let Some(input) = find_series_input(ctx, user, series_id).await? else {
		return Ok(None);
	};
	let library = library_for_series(ctx, user, &input).await?;
	let library_type =
		library_type(library.as_ref().and_then(|(_, config)| config.as_ref()));
	Ok(Some(map_series_detail(&input, library_type)))
}

async fn volume_response(
	ctx: &dyn KavitaBackend,
	auth: &AuthContext,
	volume_id: i32,
) -> APIResult<Response> {
	let user = auth.user();
	let Some((input, index)) = find_media(ctx, &user, volume_id).await? else {
		return Ok(StatusCode::NO_CONTENT.into_response());
	};
	let dto: VolumeDto = map_volume(input.id, &input.media[index]);
	Ok(Json(dto).into_response())
}

async fn volume_by_query(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<VolumeIdQuery>,
) -> APIResult<Response> {
	volume_response(ctx.as_ref(), &auth, query.volume_id.unwrap_or_default()).await
}

async fn volume_by_path(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(volume_id): Path<i32>,
) -> APIResult<Response> {
	volume_response(ctx.as_ref(), &auth, volume_id).await
}

async fn chapter_by_query(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ChapterIdQuery>,
) -> APIResult<Response> {
	let user = auth.user();
	let Some((input, index)) =
		find_media(ctx.as_ref(), &user, query.chapter_id.unwrap_or_default()).await?
	else {
		return Ok(StatusCode::NO_CONTENT.into_response());
	};
	let dto: ChapterDto = map_chapter(&input.media[index]);
	Ok(Json(dto).into_response())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn user_params_follow_kavita_defaults() {
		assert_eq!(
			UserParams::parse(""),
			UserParams {
				page_number: 1,
				requested_page_number: 1,
				page_size: UserParams::MAX_PAGE_SIZE
			}
		);
		assert_eq!(
			UserParams::parse("pageNumber=3&pageSize=20"),
			UserParams {
				page_number: 3,
				requested_page_number: 3,
				page_size: 20
			}
		);
		assert_eq!(
			UserParams::parse("PageNumber=1&PageSize=500&userId=2"),
			UserParams {
				page_number: 1,
				requested_page_number: 1,
				page_size: 500
			}
		);
		assert_eq!(
			UserParams::parse("PageSize=0").page_size,
			UserParams::MAX_PAGE_SIZE
		);
		// Kamigura's dashboard sends `PageNumber=0`: the skip math clamps it
		// but the `Pagination` header echoes it verbatim, like Kavita's
		// `PagedList`.
		let zero = UserParams::parse("PageNumber=0&PageSize=20");
		assert_eq!(zero.page_number, 1);
		assert_eq!(zero.requested_page_number, 0);
		assert_eq!(zero.offset(), 0);
		assert_eq!(
			UserParams {
				page_number: 3,
				requested_page_number: 3,
				page_size: 20
			}
			.offset(),
			40
		);
	}
}

#[cfg(test)]
mod on_deck_user_params {
	use super::*;
	// `PageNumber=0` is not page 2: verify the on-deck route math.
	#[test]
	fn user_params_zero_page_offsets_like_sqlite() {
		let params = UserParams::parse("PageNumber=0&PageSize=20");
		assert_eq!(params.offset(), 0);
	}
}
#[cfg(test)]
mod on_deck {
	use super::*;
	use crate::test_support::{auth_user, db, TestBackend};
	use ::tests::fake_data;
	use models::entity::user;
	use models::shared::enums::ReadingStatus;
	use sea_orm::{DbBackend, Statement};

	async fn backend() -> (TestBackend, user::Model, AuthUser) {
		let conn = db().await;
		let user_row = fake_data::User::new("kate").insert(&conn).await;
		let user = auth_user(&user_row);
		(TestBackend::new(conn), user_row, user)
	}

	/// One series with one media item of `pages` pages and an optional
	/// reading session whose `updated_at` is pinned to `last_read` so the
	/// ordering is deterministic.
	async fn seed(
		conn: &sea_orm::DatabaseConnection,
		user_id: &str,
		library_id: &str,
		name: &str,
		pages: i32,
		read_percentage: Option<f32>,
		last_read: Option<chrono::DateTime<chrono::FixedOffset>>,
	) -> String {
		let series = fake_data::Series {
			name: Some(name.to_owned()),
			library_id: Some(library_id.to_owned()),
			..Default::default()
		}
		.insert(conn)
		.await;
		let media = fake_data::Media {
			series_id: series.id.clone(),
			pages: Some(pages),
			..Default::default()
		}
		.insert(conn)
		.await;
		if let Some(percentage) = read_percentage {
			let session = fake_data::ReadingSession {
				media_id: media.id.clone(),
				user_id: user_id.to_owned(),
				end_percentage: percentage,
				status: ReadingStatus::Reading,
				..Default::default()
			}
			.insert(conn)
			.await;
			if let Some(at) = last_read {
				// `before_save` stamps `updated_at` with `now()` on every
				// write, so pin it with a direct update instead.
				conn.execute(Statement::from_sql_and_values(
						DbBackend::Sqlite,
						"UPDATE reading_sessions SET created_at = ?, updated_at = ? WHERE id = ?",
						[at.clone().into(), at.into(), session.id.into()],
					))
					.await
					.unwrap();
			}
		}
		series.id
	}

	async fn on_deck(
		backend: &TestBackend,
		user: &AuthUser,
		library_id: i32,
		params: UserParams,
	) -> (Vec<String>, PaginationHeader) {
		let (items, header) = list_on_deck(backend, user, library_id, params)
			.await
			.unwrap();
		(items.iter().map(|dto| dto.name.clone()).collect(), header)
	}

	#[tokio::test]
	async fn keeps_in_progress_series_newest_read_first() {
		let (backend, user_row, user) = backend().await;
		let library = fake_data::Library::default().insert(&backend.conn).await;
		let now = chrono::Utc::now();
		// Two days ago: oldest.
		seed(
			&backend.conn,
			&user_row.id,
			&library.id,
			"Alpha",
			100,
			Some(0.5),
			Some((now - chrono::Duration::days(2)).fixed_offset()),
		)
		.await;
		// Today: newest, so first.
		seed(
			&backend.conn,
			&user_row.id,
			&library.id,
			"Beta",
			100,
			Some(0.25),
			Some(now.fixed_offset()),
		)
		.await;
		// Fully read and untouched series are not on deck.
		let completed = fake_data::Series {
			name: Some("Gamma".to_owned()),
			library_id: Some(library.id.clone()),
			..Default::default()
		}
		.insert(&backend.conn)
		.await;
		let media = fake_data::Media {
			series_id: completed.id.clone(),
			pages: Some(100),
			..Default::default()
		}
		.insert(&backend.conn)
		.await;
		fake_data::ReadingSession::completed(&media.id, &user_row.id)
			.insert(&backend.conn)
			.await;
		seed(
			&backend.conn,
			&user_row.id,
			&library.id,
			"Delta",
			100,
			None,
			None,
		)
		.await;

		let (names, header) = on_deck(&backend, &user, 0, UserParams::parse("")).await;
		assert_eq!(names, vec!["Beta".to_owned(), "Alpha".to_owned()]);
		assert_eq!(header.current_page, 1);
		assert_eq!(header.total_items, 2);
	}

	#[tokio::test]
	async fn pages_like_kamigura_requests() {
		let (backend, user_row, user) = backend().await;
		let library = fake_data::Library::default().insert(&backend.conn).await;
		let now = chrono::Utc::now();
		for (index, name) in ["Alpha", "Beta", "Gamma"].iter().enumerate() {
			seed(
				&backend.conn,
				&user_row.id,
				&library.id,
				name,
				100,
				Some(0.1),
				Some((now + chrono::Duration::hours(index as i64)).fixed_offset()),
			)
			.await;
		}
		let (names, header) = on_deck(
			&backend,
			&user,
			0,
			UserParams::parse("PageNumber=0&PageSize=2"),
		)
		.await;
		// Newest first, two per page, and the header echoes the
		// requested zero-based page like Kavita's `PagedList`.
		assert_eq!(names, vec!["Gamma".to_owned(), "Beta".to_owned()]);
		assert_eq!(header.current_page, 0);
		assert_eq!(header.items_per_page, 2);
		assert_eq!(header.total_items, 3);
		assert_eq!(header.total_pages, 2);
	}

	#[tokio::test]
	async fn removals_hide_until_the_next_read_event() {
		let (backend, user_row, user) = backend().await;
		let library = fake_data::Library::default().insert(&backend.conn).await;
		let now = chrono::Utc::now();
		let alpha = seed(
			&backend.conn,
			&user_row.id,
			&library.id,
			"Alpha",
			100,
			Some(0.5),
			Some((now - chrono::Duration::days(2)).fixed_offset()),
		)
		.await;
		let beta = seed(
			&backend.conn,
			&user_row.id,
			&library.id,
			"Beta",
			100,
			Some(0.25),
			Some(now.fixed_offset()),
		)
		.await;

		remove_from_on_deck(&backend, &user_row.id, &SeriesKey::Series(beta.clone()))
			.await
			.unwrap();
		let (names, header) = on_deck(&backend, &user, 0, UserParams::parse("")).await;
		assert_eq!(names, vec!["Alpha".to_owned()]);
		assert_eq!(header.total_items, 1);

		// A read event on the removed series puts it back on deck.
		clear_on_deck_removal(&backend, &user_row.id, &SeriesKey::Series(beta.clone()))
			.await
			.unwrap();
		let (names, _) = on_deck(&backend, &user, 0, UserParams::parse("")).await;
		assert_eq!(names, vec!["Beta".to_owned(), "Alpha".to_owned()]);
		assert!(KavitaIds::lookup(&backend.conn, IdKind::Series, 1)
			.await
			.unwrap()
			.is_some());
		let _ = alpha;
	}

	#[tokio::test]
	async fn library_filter_restricts_by_kavita_library_id() {
		let (backend, user_row, user) = backend().await;
		let first = fake_data::Library::default().insert(&backend.conn).await;
		let second = fake_data::Library::default().insert(&backend.conn).await;
		let now = chrono::Utc::now();
		seed(
			&backend.conn,
			&user_row.id,
			&first.id,
			"Alpha",
			100,
			Some(0.5),
			Some(now.fixed_offset()),
		)
		.await;
		seed(
			&backend.conn,
			&user_row.id,
			&second.id,
			"Beta",
			100,
			Some(0.5),
			Some(now.fixed_offset()),
		)
		.await;
		let first_kavita_id =
			KavitaIds::resolve(&backend.conn, IdKind::Library, &first.id)
				.await
				.unwrap();

		let (all, _) = on_deck(&backend, &user, 0, UserParams::parse("")).await;
		assert_eq!(all.len(), 2);
		let (filtered, header) =
			on_deck(&backend, &user, first_kavita_id, UserParams::parse("")).await;
		assert_eq!(filtered, vec!["Alpha".to_owned()]);
		assert_eq!(header.total_items, 1);
		// An unknown library id is an empty page, not an error.
		let (empty, header) = on_deck(&backend, &user, 999, UserParams::parse("")).await;
		assert!(empty.is_empty());
		assert_eq!(header.total_items, 0);
	}
}

/// Book/LightNovel libraries: every file is its own Kavita series.
#[cfg(test)]
mod books {
	use super::*;
	use crate::dto::{LibraryType, MangaFormat};
	use crate::filter::{FilterComparison, SeriesFilterField, SeriesFilterStatementDto};
	use crate::test_support::{
		auth_user, db, library_of_type, request, series_with_files, TestBackend,
	};
	use ::tests::fake_data;
	use models::shared::enums::LibraryType as StumpLibraryType;

	async fn backend() -> (TestBackend, AuthUser) {
		let conn = db().await;
		let user_row = fake_data::User::new("bookworm").insert(&conn).await;
		let user = auth_user(&user_row);
		(TestBackend::new(conn), user)
	}

	const BOOKS: [(&str, &str, i32); 3] = [
		("alice", "epub", 15),
		("leaves", "epub", 383),
		("moby", "epub", 100),
	];

	async fn list_all(backend: &TestBackend, user: &AuthUser) -> Vec<SeriesDto> {
		list_series(
			backend,
			user,
			&SeriesFilterV2Dto::default(),
			UserParams::parse(""),
			None,
			None,
		)
		.await
		.unwrap()
		.0
	}

	#[tokio::test]
	async fn book_library_lists_one_series_per_file_with_stable_ids() {
		let (backend, user) = backend().await;
		let library = library_of_type(&backend.conn, StumpLibraryType::Book).await;
		let (folder, files) =
			series_with_files(&backend.conn, &library.id, "Collection", &BOOKS).await;

		let first = list_all(&backend, &user).await;
		let names = first
			.iter()
			.map(|dto| dto.name.as_str())
			.collect::<Vec<_>>();
		assert_eq!(names, ["alice", "leaves", "moby"]);
		assert!(first.iter().all(|dto| dto.library_name == library.name));
		assert!(
			first
				.iter()
				.all(|dto| dto.pages > 0 && dto.format == MangaFormat::Epub),
			"each book reports its own file"
		);

		// Ids are allocated once and survive a second listing.
		let second = list_all(&backend, &user).await;
		let ids = |dtos: &[SeriesDto]| dtos.iter().map(|dto| dto.id).collect::<Vec<_>>();
		assert_eq!(ids(&first), ids(&second));

		// Every id is a `book_series` mapping back to its media, distinct from
		// the folder series' own id and from the media's volume/chapter id.
		for (dto, media) in first.iter().zip(&files) {
			assert_eq!(
				KavitaIds::lookup_any(&backend.conn, dto.id).await.unwrap(),
				Some((IdKind::BookSeries, media.id.clone()))
			);
			let volumes = list_volumes(&backend, &user, dto.id).await.unwrap();
			assert_ne!(volumes[0].id, dto.id);
		}
		assert!(
			KavitaIds::lookup(&backend.conn, IdKind::Series, first[0].id)
				.await
				.unwrap()
				.is_none()
		);
		let _ = folder;
	}

	#[tokio::test]
	async fn book_series_volumes_hold_the_single_file() {
		let (backend, user) = backend().await;
		let library = library_of_type(&backend.conn, StumpLibraryType::LightNovel).await;
		let (_, files) =
			series_with_files(&backend.conn, &library.id, "Shelf", &BOOKS).await;
		let listed = list_all(&backend, &user).await;
		let leaves = listed.iter().find(|dto| dto.name == "leaves").unwrap();
		let leaves_file = files.iter().find(|media| media.name == "leaves").unwrap();

		let volumes = list_volumes(&backend, &user, leaves.id).await.unwrap();
		assert_eq!(volumes.len(), 1);
		let volume = &volumes[0];
		assert_eq!(volume.series_id, leaves.id);
		assert_eq!((volume.number, volume.name.as_str()), (-100000, "-100000"));
		assert_eq!(volume.chapters.len(), 1);
		let chapter = &volume.chapters[0];
		assert!(chapter.is_special);
		assert_eq!(chapter.files.len(), 1);
		assert_eq!(chapter.files[0].file_path, leaves_file.path);
		assert_eq!(chapter.pages, 383);
		assert_eq!(
			KavitaIds::lookup_any(&backend.conn, chapter.id)
				.await
				.unwrap(),
			Some((IdKind::Media, leaves_file.id.clone()))
		);

		let detail = series_detail_for(&backend, &user, leaves.id)
			.await
			.unwrap()
			.expect("book series detail");
		assert_eq!(detail.library_type, LibraryType::LightNovel);
		assert_eq!(detail.specials.len(), 1);
		assert_eq!(detail.specials[0].id, chapter.id);
		assert!(detail.volumes.is_empty() && detail.chapters.is_empty());
		assert_eq!((detail.total_count, detail.unread_count), (1, 1));

		// Unknown and non-series ids stay empty, like Kavita.
		assert!(list_volumes(&backend, &user, 999_999)
			.await
			.unwrap()
			.is_empty());
		assert!(list_volumes(&backend, &user, chapter.id)
			.await
			.unwrap()
			.is_empty());
	}

	#[tokio::test]
	async fn manga_library_keeps_its_grouped_shape() {
		let (backend, user) = backend().await;
		let library = library_of_type(&backend.conn, StumpLibraryType::Manga).await;
		let (series_row, _) = series_with_files(
			&backend.conn,
			&library.id,
			"Zeta",
			&[
				("Zeta v01", "cbz", 20),
				("Zeta v02", "cbz", 22),
				("Zeta v03", "cbz", 24),
			],
		)
		.await;

		let listed = list_all(&backend, &user).await;
		assert_eq!(listed.len(), 1);
		let dto = &listed[0];
		assert_eq!(dto.name, "Zeta");
		assert_eq!(dto.pages, 66);
		assert_eq!(dto.format, MangaFormat::Archive);
		assert_eq!(
			KavitaIds::lookup_any(&backend.conn, dto.id).await.unwrap(),
			Some((IdKind::Series, series_row.id.clone()))
		);

		let volumes = list_volumes(&backend, &user, dto.id).await.unwrap();
		let numbers = volumes
			.iter()
			.map(|volume| (volume.number, volume.name.clone()))
			.collect::<Vec<_>>();
		assert_eq!(
			numbers,
			[
				(1, "1".to_owned()),
				(2, "2".to_owned()),
				(3, "3".to_owned())
			]
		);
		assert!(volumes.iter().all(|volume| volume.series_id == dto.id));
		assert!(volumes.iter().all(|volume| {
			let chapter = &volume.chapters[0];
			!chapter.is_special
				&& chapter.total_count == 0
				&& chapter.range == chapter.title
		}));
		let detail = series_detail_for(&backend, &user, dto.id)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(detail.volumes.len(), 3);
		assert!(detail.specials.is_empty());
		assert_eq!((detail.total_count, detail.unread_count), (3, 3));
	}

	#[tokio::test]
	async fn listing_sorts_and_pages_books_and_series_together() {
		let (backend, user) = backend().await;
		let books = library_of_type(&backend.conn, StumpLibraryType::Book).await;
		series_with_files(&backend.conn, &books.id, "Collection", &BOOKS).await;
		let manga = library_of_type(&backend.conn, StumpLibraryType::Manga).await;
		series_with_files(
			&backend.conn,
			&manga.id,
			"Beta",
			&[("Beta v01", "cbz", 20), ("Beta v02", "cbz", 22)],
		)
		.await;

		// One sort order across both kinds: alice, Beta, leaves, moby.
		let (page, header) = list_series(
			&backend,
			&user,
			&SeriesFilterV2Dto::default(),
			UserParams::parse("PageNumber=2&PageSize=2"),
			None,
			None,
		)
		.await
		.unwrap();
		assert_eq!(
			page.iter().map(|dto| dto.name.as_str()).collect::<Vec<_>>(),
			["leaves", "moby"]
		);
		assert_eq!((header.total_items, header.total_pages), (4, 2));

		// Library statements scope both kinds; a name filter matches the book
		// itself, not the folder it is filed under.
		let books_kavita_id =
			KavitaIds::resolve(&backend.conn, IdKind::Library, &books.id)
				.await
				.unwrap();
		let mut filter = SeriesFilterV2Dto::default();
		filter.statements.push(SeriesFilterStatementDto::new(
			FilterComparison::Equal,
			SeriesFilterField::Libraries,
			books_kavita_id.to_string(),
		));
		filter.statements.push(SeriesFilterStatementDto::new(
			FilterComparison::Matches,
			SeriesFilterField::SeriesName,
			"mob",
		));
		let (page, header) =
			list_series(&backend, &user, &filter, UserParams::parse(""), None, None)
				.await
				.unwrap();
		assert_eq!(
			page.iter().map(|dto| dto.name.as_str()).collect::<Vec<_>>(),
			["moby"]
		);
		assert_eq!(header.total_items, 1);

		// Recently added leads with the newest file/series.
		let (recent, _) = list_series(
			&backend,
			&user,
			&SeriesFilterV2Dto::default(),
			UserParams::parse("PageSize=1"),
			Some(SortKey::created(Order::Desc)),
			None,
		)
		.await
		.unwrap();
		assert_eq!(recent[0].name, "Beta");
	}

	/// `POST /api/Series/series-by-ids` is the second half of Kamigura's
	/// "open a reading list" (`library/SearchScreens.kt:637-639`): the items'
	/// series ids come back as `SeriesDto`s sorted by `sortName`. It has to
	/// be registered as a literal segment, or `POST` falls through to the
	/// `GET /api/Series/{seriesId}` route and answers `405` with an empty
	/// grid in the client. Unknown ids are skipped; a body without
	/// `seriesIds` is `400`, both as `kavita-ref` answers.
	#[tokio::test]
	async fn series_by_ids_answers_the_reading_list_grid() {
		let conn = db().await;
		let user_row = fake_data::User::new("bookworm").insert(&conn).await;
		let user = auth_user(&user_row);
		let backend = std::sync::Arc::new(TestBackend::new(conn));
		let books = library_of_type(backend.conn(), StumpLibraryType::Book).await;
		let (_, files) = series_with_files(
			backend.conn(),
			&books.id,
			"Shelf",
			&[("zeta", "epub", 5), ("alpha", "epub", 7)],
		)
		.await;
		let comics = library_of_type(backend.conn(), StumpLibraryType::Comic).await;
		let (comic_series, _) = series_with_files(
			backend.conn(),
			&comics.id,
			"middle",
			&[("science_comics_001", "cbz", 36)],
		)
		.await;
		let mut ids = Vec::new();
		for file in &files {
			ids.push(
				KavitaIds::resolve(backend.conn(), IdKind::BookSeries, &file.id)
					.await
					.unwrap(),
			);
		}
		ids.push(
			KavitaIds::resolve(backend.conn(), IdKind::Series, &comic_series.id)
				.await
				.unwrap(),
		);

		let (status, body) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/Series/series-by-ids",
			Some(serde_json::json!({ "seriesIds": [ids[0], ids[1], ids[2], 999_999] })),
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(
			body.as_array()
				.unwrap()
				.iter()
				.map(|dto| dto["name"].as_str().unwrap())
				.collect::<Vec<_>>(),
			["alpha", "middle", "zeta"],
			"sorted by sortName, unknown ids skipped"
		);

		for empty in [
			serde_json::json!({ "seriesIds": [] }),
			serde_json::json!({ "seriesIds": [999_999] }),
		] {
			let (status, body) = request(
				backend.clone(),
				&user,
				"POST",
				"/api/series/series-by-ids",
				Some(empty),
			)
			.await;
			assert_eq!(status, StatusCode::OK);
			assert!(body.as_array().unwrap().is_empty());
		}

		let (status, _) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/Series/series-by-ids",
			Some(serde_json::json!({})),
		)
		.await;
		assert_eq!(status, StatusCode::BAD_REQUEST);
	}
}

/// The scan/analyze/refresh routes Kamigura triggers map onto Stump jobs.
#[cfg(test)]
mod maintenance {
	use super::*;
	use crate::test_support::{
		auth_user, db, library_of_type, request, request_raw, series_with_files,
		EnqueuedJob, TestBackend,
	};
	use ::tests::fake_data;
	use models::shared::enums::LibraryType as StumpLibraryType;

	#[tokio::test]
	async fn series_scan_analyze_and_refresh_enqueue_the_scoped_stump_jobs() {
		let conn = db().await;
		let user_row = fake_data::User::new("maint").insert(&conn).await;
		let user = auth_user(&user_row);
		let library = library_of_type(&conn, StumpLibraryType::Comic).await;
		let (series_row, _) =
			series_with_files(&conn, &library.id, "Zeta", &[("v01", "cbz", 10)]).await;
		let backend = std::sync::Arc::new(TestBackend::new(conn));
		let library_id = KavitaIds::resolve(backend.conn(), IdKind::Library, &library.id)
			.await
			.unwrap();
		let series_id =
			KavitaIds::resolve(backend.conn(), IdKind::Series, &series_row.id)
				.await
				.unwrap();
		let body = serde_json::json!({
			"libraryId": library_id,
			"seriesId": series_id,
			"forceUpdate": false,
			"forceColorscape": false,
		});

		for route in ["scan", "analyze", "refresh-metadata"] {
			let (status, _) = request(
				backend.clone(),
				&user,
				"POST",
				&format!("/api/Series/{route}"),
				Some(body.clone()),
			)
			.await;
			assert_eq!(status, StatusCode::OK, "{route}");
		}
		assert_eq!(
			backend.enqueued(),
			vec![
				EnqueuedJob::SeriesScan {
					series_id: series_row.id.clone(),
					path: series_row.path.clone(),
					force: false,
				},
				EnqueuedJob::SeriesAnalysis {
					series_id: series_row.id.clone(),
				},
				// A metadata refresh re-reads the files, so it always forces.
				EnqueuedJob::SeriesScan {
					series_id: series_row.id.clone(),
					path: series_row.path.clone(),
					force: true,
				},
			]
		);

		// `forceUpdate` forces the scan.
		request(
			backend.clone(),
			&user,
			"POST",
			"/api/series/scan",
			Some(serde_json::json!({
				"libraryId": library_id, "seriesId": series_id, "forceUpdate": true
			})),
		)
		.await;
		assert_eq!(
			backend.enqueued().last(),
			Some(&EnqueuedJob::SeriesScan {
				series_id: series_row.id.clone(),
				path: series_row.path.clone(),
				force: true,
			})
		);

		// An unknown series is `200` with nothing enqueued, like Kavita.
		let before = backend.enqueued().len();
		let (status, _) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/Series/scan",
			Some(serde_json::json!({"libraryId": library_id, "seriesId": 999999})),
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(backend.enqueued().len(), before);
	}
	#[tokio::test]
	async fn komf_metadata_and_cover_writes_round_trip_through_native_read_routes() {
		use base64::{engine::general_purpose::STANDARD, Engine as _};

		let conn = db().await;
		let user_row = fake_data::User::new("komf-writes").insert(&conn).await;
		let user = auth_user(&user_row);
		let library = library_of_type(&conn, StumpLibraryType::Comic).await;
		let (series_row, files) =
			series_with_files(&conn, &library.id, "Zeta", &[("v01", "cbz", 10)]).await;
		let backend = std::sync::Arc::new(TestBackend::new(conn));
		let series_id =
			KavitaIds::resolve(backend.conn(), IdKind::Series, &series_row.id)
				.await
				.unwrap();
		let cover_bytes = b"komf-cover-payload".to_vec();
		let cover_base64 = STANDARD.encode(&cover_bytes);
		let chapter_id = KavitaIds::resolve(backend.conn(), IdKind::Media, &files[0].id)
			.await
			.unwrap();

		let (status, update_body) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/Series/update",
			Some(serde_json::json!({
				"id": series_id,
				"localizedName": "Localized Zeta",
				"sortName": "Zeta, The",
				"coverImageLocked": false,
				"sortNameLocked": true,
				"localizedNameLocked": true,
			})),
		)
		.await;
		assert_eq!(status, StatusCode::OK, "{update_body}");
		let (status, series_body) = request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Series/{series_id}"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(series_body["localizedName"], "Localized Zeta");
		assert_eq!(series_body["sortName"], "Zeta, The");
		assert_eq!(series_body["localizedNameLocked"], true);
		assert_eq!(series_body["sortNameLocked"], true);
		assert_eq!(series_body["coverImageLocked"], false);

		let mut metadata = serde_json::json!({
					"id": series_id,
					"seriesId": series_id,
					"summary": "Written by Komf",
					"genres": [{"id": 1, "title": "Science Fiction"}],
					"tags": [{"id": 2, "title": "Komf"}],
					"writers": [{"id": 3, "name": "A. Writer"}],
					"coverArtists": [],
					"publishers": [],
					"characters": [],
					"pencillers": [],
					"inkers": [],
					"imprints": [],
					"colorists": [],
					"letterers": [],
					"editors": [],
					"translators": [],
					"teams": [],
					"locations": [],
					"ageRating": 8,
					"releaseYear": 2024,
					"language": "en",
					"maxCount": 4,
					"totalCount": 4,
					"publicationStatus": 0,
					"webLinks": "https://example.invalid/zeta",
		});
		metadata.as_object_mut().unwrap().extend(
			serde_json::json!({
				"languageLocked": true,
				"summaryLocked": true,
				"ageRatingLocked": false,
				"publicationStatusLocked": false,
				"genresLocked": true,
				"tagsLocked": true,
				"writerLocked": true,
				"characterLocked": false,
				"coloristLocked": false,
				"editorLocked": false,
				"inkerLocked": false,
				"imprintLocked": false,
				"lettererLocked": false,
				"pencillerLocked": false,
				"publisherLocked": false,
				"translatorLocked": false,
				"teamLocked": false,
				"locationLocked": false,
				"coverArtistLocked": false,
				"releaseYearLocked": true
			})
			.as_object()
			.unwrap()
			.clone(),
		);
		let (status, _) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/Series/metadata",
			Some(serde_json::json!({"seriesMetadata": metadata})),
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		let (status, metadata_body) = request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Series/metadata?seriesId={series_id}"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(metadata_body["summary"], "Written by Komf");
		assert_eq!(metadata_body["genres"][0]["title"], "Science Fiction");
		assert_eq!(metadata_body["tags"][0]["title"], "Komf");
		assert_eq!(metadata_body["writers"][0]["name"], "A. Writer");
		assert_eq!(metadata_body["languageLocked"], true);
		assert_eq!(metadata_body["releaseYearLocked"], true);

		let mut chapter_update = serde_json::json!({
				"id": chapter_id,
				"summary": "Chapter summary",
				"genres": [{"id": 4, "title": "Mystery"}],
				"tags": [{"id": 5, "title": "Chapter tag"}],
				"ageRating": 8,
				"language": "en",
				"weblinks": "https://example.invalid/chapter",
				"isbn": "9780000000000",
				"releaseDate": "2024-04-03T12:00:00",
				"titleName": "Chapter title",
				"sortOrder": 7.5,
				"writers": [{"id": 6, "name": "Chapter writer"}],
				"coverArtists": [],
				"publishers": [],
				"characters": [],
				"pencillers": [],
				"inkers": [],
				"imprints": [],
				"colorists": [],
				"letterers": [],
				"editors": [],
				"translators": [],
				"teams": [],
				"locations": [],
		});
		chapter_update.as_object_mut().unwrap().extend(
			serde_json::json!({
				"ageRatingLocked": false,
				"titleNameLocked": true,
				"genresLocked": true,
				"tagsLocked": true,
				"writerLocked": true,
				"characterLocked": false,
				"coloristLocked": false,
				"editorLocked": false,
				"inkerLocked": false,
				"imprintLocked": false,
				"lettererLocked": false,
				"pencillerLocked": false,
				"publisherLocked": false,
				"translatorLocked": false,
				"teamLocked": false,
				"locationLocked": false,
				"coverArtistLocked": false,
				"languageLocked": true,
				"summaryLocked": true,
				"isbnLocked": true,
				"releaseDateLocked": true,
				"sortOrderLocked": true
			})
			.as_object()
			.unwrap()
			.clone(),
		);
		let (status, _) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/Chapter/update",
			Some(chapter_update),
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		let (status, chapter_body) = request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Series/chapter?chapterId={chapter_id}"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(chapter_body["titleName"], "Chapter title");
		assert_eq!(chapter_body["summary"], "Chapter summary");
		assert_eq!(chapter_body["webLinks"], "https://example.invalid/chapter");
		assert_eq!(chapter_body["titleNameLocked"], true);
		assert_eq!(chapter_body["sortOrderLocked"], true);
		assert_eq!(chapter_body["sortOrder"], serde_json::json!(7.5));

		for (url, expected_cover) in [
			("/api/Upload/series", "/api/Image/series-cover"),
			("/api/Upload/chapter", "/api/Image/chapter-cover"),
		] {
			let id = if url.ends_with("series") {
				series_id
			} else {
				chapter_id
			};
			let (status, _) = request(
				backend.clone(),
				&user,
				"POST",
				url,
				Some(serde_json::json!({
					"id": id,
					"url": cover_base64.clone(),
					"lockCover": true,
				})),
			)
			.await;
			assert_eq!(status, StatusCode::OK);
			let query_name = if url.ends_with("series") {
				"seriesId"
			} else {
				"chapterId"
			};
			let (status, bytes) = request_raw(
				backend.clone(),
				&user,
				"GET",
				&format!("{expected_cover}?{query_name}={id}"),
				None,
			)
			.await;
			assert_eq!(status, StatusCode::OK);
			assert_eq!(bytes, cover_bytes);
		}
		let (status, series_body) = request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Series/{series_id}"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(series_body["coverImageLocked"], true);

		let (status, chapter_body) = request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Series/chapter?chapterId={chapter_id}"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(chapter_body["coverImageLocked"], true);

		let (status, _) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/Upload/chapter",
			Some(serde_json::json!({
				"id": chapter_id, "url": "", "lockCover": false
			})),
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		let (status, chapter_body) = request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Series/chapter?chapterId={chapter_id}"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(chapter_body["coverImageLocked"], false);
		let (status, bytes) = request_raw(
			backend,
			&user,
			"GET",
			&format!("/api/Image/chapter-cover?chapterId={chapter_id}"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(bytes, cover_bytes);
	}
}

/// The metadata every Kavita client renders. Kamigura's series detail shows
/// `summary`, the eight credit roles it merges into "Credits"
/// (`series/internal/SeriesDetailSummary.kt:265-284`), `publishers` and
/// `imprints` (`:104-105`), `genres` (`:106-110`) and `tags` (`:111-115`),
/// plus `releaseYear`/`publicationStatus`
/// (`series/internal/SeriesMetadataText.kt:24-28`), so all of them have to be
/// filled from whichever Stump row carries the value.
#[cfg(test)]
mod metadata_mapping {
	use super::*;
	use crate::dto::{AgeRating, PublicationStatus};
	use crate::filter::{FilterComparison, SeriesFilterField, SeriesFilterStatementDto};
	use crate::test_support::{
		auth_user, db, library_of_type, request, series_with_files, TestBackend,
	};
	use ::tests::fake_data;
	use models::entity::{media_metadata, media_tag, series_metadata, series_tag, tag};
	use models::shared::enums::LibraryType as StumpLibraryType;
	use sea_orm::ActiveValue::Set;

	async fn tag_id(conn: &sea_orm::DatabaseConnection, name: &str) -> i32 {
		tag::ActiveModel {
			name: Set(name.to_owned()),
			kind: Set("tag".to_owned()),
			..Default::default()
		}
		.insert(conn)
		.await
		.unwrap()
		.id
	}

	fn names(value: &serde_json::Value, field: &str) -> Vec<String> {
		value[field]
			.as_array()
			.unwrap_or_else(|| panic!("{field} is an array"))
			.iter()
			.map(|person| person["name"].as_str().unwrap().to_owned())
			.collect()
	}

	fn titles(value: &serde_json::Value, field: &str) -> Vec<String> {
		value[field]
			.as_array()
			.unwrap_or_else(|| panic!("{field} is an array"))
			.iter()
			.map(|row| row["title"].as_str().unwrap().to_owned())
			.collect()
	}

	#[tokio::test]
	async fn series_metadata_aggregates_the_series_row_and_every_file() {
		let conn = db().await;
		let user_row = fake_data::User::new("reader").insert(&conn).await;
		let user = auth_user(&user_row);
		let library = library_of_type(&conn, StumpLibraryType::Manga).await;
		let (series, files) = series_with_files(
			&conn,
			&library.id,
			"Berserk",
			&[("Berserk v01", "cbz", 200), ("Berserk v02", "cbz", 210)],
		)
		.await;

		series_metadata::ActiveModel {
			series_id: Set(series.id.clone()),
			genres: Set(Some("Fantasy".to_owned())),
			writers: Set(Some("Kentaro Miura".to_owned())),
			imprint: Set(Some("Jets Comics".to_owned())),
			status: Set(Some("Ended".to_owned())),
			total_issues: Set(Some(41)),
			links: Set(Some("https://example.test/berserk".to_owned())),
			locked_fields: Set(Some(serde_json::json!(["GENRES", "YEAR"]))),
			language_lock: Set(true),
			title_sort_lock: Set(false),
			reading_direction_lock: Set(false),
			alternate_titles_lock: Set(false),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();

		// The credits a Komf session writes into each file's ComicInfo.
		media_metadata::ActiveModel {
			media_id: Set(Some(files[0].id.clone())),
			volume: Set(Some(1)),
			genres: Set(Some("Dark Fantasy, Horror".to_owned())),
			writers: Set(Some("Kentaro Miura".to_owned())),
			pencillers: Set(Some("Kentaro Miura".to_owned())),
			inkers: Set(Some("Studio Gaga".to_owned())),
			colorists: Set(Some("Studio Gaga".to_owned())),
			letterers: Set(Some("Duncan Fredoo".to_owned())),
			editors: Set(Some("Dark Horse".to_owned())),
			cover_artists: Set(Some("Kentaro Miura".to_owned())),
			characters: Set(Some("Guts, Griffith".to_owned())),
			teams: Set(Some("Band of the Hawk".to_owned())),
			publisher: Set(Some("Hakusensha".to_owned())),
			language: Set(Some("ja".to_owned())),
			age_rating: Set(Some(18)),
			year: Set(Some(1990)),
			identifier_isbn: Set(Some("9781593070205".to_owned())),
			summary: Set(Some("The Black Swordsman.".to_owned())),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();
		media_metadata::ActiveModel {
			media_id: Set(Some(files[1].id.clone())),
			volume: Set(Some(2)),
			// `horror` repeats the first file's genre in another case.
			genres: Set(Some("horror".to_owned())),
			writers: Set(Some("Ghost Writer".to_owned())),
			year: Set(Some(1991)),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();

		let classic = tag_id(&conn, "Classic").await;
		let tagged_by_komf = tag_id(&conn, "Komf Tagged").await;
		series_tag::ActiveModel {
			series_id: Set(series.id.clone()),
			tag_id: Set(classic),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();
		media_tag::ActiveModel {
			media_id: Set(files[0].id.clone()),
			tag_id: Set(tagged_by_komf),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();

		let backend = std::sync::Arc::new(TestBackend::new(conn));
		let series_kavita_id =
			KavitaIds::resolve_many(&backend.conn, IdKind::Series, &[series.id.clone()])
				.await
				.unwrap()[&series.id];
		let chapter_id =
			KavitaIds::resolve_many(&backend.conn, IdKind::Media, &[files[0].id.clone()])
				.await
				.unwrap()[&files[0].id];

		let (status, body) = request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Series/metadata?seriesId={series_kavita_id}"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(
			titles(&body, "genres"),
			["Fantasy", "Dark Fantasy", "Horror"],
			"series row first, then each file, one entry per value"
		);
		assert_eq!(names(&body, "writers"), ["Kentaro Miura", "Ghost Writer"]);
		assert_eq!(names(&body, "pencillers"), ["Kentaro Miura"]);
		assert_eq!(names(&body, "inkers"), ["Studio Gaga"]);
		assert_eq!(names(&body, "colorists"), ["Studio Gaga"]);
		assert_eq!(names(&body, "letterers"), ["Duncan Fredoo"]);
		assert_eq!(names(&body, "editors"), ["Dark Horse"]);
		assert_eq!(names(&body, "coverArtists"), ["Kentaro Miura"]);
		assert_eq!(names(&body, "characters"), ["Guts", "Griffith"]);
		assert_eq!(names(&body, "teams"), ["Band of the Hawk"]);
		assert_eq!(names(&body, "publishers"), ["Hakusensha"]);
		assert_eq!(names(&body, "imprints"), ["Jets Comics"]);
		assert!(names(&body, "translators").is_empty());
		assert!(names(&body, "locations").is_empty());
		assert_eq!(titles(&body, "tags"), ["Classic", "Komf Tagged"]);
		assert_eq!(
			body["ageRating"],
			serde_json::to_value(AgeRating::R18Plus).unwrap()
		);
		assert_eq!(body["releaseYear"], serde_json::json!(1990));
		assert_eq!(body["language"], serde_json::json!("ja"));
		assert_eq!(body["maxCount"], serde_json::json!(41));
		assert_eq!(body["totalCount"], serde_json::json!(41));
		assert_eq!(
			body["publicationStatus"],
			serde_json::to_value(PublicationStatus::Ended).unwrap()
		);
		assert_eq!(
			body["webLinks"],
			serde_json::json!("https://example.test/berserk")
		);
		assert_eq!(body["genresLocked"], serde_json::json!(true));
		assert_eq!(body["releaseYearLocked"], serde_json::json!(true));
		assert_eq!(body["languageLocked"], serde_json::json!(true));
		assert_eq!(body["tagsLocked"], serde_json::json!(false));

		// The chapter carries its own file's metadata, tags included.
		let (status, chapter) = request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Series/chapter?chapterId={chapter_id}"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(names(&chapter, "writers"), ["Kentaro Miura"]);
		assert_eq!(titles(&chapter, "genres"), ["Dark Fantasy", "Horror"]);
		assert_eq!(titles(&chapter, "tags"), ["Komf Tagged"]);
		assert_eq!(chapter["isbn"], serde_json::json!("9781593070205"));
		assert_eq!(
			chapter["summary"],
			serde_json::json!("The Black Swordsman.")
		);
		assert_eq!(chapter["language"], serde_json::json!("ja"));
		assert_eq!(
			chapter["releaseDate"],
			serde_json::json!("1990-01-01T00:00:00.0000000")
		);

		// A tag chip a client taps filters by its id, so a tag that reached
		// the series through a file has to match the series too.
		for id in [classic, tagged_by_komf] {
			let filter = SeriesFilterV2Dto {
				statements: vec![SeriesFilterStatementDto::new(
					FilterComparison::Contains,
					SeriesFilterField::Tags,
					id.to_string(),
				)],
				..Default::default()
			};
			let found = list_series(
				backend.as_ref(),
				&user,
				&filter,
				UserParams::parse(""),
				None,
				None,
			)
			.await
			.unwrap()
			.0;
			assert_eq!(
				found.iter().map(|dto| dto.id).collect::<Vec<_>>(),
				[series_kavita_id],
				"tag {id} must select the series it is rendered on"
			);
		}

		// `GET /api/Metadata/tags` backs the filter sheet, so it lists both.
		let (status, tags) =
			request(backend.clone(), &user, "GET", "/api/Metadata/tags", None).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(
			tags.as_array()
				.unwrap()
				.iter()
				.map(|tag| tag["title"].as_str().unwrap())
				.collect::<Vec<_>>(),
			["Classic", "Komf Tagged"]
		);
	}
}
