use axum::{
	extract::{Path, State},
	middleware,
	routing::get,
	Extension, Router,
};
use models::{
	entity::{library_config, media, series},
	shared::image_processor_options::ImageProcessorOptions,
};
use sea_orm::{prelude::*, sea_query::Query, QueryOrder};
use stump_auth::AuthContext;
use stump_core::config::StumpConfig;
use stump_media::{get_saved_thumbnail, get_thumbnail, ContentType};

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	middleware::auth::auth_middleware,
	utils::http::ImageResponse,
};

use super::media::get_media_thumbnail;

pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	Router::new()
		.nest(
			"/series/{id}",
			Router::new().route("/thumbnail", get(get_series_thumbnail_handler)),
		)
		.layer(middleware::from_fn_with_state(app_state, auth_middleware))
}

pub(crate) async fn get_series_thumbnail(
	series: &series::SeriesThumbSelect,
	first_book: Option<media::MediaThumbSelect>,
	thumbnail_config: Option<ImageProcessorOptions>,
	config: &StumpConfig,
) -> APIResult<(ContentType, Vec<u8>)> {
	// Note: This doesn't hard-fail because if the saved thumbnail is missing or corrupt, we want
	// to just pull something else instead of erroring out entirely.
	if let Some(path) = &series.thumbnail_path {
		match get_saved_thumbnail(std::path::Path::new(path)).await {
			Ok(result) => return Ok(result),
			Err(_) => {
				tracing::warn!(path = ?path, "Failed to get saved thumbnail");
			},
		}
	}

	let image_format = thumbnail_config.as_ref().map(|options| options.format);
	let generated_thumb =
		get_thumbnail(config.get_thumbnails_dir(), &series.id, image_format).await?;

	match (generated_thumb, first_book) {
		(Some(result), _) => Ok(result),
		(None, Some(book)) => get_media_thumbnail(&book, thumbnail_config, config).await,
		(None, None) => Err(APIError::NotFound(
			"Series does not have a thumbnail".to_string(),
		)),
	}
}

async fn get_series_thumbnail_handler(
	Path(id): Path<String>,
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<ImageResponse> {
	let user = req.user();
	let series = series::Entity::find_for_user(&user)
		.filter(series::Column::Id.eq(id.clone()))
		.into_model::<series::SeriesThumbSelect>()
		.one(ctx.conn.as_ref())
		.await?
		.ok_or(APIError::NotFound("Series not found".to_string()))?;

	// Note: This doesn't hard-fail because if the saved thumbnail is missing or corrupt, we want
	// to just pull something else instead of erroring out entirely.
	if let Some(path) = &series.thumbnail_path {
		match get_saved_thumbnail(std::path::Path::new(path)).await {
			Ok(result) => return Ok(result.into()),
			Err(_) => {
				tracing::warn!(path = ?path, "Failed to get saved thumbnail");
			},
		}
	}

	// A provider-backed series has no local file to render a thumbnail from,
	// so its source cover is the thumbnail. An uploaded one still wins, which
	// is why this sits after the saved-thumbnail attempt above.
	#[cfg(feature = "providers")]
	if let Some(cover) =
		crate::routers::provider_virtual::virtual_series_cover(&ctx, &user, &id).await
	{
		match cover {
			Ok((content_type, bytes)) => {
				return Ok(ImageResponse::new(content_type, bytes))
			},
			Err(error) => tracing::warn!(
				error,
				series = id,
				"Provider series cover failed; falling back to the first book"
			),
		}
	}

	let first_book = media::Entity::find_for_user(&user)
		.filter(media::Column::SeriesId.eq(series.id.clone()))
		.order_by_asc(media::Column::Name)
		.into_model::<media::MediaThumbSelect>()
		.one(ctx.conn.as_ref())
		.await?;

	let library_config = library_config::Entity::find()
		.filter(
			library_config::Column::LibraryId.in_subquery(
				Query::select()
					.column(series::Column::LibraryId)
					.from(series::Entity)
					.and_where(series::Column::Id.eq(series.id.clone()))
					.to_owned(),
			),
		)
		.one(ctx.conn.as_ref())
		.await?;
	let thumbnail_config = library_config.and_then(|o| o.thumbnail_config);

	let (content_type, bytes) =
		get_series_thumbnail(&series, first_book, thumbnail_config, ctx.config.as_ref())
			.await?;

	Ok(ImageResponse::new(content_type, bytes))
}
