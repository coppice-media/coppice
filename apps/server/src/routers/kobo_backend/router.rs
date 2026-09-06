use axum::{
	body::Bytes,
	extract::{Path, Request, State},
	http::{HeaderMap, HeaderValue, Method},
	middleware::Next,
	response::{IntoResponse, Json, Response},
	Extension,
};
use chrono::{DateTime, SecondsFormat, Utc};
use models::txn::begin_write;
use models::{
	domain::reading_state::{Publication, SourceProtocol},
	entity::{media, reading_head, reading_head_event},
	shared::{
		enums::UserPermission,
		image_processor_options::{
			FitWithinResize, ImageProcessorOptions, ImageResizeMethod,
			SupportedImageFormat,
		},
	},
};
use sea_orm::{ColumnTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map};
use stump_auth::AuthContext;
use stump_core::{
	kobo::{
		entity::{
			kobo_head_update, persist_kobo_reading_state,
			MediaWithMetadataAndReadingSessions,
		},
		sync_types::*,
	},
	reading_state,
};
use stump_devices::{CredentialRef, Protocol};
use stump_kobo::{KoboSync, SyncToken};
use stump_media::{
	image::{GenericImageProcessor, ImageProcessor},
	ContentType,
};

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	middleware::host::HostExtractor,
	routers::api::v2::media::get_media_thumbnail_by_id,
	utils::http::ImageResponse,
	utils::serve_media,
};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct KoboAPIKey {
	pub(crate) api_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct KoboAPIKeyAndBookId {
	pub(crate) api_key: String,
	pub(crate) book_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct KoboThumbnail {
	pub(crate) api_key: String,
	pub(crate) book_id: String,
	pub(crate) width: u32,
	pub(crate) height: u32,
	pub(crate) is_greyscale: Option<String>,
}

// how many items should we send in each page of a sync response?
// this is a maximum; in some cases we may return fewer items in a page.
const ITEMS_PER_PAGE: usize = 100;

pub(crate) struct SyncResponse {
	sync_items: Vec<SyncItem>,
	sync_token: HeaderValue,
	should_continue: bool,
}

impl IntoResponse for SyncResponse {
	fn into_response(self) -> Response {
		let mut response = Json(self.sync_items).into_response();
		if self.should_continue {
			response
				.headers_mut()
				.insert("x-kobo-sync", HeaderValue::from_static("continue"));
		}

		response
			.headers_mut()
			.insert("x-kobo-synctoken", self.sync_token);

		response
	}
}

/// returns dummy tokens for the device, which apparently will be used for
/// some of endpoints kobo will try to call here
#[tracing::instrument(skip_all, err)]
pub(crate) async fn auth_device(body: Bytes) -> APIResult<impl IntoResponse> {
	let user_key = serde_json::from_slice::<serde_json::Value>(&body)
		.ok()
		.and_then(|v| v.get("UserKey").and_then(|k| k.as_str()).map(String::from))
		.unwrap_or_default();

	Ok(Json(json!({
		"AccessToken": uuid::Uuid::new_v4().to_string(),
		"RefreshToken": uuid::Uuid::new_v4().to_string(),
		"TrackingId": uuid::Uuid::new_v4().to_string(),
		"UserKey": user_key,
	})))
}

#[tracing::instrument(skip_all, fields(method = %method, path = %path), err)]
pub(crate) async fn stubbed_route_empty_success(
	method: Method,
	// (api_key,*path), don't need api_key
	Path((_, path)): Path<(String, String)>,
	headers: HeaderMap,
	body: Bytes,
) -> APIResult<impl IntoResponse> {
	// i dont know what comes through here so do not want to log them
	// outside local development
	if cfg!(debug_assertions) {
		let body_str = String::from_utf8_lossy(&body);
		tracing::debug!(?headers, ?body_str, "Kobo hit a stubbed route");
	} else {
		tracing::trace!("Kobo hit a stubbed route");
	}

	Ok(Json(json!({})))
}

/// A secondary authorization middleware to ensure that the user has access to the
/// kobo sync endpoints. This is purely for convenience
#[tracing::instrument(skip_all, err)]
pub(crate) async fn authorize(req: Request, next: Next) -> APIResult<Response> {
	let ctx = req
		.extensions()
		.get::<AuthContext>()
		.ok_or(APIError::Unauthorized)?;
	ctx.enforce_permissions(&[UserPermission::AccessKoboSync])
		.map_err(|_| {
			APIError::Forbidden("You do not have permission to use Kobo sync".to_string())
		})?;
	Ok(next.run(req).await)
}

#[tracing::instrument(skip_all, err)]
pub(crate) async fn initialization(
	HostExtractor(host): HostExtractor,
	Path(KoboAPIKey { api_key, .. }): Path<KoboAPIKey>,
) -> APIResult<impl IntoResponse> {
	let base_url = host.url();
	let template = format!(
		"{}/kobo/{}/v1/books/{{ImageId}}/thumbnail/{{Width}}/{{Height}}/{{IsGreyscale}}/image.jpg",
		base_url, api_key
	);
	let quality_template = format!(
		"{}/kobo/{}/v1/books/{{ImageId}}/thumbnail/{{Width}}/{{Height}}/{{Quality}}/{{IsGreyscale}}/image.jpg",
		base_url, api_key
	);

	let mut resources = stump_core::kobo::native_kobo_resources().clone();
	stump_core::kobo::rewrite_kobo_resources(&mut resources, &base_url, &api_key);
	if let Some(obj) = resources.as_object_mut() {
		obj.insert(
			"image_host".to_string(),
			serde_json::Value::String(base_url.to_string()),
		);
		obj.insert(
			"image_url_template".to_string(),
			serde_json::Value::String(template),
		);
		obj.insert(
			"image_url_quality_template".to_string(),
			serde_json::Value::String(quality_template),
		);
	}

	let mut headers = HeaderMap::new();
	// Note: i couldn't find reference to _why_ this is needed, but Komga includes the header. it should be
	// harmless, as e30= is just a base64 of "{}"
	headers.insert("x-kobo-apitoken", HeaderValue::from_static("e30="));

	Ok((headers, Json(json!({ "Resources": resources }))))
}

fn device_metadata(headers: &HeaderMap) -> serde_json::Map<String, serde_json::Value> {
	let mut result = Map::new();
	for (key, val) in headers.iter() {
		let key = key.to_string();

		if !key.starts_with("x-kobo-") || key == "x-kobo-synctoken" {
			continue;
		}

		let val = match val.to_str() {
			Ok(v) => v.to_string(),
			Err(_) => continue,
		};

		result.insert(key, serde_json::Value::String(val));
	}

	result
}

#[tracing::instrument(skip_all, err)]
pub(crate) async fn library_sync(
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	HostExtractor(host): HostExtractor,
	Path(KoboAPIKey { api_key, .. }): Path<KoboAPIKey>,
	headers: HeaderMap,
) -> APIResult<SyncResponse> {
	let conn = ctx.conn.as_ref();
	let user = req.user();

	let client_sync_token = headers.get("x-kobo-synctoken").and_then(|h| {
		match SyncToken::try_from_header_value(h) {
			Ok(sync_token) => Some(sync_token),
			Err(e) => {
				tracing::error!(?e, "Could not parse client's Kobo sync token");
				None
			},
		}
	});

	let device_id = headers.get("x-kobo-deviceid").and_then(|h| h.to_str().ok());
	if device_id.is_none() {
		// the device ID is not critical to the sync process, but it's useful metadata.
		tracing::warn!("Client did not pass a valid x-kobo-deviceid");
	}

	let device_metadata = device_metadata(&headers);

	let sync_page = KoboSync::next_page(
		conn,
		&user,
		device_id,
		serde_json::Value::Object(device_metadata),
		client_sync_token.as_ref(),
		ITEMS_PER_PAGE,
		super::comic_transform::sync_extensions(&ctx.config),
	)
	.await?;

	let kobo_api_base_url = format!("{}/kobo/{}", host.url(), api_key);
	let mut sync_items = sync_page.sync_items(kobo_api_base_url.as_str()).await?;
	if ctx.config.transform.transform_enabled {
		super::comic_transform::advertise_cached_sizes(&ctx, &req, &mut sync_items).await;
	}
	if ctx.config.protocols.kobo_kepub_conversion {
		for item in &sync_items {
			if let SyncItem::NewEntitlement(entitlement) = item {
				super::kepub_cache::enqueue(
					ctx.clone(),
					entitlement.book_entitlement.id.clone(),
				);
			}
		}
	}

	// Record the sync on the device this key belongs to; the sync itself must
	// never fail because of the registry.
	let sync_summary = serde_json::json!({
		"protocol": "kobo",
		"items": sync_items.len(),
		"new_entitlements": sync_items
			.iter()
			.filter(|item| matches!(item, SyncItem::NewEntitlement(_)))
			.count(),
		"should_continue": sync_page.should_continue,
		"kobo_device_id": device_id,
	});
	if let Err(error) = ctx
		.devices()
		.touch(
			CredentialRef::ApiKey(&api_key),
			Protocol::Kobo,
			Some(sync_summary),
		)
		.await
	{
		tracing::warn!(?error, "Failed to record the Kobo sync on its device");
	}

	// if we don't send a sync token the client will send no sync token on its next sync,
	// essentially starting the sync process from scratch. that's not a disaster, but it's
	// a weird enough case that it's simpler to error loudly.
	let sync_token = sync_page.sync_token.try_to_header_value().map_err(|e| {
		tracing::warn!(?e, "Failed to produce Kobo sync token");
		APIError::InternalServerError("Could not produce a Kobo sync token".to_string())
	})?;

	Ok(SyncResponse {
		sync_items,
		should_continue: sync_page.should_continue,
		sync_token,
	})
}

fn kobo_timestamp(timestamp: DateTime<Utc>) -> String {
	timestamp.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The raw Kobo state request retained as provenance for `event`.
fn raw_reading_state_update(
	event: Option<&reading_head_event::Model>,
) -> Option<ReadingStateUpdate> {
	event
		.and_then(|event| {
			serde_json::from_value::<ReadingStateUpdateRequest>(event.raw_payload.clone())
				.ok()
		})
		.and_then(|request| request.reading_states.into_iter().next())
}

fn head_status_name(head: Option<&reading_head::Model>) -> &'static str {
	match head {
		Some(head) if head.completed => "Finished",
		Some(_) => "Reading",
		None => "ReadyToRead",
	}
}

fn response_location(
	raw: Option<&LocationUpdate>,
	locator: Option<&models::shared::readium::ReadiumLocator>,
) -> Option<KoboLocationResponse> {
	if let Some(location) = raw {
		if location
			.value
			.as_deref()
			.is_some_and(|value| !value.is_empty())
		{
			return Some(KoboLocationResponse {
				value: location.value.clone(),
				type_: location.type_.clone(),
				source: location.source.clone().unwrap_or_default(),
			});
		}
	}

	locator.and_then(|locator| {
		locator.kobo_span.as_ref().map(|span| KoboLocationResponse {
			value: Some(span.clone()),
			type_: Some("KoboSpan".to_string()),
			source: locator.href.clone(),
		})
	})
}

/// Project the unified head back into Kobo's `ReadingState`.
///
/// Progress and location come from the head; when the head was last moved by
/// this Kobo lane (`kobo_event` is its winning event) the raw bookmark values
/// are echoed for an exact round trip. Statistics and status extras always
/// come from the latest Kobo request, whether or not it won the head.
fn reading_state_response(
	book: &media::Model,
	head: Option<&reading_head::Model>,
	kobo_event: Option<&reading_head_event::Model>,
) -> KoboReadingStateResponse {
	let modified = head
		.map(|head| head.updated_at.to_utc())
		.unwrap_or_else(|| book.modified_at.unwrap_or(book.created_at).to_utc());
	let raw_update = raw_reading_state_update(kobo_event);
	let kobo_won = match (head, kobo_event) {
		(Some(head), Some(event)) => head.event_id == event.id,
		_ => false,
	};
	let raw_bookmark = raw_update
		.as_ref()
		.filter(|_| kobo_won)
		.and_then(|update| update.current_bookmark.as_ref());
	let raw_statistics = raw_update
		.as_ref()
		.and_then(|update| update.statistics.as_ref());
	let raw_status_info = raw_update
		.as_ref()
		.and_then(|update| update.status_info.as_ref());

	let status = raw_status_info
		.filter(|_| kobo_won)
		.and_then(|status_info| status_info.status.clone())
		.unwrap_or_else(|| head_status_name(head).to_string());
	let head_progress = head.map(|head| head.progression as f32 * 100.0);
	let head_content_progress = head
		.and_then(|head| head.locator.as_ref())
		.and_then(|locator| locator.locations.as_ref())
		.and_then(|locations| locations.progression)
		.and_then(|value| rust_decimal::prelude::ToPrimitive::to_f32(&value))
		.map(|value| value * 100.0);
	let progress_percent = raw_bookmark
		.and_then(|bookmark| bookmark.progress_percent)
		.or(head_progress);
	let content_source_progress_percent = raw_bookmark
		.and_then(|bookmark| bookmark.content_source_progress_percent)
		.or(head_content_progress)
		.or(progress_percent);

	let times_started_reading = raw_status_info
		.and_then(|status_info| status_info.times_started_reading)
		.unwrap_or_else(|| u32::from(head.is_some_and(|head| !head.completed)));
	let last_time_started_reading = raw_status_info
		.and_then(|status_info| status_info.last_time_started_reading.clone());

	KoboReadingStateResponse {
		entitlement_id: book.id.clone(),
		created: kobo_timestamp(
			head.map_or_else(
				|| book.created_at.to_utc(),
				|head| head.created_at.to_utc(),
			),
		),
		last_modified: kobo_timestamp(modified),
		priority_timestamp: kobo_timestamp(modified),
		status_info: KoboStatusInfoResponse {
			last_modified: kobo_timestamp(modified),
			status,
			times_started_reading,
			last_time_started_reading,
		},
		statistics: KoboStatisticsResponse {
			last_modified: kobo_timestamp(modified),
			spent_reading_minutes: raw_statistics
				.and_then(|statistics| statistics.spent_reading_minutes),
			remaining_time_minutes: raw_statistics
				.and_then(|statistics| statistics.remaining_time_minutes),
		},
		current_bookmark: KoboCurrentBookmarkResponse {
			last_modified: kobo_timestamp(modified),
			progress_percent,
			content_source_progress_percent,
			location: response_location(
				raw_bookmark.and_then(|bookmark| bookmark.location.as_ref()),
				head.and_then(|head| head.locator.as_ref()),
			),
		},
	}
}

#[tracing::instrument(skip_all, fields(book_id = %book_id), err)]
pub(crate) async fn book_state(
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	Path(KoboAPIKeyAndBookId { book_id, .. }): Path<KoboAPIKeyAndBookId>,
) -> APIResult<Json<Vec<KoboReadingStateResponse>>> {
	let user = req.user();
	let conn = ctx.conn.as_ref();
	let book = media::Entity::find_for_user(&user)
		.filter(media::Column::Id.eq(book_id))
		.one(conn)
		.await?
		.ok_or(APIError::NotFound("Book not found".to_string()))?;
	let head = reading_state::head(conn, &user.id, &book.id).await?;
	let kobo_event =
		reading_state::latest_event(conn, &user.id, &book.id, SourceProtocol::Kobo)
			.await?;

	Ok(Json(vec![reading_state_response(
		&book,
		head.as_ref(),
		kobo_event.as_ref(),
	)]))
}

#[tracing::instrument(skip_all, fields(book_id = %book_id), err)]
pub(crate) async fn update_book_state(
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	Path(KoboAPIKeyAndBookId { book_id, .. }): Path<KoboAPIKeyAndBookId>,
	headers: HeaderMap,
	body: Bytes,
) -> APIResult<Json<KoboStateUpdateResponse>> {
	let update_request = serde_json::from_slice::<ReadingStateUpdateRequest>(&body)
		.map_err(|error| {
			APIError::BadRequest(format!("Malformed Kobo state request: {error}"))
		})?;
	let update = update_request.reading_states.first().ok_or_else(|| {
		APIError::BadRequest("ReadingStates must contain one state".to_string())
	})?;
	let raw_payload =
		serde_json::from_slice::<serde_json::Value>(&body).map_err(|error| {
			APIError::BadRequest(format!("Malformed Kobo state request: {error}"))
		})?;
	let device_id = headers
		.get("x-kobo-deviceid")
		.and_then(|value| value.to_str().ok())
		.map(str::to_string);
	let head_update = kobo_head_update(update, raw_payload.clone(), device_id.clone())
		.map_err(APIError::BadRequest)?;

	let user = req.user();
	let conn = ctx.conn.as_ref();
	let book = media::Entity::find_for_user(&user)
		.filter(media::Column::Id.eq(book_id.clone()))
		.one(conn)
		.await?
		.ok_or(APIError::NotFound("Book not found".to_string()))?;

	let transaction = begin_write(conn).await?;
	persist_kobo_reading_state(
		&transaction,
		&user,
		&book_id,
		update,
		raw_payload,
		device_id,
	)
	.await?;
	let applied = reading_state::apply(
		&transaction,
		&user.id,
		Publication::from(&book),
		head_update,
	)
	.await?;
	transaction.commit().await?;
	reading_state::announce(&ctx, &book, &applied);

	let modified = applied.head.updated_at.to_utc();
	let result = KoboStateUpdateResult {
		entitlement_id: book_id,
		current_bookmark_result: update.current_bookmark.as_ref().map(|_| {
			KoboStateResult {
				result: "Success".to_string(),
			}
		}),
		statistics_result: update.statistics.as_ref().map(|_| KoboStateResult {
			result: "Success".to_string(),
		}),
		status_info_result: update.status_info.as_ref().map(|_| KoboStateResult {
			result: "Success".to_string(),
		}),
		last_modified: kobo_timestamp(modified),
		priority_timestamp: kobo_timestamp(modified),
	};

	Ok(Json(KoboStateUpdateResponse {
		request_result: "Success".to_string(),
		update_results: vec![result],
	}))
}

#[tracing::instrument(skip_all, fields(book_id = %book_id), err)]
pub(crate) async fn book_metadata(
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	HostExtractor(host): HostExtractor,
	Path(KoboAPIKeyAndBookId { api_key, book_id }): Path<KoboAPIKeyAndBookId>,
) -> APIResult<Json<Vec<BookMetadata>>> {
	let conn = ctx.conn.as_ref();
	let user = req.user();

	// TODO: ive noticed that restoring the kobo api_endpoint doesn't remove books when syncing, and it doesn't fail to sync.
	// i assume it would have returned 404s, however when i came back and tested a local server (so none of the synced books were present)
	// the sync would fail with error message "Book not found". that makes me feel like perhaps we should not be hard-erroring
	// if not found, just don't return it in the response?

	let m = MediaWithMetadataAndReadingSessions::find_by_id_for_user(book_id, &user)
		.into_model::<MediaWithMetadataAndReadingSessions>()
		.one(conn)
		.await?
		.ok_or(APIError::NotFound("Book not found".to_string()))?;

	let book_url = format!(
		"{}/kobo/{}/v1/books/{}/file/epub",
		host.url(),
		api_key,
		m.media.id
	);

	let is_comic = ctx.config.transform.transform_enabled
		&& stump_media::transform::is_comic_source(&m.media.path);
	let (format, size) = if is_comic {
		let book = media::MediaIdentSelect {
			id: m.media.id.clone(),
			path: m.media.path.clone(),
		};
		super::comic_transform::advertised_download(&ctx, &req, &book).await
	} else if ctx.config.protocols.kobo_kepub_conversion
		&& m.media.extension.eq_ignore_ascii_case("epub")
	{
		(Format::KEPUB, None)
	} else {
		(Format::EPUB3, None)
	};
	let mut result = BookMetadata::from_media_with_format(&m, book_url, format);
	if let Some(size) = size {
		for url in &mut result.download_urls {
			url.size = size;
		}
	}
	Ok(Json(vec![result]))
}

#[tracing::instrument(skip_all, fields(book_id = %book_id, width, height), err)]
pub(crate) async fn book_thumbnail(
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	Path(KoboThumbnail {
		book_id,
		width,
		height,
		..
	}): Path<KoboThumbnail>,
) -> APIResult<ImageResponse> {
	let result = get_media_thumbnail_by_id(&ctx, &req.user(), book_id).await?;

	// the Kobo only supports JPEGs, and doesn't need large thumbnails.
	let jpeg_buffer = tokio::task::block_in_place(|| {
		let converted = GenericImageProcessor::generate(
			&result.data,
			ImageProcessorOptions {
				format: SupportedImageFormat::Jpeg,
				resize_method: Some(ImageResizeMethod::FitWithin(FitWithinResize {
					width,
					height,
				})),
				..Default::default()
			},
		)?;
		Ok::<Vec<u8>, APIError>(converted)
	})?;

	Ok(ImageResponse::new(ContentType::JPEG, jpeg_buffer))
}

#[tracing::instrument(skip_all, fields(book_id = %book_id), err)]
pub(crate) async fn book_download(
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	Path(KoboAPIKeyAndBookId { book_id, .. }): Path<KoboAPIKeyAndBookId>,
	headers: HeaderMap,
) -> APIResult<impl IntoResponse> {
	serve_media::serve_media_file(req, headers, ctx.conn.as_ref(), book_id).await
}
