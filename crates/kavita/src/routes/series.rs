//! `SeriesController`, `VolumeController` and `ChapterController` reads.

use std::sync::Arc;

use axum::{
	extract::{Path, Query},
	http::{header, HeaderValue, StatusCode},
	response::{IntoResponse, Response},
	routing::{get, post},
	Extension, Json, Router,
};
use models::entity::{series, series_tag, tag, user::AuthUser};
use sea_orm::{prelude::*, QueryOrder, QuerySelect};
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::{
	dto::{
		ChapterDto, PaginationHeader, SeriesDetailDto, SeriesDto, SeriesMetadataDto,
		TagDto, VolumeDto,
	},
	errors::{APIError, APIResult},
	filter::SeriesFilterV2Dto,
	mapper::{
		library_type, map_chapter, map_series, map_series_detail, map_series_metadata,
		map_volume, SeriesInput,
	},
};

use super::{
	query::{
		find_media, find_series_by_kavita_id, library_for_series, load_series_input,
		load_series_inputs,
	},
	route_ci,
	series_filter::{plan, FilterPlan, ProgressSort},
	KavitaBackend,
};

/// `UserParams`: `PageNumber` defaults to 1, `PageSize` to "everything".
/// Query names are matched case-insensitively (the extension sends
/// `pageNumber`, Turnleaf `PageNumber`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UserParams {
	pub page_number: i32,
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
			page_size,
		}
	}

	fn offset(&self) -> usize {
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
	let router = route_ci(router, "/api/Series/v2", post(series_v2));
	let router = route_ci(router, "/api/Series/volumes", get(series_volumes));
	let router = route_ci(router, "/api/Series/volume", get(volume_by_query));
	let router = route_ci(router, "/api/Series/metadata", get(series_metadata));
	let router = route_ci(router, "/api/Series/series-detail", get(series_detail));
	let router = route_ci(router, "/api/Series/chapter", get(chapter_by_query));
	let router = route_ci(router, "/api/Series/{seriesId}", get(series_by_id));
	let router = route_ci(router, "/api/Volume", get(volume_by_query));
	let router = route_ci(router, "/api/Volume/{volumeId}", get(volume_by_path));
	route_ci(router, "/api/Chapter", get(chapter_by_query))
}

fn pagination_response<T: serde::Serialize>(
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
	response.headers_mut().insert(
		header::ACCESS_CONTROL_EXPOSE_HEADERS,
		HeaderValue::from_static(PaginationHeader::NAME),
	);
	Ok(response)
}

/// A page of series plus the `Pagination` header Kavita attaches.
pub(crate) async fn list_series(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	filter: &SeriesFilterV2Dto,
	params: UserParams,
) -> APIResult<(Vec<SeriesDto>, PaginationHeader)> {
	let plan = plan(ctx.conn(), user, filter).await?;
	let mut query =
		series::ModelWithMetadata::find_for_user(user).filter(plan.condition.clone());
	for (expr, order) in &plan.order {
		query = query.order_by(expr.clone(), order.clone());
	}

	if plan.needs_memory_pass() {
		let rows = query
			.into_model::<series::ModelWithMetadata>()
			.all(ctx.conn())
			.await?;
		let inputs = load_series_inputs(ctx, user, rows).await?;
		let (items, total) = paginate_in_memory(inputs, &plan, params);
		return Ok((
			items,
			PaginationHeader::new(params.page_number, params.page_size, total),
		));
	}

	let total = i32::try_from(query.clone().count(ctx.conn()).await?)?;
	if params.page_size != UserParams::MAX_PAGE_SIZE {
		query = query
			.offset(u64::try_from(params.offset())?)
			.limit(u64::try_from(params.page_size)?);
	}
	let rows = query
		.into_model::<series::ModelWithMetadata>()
		.all(ctx.conn())
		.await?;
	let inputs = load_series_inputs(ctx, user, rows).await?;
	let items = inputs.iter().map(map_series).collect();
	Ok((
		items,
		PaginationHeader::new(params.page_number, params.page_size, total),
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

fn user_params(uri: &axum::http::Uri) -> UserParams {
	UserParams::parse(uri.query().unwrap_or_default())
}

async fn series_all_v2(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	uri: axum::http::Uri,
	Json(filter): Json<SeriesFilterV2Dto>,
) -> APIResult<Response> {
	let user = auth.user();
	let (items, header) =
		list_series(ctx.as_ref(), &user, &filter, user_params(&uri)).await?;
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
		list_series(ctx.as_ref(), &user, &filter, user_params(&uri)).await?;
	pagination_response(items, header)
}

async fn load_input(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	series_id: i32,
) -> APIResult<Option<SeriesInput>> {
	let Some(row) = find_series_by_kavita_id(ctx, user, series_id).await? else {
		return Ok(None);
	};
	Ok(Some(load_series_input(ctx, user, row).await?))
}

async fn series_by_id(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(series_id): Path<i32>,
) -> APIResult<Json<SeriesDto>> {
	let user = auth.user();
	let input = load_input(ctx.as_ref(), &user, series_id)
		.await?
		.ok_or_else(|| APIError::NotFound("Series does not exist".to_owned()))?;
	Ok(Json(map_series(&input)))
}

/// Kavita returns an empty list for an unknown series.
async fn series_volumes(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesIdQuery>,
) -> APIResult<Json<Vec<VolumeDto>>> {
	let user = auth.user();
	let Some(input) =
		load_input(ctx.as_ref(), &user, query.series_id.unwrap_or_default()).await?
	else {
		return Ok(Json(Vec::new()));
	};
	Ok(Json(
		input
			.media
			.iter()
			.map(|media| map_volume(input.id, media))
			.collect(),
	))
}

async fn series_tags(ctx: &dyn KavitaBackend, series_id: &str) -> APIResult<Vec<TagDto>> {
	let rows = tag::Entity::find()
		.filter(
			tag::Column::Id.in_subquery(
				sea_orm::sea_query::Query::select()
					.column(series_tag::Column::TagId)
					.from(series_tag::Entity)
					.and_where(series_tag::Column::SeriesId.eq(series_id))
					.to_owned(),
			),
		)
		.order_by_asc(tag::Column::Name)
		.all(ctx.conn())
		.await?;
	Ok(rows
		.into_iter()
		.map(|tag| TagDto {
			id: tag.id,
			title: tag.name,
		})
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
		load_input(ctx.as_ref(), &user, query.series_id.unwrap_or_default()).await?
	else {
		return Ok(StatusCode::NO_CONTENT.into_response());
	};
	let tags = series_tags(ctx.as_ref(), &input.series.id).await?;
	let dto: SeriesMetadataDto = map_series_metadata(&input, tags);
	Ok(Json(dto).into_response())
}

/// Kavita answers `400 "Series does not exist"` (plain text) here.
async fn series_detail(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesIdQuery>,
) -> APIResult<Response> {
	let user = auth.user();
	let Some(input) =
		load_input(ctx.as_ref(), &user, query.series_id.unwrap_or_default()).await?
	else {
		return Ok((
			StatusCode::BAD_REQUEST,
			[(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
			"Series does not exist",
		)
			.into_response());
	};
	let library = library_for_series(ctx.as_ref(), &user, &input).await?;
	let library_type =
		library_type(library.as_ref().and_then(|(_, config)| config.as_ref()));
	let dto: SeriesDetailDto = map_series_detail(&input, library_type);
	Ok(Json(dto).into_response())
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
				page_size: UserParams::MAX_PAGE_SIZE
			}
		);
		assert_eq!(
			UserParams::parse("pageNumber=3&pageSize=20"),
			UserParams {
				page_number: 3,
				page_size: 20
			}
		);
		assert_eq!(
			UserParams::parse("PageNumber=1&PageSize=500&userId=2"),
			UserParams {
				page_number: 1,
				page_size: 500
			}
		);
		assert_eq!(
			UserParams::parse("PageSize=0").page_size,
			UserParams::MAX_PAGE_SIZE
		);
		assert_eq!(UserParams::parse("pageNumber=0").page_number, 1);
		assert_eq!(
			UserParams {
				page_number: 3,
				page_size: 20
			}
			.offset(),
			40
		);
	}
}
