//! Keyed CrossPoint rich-sync routes.
//!
//! The existing `stump_koreader` router remains the stock KOSync contract. This
//! module adds only `/koreader/{api_key}/api/v1/*` and always derives ownership
//! from `AuthContext::device_id`, never from a client-supplied device id.

use axum::{
	extract::{Path, Query, State},
	response::Json,
	routing::{get, put},
	Extension, Router,
};
use chrono::{DateTime, Utc};
use models::entity::{
	crosspoint_bookmark, crosspoint_clipping, crosspoint_document, crosspoint_progress,
	crosspoint_stats_book, crosspoint_stats_global, device,
};
use models::shared::enums::DeviceKind;
use sea_orm::{
	sea_query::OnConflict, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde::Deserialize;
use serde_json::{json, Value};
use stump_auth::AuthContext;
use stump_core::Ctx;
use stump_crosspoint::sync::{Bookmark, Clipping, Position};

use crate::{config::state::AppState, errors::APIError, errors::APIResult};

/// Mounts all rich routes below the full keyed path. The API-key middleware is
/// applied by the parent Koreader router after this router is merged.
pub(crate) fn router() -> Router<AppState> {
	Router::new()
		.route(
			"/koreader/{api_key}/api/v1/progress",
			put(put_progress).get(list_progress),
		)
		.route(
			"/koreader/{api_key}/api/v1/progress/{document}",
			get(get_progress),
		)
		.route(
			"/koreader/{api_key}/api/v1/bookmarks/{document}",
			put(put_bookmarks).get(get_bookmarks),
		)
		.route(
			"/koreader/{api_key}/api/v1/clippings/{document}",
			put(put_clippings).get(get_clippings),
		)
		.route(
			"/koreader/{api_key}/api/v1/documents",
			put(put_documents).get(get_documents),
		)
		.route(
			"/koreader/{api_key}/api/v1/stats/global",
			put(put_global_stats).get(get_global_stats),
		)
		.route(
			"/koreader/{api_key}/api/v1/stats/books",
			put(put_book_stats),
		)
		.route(
			"/koreader/{api_key}/api/v1/stats/books/{document}",
			get(get_book_stats),
		)
		.route(
			"/koreader/{api_key}/api/v1/stats/summary",
			get(get_stats_summary),
		)
}

#[derive(Debug, Deserialize)]
struct ProgressQuery {
	#[serde(default)]
	limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DeltaQuery {
	#[serde(default)]
	since: Option<i64>,
	#[serde(default)]
	limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawProgress {
	#[serde(default)]
	document: Option<String>,
	#[serde(default)]
	progress: Option<String>,
	#[serde(default)]
	percentage: Option<f32>,
	#[serde(default)]
	device: Option<String>,
	#[serde(default)]
	device_id: Option<String>,
	#[serde(default)]
	position: Option<Value>,
	#[serde(default)]
	metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct Items<T> {
	items: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct RawDocument {
	document: String,
	#[serde(default)]
	title: Option<String>,
	#[serde(default)]
	author: Option<String>,
	#[serde(default)]
	filename: Option<String>,
	#[serde(default)]
	filesize: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RawGlobalStats {
	#[serde(default)]
	device_id: Option<String>,
	#[serde(default)]
	device: Option<String>,
	#[serde(default, rename = "v")]
	version: Option<i32>,
	#[serde(default)]
	sessions: Option<i64>,
	#[serde(default)]
	seconds: Option<i64>,
	#[serde(default)]
	pages: Option<i64>,
	#[serde(default)]
	completed: Option<i64>,
	#[serde(default)]
	tod: Option<Value>,
	#[serde(default)]
	dow: Option<Value>,
	#[serde(default)]
	anchor_day: Option<i64>,
	#[serde(default)]
	history_b64: Option<String>,
	#[serde(default)]
	streak: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RawBookStats {
	document: String,
	#[serde(default, rename = "v")]
	version: i32,
	#[serde(default)]
	sessions: i64,
	#[serde(default)]
	seconds: i64,
	#[serde(default)]
	pages: i64,
	#[serde(default)]
	completed: bool,
	#[serde(default)]
	avg_fwd: i64,
	#[serde(default)]
	pace_n: i64,
	#[serde(default)]
	eta: i64,
	#[serde(default)]
	start_manual: bool,
	#[serde(default)]
	finish_manual: bool,
	#[serde(default)]
	start_date: i64,
	#[serde(default)]
	finished_date: i64,
	#[serde(default)]
	tod: Value,
	#[serde(default)]
	dow: Value,
}

fn now() -> DateTime<chrono::FixedOffset> {
	DateTime::from(Utc::now())
}

fn now_seconds() -> i64 {
	Utc::now().timestamp()
}

fn db_error(error: sea_orm::DbErr) -> APIError {
	APIError::from(error)
}

fn valid_document(document: &str) -> bool {
	!document.is_empty()
		&& document.len() <= stump_crosspoint::sync::MAX_DOCUMENT_BYTES
		&& document.bytes().all(|byte| {
			byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
		})
}

fn check_device_id(value: Option<&str>, bound: &str) -> APIResult<()> {
	if let Some(reported) = value {
		if reported != bound {
			return Err(APIError::Forbidden(
				"device_id does not match the credential-bound device".to_string(),
			));
		}
	}
	Ok(())
}

async fn bound_device(ctx: &Ctx, auth: &AuthContext) -> APIResult<(String, String)> {
	let device_id = auth
		.device_id
		.as_deref()
		.ok_or(APIError::Unauthorized)?
		.to_owned();
	let user_id = auth.user.id.clone();
	let device = device::Entity::find_by_id(device_id.clone())
		.filter(device::Column::UserId.eq(user_id.clone()))
		.filter(device::Column::Kind.eq(DeviceKind::Crosspoint))
		.filter(device::Column::RevokedAt.is_null())
		.one(ctx.conn.as_ref())
		.await
		.map_err(db_error)?
		.ok_or_else(|| {
			APIError::Forbidden("credential is not a CrossPoint device".to_string())
		})?;
	Ok((user_id, device.id))
}

fn parse_position(value: Option<Value>) -> Option<Value> {
	let position = serde_json::from_value::<Position>(value?).ok()?;
	position
		.is_valid()
		.then(|| serde_json::to_value(position).ok())
		.flatten()
}

fn parse_limit(value: Option<u64>, default: u64, maximum: u64) -> usize {
	value.unwrap_or(default).clamp(1, maximum) as usize
}

fn valid_progress(input: &RawProgress) -> APIResult<(&str, &str, f32)> {
	let document = input
		.document
		.as_deref()
		.filter(|value| valid_document(value))
		.ok_or_else(|| APIError::BadRequest("invalid document".to_string()))?;
	let progress = input
		.progress
		.as_deref()
		.filter(|value| {
			!value.is_empty() && value.len() <= stump_crosspoint::sync::MAX_PROGRESS_BYTES
		})
		.ok_or_else(|| APIError::BadRequest("invalid progress".to_string()))?;
	let percentage = input
		.percentage
		.filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
		.ok_or_else(|| APIError::BadRequest("invalid percentage".to_string()))?;
	Ok((document, progress, percentage))
}

fn metadata_field(metadata: Option<&Value>, key: &str) -> Option<String> {
	metadata
		.and_then(Value::as_object)
		.and_then(|object| object.get(key))
		.and_then(Value::as_str)
		.map(str::to_owned)
		.filter(|value| !value.is_empty() && value.len() <= 512)
}

fn valid_metadata(metadata: Option<&Value>) -> bool {
	metadata.and_then(Value::as_object).is_some_and(|object| {
		object.values().all(|value| {
			value
				.as_str()
				.is_some_and(|value| !value.is_empty() && value.len() <= 512)
		})
	})
}

async fn put_progress(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Json(input): Json<RawProgress>,
) -> APIResult<Json<Value>> {
	let (user_id, device_id) = bound_device(ctx.as_ref(), &auth).await?;
	check_device_id(input.device_id.as_deref(), &device_id)?;
	let (document, progress, percentage) = valid_progress(&input)?;
	let document = document.to_owned();
	let progress = progress.to_owned();
	let old = crosspoint_progress::Entity::find()
		.filter(crosspoint_progress::Column::UserId.eq(user_id.clone()))
		.filter(crosspoint_progress::Column::Document.eq(document.clone()))
		.filter(crosspoint_progress::Column::DeviceId.eq(device_id.clone()))
		.one(ctx.conn.as_ref())
		.await
		.map_err(db_error)?;
	let position_json = parse_position(input.position)
		.or_else(|| old.as_ref().and_then(|row| row.position_json.clone()));
	let metadata_json = input
		.metadata
		.as_ref()
		.filter(|value| valid_metadata(Some(value)))
		.cloned()
		.or_else(|| old.as_ref().and_then(|row| row.metadata_json.clone()));
	let active = crosspoint_progress::ActiveModel {
		user_id: Set(user_id.clone()),
		document: Set(document.clone()),
		device_id: Set(device_id),
		device: Set(input.device),
		progress: Set(progress),
		percentage: Set(f64::from(percentage)),
		position_json: Set(position_json),
		metadata_json: Set(metadata_json),
		updated_at: Set(now()),
	};
	crosspoint_progress::Entity::insert(active)
		.on_conflict(
			OnConflict::columns([
				crosspoint_progress::Column::UserId,
				crosspoint_progress::Column::Document,
				crosspoint_progress::Column::DeviceId,
			])
			.update_columns([
				crosspoint_progress::Column::Device,
				crosspoint_progress::Column::Progress,
				crosspoint_progress::Column::Percentage,
				crosspoint_progress::Column::PositionJson,
				crosspoint_progress::Column::MetadataJson,
				crosspoint_progress::Column::UpdatedAt,
			])
			.to_owned(),
		)
		.exec(ctx.conn.as_ref())
		.await
		.map_err(db_error)?;

	if let Some(metadata) = input.metadata.filter(|value| valid_metadata(Some(value))) {
		let old_document = crosspoint_document::Entity::find()
			.filter(crosspoint_document::Column::UserId.eq(user_id.clone()))
			.filter(crosspoint_document::Column::Document.eq(document.clone()))
			.one(ctx.conn.as_ref())
			.await
			.map_err(db_error)?;
		let active = crosspoint_document::ActiveModel {
			user_id: Set(user_id),
			document: Set(document.clone()),
			title: Set(metadata_field(Some(&metadata), "title")
				.or_else(|| old_document.as_ref().and_then(|row| row.title.clone()))),
			author: Set(metadata_field(Some(&metadata), "authors")
				.or_else(|| metadata_field(Some(&metadata), "author"))
				.or_else(|| old_document.as_ref().and_then(|row| row.author.clone()))),
			filename: Set(metadata_field(Some(&metadata), "filename")
				.or_else(|| old_document.as_ref().and_then(|row| row.filename.clone()))),
			filesize: Set(old_document.and_then(|row| row.filesize)),
			updated_at: Set(now()),
		};
		crosspoint_document::Entity::insert(active)
			.on_conflict(
				OnConflict::columns([
					crosspoint_document::Column::UserId,
					crosspoint_document::Column::Document,
				])
				.update_columns([
					crosspoint_document::Column::Title,
					crosspoint_document::Column::Author,
					crosspoint_document::Column::Filename,
					crosspoint_document::Column::UpdatedAt,
				])
				.to_owned(),
			)
			.exec(ctx.conn.as_ref())
			.await
			.map_err(db_error)?;
	}
	Ok(Json(
		json!({"document": document, "timestamp": now_seconds()}),
	))
}

fn progress_json(
	row: &crosspoint_progress::Model,
	metadata: Option<&crosspoint_document::Model>,
) -> Value {
	json!({
		"document": row.document,
		"device_id": row.device_id,
		"device": row.device,
		"progress": row.progress,
		"percentage": row.percentage,
		"position": row.position_json,
		"timestamp": row.updated_at.timestamp(),
		"title": metadata.and_then(|value| value.title.clone()),
		"author": metadata.and_then(|value| value.author.clone()),
		"filename": metadata.and_then(|value| value.filename.clone()),
	})
}

async fn list_progress(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ProgressQuery>,
) -> APIResult<Json<Value>> {
	let (user_id, _) = bound_device(ctx.as_ref(), &auth).await?;
	let limit = parse_limit(query.limit, 100, stump_crosspoint::sync::MAX_DOCUMENTS_PAGE);
	let rows = crosspoint_progress::Entity::find()
		.filter(crosspoint_progress::Column::UserId.eq(user_id.clone()))
		.order_by_desc(crosspoint_progress::Column::UpdatedAt)
		.order_by_asc(crosspoint_progress::Column::Document)
		.all(ctx.conn.as_ref())
		.await
		.map_err(db_error)?;
	let mut seen = std::collections::HashSet::new();
	let mut items = Vec::new();
	for row in rows {
		if !seen.insert(row.document.clone()) {
			continue;
		}
		if items.len() >= limit {
			break;
		}
		let metadata = crosspoint_document::Entity::find()
			.filter(crosspoint_document::Column::UserId.eq(user_id.clone()))
			.filter(crosspoint_document::Column::Document.eq(row.document.clone()))
			.one(ctx.conn.as_ref())
			.await
			.map_err(db_error)?;
		items.push(progress_json(&row, metadata.as_ref()));
	}
	Ok(Json(json!({"items": items})))
}

async fn get_progress(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Path(document): Path<String>,
) -> APIResult<Json<Value>> {
	let (user_id, _) = bound_device(ctx.as_ref(), &auth).await?;
	if !valid_document(&document) {
		return Err(APIError::BadRequest("invalid document".to_string()));
	}
	let rows = crosspoint_progress::Entity::find()
		.filter(crosspoint_progress::Column::UserId.eq(user_id.clone()))
		.filter(crosspoint_progress::Column::Document.eq(document.clone()))
		.order_by_desc(crosspoint_progress::Column::UpdatedAt)
		.order_by_asc(crosspoint_progress::Column::DeviceId)
		.all(ctx.conn.as_ref())
		.await
		.map_err(db_error)?;
	let metadata = crosspoint_document::Entity::find()
		.filter(crosspoint_document::Column::UserId.eq(user_id))
		.filter(crosspoint_document::Column::Document.eq(document.clone()))
		.one(ctx.conn.as_ref())
		.await
		.map_err(db_error)?;
	Ok(Json(json!({
		"document": document,
		"devices": rows.iter().map(|row| progress_json(row, metadata.as_ref())).collect::<Vec<_>>(),
	})))
}

fn valid_hex_id(value: &str) -> bool {
	value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_bookmark(value: &Bookmark) -> bool {
	valid_hex_id(&value.id)
		&& (value.deleted == 1
			|| value
				.xpath
				.as_deref()
				.is_some_and(|xpath| !xpath.is_empty() && xpath.len() <= 512))
		&& value.percentage.is_none_or(|percentage| {
			percentage.is_finite() && (0.0..=1.0).contains(&percentage)
		}) && value
		.summary
		.as_deref()
		.is_none_or(|summary| summary.len() <= stump_crosspoint::sync::MAX_SUMMARY_BYTES)
}

fn valid_clipping(value: &Clipping) -> bool {
	valid_hex_id(&value.id)
		&& (value.deleted == 1
			|| value.text.as_deref().is_some_and(|text| {
				!text.is_empty()
					&& text.len() <= stump_crosspoint::sync::MAX_CLIPPING_TEXT_BYTES
			})) && value
		.chapter
		.as_deref()
		.is_none_or(|chapter| chapter.chars().count() <= 64)
		&& value.note.as_deref().is_none_or(|note| {
			note.len() <= stump_crosspoint::sync::MAX_CLIPPING_NOTE_BYTES
		})
}

fn bookmark_json(row: &crosspoint_bookmark::Model) -> Value {
	json!({
		"id": row.id,
		"xpath": row.xpath,
		"percentage": row.percentage,
		"si": row.spine_index,
		"pc": row.paragraph_count,
		"pp": row.paragraph_pos,
		"summary": row.summary,
		"deleted": i32::from(row.deleted),
		"updated_at": row.updated_at.timestamp(),
	})
}

async fn get_bookmarks(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Path(document): Path<String>,
	Query(query): Query<DeltaQuery>,
) -> APIResult<Json<Value>> {
	let (user_id, _) = bound_device(ctx.as_ref(), &auth).await?;
	if !valid_document(&document) {
		return Err(APIError::BadRequest("invalid document".to_string()));
	}
	let since = query.since.unwrap_or(0);
	let limit = parse_limit(query.limit, 50, stump_crosspoint::sync::MAX_DELTA_PAGE);
	let since_time = DateTime::<Utc>::from_timestamp(since, 0)
		.map(DateTime::from)
		.unwrap_or_else(now);
	let rows = crosspoint_bookmark::Entity::find()
		.filter(crosspoint_bookmark::Column::UserId.eq(user_id))
		.filter(crosspoint_bookmark::Column::Document.eq(document.clone()))
		.filter(crosspoint_bookmark::Column::UpdatedAt.gt(since_time))
		.order_by_asc(crosspoint_bookmark::Column::UpdatedAt)
		.order_by_asc(crosspoint_bookmark::Column::Id)
		.all(ctx.conn.as_ref())
		.await
		.map_err(db_error)?;
	let more = rows.len() > limit;
	let until = rows
		.iter()
		.take(limit)
		.map(|row| row.updated_at.timestamp())
		.max()
		.unwrap_or_else(now_seconds);
	Ok(Json(json!({
		"document": document,
		"until": until,
		"more": more,
		"items": rows.iter().take(limit).map(bookmark_json).collect::<Vec<_>>(),
	})))
}

async fn put_bookmarks(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Path(document): Path<String>,
	Json(input): Json<Items<Bookmark>>,
) -> APIResult<Json<Value>> {
	let (user_id, _) = bound_device(ctx.as_ref(), &auth).await?;
	if !valid_document(&document) {
		return Err(APIError::BadRequest("invalid document".to_string()));
	}
	if input.items.len() > stump_crosspoint::sync::MAX_BATCH_ITEMS {
		return Err(APIError::BadRequest(
			"bookmark batch is too large".to_string(),
		));
	}
	let update_time = now();
	for bookmark in &input.items {
		if !valid_bookmark(bookmark) {
			return Err(APIError::BadRequest("invalid bookmark".to_string()));
		}
		let old = crosspoint_bookmark::Entity::find()
			.filter(crosspoint_bookmark::Column::UserId.eq(user_id.clone()))
			.filter(crosspoint_bookmark::Column::Document.eq(document.clone()))
			.filter(crosspoint_bookmark::Column::Id.eq(bookmark.id.clone()))
			.one(ctx.conn.as_ref())
			.await
			.map_err(db_error)?;
		let deleted = bookmark.deleted == 1;
		let active = crosspoint_bookmark::ActiveModel {
			user_id: Set(user_id.clone()),
			document: Set(document.clone()),
			id: Set(bookmark.id.clone()),
			xpath: Set(if deleted {
				old.as_ref().and_then(|row| row.xpath.clone())
			} else {
				bookmark.xpath.clone()
			}),
			percentage: Set(if deleted {
				old.as_ref().and_then(|row| row.percentage)
			} else {
				bookmark.percentage.map(f64::from)
			}),
			summary: Set(if deleted {
				old.as_ref().and_then(|row| row.summary.clone())
			} else {
				bookmark.summary.clone()
			}),
			spine_index: Set(bookmark.spine_index.map(i32::from)),
			paragraph_count: Set(bookmark.paragraph_count.map(i32::from)),
			paragraph_pos: Set(bookmark.paragraph_pos.map(i32::from)),
			deleted: Set(deleted),
			updated_at: Set(update_time),
		};
		crosspoint_bookmark::Entity::insert(active)
			.on_conflict(
				OnConflict::columns([
					crosspoint_bookmark::Column::UserId,
					crosspoint_bookmark::Column::Document,
					crosspoint_bookmark::Column::Id,
				])
				.update_columns([
					crosspoint_bookmark::Column::Xpath,
					crosspoint_bookmark::Column::Percentage,
					crosspoint_bookmark::Column::Summary,
					crosspoint_bookmark::Column::SpineIndex,
					crosspoint_bookmark::Column::ParagraphCount,
					crosspoint_bookmark::Column::ParagraphPos,
					crosspoint_bookmark::Column::Deleted,
					crosspoint_bookmark::Column::UpdatedAt,
				])
				.to_owned(),
			)
			.exec(ctx.conn.as_ref())
			.await
			.map_err(db_error)?;
	}
	Ok(Json(json!({
		"until": update_time.timestamp(),
		"accepted": input.items.len(),
	})))
}

fn clipping_json(row: &crosspoint_clipping::Model) -> Value {
	json!({
		"id": row.id,
		"spine": row.spine,
		"start_page": row.start_page,
		"end_page": row.end_page,
		"pages": row.pages,
		"start_word": row.start_word,
		"end_word": row.end_word,
		"words": row.words,
		"para": row.para,
		"chapter": row.chapter,
		"text": row.text,
		"note": row.note,
		"color": row.color,
		"created_at": row.created_at,
		"deleted": i32::from(row.deleted),
		"updated_at": row.updated_at.timestamp(),
	})
}

async fn get_clippings(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Path(document): Path<String>,
	Query(query): Query<DeltaQuery>,
) -> APIResult<Json<Value>> {
	let (user_id, _) = bound_device(ctx.as_ref(), &auth).await?;
	if !valid_document(&document) {
		return Err(APIError::BadRequest("invalid document".to_string()));
	}
	let since = query.since.unwrap_or(0);
	let limit = parse_limit(query.limit, 50, stump_crosspoint::sync::MAX_DELTA_PAGE);
	let since_time = DateTime::<Utc>::from_timestamp(since, 0)
		.map(DateTime::from)
		.unwrap_or_else(now);
	let rows = crosspoint_clipping::Entity::find()
		.filter(crosspoint_clipping::Column::UserId.eq(user_id))
		.filter(crosspoint_clipping::Column::Document.eq(document.clone()))
		.filter(crosspoint_clipping::Column::UpdatedAt.gt(since_time))
		.order_by_asc(crosspoint_clipping::Column::UpdatedAt)
		.order_by_asc(crosspoint_clipping::Column::Id)
		.all(ctx.conn.as_ref())
		.await
		.map_err(db_error)?;
	let more = rows.len() > limit;
	let until = rows
		.iter()
		.take(limit)
		.map(|row| row.updated_at.timestamp())
		.max()
		.unwrap_or_else(now_seconds);
	Ok(Json(json!({
		"document": document,
		"until": until,
		"more": more,
		"items": rows.iter().take(limit).map(clipping_json).collect::<Vec<_>>(),
	})))
}

async fn put_clippings(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Path(document): Path<String>,
	Json(input): Json<Items<Clipping>>,
) -> APIResult<Json<Value>> {
	let (user_id, _) = bound_device(ctx.as_ref(), &auth).await?;
	if !valid_document(&document) {
		return Err(APIError::BadRequest("invalid document".to_string()));
	}
	if input.items.len() > stump_crosspoint::sync::MAX_BATCH_ITEMS {
		return Err(APIError::BadRequest(
			"clipping batch is too large".to_string(),
		));
	}
	let update_time = now();
	for clipping in &input.items {
		if !valid_clipping(clipping) {
			return Err(APIError::BadRequest("invalid clipping".to_string()));
		}
		let old = crosspoint_clipping::Entity::find()
			.filter(crosspoint_clipping::Column::UserId.eq(user_id.clone()))
			.filter(crosspoint_clipping::Column::Document.eq(document.clone()))
			.filter(crosspoint_clipping::Column::Id.eq(clipping.id.clone()))
			.one(ctx.conn.as_ref())
			.await
			.map_err(db_error)?;
		let deleted = clipping.deleted == 1;
		let active = crosspoint_clipping::ActiveModel {
			user_id: Set(user_id.clone()),
			document: Set(document.clone()),
			id: Set(clipping.id.clone()),
			spine: Set(clipping.spine.map(i64::from)),
			start_page: Set(clipping.start_page.map(i64::from)),
			end_page: Set(clipping.end_page.map(i64::from)),
			pages: Set(clipping.pages.map(i64::from)),
			start_word: Set(clipping.start_word.map(i64::from)),
			end_word: Set(clipping.end_word.map(i64::from)),
			words: Set(clipping.words.map(i64::from)),
			para: Set(clipping.para.map(i64::from)),
			chapter: Set(if deleted {
				old.as_ref().and_then(|row| row.chapter.clone())
			} else {
				clipping.chapter.clone()
			}),
			text: Set(if deleted {
				old.as_ref().and_then(|row| row.text.clone())
			} else {
				clipping.text.clone()
			}),
			note: Set(if deleted {
				old.as_ref().and_then(|row| row.note.clone())
			} else {
				clipping.note.clone()
			}),
			color: Set(if deleted {
				old.as_ref().and_then(|row| row.color.clone())
			} else {
				clipping.color.clone()
			}),
			created_at: Set(clipping.created_at.map(|value| value as i64)),
			deleted: Set(deleted),
			updated_at: Set(update_time),
		};
		crosspoint_clipping::Entity::insert(active)
			.on_conflict(
				OnConflict::columns([
					crosspoint_clipping::Column::UserId,
					crosspoint_clipping::Column::Document,
					crosspoint_clipping::Column::Id,
				])
				.update_columns([
					crosspoint_clipping::Column::Spine,
					crosspoint_clipping::Column::StartPage,
					crosspoint_clipping::Column::EndPage,
					crosspoint_clipping::Column::Pages,
					crosspoint_clipping::Column::StartWord,
					crosspoint_clipping::Column::EndWord,
					crosspoint_clipping::Column::Words,
					crosspoint_clipping::Column::Para,
					crosspoint_clipping::Column::Chapter,
					crosspoint_clipping::Column::Text,
					crosspoint_clipping::Column::Note,
					crosspoint_clipping::Column::Color,
					crosspoint_clipping::Column::CreatedAt,
					crosspoint_clipping::Column::Deleted,
					crosspoint_clipping::Column::UpdatedAt,
				])
				.to_owned(),
			)
			.exec(ctx.conn.as_ref())
			.await
			.map_err(db_error)?;
	}
	Ok(Json(json!({
		"until": update_time.timestamp(),
		"accepted": input.items.len(),
	})))
}

async fn put_documents(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Json(input): Json<Items<RawDocument>>,
) -> APIResult<Json<Value>> {
	let (user_id, _) = bound_device(ctx.as_ref(), &auth).await?;
	if input.items.len() > stump_crosspoint::sync::MAX_BATCH_ITEMS {
		return Err(APIError::BadRequest(
			"document batch is too large".to_string(),
		));
	}
	let update_time = now();
	for item in &input.items {
		if !valid_document(&item.document)
			|| item.filesize.is_some_and(|value| value < 0)
			|| item
				.title
				.as_deref()
				.into_iter()
				.chain(item.author.as_deref())
				.chain(item.filename.as_deref())
				.any(|value| value.len() > 512)
		{
			return Err(APIError::BadRequest(
				"invalid document metadata".to_string(),
			));
		}
		let old = crosspoint_document::Entity::find()
			.filter(crosspoint_document::Column::UserId.eq(user_id.clone()))
			.filter(crosspoint_document::Column::Document.eq(item.document.clone()))
			.one(ctx.conn.as_ref())
			.await
			.map_err(db_error)?;
		let active = crosspoint_document::ActiveModel {
			user_id: Set(user_id.clone()),
			document: Set(item.document.clone()),
			title: Set(item
				.title
				.clone()
				.or_else(|| old.as_ref().and_then(|row| row.title.clone()))),
			author: Set(item
				.author
				.clone()
				.or_else(|| old.as_ref().and_then(|row| row.author.clone()))),
			filename: Set(item
				.filename
				.clone()
				.or_else(|| old.as_ref().and_then(|row| row.filename.clone()))),
			filesize: Set(item
				.filesize
				.or_else(|| old.as_ref().and_then(|row| row.filesize))),
			updated_at: Set(update_time),
		};
		crosspoint_document::Entity::insert(active)
			.on_conflict(
				OnConflict::columns([
					crosspoint_document::Column::UserId,
					crosspoint_document::Column::Document,
				])
				.update_columns([
					crosspoint_document::Column::Title,
					crosspoint_document::Column::Author,
					crosspoint_document::Column::Filename,
					crosspoint_document::Column::Filesize,
					crosspoint_document::Column::UpdatedAt,
				])
				.to_owned(),
			)
			.exec(ctx.conn.as_ref())
			.await
			.map_err(db_error)?;
	}
	Ok(Json(json!({
		"until": update_time.timestamp(),
		"accepted": input.items.len(),
	})))
}

async fn get_documents(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<Json<Value>> {
	let (user_id, _) = bound_device(ctx.as_ref(), &auth).await?;
	let rows = crosspoint_document::Entity::find()
		.filter(crosspoint_document::Column::UserId.eq(user_id))
		.order_by_asc(crosspoint_document::Column::Document)
		.all(ctx.conn.as_ref())
		.await
		.map_err(db_error)?;
	Ok(Json(json!({
		"items": rows.into_iter().map(|row| json!({
			"document": row.document,
			"title": row.title,
			"author": row.author,
			"filename": row.filename,
			"filesize": row.filesize,
			"updated_at": row.updated_at.timestamp(),
		})).collect::<Vec<_>>(),
	})))
}

fn stats_array(value: Option<Value>, expected: usize) -> APIResult<Value> {
	let value = value.unwrap_or_else(|| Value::Array(vec![Value::from(0); expected]));
	let Some(values) = value.as_array() else {
		return Err(APIError::BadRequest(
			"stats bucket must be an array".to_string(),
		));
	};
	if values.len() != expected
		|| values
			.iter()
			.any(|value| value.as_i64().is_none_or(|number| number < 0))
	{
		return Err(APIError::BadRequest("invalid stats bucket".to_string()));
	}
	Ok(value)
}

fn decode_history(value: Option<String>) -> APIResult<Vec<u8>> {
	use base64::{engine::general_purpose::STANDARD, Engine};
	let value = value.unwrap_or_default();
	let bytes = STANDARD
		.decode(value.as_bytes())
		.map_err(|_| APIError::BadRequest("invalid stats history".to_string()))?;
	if bytes.len() != 92 {
		return Err(APIError::BadRequest(
			"stats history must be 92 bytes".to_string(),
		));
	}
	Ok(bytes)
}

async fn put_global_stats(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Json(input): Json<RawGlobalStats>,
) -> APIResult<Json<Value>> {
	let (user_id, device_id) = bound_device(ctx.as_ref(), &auth).await?;
	check_device_id(input.device_id.as_deref(), &device_id)?;
	let numbers = [
		input.sessions.unwrap_or(0),
		input.seconds.unwrap_or(0),
		input.pages.unwrap_or(0),
		input.completed.unwrap_or(0),
		input.anchor_day.unwrap_or(0),
		input.streak.unwrap_or(0),
	];
	if numbers.iter().any(|value| *value < 0) {
		return Err(APIError::BadRequest("negative stats value".to_string()));
	}
	let active = crosspoint_stats_global::ActiveModel {
		user_id: Set(user_id),
		device_id: Set(device_id),
		device: Set(input.device.unwrap_or_default()),
		version: Set(input.version.unwrap_or(1)),
		sessions: Set(numbers[0]),
		seconds: Set(numbers[1]),
		pages: Set(numbers[2]),
		completed: Set(numbers[3]),
		tod_json: Set(stats_array(input.tod, 4)?),
		dow_json: Set(stats_array(input.dow, 7)?),
		anchor_day: Set(numbers[4]),
		history_blob: Set(decode_history(input.history_b64)?),
		streak: Set(numbers[5]),
		updated_at: Set(now()),
	};
	crosspoint_stats_global::Entity::insert(active)
		.on_conflict(
			OnConflict::columns([
				crosspoint_stats_global::Column::UserId,
				crosspoint_stats_global::Column::DeviceId,
			])
			.update_columns([
				crosspoint_stats_global::Column::Device,
				crosspoint_stats_global::Column::Version,
				crosspoint_stats_global::Column::Sessions,
				crosspoint_stats_global::Column::Seconds,
				crosspoint_stats_global::Column::Pages,
				crosspoint_stats_global::Column::Completed,
				crosspoint_stats_global::Column::TodJson,
				crosspoint_stats_global::Column::DowJson,
				crosspoint_stats_global::Column::AnchorDay,
				crosspoint_stats_global::Column::HistoryBlob,
				crosspoint_stats_global::Column::Streak,
				crosspoint_stats_global::Column::UpdatedAt,
			])
			.to_owned(),
		)
		.exec(ctx.conn.as_ref())
		.await
		.map_err(db_error)?;
	Ok(Json(json!({"accepted": true, "updated_at": now_seconds()})))
}

async fn get_global_stats(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<Json<Value>> {
	let (user_id, _) = bound_device(ctx.as_ref(), &auth).await?;
	let rows = crosspoint_stats_global::Entity::find()
		.filter(crosspoint_stats_global::Column::UserId.eq(user_id))
		.order_by_asc(crosspoint_stats_global::Column::DeviceId)
		.all(ctx.conn.as_ref())
		.await
		.map_err(db_error)?;
	use base64::{engine::general_purpose::STANDARD, Engine};
	Ok(Json(json!({
		"items": rows.into_iter().map(|row| json!({
			"device_id": row.device_id,
			"device": row.device,
			"v": row.version,
			"sessions": row.sessions,
			"seconds": row.seconds,
			"pages": row.pages,
			"completed": row.completed,
			"tod": row.tod_json,
			"dow": row.dow_json,
			"anchor_day": row.anchor_day,
			"history_b64": STANDARD.encode(row.history_blob),
			"streak": row.streak,
			"updated_at": row.updated_at.timestamp(),
		})).collect::<Vec<_>>(),
	})))
}

async fn put_book_stats(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Json(input): Json<Items<RawBookStats>>,
) -> APIResult<Json<Value>> {
	let (user_id, device_id) = bound_device(ctx.as_ref(), &auth).await?;
	if input.items.len() > stump_crosspoint::sync::MAX_STATS_BOOK_BATCH {
		return Err(APIError::BadRequest(
			"book stats batch is too large".to_string(),
		));
	}
	let update_time = now();
	for item in &input.items {
		if !valid_document(&item.document)
			|| [
				item.sessions,
				item.seconds,
				item.pages,
				item.avg_fwd,
				item.pace_n,
				item.eta,
				item.start_date,
				item.finished_date,
			]
			.iter()
			.any(|value| *value < 0)
		{
			return Err(APIError::BadRequest("invalid book stats".to_string()));
		}
		let active = crosspoint_stats_book::ActiveModel {
			user_id: Set(user_id.clone()),
			device_id: Set(device_id.clone()),
			document: Set(item.document.clone()),
			version: Set(item.version),
			sessions: Set(item.sessions),
			seconds: Set(item.seconds),
			pages: Set(item.pages),
			completed: Set(item.completed),
			avg_fwd: Set(item.avg_fwd),
			pace_n: Set(item.pace_n),
			eta: Set(item.eta),
			start_manual: Set(item.start_manual),
			finish_manual: Set(item.finish_manual),
			start_date: Set(item.start_date),
			finished_date: Set(item.finished_date),
			tod_json: Set(stats_array(Some(item.tod.clone()), 4)?),
			dow_json: Set(stats_array(Some(item.dow.clone()), 7)?),
			updated_at: Set(update_time),
		};
		crosspoint_stats_book::Entity::insert(active)
			.on_conflict(
				OnConflict::columns([
					crosspoint_stats_book::Column::UserId,
					crosspoint_stats_book::Column::DeviceId,
					crosspoint_stats_book::Column::Document,
				])
				.update_columns([
					crosspoint_stats_book::Column::Version,
					crosspoint_stats_book::Column::Sessions,
					crosspoint_stats_book::Column::Seconds,
					crosspoint_stats_book::Column::Pages,
					crosspoint_stats_book::Column::Completed,
					crosspoint_stats_book::Column::AvgFwd,
					crosspoint_stats_book::Column::PaceN,
					crosspoint_stats_book::Column::Eta,
					crosspoint_stats_book::Column::StartManual,
					crosspoint_stats_book::Column::FinishManual,
					crosspoint_stats_book::Column::StartDate,
					crosspoint_stats_book::Column::FinishedDate,
					crosspoint_stats_book::Column::TodJson,
					crosspoint_stats_book::Column::DowJson,
					crosspoint_stats_book::Column::UpdatedAt,
				])
				.to_owned(),
			)
			.exec(ctx.conn.as_ref())
			.await
			.map_err(db_error)?;
	}
	Ok(Json(
		json!({"until": update_time.timestamp(), "accepted": input.items.len()}),
	))
}

async fn get_book_stats(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Path(document): Path<String>,
) -> APIResult<Json<Value>> {
	let (user_id, _) = bound_device(ctx.as_ref(), &auth).await?;
	if !valid_document(&document) {
		return Err(APIError::BadRequest("invalid document".to_string()));
	}
	let rows = crosspoint_stats_book::Entity::find()
		.filter(crosspoint_stats_book::Column::UserId.eq(user_id))
		.filter(crosspoint_stats_book::Column::Document.eq(document.clone()))
		.order_by_asc(crosspoint_stats_book::Column::DeviceId)
		.all(ctx.conn.as_ref())
		.await
		.map_err(db_error)?;
	Ok(Json(json!({
		"document": document,
		"items": rows.into_iter().map(|row| json!({
			"device_id": row.device_id,
			"v": row.version,
			"sessions": row.sessions,
			"seconds": row.seconds,
			"pages": row.pages,
			"completed": row.completed,
			"avg_fwd": row.avg_fwd,
			"pace_n": row.pace_n,
			"eta": row.eta,
			"start_manual": row.start_manual,
			"finish_manual": row.finish_manual,
			"start_date": row.start_date,
			"finished_date": row.finished_date,
			"tod": row.tod_json,
			"dow": row.dow_json,
			"updated_at": row.updated_at.timestamp(),
		})).collect::<Vec<_>>(),
	})))
}

fn add_bucket(total: &mut Vec<i64>, value: &Value) {
	let Some(values) = value.as_array() else {
		return;
	};
	if total.len() < values.len() {
		total.resize(values.len(), 0);
	}
	for (index, value) in values.iter().enumerate() {
		if let Some(value) = value.as_i64() {
			total[index] = total[index].saturating_add(value);
		}
	}
}

async fn get_stats_summary(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<Json<Value>> {
	let (user_id, _) = bound_device(ctx.as_ref(), &auth).await?;
	let rows = crosspoint_stats_global::Entity::find()
		.filter(crosspoint_stats_global::Column::UserId.eq(user_id))
		.all(ctx.conn.as_ref())
		.await
		.map_err(db_error)?;
	let mut sessions = 0_i64;
	let mut seconds = 0_i64;
	let mut pages = 0_i64;
	let mut completed = 0_i64;
	let mut streak = 0_i64;
	let mut tod = Vec::new();
	let mut dow = Vec::new();
	for row in rows {
		sessions = sessions.saturating_add(row.sessions);
		seconds = seconds.saturating_add(row.seconds);
		pages = pages.saturating_add(row.pages);
		completed = completed.saturating_add(row.completed);
		streak = streak.max(row.streak);
		add_bucket(&mut tod, &row.tod_json);
		add_bucket(&mut dow, &row.dow_json);
	}
	Ok(Json(json!({
		"sessions": sessions,
		"seconds": seconds,
		"pages": pages,
		"completed": completed,
		"tod": tod,
		"dow": dow,
		"streak": streak,
	})))
}
