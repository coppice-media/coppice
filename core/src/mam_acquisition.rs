//! Explicit, manager-triggered acquisition through the authenticated MAM Bridge.
//!
//! Coppice owns request intent and staged-ingest review; this module only
//! forwards bounded bridge operations and copies completed handoffs into ingest.

use std::{
	collections::HashSet,
	path::{Component, Path, PathBuf},
	sync::{atomic::Ordering, Mutex},
	time::Duration,
};

use chrono::{DateTime, FixedOffset, Utc};
use html5ever::{parse_document, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use models::entity::{
	book_request, book_work_metadata, library, mam_acquisition_grab, mam_release_search,
	media_metadata, remote_source, remote_source_item,
};
use reqwest::{header, Method, Response};
use sea_orm::{
	sea_query::OnConflict, ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition,
	DatabaseConnection, DbErr, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
	QuerySelect,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use stump_worker::{
	source_hub::SourceHub,
	source_protocol::{SourceReadMode, SourceReadRequest, SourceTransport},
};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::{
	config::StumpConfig,
	context::Ctx,
	error::{CoreError, CoreResult},
	job::JobServices,
};

const SEARCH_PAUSED: &str = "Search is paused, check the bridge";
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const SOURCE_READ_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const SEARCH_FIELDS: &[&str] = &["title", "author", "narrator", "series"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseCandidate {
	pub torrent_id: i32,
	pub title: String,
	#[serde(default)]
	pub description: Option<String>,
	pub authors: Vec<String>,
	pub narrators: Vec<String>,
	pub series: Vec<String>,
	pub kind: String,
	pub category_name: Option<String>,
	pub language_code: Option<String>,
	pub file_type: Option<String>,
	pub size: Option<String>,
	pub num_files: Option<i32>,
	pub added: Option<String>,
	pub seeders: Option<i32>,
	pub leechers: Option<i32>,
	pub times_completed: Option<i32>,
	pub freeleech: bool,
	pub vip: bool,
	pub snatched: bool,
	pub isbn: Option<String>,
	pub score: f64,
	pub match_reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseSearch {
	pub candidates: Vec<ReleaseCandidate>,
	pub found: Option<i32>,
	pub probe: bool,
	pub searched_at: DateTime<FixedOffset>,
}

#[derive(Clone, Debug)]
pub struct AcquisitionStatus {
	pub configured: bool,
	pub reachable: bool,
	pub ready: bool,
	pub mode: Option<String>,
	pub message: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AcquisitionGrab {
	pub id: String,
	pub request_id: String,
	pub torrent_id: i32,
	pub title: String,
	pub phase: String,
	pub progress: f32,
	pub error: Option<String>,
	pub ingest_item_id: Option<String>,
	pub created_at: DateTime<FixedOffset>,
	pub updated_at: DateTime<FixedOffset>,
}

#[derive(Debug, Serialize)]
struct BridgeSearchRequest<'a> {
	text: &'a str,
	fields: &'static [&'static str],
	#[serde(skip_serializing_if = "Option::is_none")]
	main_categories: Option<Vec<i32>>,
	sort: &'static str,
	limit: i32,
	offset: i32,
}

#[derive(Debug, Deserialize)]
struct BridgeSearchResponse {
	#[serde(default)]
	candidates: Vec<Value>,
	#[serde(default)]
	found: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct BridgeGrabRecord {
	grab_id: String,
	torrent_id: i64,
	#[serde(default)]
	phase: String,
	#[serde(default)]
	progress_millis: i32,
	#[serde(default)]
	error: Option<String>,
	#[serde(default)]
	source_path: Option<String>,
	#[serde(default)]
	created_at_unix: u64,
}

impl BridgeGrabRecord {
	/// The bridge stops touching a grab in these phases; `completed` still
	/// waits for its handoff, so it stays active for adoption.
	fn is_terminal(&self) -> bool {
		matches!(self.phase.as_str(), "error" | "aborted" | "missing")
	}
}

/// `GET api/state`: every grab the bridge still remembers.
#[derive(Debug, Deserialize)]
struct BridgeStateResponse {
	#[serde(default)]
	grabs: Vec<BridgeGrabRecord>,
}

struct BridgeClient {
	base: reqwest::Url,
	token_file: PathBuf,
	client: reqwest::Client,
}

impl BridgeClient {
	fn configured(config: &StumpConfig) -> bool {
		config.mam_acquisition.mam_bridge_url.is_some()
			&& config.mam_acquisition.mam_bridge_token_file.is_some()
	}

	fn from_config(config: &StumpConfig) -> Result<Self, String> {
		if !config.mam_acquisition.enable_mam_acquisition {
			return Err("MAM acquisition is disabled".to_string());
		}
		let base = config
			.mam_acquisition
			.mam_bridge_url
			.as_deref()
			.ok_or_else(|| "MAM Bridge is not configured".to_string())?;
		let token_file = config
			.mam_acquisition
			.mam_bridge_token_file
			.as_deref()
			.ok_or_else(|| "MAM Bridge is not configured".to_string())?;
		let mut base = reqwest::Url::parse(base)
			.map_err(|_| "MAM Bridge URL is invalid".to_string())?;
		if base.query().is_some() || base.fragment().is_some() {
			return Err("MAM Bridge URL must not contain a query or fragment".to_string());
		}
		let local_http = base.scheme() == "http"
			&& base.host_str().is_some_and(|host| {
				matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
			});
		if base.scheme() != "https" && !local_http {
			return Err("MAM Bridge URL must use HTTPS".to_string());
		}
		if !base.path().ends_with('/') {
			let path = format!("{}/", base.path());
			base.set_path(&path);
		}
		let client = reqwest::Client::builder()
			.redirect(reqwest::redirect::Policy::none())
			.connect_timeout(Duration::from_secs(10))
			.timeout(Duration::from_secs(20))
			.build()
			.map_err(|_| "MAM Bridge HTTP client could not be initialized".to_string())?;
		Ok(Self {
			base,
			token_file: PathBuf::from(token_file),
			client,
		})
	}

	async fn token_header(&self) -> Result<header::HeaderValue, String> {
		let bytes = tokio::fs::read(&self.token_file)
			.await
			.map_err(|_| "MAM Bridge token file is unreadable".to_string())?;
		if bytes.is_empty() || bytes.len() > 4096 {
			return Err("MAM Bridge token file is invalid".to_string());
		}
		let token = std::str::from_utf8(&bytes)
			.map_err(|_| "MAM Bridge token file is invalid".to_string())?
			.trim();
		if token.is_empty() {
			return Err("MAM Bridge token file is invalid".to_string());
		}
		header::HeaderValue::from_str(&format!("Bearer {token}"))
			.map_err(|_| "MAM Bridge token file is invalid".to_string())
	}

	fn endpoint(&self, path: &str) -> Result<reqwest::Url, String> {
		self.base
			.join(path)
			.map_err(|_| "MAM Bridge endpoint is invalid".to_string())
	}

	async fn request(
		&self,
		method: Method,
		url: reqwest::Url,
		body: Option<Vec<u8>>,
		authenticated: bool,
	) -> Result<Response, String> {
		let mut request = self.client.request(method, url);
		if authenticated {
			request = request.header(header::AUTHORIZATION, self.token_header().await?);
		}
		if let Some(body) = body {
			request = request
				.header(header::CONTENT_TYPE, "application/json")
				.body(body);
		}
		request
			.send()
			.await
			.map_err(|_| "MAM Bridge is unavailable".to_string())
	}

	async fn json_response(
		&self,
		response: Response,
		search: bool,
	) -> Result<Value, String> {
		let status = response.status();
		if !status.is_success() {
			if search
				&& (status == reqwest::StatusCode::FORBIDDEN
					|| status == reqwest::StatusCode::CONFLICT)
			{
				return Err(SEARCH_PAUSED.to_string());
			}
			if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
				let retry = response
					.headers()
					.get(header::RETRY_AFTER)
					.and_then(|value| value.to_str().ok())
					.and_then(|value| value.parse::<u64>().ok())
					.unwrap_or(10);
				return Err(format!("Try again in {retry} s"));
			}
			return Err(match status {
				reqwest::StatusCode::UNAUTHORIZED => {
					"MAM Bridge authentication failed".to_string()
				},
				reqwest::StatusCode::NOT_FOUND => {
					"MAM Bridge grab was not found".to_string()
				},
				_ => format!("MAM Bridge returned HTTP {}", status.as_u16()),
			});
		}
		let content_type = response
			.headers()
			.get(header::CONTENT_TYPE)
			.and_then(|value| value.to_str().ok())
			.unwrap_or_default();
		if !content_type
			.to_ascii_lowercase()
			.starts_with("application/json")
		{
			return Err("MAM Bridge returned an unexpected content type".to_string());
		}
		let mut response = response;
		if response
			.content_length()
			.is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
		{
			return Err("MAM Bridge response exceeded the size limit".to_string());
		}
		let mut bytes = Vec::new();
		while let Some(chunk) = response
			.chunk()
			.await
			.map_err(|_| "MAM Bridge response could not be read".to_string())?
		{
			if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
				return Err("MAM Bridge response exceeded the size limit".to_string());
			}
			bytes.extend_from_slice(&chunk);
		}
		serde_json::from_slice(&bytes)
			.map_err(|_| "MAM Bridge returned invalid JSON".to_string())
	}

	async fn health(&self) -> Result<(bool, Option<String>, bool), String> {
		let health_url = self.endpoint("health")?;
		let health = self.request(Method::GET, health_url, None, false).await?;
		let health_body = self.json_response(health, false).await?;
		let reachable = health_body.get("status").and_then(Value::as_str) == Some("ok");
		if !reachable {
			return Ok((false, None, false));
		}
		let operations_url = self.endpoint("api/operations/health")?;
		let operations = self
			.request(Method::GET, operations_url, None, true)
			.await?;
		let operations = self.json_response(operations, false).await?;
		let mode = operations
			.get("mode")
			.and_then(Value::as_str)
			.map(str::to_owned);
		let ready = operations
			.get("ready")
			.and_then(Value::as_bool)
			.unwrap_or(false);
		Ok((true, mode, ready))
	}

	async fn search(
		&self,
		text: &str,
		main_categories: Option<Vec<i32>>,
		limit: i32,
	) -> Result<BridgeSearchResponse, String> {
		let payload = BridgeSearchRequest {
			text,
			fields: SEARCH_FIELDS,
			main_categories,
			sort: "default",
			limit,
			offset: 0,
		};
		let body = serde_json::to_vec(&payload)
			.map_err(|_| "MAM search request could not be encoded".to_string())?;
		let url = self.endpoint("api/search")?;
		let response = self.request(Method::POST, url, Some(body), true).await?;
		serde_json::from_value(self.json_response(response, true).await?)
			.map_err(|_| "MAM Bridge returned an invalid search result".to_string())
	}

	async fn grab(&self, torrent_id: i32) -> Result<BridgeGrabRecord, String> {
		let body = serde_json::to_vec(&json!({
			"torrent_id": torrent_id,
			"confirmation": format!("download torrent {torrent_id}"),
		}))
		.map_err(|_| "MAM grab request could not be encoded".to_string())?;
		let url = self.endpoint("api/grabs")?;
		let response = self.request(Method::POST, url, Some(body), true).await?;
		serde_json::from_value(self.json_response(response, false).await?)
			.map_err(|_| "MAM Bridge returned an invalid grab".to_string())
	}

	async fn refresh(&self, grab_id: &str) -> Result<BridgeGrabRecord, String> {
		let mut url = self.endpoint("api/grabs/")?;
		// The base path ends in a slash; without `pop_if_empty` the pushed
		// segment follows an empty one and the bridge sees `api/grabs//<id>`.
		url.path_segments_mut()
			.map_err(|_| "MAM Bridge endpoint is invalid".to_string())?
			.pop_if_empty()
			.push(grab_id)
			.push("refresh");
		let response = self
			.request(Method::POST, url, Some(b"{}".to_vec()), true)
			.await?;
		serde_json::from_value(self.json_response(response, false).await?)
			.map_err(|_| "MAM Bridge returned an invalid grab status".to_string())
	}

	/// Every grab the bridge still remembers, used to recover a local grab
	/// whose bridge id was never saved.
	async fn list_grabs(&self) -> Result<Vec<BridgeGrabRecord>, String> {
		let url = self.endpoint("api/state")?;
		let response = self.request(Method::GET, url, None, true).await?;
		serde_json::from_value::<BridgeStateResponse>(
			self.json_response(response, false).await?,
		)
		.map(|state| state.grabs)
		.map_err(|_| "MAM Bridge returned an invalid grab listing".to_string())
	}
}

pub async fn acquisition_status(config: &StumpConfig) -> AcquisitionStatus {
	let configured = BridgeClient::configured(config);
	if !config.mam_acquisition.enable_mam_acquisition {
		return AcquisitionStatus {
			configured,
			reachable: false,
			ready: false,
			mode: None,
			message: Some("MAM acquisition is disabled".to_string()),
		};
	}
	let Ok(client) = BridgeClient::from_config(config) else {
		return AcquisitionStatus {
			configured: false,
			reachable: false,
			ready: false,
			mode: None,
			message: Some("MAM Bridge is not configured".to_string()),
		};
	};
	match client.health().await {
		Ok((reachable, mode, ready)) => AcquisitionStatus {
			configured,
			reachable,
			ready: reachable && ready,
			mode,
			message: (!reachable || !ready)
				.then(|| "MAM Bridge is not ready".to_string()),
		},
		Err(_) => AcquisitionStatus {
			configured,
			reachable: false,
			ready: false,
			mode: None,
			message: Some("MAM Bridge is unavailable".to_string()),
		},
	}
}

pub async fn search_releases(
	ctx: &Ctx,
	request: &book_request::Model,
	text: Option<&str>,
	format: Option<&str>,
	limit: i32,
) -> CoreResult<ReleaseSearch> {
	let text = text.unwrap_or(&request.title).trim();
	if text.is_empty() || text.len() > 256 || text.chars().any(char::is_control) {
		return Err(CoreError::BadRequest(
			"search text must be 1–256 bytes and contain no control characters"
				.to_string(),
		));
	}
	let config = ctx.config.as_ref();
	let client = BridgeClient::from_config(config).map_err(CoreError::InternalError)?;
	let requested_format = format.unwrap_or(&request.format);
	let main_categories = match requested_format.trim().to_ascii_uppercase().as_str() {
		"AUDIOBOOK" => Some(vec![13]),
		"EBOOK" => Some(vec![14]),
		_ => Some(vec![13, 14]),
	};
	let probe = !ctx.mam_search_probe_complete.load(Ordering::Acquire);
	let requested_limit = if probe { 5 } else { limit.clamp(5, 100) };
	let bridge_result = client.search(text, main_categories, requested_limit).await;
	let response = match bridge_result {
		Ok(response) => response,
		Err(error) => {
			if error == SEARCH_PAUSED {
				ctx.mam_search_probe_complete
					.store(false, Ordering::Release);
			}
			return Err(CoreError::InternalError(error));
		},
	};
	let (series_name, requested_language, request_is_comic) =
		requested_search_metadata(ctx, request).await?;
	let mut candidates = response
		.candidates
		.into_iter()
		.filter_map(|mut value| {
			decode_bridge_text_fields(&mut value);
			parse_candidate(&value)
		})
		.map(|candidate| {
			score_candidate(
				candidate,
				request,
				requested_format,
				series_name.as_deref(),
				requested_language.as_deref(),
				request_is_comic,
			)
		})
		.collect::<Vec<_>>();
	candidates.sort_by(|left, right| {
		right
			.score
			.total_cmp(&left.score)
			.then_with(|| right.seeders.cmp(&left.seeders))
			.then_with(|| left.torrent_id.cmp(&right.torrent_id))
	});
	ctx.mam_search_probe_complete.store(true, Ordering::Release);
	let result = ReleaseSearch {
		candidates,
		found: response.found.as_ref().and_then(parse_i32),
		probe,
		searched_at: Utc::now().fixed_offset(),
	};
	let candidates_json = serde_json::to_string(&result.candidates)?;
	let active = mam_release_search::ActiveModel {
		request_id: Set(request.id.clone()),
		candidates_json: Set(candidates_json),
		found: Set(result.found),
		probe: Set(result.probe),
		searched_at: Set(result.searched_at),
		updated_at: Set(Utc::now().fixed_offset()),
	};
	mam_release_search::Entity::insert(active)
		.on_conflict(
			OnConflict::column(mam_release_search::Column::RequestId)
				.update_columns([
					mam_release_search::Column::CandidatesJson,
					mam_release_search::Column::Found,
					mam_release_search::Column::Probe,
					mam_release_search::Column::SearchedAt,
					mam_release_search::Column::UpdatedAt,
				])
				.to_owned(),
		)
		.exec_without_returning(ctx.conn.as_ref())
		.await?;
	Ok(result)
}

async fn requested_search_metadata(
	ctx: &Ctx,
	request: &book_request::Model,
) -> CoreResult<(Option<String>, Option<String>, bool)> {
	let linked_metadata = if let Some(media_id) = request.internal_media_id.as_deref() {
		media_metadata::Entity::find()
			.filter(media_metadata::Column::MediaId.eq(media_id))
			.one(ctx.conn.as_ref())
			.await?
	} else {
		None
	};
	let mut language = linked_metadata
		.as_ref()
		.and_then(|metadata| metadata.language.clone());
	let mut is_comic = linked_metadata
		.as_ref()
		.is_some_and(media_metadata_indicates_comic);
	let Some(work_id) = request.internal_work_id.as_deref() else {
		return Ok((None, language, is_comic));
	};
	let Some(work) = book_work_metadata::Entity::find_by_id(work_id)
		.one(ctx.conn.as_ref())
		.await?
	else {
		return Ok((None, language, is_comic));
	};
	let Some(Value::Object(metadata)) = work.metadata else {
		return Ok((None, language, is_comic));
	};
	let metadata = Value::Object(metadata);
	let series = metadata_text(
		&metadata,
		&["series", "seriesName", "series_name"],
		&["name"],
	);
	if language.is_none() {
		language = metadata_text(
			&metadata,
			&["language", "languageCode", "language_code"],
			&["code", "languageCode", "tag", "name"],
		);
	}
	is_comic |= work_metadata_indicates_comic(&metadata);
	Ok((series, language, is_comic))
}

fn metadata_text(
	metadata: &Value,
	keys: &[&str],
	nested_keys: &[&str],
) -> Option<String> {
	keys.iter().find_map(|key| {
		let value = metadata.get(*key)?;
		let value = value.as_str().or_else(|| {
			nested_keys
				.iter()
				.find_map(|nested| value.get(*nested).and_then(Value::as_str))
		})?;
		(!value.trim().is_empty()).then(|| value.trim().to_owned())
	})
}

fn media_metadata_indicates_comic(metadata: &media_metadata::Model) -> bool {
	[metadata.format.as_deref(), metadata.genres.as_deref()]
		.into_iter()
		.flatten()
		.any(text_indicates_comic)
}

fn work_metadata_indicates_comic(metadata: &Value) -> bool {
	[
		"format",
		"genre",
		"genres",
		"category",
		"categories",
		"subject",
		"subjects",
		"tags",
		"mediaType",
		"media_type",
	]
	.into_iter()
	.filter_map(|key| metadata.get(key))
	.any(value_indicates_comic)
}

fn value_indicates_comic(value: &Value) -> bool {
	match value {
		Value::String(text) => text_indicates_comic(text),
		Value::Array(values) => values.iter().any(value_indicates_comic),
		Value::Object(values) => values.values().any(value_indicates_comic),
		_ => false,
	}
}

fn text_indicates_comic(value: &str) -> bool {
	if let Ok(value) = serde_json::from_str::<Value>(value) {
		value_indicates_comic(&value)
	} else {
		let normalized = normalize(value);
		normalized.contains("comic")
			|| normalized.contains("manga")
			|| normalized.contains("graphicnovel")
	}
}

pub async fn cached_release_search(
	ctx: &Ctx,
	request_id: &str,
) -> CoreResult<Option<ReleaseSearch>> {
	let Some(snapshot) = mam_release_search::Entity::find_by_id(request_id)
		.one(ctx.conn.as_ref())
		.await?
	else {
		return Ok(None);
	};
	let candidates = serde_json::from_str(&snapshot.candidates_json)?;
	Ok(Some(ReleaseSearch {
		candidates,
		found: snapshot.found,
		probe: snapshot.probe,
		searched_at: snapshot.searched_at,
	}))
}

/// Phases in which a local grab still tracks (or awaits) bridge work: the
/// row is the grab's idempotency key, so a second confirmation of the same
/// release returns it instead of starting the bridge again.
const ACTIVE_GRAB_PHASES: [&str; 4] = ["starting", "queued", "downloading", "completed"];

pub async fn create_grab(
	ctx: &Ctx,
	request: &book_request::Model,
	torrent_id: i32,
) -> CoreResult<AcquisitionGrab> {
	let cached = cached_release_search(ctx, &request.id)
		.await?
		.ok_or_else(|| {
			CoreError::BadRequest("search this request before grabbing".to_string())
		})?;
	let candidate = cached
		.candidates
		.iter()
		.find(|candidate| candidate.torrent_id == torrent_id)
		.ok_or_else(|| {
			CoreError::BadRequest(
				"release is not in the cached search results".to_string(),
			)
		})?;
	let client = BridgeClient::from_config(ctx.config.as_ref())
		.map_err(CoreError::InternalError)?;
	// The bridge refuses a second grab of a torrent it is already working on,
	// and a local row in an active phase is what proves one was started —
	// including a `starting` row whose bridge id the refresh job still has
	// to recover. Never send the POST twice for it.
	if let Some(active) = mam_acquisition_grab::Entity::find()
		.filter(mam_acquisition_grab::Column::TorrentId.eq(i64::from(torrent_id)))
		.filter(mam_acquisition_grab::Column::Phase.is_in(ACTIVE_GRAB_PHASES))
		.order_by_desc(mam_acquisition_grab::Column::CreatedAt)
		.one(ctx.conn.as_ref())
		.await?
	{
		if active.request_id == request.id {
			return Ok(active.into());
		}
		return Err(CoreError::BadRequest(
			"release is already being acquired for another request".to_string(),
		));
	}
	let id = Uuid::new_v4().to_string();
	let now = Utc::now().fixed_offset();
	let model = mam_acquisition_grab::ActiveModel {
		id: Set(id.clone()),
		request_id: Set(request.id.clone()),
		torrent_id: Set(i64::from(torrent_id)),
		bridge_grab_id: Set(None),
		title: Set(candidate.title.clone()),
		phase: Set("starting".to_string()),
		progress: Set(0.0),
		error: Set(None),
		ingest_item_id: Set(None),
		handoff_attempts: Set(0),
		created_at: Set(now),
		updated_at: Set(now),
	};
	let model = model.insert(ctx.conn.as_ref()).await?;
	let bridge = match client.grab(torrent_id).await {
		Ok(grab) if grab.torrent_id == i64::from(torrent_id) => grab,
		Ok(_) => {
			mark_grab_error(
				ctx,
				model,
				"MAM Bridge returned a different torrent id".to_string(),
			)
			.await?;
			return Err(CoreError::InternalError(
				"MAM Bridge returned a different torrent id".to_string(),
			));
		},
		Err(error) => {
			mark_grab_error(ctx, model.clone(), error.clone()).await?;
			return Err(CoreError::InternalError(error));
		},
	};
	// If this save fails (or the process dies first) the row stays `starting`
	// and `refresh_grabs` adopts the bridge grab from the bridge listing.
	let mut active = model.into_active_model();
	active.bridge_grab_id = Set(Some(bridge.grab_id));
	active.phase = Set(bridge.phase);
	active.progress = Set(f64::from(bridge.progress_millis.clamp(0, 1000)) / 1000.0);
	active.error = Set(bridge.error);
	let model = active.update(ctx.conn.as_ref()).await?;
	Ok(model.into())
}

async fn mark_grab_error(
	ctx: &Ctx,
	model: mam_acquisition_grab::Model,
	message: String,
) -> CoreResult<()> {
	let mut active = model.into_active_model();
	active.phase = Set("error".to_string());
	active.error = Set(Some(message));
	active.update(ctx.conn.as_ref()).await?;
	Ok(())
}

pub async fn list_grabs(ctx: &Ctx, request_id: &str) -> CoreResult<Vec<AcquisitionGrab>> {
	Ok(mam_acquisition_grab::Entity::find()
		.filter(mam_acquisition_grab::Column::RequestId.eq(request_id))
		.order_by_desc(mam_acquisition_grab::Column::CreatedAt)
		.all(ctx.conn.as_ref())
		.await?
		.into_iter()
		.map(Into::into)
		.collect())
}

impl From<mam_acquisition_grab::Model> for AcquisitionGrab {
	fn from(model: mam_acquisition_grab::Model) -> Self {
		Self {
			id: model.id,
			request_id: model.request_id,
			torrent_id: model
				.torrent_id
				.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
			title: model.title,
			phase: model.phase,
			progress: model.progress.clamp(0.0, 1.0) as f32,
			error: model.error,
			ingest_item_id: model.ingest_item_id,
			created_at: model.created_at,
			updated_at: model.updated_at,
		}
	}
}

fn parse_i32(value: &Value) -> Option<i32> {
	value
		.as_i64()
		.and_then(|value| i32::try_from(value).ok())
		.or_else(|| value.as_str()?.parse::<i32>().ok())
}
fn identifier_text(value: &Value) -> Option<String> {
	value
		.as_str()
		.map(str::to_owned)
		.or_else(|| value.as_i64().map(|number| number.to_string()))
		.or_else(|| value.as_u64().map(|number| number.to_string()))
}

fn decode_html_entities(value: &str) -> String {
	if !value.contains('&') {
		return value.to_owned();
	}
	let safe_text = value.replace('<', "&lt;");
	let source = format!(
		"<!doctype html><html><body><textarea>&#x200B;{safe_text}</textarea></body></html>"
	);
	let dom = parse_document(RcDom::default(), Default::default()).one(source);
	fn append_text(handle: &Handle, output: &mut String) {
		if let NodeData::Text { contents } = &handle.data {
			output.push_str(contents.borrow().as_ref());
		}
		for child in handle.children.borrow().iter() {
			append_text(child, output);
		}
	}
	let mut decoded = String::with_capacity(value.len());
	append_text(&dom.document, &mut decoded);
	decoded
		.strip_prefix('\u{200b}')
		.unwrap_or(&decoded)
		.to_owned()
}

fn decode_bridge_text_fields(value: &mut Value) {
	let Some(object) = value.as_object_mut() else {
		return;
	};
	for key in [
		"title",
		"kind",
		"category_name",
		"category",
		"description",
		"language_code",
		"file_type",
		"size",
		"isbn",
	] {
		if let Some(Value::String(text)) = object.get_mut(key) {
			let decoded = decode_html_entities(text);
			*text = decoded;
		}
	}
	for key in ["authors", "narrators", "series"] {
		if let Some(Value::Array(values)) = object.get_mut(key) {
			for value in values {
				match value {
					Value::String(text) => {
						let decoded = decode_html_entities(text);
						*text = decoded;
					},
					Value::Object(value) => {
						if let Some(Value::String(text)) = value.get_mut("name") {
							let decoded = decode_html_entities(text);
							*text = decoded;
						}
					},
					_ => {},
				}
			}
		}
	}
}

fn parse_candidate(value: &Value) -> Option<ReleaseCandidate> {
	let torrent_id = value.get("torrent_id").and_then(parse_i32)?;
	let title = value.get("title")?.as_str()?.trim();
	if title.is_empty() || title.len() > 1024 {
		return None;
	}
	let series_values = value.get("series").and_then(Value::as_array);
	let series = series_values
		.into_iter()
		.flatten()
		.filter_map(|item| item.get("name").and_then(Value::as_str))
		.map(str::to_owned)
		.collect::<Vec<_>>();
	let names = |key: &str| {
		value
			.get(key)
			.and_then(Value::as_array)
			.into_iter()
			.flatten()
			.filter_map(|item| {
				item.as_str()
					.or_else(|| item.get("name").and_then(Value::as_str))
			})
			.map(str::to_owned)
			.collect::<Vec<_>>()
	};
	let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
	let integer = |key: &str| value.get(key).and_then(parse_i32);
	let boolean = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
	Some(ReleaseCandidate {
		torrent_id,
		title: title.to_string(),
		description: text("description"),
		authors: names("authors"),
		narrators: names("narrators"),
		series,
		kind: text("kind").unwrap_or_else(|| match integer("main_category") {
			Some(13) => "audiobook".to_string(),
			Some(14) => "ebook".to_string(),
			_ => "unknown".to_string(),
		}),
		category_name: text("category_name"),
		language_code: text("language_code"),
		file_type: text("file_type"),
		size: text("size"),
		num_files: integer("num_files"),
		added: text("added"),
		seeders: integer("seeders"),
		leechers: integer("leechers"),
		times_completed: integer("times_completed"),
		freeleech: boolean("freeleech"),
		vip: boolean("vip"),
		snatched: boolean("snatched"),
		isbn: text("isbn").or_else(|| value.get("isbn").and_then(identifier_text)),
		score: 0.0,
		match_reasons: Vec::new(),
	})
}

fn normalize(value: &str) -> String {
	value
		.chars()
		.filter(|character| character.is_alphanumeric())
		.flat_map(char::to_lowercase)
		.collect()
}

/// Whether two person names are the same once punctuation, case and the
/// "Last, First" inversion trackers favour are ignored. "Porter, Ray" and
/// "Ray Porter" match; "Ray Porter Jr." does not match "Ray Porter".
fn person_names_match(wanted: &str, found: &str) -> bool {
	let wanted_normalized = normalize(wanted);
	if wanted_normalized.is_empty() {
		return false;
	}
	if normalize(found) == wanted_normalized {
		return true;
	}
	let inverted = |name: &str| -> Option<String> {
		let (last, first) = name.split_once(',')?;
		Some(normalize(&format!("{first} {last}")))
	};
	inverted(found).is_some_and(|name| name == wanted_normalized)
		|| inverted(wanted).is_some_and(|name| name == normalize(found))
}

fn score_candidate(
	mut candidate: ReleaseCandidate,
	request: &book_request::Model,
	format: &str,
	series_name: Option<&str>,
	requested_language: Option<&str>,
	requested_is_comic: bool,
) -> ReleaseCandidate {
	let title = normalize(&candidate.title);
	let wanted_title = normalize(&request.title);
	if !wanted_title.is_empty() && title == wanted_title {
		candidate.score += 50.0;
		candidate.match_reasons.push("exact_title".to_string());
	} else if !wanted_title.is_empty()
		&& (title.contains(&wanted_title) || wanted_title.contains(&title))
	{
		candidate.score += 20.0;
		candidate.match_reasons.push("title_partial".to_string());
	}
	// The bridge exposes one identifier column; an ISBN-backed request compares
	// its ISBN against it, an Audible-backed request its ASIN (the remote ID).
	let wanted_isbn = request
		.isbn
		.as_deref()
		.map(normalize)
		.filter(|value| !value.is_empty())
		.map(|isbn| (isbn, "isbn"));
	let wanted_asin = request
		.remote_id
		.as_deref()
		.filter(|_| request.source_provider.as_deref() == Some("audible"))
		.map(normalize)
		.filter(|value| !value.is_empty())
		.map(|asin| (asin, "asin"));
	let found_identifier = candidate.isbn.as_deref().map(normalize);
	for (wanted, reason) in wanted_isbn.into_iter().chain(wanted_asin) {
		if found_identifier.as_deref() == Some(wanted.as_str()) {
			candidate.score += 100.0;
			candidate.match_reasons.push(reason.to_string());
		}
	}
	if let Some(wanted_authors) = request.authors.as_deref() {
		let wanted = wanted_authors
			.split([',', ';', '&'])
			.map(normalize)
			.filter(|author| !author.is_empty())
			.collect::<Vec<_>>();
		if wanted.iter().any(|wanted| {
			candidate
				.authors
				.iter()
				.any(|author| normalize(author) == *wanted)
		}) {
			candidate.score += 30.0;
			candidate.match_reasons.push("author".to_string());
		}
	}
	if let Some(wanted_narrator) = request
		.preferred_narrator
		.as_deref()
		.filter(|_| !format.trim().eq_ignore_ascii_case("EBOOK"))
	{
		if candidate
			.narrators
			.iter()
			.any(|narrator| person_names_match(wanted_narrator, narrator))
		{
			candidate.score += 30.0;
			candidate.match_reasons.push("narrator".to_string());
		}
	}
	if let Some(wanted_series) =
		series_name.map(normalize).filter(|value| !value.is_empty())
	{
		if candidate
			.series
			.iter()
			.any(|series| normalize(series) == wanted_series)
		{
			candidate.score += 15.0;
			candidate.match_reasons.push("series".to_string());
		}
	}
	match format.trim().to_ascii_uppercase().as_str() {
		"AUDIOBOOK" if candidate.kind.to_ascii_lowercase().contains("audio") => {
			candidate.score += 20.0;
			candidate.match_reasons.push("format".to_string());
		},
		"AUDIOBOOK" => candidate.score -= 25.0,
		"EBOOK" if candidate.kind.to_ascii_lowercase().contains("ebook") => {
			candidate.score += 20.0;
			candidate.match_reasons.push("format".to_string());
		},
		"EBOOK" => candidate.score -= 25.0,
		_ => {},
	}
	if matches!(format.trim().to_ascii_uppercase().as_str(), "EBOOK" | "ANY")
		&& !requested_is_comic
		&& (candidate
			.category_name
			.as_deref()
			.is_some_and(text_indicates_comic)
			|| candidate.file_type.as_deref().is_some_and(|file_type| {
				matches!(file_type.to_ascii_lowercase().as_str(), "cbr" | "cbz")
			})) {
		candidate.score -= 50.0;
		candidate
			.match_reasons
			.push("category_mismatch".to_string());
	}
	if let Some(matches) = requested_language
		.zip(candidate.language_code.as_deref())
		.map(|(requested, found)| languages_match(requested, found))
	{
		if matches {
			candidate.score += 12.0;
			candidate.match_reasons.push("language".to_string());
		} else {
			candidate.score -= 12.0;
		}
	}
	if let Some(seeders) = candidate.seeders {
		candidate.score += f64::from(seeders.clamp(0, 100)) * 0.1;
		if seeders > 0 {
			candidate.match_reasons.push("seeders".to_string());
		}
	}
	candidate
}

fn languages_match(requested: &str, found: &str) -> bool {
	let requested = requested
		.trim()
		.split(['-', '_'])
		.next()
		.unwrap_or_default();
	let found = found.trim().split(['-', '_']).next().unwrap_or_default();
	!requested.is_empty() && normalize(requested) == normalize(found)
}

/// How long a `queued`/`downloading` grab rests between bridge refreshes.
const REFRESH_INTERVAL: chrono::Duration = chrono::Duration::seconds(60);
/// A `starting` row older than this was interrupted between the bridge POST
/// and the save of its bridge id (the POST itself is bounded to 30 s), so the
/// refresh job looks it up in the bridge listing instead of leaving it.
const STARTING_STALE_AFTER: chrono::Duration = chrono::Duration::minutes(2);
/// How long a `completed` grab whose handoff failed rests before staging is
/// retried; the usual causes (no local library yet, source worker offline)
/// are fixed by an operator, not by polling faster.
const HANDOFF_RETRY_INTERVAL: chrono::Duration = chrono::Duration::minutes(5);
/// Failed handoffs after which a grab is marked `error` with the last reason
/// instead of being retried again: a day at [`HANDOFF_RETRY_INTERVAL`].
pub(crate) const HANDOFF_MAX_ATTEMPTS: i32 = 288;
/// Rows one maintenance tick hands to the refresh job.
const DUE_BATCH_LIMIT: u64 = 100;

/// Grab ids handed to a queued or running refresh job. The maintenance loop
/// ticks every 15 s while one job can spend 20 s per bridge call, so a row
/// stays claimed — invisible to [`due_grab_ids`] — until the job that took it
/// releases it, rather than for a fixed stamp on `updated_at`.
#[derive(Default)]
pub(crate) struct RefreshClaims(Mutex<HashSet<String>>);

impl RefreshClaims {
	fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
		self.0
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
	}

	pub(crate) fn claimed(&self) -> Vec<String> {
		self.lock().iter().cloned().collect()
	}

	pub(crate) fn claim(&self, ids: &[String]) {
		self.lock().extend(ids.iter().cloned());
	}

	/// Releases `ids` when the returned guard drops, whichever way the job ends.
	pub(crate) fn release_on_drop<'a>(&'a self, ids: &[String]) -> ReleaseClaims<'a> {
		ReleaseClaims {
			claims: self,
			ids: ids.to_vec(),
		}
	}
}

pub(crate) struct ReleaseClaims<'a> {
	claims: &'a RefreshClaims,
	ids: Vec<String>,
}

impl Drop for ReleaseClaims<'_> {
	fn drop(&mut self) {
		let mut claimed = self.claims.lock();
		for id in &self.ids {
			claimed.remove(id);
		}
	}
}

/// Grabs the refresh job should visit now, oldest first and excluding the
/// ones a queued or running job already holds: bridge-owned rows due for a
/// poll, `completed` rows whose staging failed and is due for another try,
/// and `starting` rows old enough to have lost their bridge id.
pub(crate) async fn due_grab_ids(
	conn: &DatabaseConnection,
	now: DateTime<FixedOffset>,
	claimed: &[String],
) -> Result<Vec<String>, DbErr> {
	let due = Condition::any()
		.add(
			Condition::all()
				.add(mam_acquisition_grab::Column::Phase.is_in(["queued", "downloading"]))
				.add(mam_acquisition_grab::Column::UpdatedAt.lte(now - REFRESH_INTERVAL)),
		)
		.add(
			Condition::all()
				.add(mam_acquisition_grab::Column::Phase.eq("completed"))
				.add(mam_acquisition_grab::Column::IngestItemId.is_null())
				.add(
					mam_acquisition_grab::Column::UpdatedAt
						.lte(now - HANDOFF_RETRY_INTERVAL),
				),
		)
		.add(
			Condition::all()
				.add(mam_acquisition_grab::Column::Phase.eq("starting"))
				.add(
					mam_acquisition_grab::Column::UpdatedAt
						.lte(now - STARTING_STALE_AFTER),
				),
		);
	let mut query = mam_acquisition_grab::Entity::find().filter(due);
	if !claimed.is_empty() {
		query =
			query.filter(mam_acquisition_grab::Column::Id.is_not_in(claimed.to_vec()));
	}
	query
		.select_only()
		.column(mam_acquisition_grab::Column::Id)
		.order_by_asc(mam_acquisition_grab::Column::UpdatedAt)
		.limit(DUE_BATCH_LIMIT)
		.into_tuple::<String>()
		.all(conn)
		.await
}

/// Refreshes bridge-owned grab state from the scheduled job executor. A row
/// that fails on its own (bridge error, failed handoff) records the failure
/// and the loop moves on; only a local database failure stops the batch.
pub(crate) async fn refresh_grabs(
	services: &JobServices,
	grab_ids: &[String],
) -> Result<(), String> {
	if !services.config.mam_acquisition.enable_mam_acquisition {
		return Ok(());
	}
	let client = BridgeClient::from_config(&services.config)?;
	for id in grab_ids {
		let Some(model) = mam_acquisition_grab::Entity::find_by_id(id)
			.one(services.conn.as_ref())
			.await
			.map_err(|_| "failed to read acquisition grab state".to_string())?
		else {
			continue;
		};
		let (model, bridge) = if model.phase == "starting" {
			match adopt_bridge_grab(services, &client, model).await? {
				Some(adopted) => adopted,
				None => continue,
			}
		} else {
			let Some(bridge_id) = model.bridge_grab_id.as_deref() else {
				continue;
			};
			match client.refresh(bridge_id).await {
				Ok(bridge) if bridge.torrent_id == model.torrent_id => (model, bridge),
				Ok(_) => {
					persist_bridge_error(
						services,
						model,
						"MAM Bridge returned a different torrent id".to_string(),
					)
					.await?;
					continue;
				},
				Err(error) => {
					persist_bridge_error(services, model, error).await?;
					continue;
				},
			}
		};
		let mut active = model.into_active_model();
		active.phase = Set(bridge.phase.clone());
		active.progress = Set(f64::from(bridge.progress_millis.clamp(0, 1000)) / 1000.0);
		active.error = Set(bridge.error.clone());
		let updated = active
			.update(services.conn.as_ref())
			.await
			.map_err(|_| "failed to update acquisition grab state".to_string())?;
		if bridge.phase == "completed" && updated.ingest_item_id.is_none() {
			stage_completed_grab(services, updated, bridge.source_path.as_deref())
				.await?;
		}
	}
	Ok(())
}

/// A `starting` row never saved its bridge id: find the bridge grab for its
/// torrent in the bridge listing and take it over. Any bridge grab another
/// local row already tracks is off limits, an active grab beats a finished
/// one, and the newest wins among equals. With none to adopt the bridge never
/// accepted the POST, so the row ends in `error` and the manager may grab
/// again; a bridge that cannot be listed leaves the row for the next pass.
async fn adopt_bridge_grab(
	services: &JobServices,
	client: &BridgeClient,
	model: mam_acquisition_grab::Model,
) -> Result<Option<(mam_acquisition_grab::Model, BridgeGrabRecord)>, String> {
	let listing = match client.list_grabs().await {
		Ok(listing) => listing,
		Err(error) => {
			persist_bridge_error(services, model, error).await?;
			return Ok(None);
		},
	};
	let tracked = mam_acquisition_grab::Entity::find()
		.filter(mam_acquisition_grab::Column::BridgeGrabId.is_not_null())
		.select_only()
		.column(mam_acquisition_grab::Column::BridgeGrabId)
		.into_tuple::<Option<String>>()
		.all(services.conn.as_ref())
		.await
		.map_err(|_| "failed to read acquisition grab state".to_string())?
		.into_iter()
		.flatten()
		.collect::<HashSet<_>>();
	let adopted = listing
		.into_iter()
		.filter(|grab| grab.torrent_id == model.torrent_id)
		.filter(|grab| !tracked.contains(&grab.grab_id))
		.max_by_key(|grab| (!grab.is_terminal(), grab.created_at_unix));
	let Some(bridge) = adopted else {
		let mut active = model.into_active_model();
		active.phase = Set("error".to_string());
		active.error = Set(Some(
			"MAM Bridge has no grab for this release; grab it again".to_string(),
		));
		active
			.update(services.conn.as_ref())
			.await
			.map_err(|_| "failed to save acquisition grab state".to_string())?;
		return Ok(None);
	};
	let mut active = model.into_active_model();
	active.bridge_grab_id = Set(Some(bridge.grab_id.clone()));
	let model = active
		.update(services.conn.as_ref())
		.await
		.map_err(|_| "failed to save the recovered bridge grab id".to_string())?;
	Ok(Some((model, bridge)))
}

/// Copy a finished download into staged ingest. Staging is idempotent per
/// grab (`mam-acquisition-<grab id>`), so a retry after a failure — or after
/// a crash between staging and this link — finds the item it already made.
/// A failure stays visible in `error`, counts against
/// [`HANDOFF_MAX_ATTEMPTS`], and leaves the row `completed` for
/// [`due_grab_ids`] to bring back; the last allowed failure ends the grab in
/// `error`.
async fn stage_completed_grab(
	services: &JobServices,
	grab: mam_acquisition_grab::Model,
	source_path: Option<&str>,
) -> Result<(), String> {
	match stage_handoff(services, &grab, source_path).await {
		Ok(item_id) => {
			let mut active = grab.into_active_model();
			active.ingest_item_id = Set(Some(item_id));
			active.error = Set(None);
			active
				.update(services.conn.as_ref())
				.await
				.map_err(|_| "failed to link staged acquisition item".to_string())?;
		},
		Err(error) => {
			let attempts = grab.handoff_attempts.saturating_add(1);
			let mut active = grab.into_active_model();
			active.handoff_attempts = Set(attempts);
			if attempts >= HANDOFF_MAX_ATTEMPTS {
				active.phase = Set("error".to_string());
				active.error = Set(Some(format!(
					"handoff failed after {attempts} attempts: {error}"
				)));
			} else {
				active.error = Set(Some(error));
			}
			active
				.update(services.conn.as_ref())
				.await
				.map_err(|_| "failed to save acquisition handoff error".to_string())?;
		},
	}
	Ok(())
}

async fn persist_bridge_error(
	services: &JobServices,
	model: mam_acquisition_grab::Model,
	error: String,
) -> Result<(), String> {
	let mut active = model.into_active_model();
	active.error = Set(Some(error));
	active
		.update(services.conn.as_ref())
		.await
		.map_err(|_| "failed to save acquisition bridge error".to_string())?;
	Ok(())
}

async fn stage_handoff(
	services: &JobServices,
	grab: &mam_acquisition_grab::Model,
	source_path: Option<&str>,
) -> Result<String, String> {
	let source_path = source_path
		.ok_or_else(|| "completed bridge grab has no handoff source path".to_string())?;
	let library = library::Entity::find()
		.filter(library::Column::SourceProvider.is_null())
		.order_by_asc(library::Column::CreatedAt)
		.order_by_asc(library::Column::Id)
		.one(services.conn.as_ref())
		.await
		.map_err(|_| "failed to select a staged-ingest library".to_string())?
		.ok_or_else(|| {
			"create a local library before accepting MAM handoffs".to_string()
		})?;
	// `created_by` is a user: the request's requester, not the request id
	// (which the ingest FK rejected, failing every handoff).
	let requester_id = book_request::Entity::find_by_id(&grab.request_id)
		.one(services.conn.as_ref())
		.await
		.map_err(|_| "failed to read the grab's request".to_string())?
		.ok_or_else(|| "the grab's request no longer exists".to_string())?
		.requester_id;
	let idempotency_key = format!("mam-acquisition-{}", grab.id);
	let staged = if let Some(source_label) = services
		.config
		.mam_acquisition
		.mam_bridge_source_root
		.as_deref()
		.filter(|label| !label.trim().is_empty())
	{
		stage_from_source_worker(
			services,
			source_label,
			source_path,
			&library.id,
			&requester_id,
			&idempotency_key,
		)
		.await?
	} else {
		let root = services
			.config
			.mam_acquisition
			.mam_bridge_handoff_root
			.as_deref()
			.ok_or_else(|| "MAM_BRIDGE_HANDOFF_ROOT is not configured".to_string())?;
		let path = local_handoff_path(root, source_path).await?;
		stage_local_path(
			services,
			&path,
			&library.id,
			&requester_id,
			&idempotency_key,
		)
		.await?
	};
	Ok(staged.id)
}

async fn stage_local_path(
	services: &JobServices,
	path: &Path,
	library_id: &str,
	requester_id: &str,
	idempotency_key: &str,
) -> Result<stump_ingest::store::DropItemModel, String> {
	let filename = path
		.file_name()
		.and_then(|name| name.to_str())
		.ok_or_else(|| "handoff source has no valid filename".to_string())?;
	let metadata = tokio::fs::metadata(path)
		.await
		.map_err(|_| "handoff source is not accessible".to_string())?;
	if metadata.is_dir() {
		let staged = services
			.mam_ingest()
			.store
			.stage_directory_copy(
				library_id,
				Some(requester_id),
				None,
				filename,
				path,
				None,
				Some(idempotency_key),
			)
			.await
			.map_err(|error| format!("could not stage MAM handoff directory: {error}"))?;
		return Ok(staged.item);
	}
	if !metadata.is_file() {
		return Err("handoff source is not a regular file or directory".to_string());
	}
	let reader = tokio::fs::File::open(path)
		.await
		.map_err(|_| "handoff source could not be opened".to_string())?;
	let staged = services
		.mam_ingest()
		.store
		.stage_upload(
			library_id,
			Some(requester_id),
			None,
			filename,
			reader,
			Some(idempotency_key),
		)
		.await
		.map_err(|error| format!("could not stage MAM handoff file: {error}"))?;
	Ok(staged.item)
}

async fn local_handoff_path(root: &str, source_path: &str) -> Result<PathBuf, String> {
	let relative = download_relative_path(source_path)?;
	let canonical_root = tokio::fs::canonicalize(root)
		.await
		.map_err(|_| "MAM handoff root is not accessible".to_string())?;
	let candidate = tokio::fs::canonicalize(canonical_root.join(relative))
		.await
		.map_err(|_| "MAM handoff source does not exist".to_string())?;
	if !candidate.starts_with(&canonical_root) {
		return Err("MAM handoff source escapes its configured root".to_string());
	}
	Ok(candidate)
}

fn download_relative_path(source_path: &str) -> Result<&Path, String> {
	let relative = source_path
		.strip_prefix("/downloads/")
		.ok_or_else(|| "bridge source path is outside /downloads".to_string())?;
	if relative.is_empty() || relative.contains('\\') {
		return Err("bridge source path is invalid".to_string());
	}
	let path = Path::new(relative);
	if path
		.components()
		.any(|component| !matches!(component, Component::Normal(_)))
	{
		return Err("bridge source path is invalid".to_string());
	}
	Ok(path)
}
fn safe_relative_path(value: &str) -> Result<PathBuf, String> {
	if value.is_empty() || value.contains('\\') {
		return Err("source-worker relative path is invalid".to_string());
	}
	let path = Path::new(value);
	if path
		.components()
		.any(|component| !matches!(component, Component::Normal(_)))
	{
		return Err("source-worker relative path is invalid".to_string());
	}
	Ok(path.to_path_buf())
}

async fn stage_from_source_worker(
	services: &JobServices,
	source_label: &str,
	source_path: &str,
	library_id: &str,
	requester_id: &str,
	idempotency_key: &str,
) -> Result<stump_ingest::store::DropItemModel, String> {
	let relative = download_relative_path(source_path)?;
	let relative = relative.to_string_lossy().replace('\\', "/");
	let sources = remote_source::Entity::find()
		.filter(remote_source::Column::Label.eq(source_label))
		.all(services.conn.as_ref())
		.await
		.map_err(|_| "failed to find configured MAM source worker".to_string())?;
	if sources.len() != 1 {
		return Err("configured MAM source root is missing or ambiguous".to_string());
	}
	let source = &sources[0];
	let items = remote_source_item::Entity::find()
		.filter(remote_source_item::Column::SourceId.eq(&source.id))
		.all(services.conn.as_ref())
		.await
		.map_err(|_| "failed to read MAM source-worker inventory".to_string())?;
	let mut matches = items
		.into_iter()
		.filter(|item| {
			item.relative_path.as_deref().is_some_and(|path| {
				path == relative || path.starts_with(&format!("{relative}/"))
			})
		})
		.collect::<Vec<_>>();
	matches.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
	if matches.is_empty() {
		return Err("completed MAM handoff is not present in the configured source-worker inventory".to_string());
	}
	let exact_file = matches.len() == 1
		&& matches[0].relative_path.as_deref() == Some(relative.as_str());
	let filename = Path::new(&relative)
		.file_name()
		.and_then(|name| name.to_str())
		.ok_or_else(|| "bridge source path has no valid filename".to_string())?
		.to_string();
	let temporary =
		std::env::temp_dir().join(format!("coppice-mam-handoff-{}", Uuid::new_v4()));
	tokio::fs::create_dir(&temporary)
		.await
		.map_err(|_| "could not create a temporary MAM handoff directory".to_string())?;
	let materialize = async {
		for item in &matches {
			let item_path = item
				.relative_path
				.as_deref()
				.ok_or_else(|| "source-worker item has no relative path".to_string())?;
			let tail = if exact_file {
				let filename = Path::new(item_path)
					.file_name()
					.and_then(|name| name.to_str())
					.ok_or_else(|| "source-worker filename is invalid".to_string())?;
				safe_relative_path(filename)?
			} else {
				let suffix =
					item_path
						.strip_prefix(&format!("{relative}/"))
						.ok_or_else(|| {
							"source-worker item escaped the requested handoff".to_string()
						})?;
				safe_relative_path(suffix)?
			};
			let destination = temporary.join(tail);
			if let Some(parent) = destination.parent() {
				tokio::fs::create_dir_all(parent).await.map_err(|_| {
					"could not prepare the temporary source-worker handoff".to_string()
				})?;
			}
			copy_source_worker_item(
				services.mam_source_hub(),
				source,
				item,
				&destination,
			)
			.await?;
		}
		if exact_file {
			let path = temporary.join(&filename);
			let reader = tokio::fs::File::open(&path).await.map_err(|_| {
				"verified source-worker file could not be opened".to_string()
			})?;
			let staged = services
				.mam_ingest()
				.store
				.stage_upload(
					library_id,
					Some(requester_id),
					None,
					&filename,
					reader,
					Some(idempotency_key),
				)
				.await
				.map_err(|error| {
					format!("could not stage verified source-worker file: {error}")
				})?;
			Ok(staged.item)
		} else {
			let staged = services
				.mam_ingest()
				.store
				.stage_directory_copy(
					library_id,
					Some(requester_id),
					None,
					&filename,
					&temporary,
					None,
					Some(idempotency_key),
				)
				.await
				.map_err(|error| {
					format!("could not stage verified source-worker directory: {error}")
				})?;
			Ok(staged.item)
		}
	}
	.await;
	let cleanup = tokio::fs::remove_dir_all(&temporary).await;
	match (materialize, cleanup) {
		(Ok(item), Ok(())) => Ok(item),
		(Ok(item), Err(_)) => {
			tracing::warn!("Could not remove temporary MAM handoff files");
			Ok(item)
		},
		(Err(error), _) => Err(error),
	}
}

async fn copy_source_worker_item(
	hub: &SourceHub,
	source: &remote_source::Model,
	item: &remote_source_item::Model,
	destination: &Path,
) -> Result<(), String> {
	let size = u64::try_from(item.size)
		.map_err(|_| "source-worker item has an invalid size".to_string())?;
	let transport = match source.transport.to_ascii_lowercase().as_str() {
		"direct" => SourceTransport::Direct,
		"tunnel" => SourceTransport::Tunnel,
		_ => {
			return Err(
				"configured source worker has an unsupported transport".to_string()
			)
		},
	};
	let request = SourceReadRequest {
		root_id: source.root_id.clone(),
		worker_item_id: item.worker_item_id.clone(),
		worker_content_version: item.worker_content_version.clone(),
		expected_sha256: item.sha256.clone(),
		mode: SourceReadMode::Full,
		offset: 0,
		length: size,
		transport,
		expires_at: Utc::now()
			.timestamp()
			.saturating_add(SOURCE_READ_TIMEOUT.as_secs() as i64),
		max_bytes: size,
	};
	let grant = hub
		.issue_grant(&source.device_id, request)
		.await
		.map_err(|_| "could not authorize a verified source-worker read".to_string())?;
	hub.wait_ready(&grant.grant_id)
		.await
		.map_err(|_| "source worker did not prepare the verified read".to_string())?;
	let mut output = tokio::fs::File::create(destination).await.map_err(|_| {
		"could not create a source-worker materialization file".to_string()
	})?;
	let mut hasher = ring::digest::Context::new(&ring::digest::SHA256);
	let mut total = 0_u64;
	match grant.transport {
		SourceTransport::Direct => {
			let grant = hub
				.consume_direct(&source.device_id, &grant.grant_id)
				.await
				.map_err(|_| {
					"source-worker direct grant could not be consumed".to_string()
				})?;
			let base = match source.direct_base_url.clone() {
				Some(base) => Some(base),
				None => {
					hub.direct_base_url(&source.device_id, &source.root_id)
						.await
				},
			}
			.ok_or_else(|| "source-worker direct endpoint is unavailable".to_string())?;
			let url = format!(
				"{}/v1/source/read/{}",
				base.trim_end_matches('/'),
				grant.grant_id
			);
			let client = reqwest::Client::builder()
				.redirect(reqwest::redirect::Policy::none())
				.timeout(SOURCE_READ_TIMEOUT)
				.build()
				.map_err(|_| {
					"source-worker client could not be initialized".to_string()
				})?;
			let mut response = client
				.get(url)
				.send()
				.await
				.map_err(|_| "source worker is unavailable".to_string())?;
			if !response.status().is_success() {
				return Err("source worker rejected the verified read".to_string());
			}
			while let Some(chunk) = response
				.chunk()
				.await
				.map_err(|_| "source-worker response could not be read".to_string())?
			{
				write_verified_chunk(&mut output, &mut hasher, &mut total, &chunk, size)
					.await?;
			}
		},
		SourceTransport::Tunnel => {
			let mut receiver = hub
				.accept_tunnel(&source.device_id, &grant.grant_id)
				.await
				.map_err(|_| "source-worker tunnel could not be opened".to_string())?;
			while let Some(chunk) = receiver.recv().await {
				write_verified_chunk(&mut output, &mut hasher, &mut total, &chunk, size)
					.await?;
			}
		},
	}
	output
		.flush()
		.await
		.map_err(|_| "source-worker materialization could not be flushed".to_string())?;
	if total != size {
		return Err(
			"source-worker materialization had an unexpected byte count".to_string()
		);
	}
	let digest = digest_hex(hasher.finish());
	if item
		.sha256
		.as_deref()
		.is_some_and(|expected| expected != digest)
	{
		return Err(
			"source-worker materialization failed digest verification".to_string()
		);
	}
	Ok(())
}

async fn write_verified_chunk(
	output: &mut tokio::fs::File,
	hasher: &mut ring::digest::Context,
	total: &mut u64,
	chunk: &[u8],
	expected_size: u64,
) -> Result<(), String> {
	*total = total.saturating_add(chunk.len() as u64);
	if *total > expected_size {
		return Err("source worker exceeded the verified byte budget".to_string());
	}
	output
		.write_all(chunk)
		.await
		.map_err(|_| "source-worker materialization could not be written".to_string())?;
	hasher.update(chunk);
	Ok(())
}

fn digest_hex(digest: ring::digest::Digest) -> String {
	const HEX: &[u8; 16] = b"0123456789abcdef";
	let mut output = String::with_capacity(digest.as_ref().len() * 2);
	for byte in digest.as_ref() {
		output.push(HEX[(byte >> 4) as usize] as char);
		output.push(HEX[(byte & 0x0f) as usize] as char);
	}
	output
}

#[cfg(test)]
mod tests {
	use std::{
		io::{Read, Write},
		net::TcpListener,
		thread::JoinHandle,
		time::Duration,
	};

	use chrono::Utc;
	use models::entity::book_request;
	use serde_json::{json, Value};

	use super::{
		decode_bridge_text_fields, languages_match, parse_candidate, person_names_match,
		score_candidate, BridgeClient,
	};

	fn mock_bridge(
		status: u16,
		additional_headers: &'static str,
		body: &'static str,
	) -> (String, JoinHandle<String>) {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let address = listener.local_addr().unwrap();
		let server = std::thread::spawn(move || {
			let (mut stream, _) = listener.accept().unwrap();
			stream
				.set_read_timeout(Some(Duration::from_secs(5)))
				.unwrap();
			let mut request = Vec::new();
			loop {
				let mut chunk = [0; 4096];
				let read = stream.read(&mut chunk).unwrap();
				if read == 0 {
					break;
				}
				request.extend_from_slice(&chunk[..read]);
				if let Some(header_end) =
					request.windows(4).position(|window| window == b"\r\n\r\n")
				{
					let headers = String::from_utf8_lossy(&request[..header_end]);
					let content_length = headers
						.lines()
						.find_map(|line| {
							let (name, value) = line.split_once(':')?;
							name.eq_ignore_ascii_case("content-length")
								.then(|| value.trim().parse::<usize>().ok())
								.flatten()
						})
						.unwrap_or(0);
					if request.len() >= header_end + 4 + content_length {
						break;
					}
				}
			}
			let status_text = match status {
				200 => "OK",
				403 => "Forbidden",
				429 => "Too Many Requests",
				_ => "Error",
			};
			let response = format!(
				"HTTP/1.1 {status} {status_text}\r\n\
				 Content-Type: application/json\r\n\
				 {additional_headers}\
				 Content-Length: {}\r\n\
				 Connection: close\r\n\r\n\
				 {body}",
				body.len(),
			);
			stream.write_all(response.as_bytes()).unwrap();
			String::from_utf8_lossy(&request).into_owned()
		});
		(format!("http://{address}/"), server)
	}

	fn client_for(base: &str) -> (BridgeClient, tempfile::NamedTempFile) {
		let mut token = tempfile::NamedTempFile::new().unwrap();
		token.write_all(b"fixture-test-token\n").unwrap();
		let client = BridgeClient {
			base: reqwest::Url::parse(base).unwrap(),
			token_file: token.path().to_owned(),
			client: reqwest::Client::builder().no_proxy().build().unwrap(),
		};
		(client, token)
	}

	fn request_parts(request: &str) -> (&str, Value) {
		let (headers, body) = request.split_once("\r\n\r\n").unwrap();
		(headers, serde_json::from_str(body).unwrap())
	}

	fn test_request(
		title: &str,
		format: &str,
		authors: Option<&str>,
	) -> book_request::Model {
		let now = Utc::now().fixed_offset();
		book_request::Model {
			id: "request-1".to_string(),
			requester_id: "user-1".to_string(),
			internal_media_id: None,
			internal_work_id: None,
			source_provider: None,
			remote_id: None,
			external_key: None,
			format: format.to_string(),
			isbn: None,
			title: title.to_string(),
			authors: authors.map(str::to_owned),
			cover_url: None,
			destination_shelf_id: None,
			destination_device_id: None,
			status: "APPROVED".to_string(),
			approval_policy: "MANUAL".to_string(),
			approved_by: None,
			rejected_by: None,
			failure_code: None,
			failure_message: None,
			created_at: now.clone(),
			updated_at: now,
			approved_at: None,
			completed_at: None,
			preferred_narrator: None,
		}
	}

	#[tokio::test]
	async fn search_sends_authenticated_probe_payload() {
		let (base, server) = mock_bridge(200, "", r#"{"candidates":[],"found":0}"#);
		let (client, _token) = client_for(&base);
		let result = client.search("A book", Some(vec![14]), 5).await.unwrap();
		assert!(result.candidates.is_empty());
		assert_eq!(result.found, Some(serde_json::json!(0)));

		let request = server.join().unwrap();
		let (headers, payload) = request_parts(&request);
		assert!(headers
			.to_ascii_lowercase()
			.contains("authorization: bearer fixture-test-token"));
		assert!(headers.starts_with("POST /api/search "));
		assert_eq!(payload["text"], "A book");
		assert_eq!(
			payload["fields"],
			serde_json::json!(["title", "author", "narrator", "series"])
		);
		assert_eq!(payload["main_categories"], serde_json::json!([14]));
		assert_eq!(payload["limit"], 5);
		assert_eq!(payload["offset"], 0);
	}

	#[tokio::test]
	async fn search_maps_paused_and_rate_limited_responses() {
		for (status, headers, body, expected) in [
			(
				403,
				"",
				r#"{"detail":"Search disabled"}"#,
				"Search is paused, check the bridge",
			),
			(
				429,
				"Retry-After: 7\r\n",
				r#"{"detail":"slow down"}"#,
				"Try again in 7 s",
			),
		] {
			let (base, server) = mock_bridge(status, headers, body);
			let (client, _token) = client_for(&base);
			let error = client.search("A book", None, 5).await.unwrap_err();
			assert_eq!(error, expected);
			let request = server.join().unwrap();
			assert!(request.starts_with("POST /api/search "));
		}
	}

	#[tokio::test]
	async fn grab_sends_required_confirmation_text() {
		let (base, server) = mock_bridge(
			200,
			"",
			r#"{"grab_id":"grab-1","torrent_id":42,"phase":"queued","progress_millis":0}"#,
		);
		let (client, _token) = client_for(&base);
		let grab = client.grab(42).await.unwrap();
		assert_eq!(grab.grab_id, "grab-1");
		assert_eq!(grab.torrent_id, 42);

		let request = server.join().unwrap();
		let (headers, payload) = request_parts(&request);
		assert!(headers.starts_with("POST /api/grabs "));
		assert_eq!(payload["torrent_id"], 42);
		assert_eq!(payload["confirmation"], "download torrent 42");
		assert!(payload.get("fl").is_none());
	}

	/// The base path ends in `/`; the grab id must follow it directly, not
	/// an empty segment (`api/grabs//grab-1/refresh`), which the bridge
	/// router does not route.
	#[tokio::test]
	async fn refresh_and_listing_hit_the_documented_bridge_paths() {
		let (base, server) = mock_bridge(
			200,
			"",
			r#"{"grab_id":"grab-1","torrent_id":42,"phase":"downloading","progress_millis":400}"#,
		);
		let (client, _token) = client_for(&base);
		let grab = client.refresh("grab-1").await.unwrap();
		assert_eq!(grab.phase, "downloading");
		let request = server.join().unwrap();
		assert!(
			request.starts_with("POST /api/grabs/grab-1/refresh "),
			"{request}"
		);

		let (base, server) = mock_bridge(
			200,
			"",
			r#"{"schema_version":1,"grabs":[{"grab_id":"grab-1","torrent_id":42,"phase":"queued","created_at_unix":7}]}"#,
		);
		let (client, _token) = client_for(&base);
		let grabs = client.list_grabs().await.unwrap();
		assert_eq!(grabs.len(), 1);
		assert_eq!(grabs[0].created_at_unix, 7);
		let request = server.join().unwrap();
		assert!(request.starts_with("GET /api/state "), "{request}");
		assert!(request
			.to_ascii_lowercase()
			.contains("authorization: bearer fixture-test-token"));
	}

	#[test]
	fn candidate_language_matches_primary_bcp47_tag() {
		assert!(languages_match("en-US", "en"));
		assert!(languages_match("pt-BR", "pt-PT"));
		assert!(!languages_match("fr-FR", "en"));
		assert!(!languages_match("", "en"));
	}

	#[test]
	fn bridge_candidate_text_decodes_html_entities() {
		let mut payload = json!({
			"torrent_id": 42,
			"title": "Ender&#039;s Game: Battle School (2008)",
			"authors": [{"name": "Orson Card &amp; Scott Card"}],
			"narrators": ["N&#xE9;e"],
			"series": [{"name": "Ender&#039;s Game"}],
			"category_name": "Ebooks &ndash; Comics / Graphic Novels",
			"description": "A &rsquo; tale &amp; more",
			"file_type": "cbr",
			"kind": "ebook"
		});
		decode_bridge_text_fields(&mut payload);

		let candidate = parse_candidate(&payload).unwrap();
		assert_eq!(candidate.title, "Ender's Game: Battle School (2008)");
		assert_eq!(candidate.authors, ["Orson Card & Scott Card"]);
		assert_eq!(candidate.narrators, ["Née"]);
		assert_eq!(candidate.series, ["Ender's Game"]);
		assert_eq!(
			candidate.category_name.as_deref(),
			Some("Ebooks – Comics / Graphic Novels")
		);
		assert_eq!(candidate.description.as_deref(), Some("A ’ tale & more"));
		let request = test_request("Ender's Game: Battle School (2008)", "EBOOK", None);
		let scored = score_candidate(candidate, &request, "EBOOK", None, None, false);
		assert!(scored
			.match_reasons
			.iter()
			.any(|reason| reason == "exact_title"));
	}

	#[test]
	fn candidate_ranking_distinguishes_exact_titles_and_penalizes_comics() {
		let request = test_request("Ender's Game", "EBOOK", None);
		let exact = parse_candidate(&json!({
			"torrent_id": 1,
			"title": "Ender's Game",
			"kind": "ebook",
			"seeders": 21
		}))
		.unwrap();
		let exact = score_candidate(exact, &request, "EBOOK", None, None, false);
		assert!(exact
			.match_reasons
			.iter()
			.any(|reason| reason == "exact_title"));
		assert!(!exact.match_reasons.iter().any(|reason| reason == "author"));
		assert_eq!(serde_json::to_string(&exact.score).unwrap(), "72.1");

		let partial = parse_candidate(&json!({
			"torrent_id": 2,
			"title": "Ender's Game: Battle School (2008)",
			"kind": "ebook",
			"category_name": "Ebooks - Comics / Graphic Novels",
			"file_type": "epub",
			"seeders": 21
		}))
		.unwrap();
		let ebook =
			score_candidate(partial.clone(), &request, "EBOOK", None, None, false);
		assert!(ebook
			.match_reasons
			.iter()
			.any(|reason| reason == "title_partial"));
		assert!(ebook
			.match_reasons
			.iter()
			.any(|reason| reason == "category_mismatch"));
		assert!(ebook.score < exact.score);

		let any = score_candidate(partial.clone(), &request, "ANY", None, None, false);
		assert!(any
			.match_reasons
			.iter()
			.any(|reason| reason == "category_mismatch"));
		let mut cbr_only = partial.clone();
		cbr_only.category_name = None;
		cbr_only.file_type = Some("cbr".to_string());
		let cbr = score_candidate(cbr_only.clone(), &request, "EBOOK", None, None, false);
		assert!(cbr
			.match_reasons
			.iter()
			.any(|reason| reason == "category_mismatch"));
		cbr_only.file_type = Some("cbz".to_string());
		let cbz = score_candidate(cbr_only, &request, "ANY", None, None, false);
		assert!(cbz
			.match_reasons
			.iter()
			.any(|reason| reason == "category_mismatch"));
		let linked_comic = score_candidate(partial, &request, "EBOOK", None, None, true);
		assert!(!linked_comic
			.match_reasons
			.iter()
			.any(|reason| reason == "category_mismatch"));
	}

	#[test]
	fn preferred_narrator_biases_audiobook_ranking_without_excluding() {
		let mut request =
			test_request("Project Hail Mary", "AUDIOBOOK", Some("Andy Weir"));
		request.preferred_narrator = Some("Ray Porter".to_string());
		let candidate = |id: i32, narrators: Value| {
			parse_candidate(&json!({
				"torrent_id": id,
				"title": "Project Hail Mary",
				"kind": "audiobook",
				"authors": ["Andy Weir"],
				"narrators": narrators,
				"seeders": 10
			}))
			.unwrap()
		};

		let preferred = score_candidate(
			candidate(1, json!(["Porter, Ray"])),
			&request,
			"AUDIOBOOK",
			None,
			None,
			false,
		);
		let other = score_candidate(
			candidate(2, json!(["Kate Reading"])),
			&request,
			"AUDIOBOOK",
			None,
			None,
			false,
		);
		let unnamed = score_candidate(
			candidate(3, json!([])),
			&request,
			"AUDIOBOOK",
			None,
			None,
			false,
		);

		assert!(preferred
			.match_reasons
			.iter()
			.any(|reason| reason == "narrator"));
		assert!(preferred
			.match_reasons
			.iter()
			.any(|reason| reason == "author"));
		assert!(!other
			.match_reasons
			.iter()
			.any(|reason| reason == "narrator"));
		assert_eq!(
			preferred.score - other.score,
			30.0,
			"same weight as the author match"
		);
		assert_eq!(
			other.score, unnamed.score,
			"other readers are never penalised"
		);

		let ebook = score_candidate(
			candidate(4, json!(["Ray Porter"])),
			&request,
			"EBOOK",
			None,
			None,
			false,
		);
		assert!(
			!ebook
				.match_reasons
				.iter()
				.any(|reason| reason == "narrator"),
			"an ebook search ignores the narrator"
		);
		let any = score_candidate(
			candidate(5, json!(["ray porter"])),
			&request,
			"ANY",
			None,
			None,
			false,
		);
		assert!(any.match_reasons.iter().any(|reason| reason == "narrator"));

		assert!(person_names_match("Ray Porter", "Porter, Ray"));
		assert!(person_names_match("Porter, Ray", "Ray Porter"));
		assert!(!person_names_match("Ray Porter", "Ray Porter Jr."));
		assert!(!person_names_match("", "Ray Porter"));
	}

	#[test]
	fn audible_requests_match_their_asin_against_the_bridge_identifier() {
		let mut request = test_request("Project Hail Mary", "AUDIOBOOK", None);
		request.source_provider = Some("audible".to_string());
		request.remote_id = Some("B08G9PRS1K".to_string());
		let candidate = |id: i32, isbn: Value| {
			parse_candidate(&json!({
				"torrent_id": id,
				"title": "Project Hail Mary",
				"kind": "audiobook",
				"isbn": isbn
			}))
			.unwrap()
		};

		let matched = score_candidate(
			candidate(1, json!("b08g9prs1k")),
			&request,
			"AUDIOBOOK",
			None,
			None,
			false,
		);
		let other = score_candidate(
			candidate(2, json!("9780593135204")),
			&request,
			"AUDIOBOOK",
			None,
			None,
			false,
		);
		assert!(matched.match_reasons.iter().any(|reason| reason == "asin"));
		assert!(!other.match_reasons.iter().any(|reason| reason == "asin"));
		assert_eq!(matched.score - other.score, 100.0);

		request.source_provider = Some("hardcover".to_string());
		let hardcover = score_candidate(
			candidate(3, json!("b08g9prs1k")),
			&request,
			"AUDIOBOOK",
			None,
			None,
			false,
		);
		assert!(
			!hardcover
				.match_reasons
				.iter()
				.any(|reason| reason == "asin"),
			"a Hardcover book id is not an ASIN"
		);
	}
}

/// The refresh job and its dispatcher against a migrated database and a
/// scripted bridge; nothing here reaches a real MAM Bridge.
#[cfg(test)]
mod job_tests {
	use std::{
		io::{Read, Write},
		net::TcpListener,
		sync::{Arc, Mutex},
		time::Duration,
	};

	use chrono::{DateTime, FixedOffset, Utc};
	use migrations::MigratorTrait;
	use models::{
		entity::{
			book_request, ingest_drop_item, library, library_config,
			mam_acquisition_grab, mam_release_search,
		},
		shared::enums::FileStatus,
	};
	use sea_orm::{
		sea_query::Expr, ActiveModelTrait, ActiveValue::Set, ColumnTrait, Database,
		DatabaseConnection, EntityTrait, QueryFilter,
	};
	use serde_json::json;
	use stump_jobs::{JobRuntime, ScheduledJobDispatcher};

	use super::{
		create_grab, due_grab_ids, refresh_grabs, RefreshClaims, HANDOFF_MAX_ATTEMPTS,
	};
	use crate::{config::StumpConfig, job::JobServices, Ctx};

	/// One scripted response: the request line prefix it answers (`"POST
	/// /api/grabs/grab-1/refresh"`), the status and JSON body, and how long
	/// the bridge sits on the request before answering.
	struct Route {
		prefix: &'static str,
		status: u16,
		body: String,
		delay: Duration,
	}

	fn route(prefix: &'static str, body: serde_json::Value) -> Route {
		Route {
			prefix,
			status: 200,
			body: body.to_string(),
			delay: Duration::ZERO,
		}
	}

	/// A bridge that answers every connection from its routes (404 for any
	/// other request) and records each request line, for as long as the test
	/// binary lives.
	struct ScriptedBridge {
		url: String,
		requests: Arc<Mutex<Vec<String>>>,
	}

	impl ScriptedBridge {
		fn spawn(routes: Vec<Route>) -> Self {
			let listener = TcpListener::bind("127.0.0.1:0").unwrap();
			let url = format!("http://{}/", listener.local_addr().unwrap());
			let requests = Arc::new(Mutex::new(Vec::new()));
			let log = Arc::clone(&requests);
			std::thread::spawn(move || {
				for connection in listener.incoming() {
					let Ok(mut stream) = connection else {
						break;
					};
					let request = read_request(&mut stream);
					let request_line =
						request.lines().next().unwrap_or_default().to_owned();
					log.lock().unwrap().push(request_line.clone());
					let (status, body, delay) = routes
						.iter()
						.find(|route| request_line.starts_with(route.prefix))
						.map(|route| (route.status, route.body.as_str(), route.delay))
						.unwrap_or((404, r#"{"detail":"unknown"}"#, Duration::ZERO));
					std::thread::sleep(delay);
					let response = format!(
						"HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
						body.len()
					);
					let _ = stream.write_all(response.as_bytes());
				}
			});
			Self { url, requests }
		}

		fn requests(&self) -> Vec<String> {
			self.requests.lock().unwrap().clone()
		}
	}

	fn read_request(stream: &mut std::net::TcpStream) -> String {
		stream
			.set_read_timeout(Some(Duration::from_secs(5)))
			.unwrap();
		let mut request = Vec::new();
		let mut chunk = [0; 4096];
		let header_end = loop {
			let read = stream.read(&mut chunk).unwrap_or(0);
			if read == 0 {
				return String::from_utf8_lossy(&request).into_owned();
			}
			request.extend_from_slice(&chunk[..read]);
			if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n")
			{
				break end + 4;
			}
		};
		let content_length = String::from_utf8_lossy(&request[..header_end])
			.lines()
			.find_map(|line| {
				let (name, value) = line.split_once(':')?;
				name.eq_ignore_ascii_case("content-length")
					.then(|| value.trim().parse::<usize>().ok())
					.flatten()
			})
			.unwrap_or(0);
		while request.len() < header_end + content_length {
			let read = stream.read(&mut chunk).unwrap_or(0);
			if read == 0 {
				break;
			}
			request.extend_from_slice(&chunk[..read]);
		}
		String::from_utf8_lossy(&request).into_owned()
	}

	struct Harness {
		ctx: Ctx,
		services: Arc<JobServices>,
		bridge: ScriptedBridge,
		_root: tempfile::TempDir,
		_token: tempfile::NamedTempFile,
	}

	impl Harness {
		fn conn(&self) -> &DatabaseConnection {
			self.ctx.conn.as_ref()
		}
	}

	async fn harness(routes: Vec<Route>) -> Harness {
		let bridge = ScriptedBridge::spawn(routes);
		let root = tempfile::tempdir().unwrap();
		let mut token = tempfile::NamedTempFile::new().unwrap();
		token.write_all(b"fixture-test-token\n").unwrap();
		std::fs::create_dir_all(root.path().join("downloads")).unwrap();
		std::fs::write(
			root.path().join("downloads/book.epub"),
			b"not really an epub",
		)
		.unwrap();

		let conn = Database::connect("sqlite::memory:").await.unwrap();
		migrations::Migrator::up(&conn, None).await.unwrap();

		let mut config = StumpConfig::debug();
		config.config_dir = root.path().to_string_lossy().into_owned();
		config.mam_acquisition.enable_mam_acquisition = true;
		config.mam_acquisition.mam_bridge_url = Some(bridge.url.clone());
		config.mam_acquisition.mam_bridge_token_file =
			Some(token.path().to_string_lossy().into_owned());
		config.mam_acquisition.mam_bridge_handoff_root =
			Some(root.path().join("downloads").to_string_lossy().into_owned());
		let ctx = Ctx::for_testing_with_config(conn, config);
		let services = Arc::new(
			JobServices::new(
				ctx.conn.clone(),
				ctx.config.clone(),
				ctx.get_event_tx(),
				ctx.visible_pages_cache(),
			)
			.with_mam_dependencies(ctx.ingest(), ctx.source_hub()),
		);
		Harness {
			ctx,
			services,
			bridge,
			_root: root,
			_token: token,
		}
	}

	async fn seed_request(conn: &DatabaseConnection, id: &str) {
		let user = ::tests::fake_data::User::new(format!("manager-{id}"))
			.insert(conn)
			.await;
		let now = Utc::now().fixed_offset();
		book_request::ActiveModel {
			id: Set(id.to_owned()),
			requester_id: Set(user.id),
			format: Set("EBOOK".to_owned()),
			title: Set("Ender's Game".to_owned()),
			status: Set("APPROVED".to_owned()),
			approval_policy: Set("MANUAL".to_owned()),
			created_at: Set(now),
			updated_at: Set(now),
			..Default::default()
		}
		.insert(conn)
		.await
		.unwrap();
	}

	async fn seed_library(conn: &DatabaseConnection) {
		let config = <library_config::ActiveModel as Default>::default()
			.insert(conn)
			.await
			.unwrap();
		library::ActiveModel {
			id: Set("library".to_owned()),
			name: Set("Library".to_owned()),
			path: Set("/tmp/library".to_owned()),
			status: Set(FileStatus::Ready),
			config_id: Set(config.id),
			..Default::default()
		}
		.insert(conn)
		.await
		.unwrap();
	}

	struct GrabRow {
		id: &'static str,
		request_id: &'static str,
		torrent_id: i64,
		bridge_grab_id: Option<&'static str>,
		phase: &'static str,
		ingest_item_id: Option<String>,
		handoff_attempts: i32,
		updated_at: DateTime<FixedOffset>,
	}

	fn grab(id: &'static str, phase: &'static str, age: chrono::Duration) -> GrabRow {
		GrabRow {
			id,
			request_id: "request-1",
			torrent_id: 42,
			bridge_grab_id: Some("grab-1"),
			phase,
			ingest_item_id: None,
			handoff_attempts: 0,
			updated_at: Utc::now().fixed_offset() - age,
		}
	}

	async fn insert_grab(conn: &DatabaseConnection, row: GrabRow) {
		mam_acquisition_grab::ActiveModel {
			id: Set(row.id.to_owned()),
			request_id: Set(row.request_id.to_owned()),
			torrent_id: Set(row.torrent_id),
			bridge_grab_id: Set(row.bridge_grab_id.map(str::to_owned)),
			title: Set("Ender's Game".to_owned()),
			phase: Set(row.phase.to_owned()),
			progress: Set(0.0),
			error: Set(None),
			ingest_item_id: Set(row.ingest_item_id),
			handoff_attempts: Set(row.handoff_attempts),
			created_at: Set(row.updated_at),
			updated_at: Set(row.updated_at),
		}
		.insert(conn)
		.await
		.unwrap();
		// `before_save` stamps `updated_at` with the present; the row must
		// look as old as the scenario says.
		age_grab(conn, row.id, row.updated_at).await;
	}

	async fn age_grab(
		conn: &DatabaseConnection,
		id: &str,
		updated_at: DateTime<FixedOffset>,
	) {
		mam_acquisition_grab::Entity::update_many()
			.col_expr(
				mam_acquisition_grab::Column::UpdatedAt,
				Expr::value(updated_at),
			)
			.filter(mam_acquisition_grab::Column::Id.eq(id))
			.exec(conn)
			.await
			.unwrap();
	}

	async fn load_grab(
		conn: &DatabaseConnection,
		id: &str,
	) -> mam_acquisition_grab::Model {
		mam_acquisition_grab::Entity::find_by_id(id)
			.one(conn)
			.await
			.unwrap()
			.expect("grab row")
	}

	fn bridge_grab(grab_id: &str, torrent_id: i64, phase: &str) -> serde_json::Value {
		json!({
			"grab_id": grab_id,
			"torrent_id": torrent_id,
			"phase": phase,
			"progress_millis": if phase == "completed" { 1000 } else { 250 },
			"source_path": if phase == "completed" { Some("/downloads/book.epub") } else { None },
			"created_at_unix": 1_700_000_000,
		})
	}

	fn minutes(count: i64) -> chrono::Duration {
		chrono::Duration::minutes(count)
	}

	#[tokio::test]
	async fn due_rows_follow_phase_cadence_and_skip_claimed_ids() {
		let harness = harness(Vec::new()).await;
		let conn = harness.conn();
		seed_request(conn, "request-1").await;
		for row in [
			grab("poll-due", "queued", minutes(2)),
			grab("poll-fresh", "downloading", chrono::Duration::seconds(10)),
			grab("handoff-due", "completed", minutes(6)),
			grab("handoff-fresh", "completed", minutes(2)),
			GrabRow {
				ingest_item_id: None,
				..grab("handoff-done", "completed", minutes(60))
			},
			GrabRow {
				bridge_grab_id: None,
				..grab("starting-stale", "starting", minutes(3))
			},
			GrabRow {
				bridge_grab_id: None,
				..grab("starting-inflight", "starting", minutes(1))
			},
			grab("failed", "error", minutes(60)),
			grab("aborted", "aborted", minutes(60)),
		] {
			insert_grab(conn, row).await;
		}
		// A staged handoff is finished work whatever its age.
		seed_library(conn).await;
		let item = harness
			.ctx
			.ingest()
			.store
			.stage_upload("library", None, None, "done.epub", &b"done"[..], None)
			.await
			.unwrap()
			.item;
		mam_acquisition_grab::Entity::update_many()
			.col_expr(
				mam_acquisition_grab::Column::IngestItemId,
				Expr::value(Some(item.id)),
			)
			.filter(mam_acquisition_grab::Column::Id.eq("handoff-done"))
			.exec(conn)
			.await
			.unwrap();
		age_grab(
			conn,
			"handoff-done",
			Utc::now().fixed_offset() - minutes(60),
		)
		.await;

		let now = Utc::now().fixed_offset();
		let due = due_grab_ids(conn, now, &[]).await.unwrap();
		assert_eq!(
			due,
			["handoff-due", "starting-stale", "poll-due"],
			"oldest first"
		);

		let due = due_grab_ids(conn, now, &["poll-due".to_owned()])
			.await
			.unwrap();
		assert_eq!(
			due,
			["handoff-due", "starting-stale"],
			"a claimed row is not due"
		);

		let claims = RefreshClaims::default();
		claims.claim(&["a".to_owned(), "b".to_owned()]);
		{
			let _release = claims.release_on_drop(&["a".to_owned()]);
			assert_eq!(claims.claimed().len(), 2);
		}
		assert_eq!(claims.claimed(), ["b"], "released when the guard drops");
	}

	#[tokio::test]
	async fn a_dispatched_row_stays_claimed_until_its_job_finishes() {
		let harness = harness(vec![Route {
			delay: Duration::from_millis(600),
			..route(
				"POST /api/grabs/grab-1/refresh",
				bridge_grab("grab-1", 42, "downloading"),
			)
		}])
		.await;
		let conn = harness.conn();
		seed_request(conn, "request-1").await;
		insert_grab(conn, grab("slow", "queued", minutes(2))).await;
		let runtime = JobRuntime::inline(Arc::clone(&harness.services));

		harness.services.dispatch_due(&runtime).await.unwrap();
		tokio::time::sleep(Duration::from_millis(150)).await;
		// The job is still inside its 600 ms bridge call. Pretend the whole
		// refresh interval passed meanwhile, as it does for a slow batch.
		age_grab(conn, "slow", Utc::now().fixed_offset() - minutes(2)).await;
		harness.services.dispatch_due(&runtime).await.unwrap();
		tokio::time::sleep(Duration::from_millis(1_200)).await;
		assert_eq!(
			harness.bridge.requests().len(),
			1,
			"the running job holds the row; no second refresh: {:?}",
			harness.bridge.requests()
		);
		let refreshed = load_grab(conn, "slow").await;
		assert_eq!(refreshed.phase, "downloading");

		// Once the job is done the row is released and due again when its time comes.
		age_grab(conn, "slow", Utc::now().fixed_offset() - minutes(2)).await;
		harness.services.dispatch_due(&runtime).await.unwrap();
		tokio::time::sleep(Duration::from_millis(1_200)).await;
		assert_eq!(harness.bridge.requests().len(), 2);
		runtime.stop().await;
	}

	#[tokio::test]
	async fn a_stale_starting_row_adopts_its_bridge_grab_or_ends_in_error() {
		let harness = harness(vec![route(
			"GET /api/state",
			json!({
				"schema_version": 1,
				"grabs": [
					bridge_grab("grab-0", 42, "error"),
					bridge_grab("grab-7", 42, "downloading"),
					bridge_grab("grab-8", 42, "aborted"),
				]
			}),
		)])
		.await;
		let conn = harness.conn();
		seed_request(conn, "request-1").await;
		// An earlier attempt at the same torrent already tracks grab-0.
		insert_grab(
			conn,
			GrabRow {
				bridge_grab_id: Some("grab-0"),
				..grab("earlier", "error", minutes(30))
			},
		)
		.await;
		insert_grab(
			conn,
			GrabRow {
				bridge_grab_id: None,
				..grab("interrupted", "starting", minutes(3))
			},
		)
		.await;
		insert_grab(
			conn,
			GrabRow {
				bridge_grab_id: None,
				torrent_id: 99,
				..grab("never-accepted", "starting", minutes(3))
			},
		)
		.await;

		refresh_grabs(
			&harness.services,
			&["interrupted".to_owned(), "never-accepted".to_owned()],
		)
		.await
		.unwrap();

		let adopted = load_grab(conn, "interrupted").await;
		assert_eq!(adopted.bridge_grab_id.as_deref(), Some("grab-7"));
		assert_eq!(adopted.phase, "downloading");
		assert_eq!(adopted.progress, 0.25);
		let lost = load_grab(conn, "never-accepted").await;
		assert_eq!(lost.phase, "error");
		assert_eq!(
			lost.error.as_deref(),
			Some("MAM Bridge has no grab for this release; grab it again")
		);
		assert!(
			harness
				.bridge
				.requests()
				.iter()
				.all(|line| line.starts_with("GET /api/state")),
			"recovery only reads the listing, it never grabs again: {:?}",
			harness.bridge.requests()
		);
	}

	#[tokio::test]
	async fn confirming_an_actively_grabbed_release_again_never_posts_twice() {
		let harness = harness(Vec::new()).await;
		let conn = harness.conn();
		seed_request(conn, "request-1").await;
		seed_request(conn, "request-2").await;
		let now = Utc::now().fixed_offset();
		for request_id in ["request-1", "request-2"] {
			mam_release_search::ActiveModel {
				request_id: Set(request_id.to_owned()),
				candidates_json: Set(json!([{
					"torrentId": 42, "title": "Ender's Game", "authors": [], "narrators": [],
					"series": [], "kind": "ebook", "categoryName": null, "languageCode": null,
					"fileType": null, "size": null, "numFiles": null, "added": null,
					"seeders": null, "leechers": null, "timesCompleted": null,
					"freeleech": false, "vip": false, "snatched": false, "isbn": null,
					"score": 1.0, "matchReasons": []
				}])
				.to_string()),
				found: Set(Some(1)),
				probe: Set(false),
				searched_at: Set(now),
				updated_at: Set(now),
			}
			.insert(conn)
			.await
			.unwrap();
		}
		insert_grab(
			conn,
			GrabRow {
				bridge_grab_id: None,
				..grab("interrupted", "starting", minutes(1))
			},
		)
		.await;
		let request = book_request::Entity::find_by_id("request-1")
			.one(conn)
			.await
			.unwrap()
			.unwrap();

		let again = create_grab(&harness.ctx, &request, 42).await.unwrap();
		assert_eq!(again.id, "interrupted", "the persisted row is the answer");

		let other = book_request::Entity::find_by_id("request-2")
			.one(conn)
			.await
			.unwrap()
			.unwrap();
		let refused = create_grab(&harness.ctx, &other, 42).await.unwrap_err();
		assert!(
			refused
				.to_string()
				.contains("already being acquired for another request"),
			"{refused}"
		);
		assert!(
			harness.bridge.requests().is_empty(),
			"no bridge POST at all"
		);
		assert_eq!(
			mam_acquisition_grab::Entity::find()
				.all(conn)
				.await
				.unwrap()
				.len(),
			1
		);
	}

	#[tokio::test]
	async fn a_completed_grab_whose_handoff_failed_is_retried_and_bounded() {
		let harness = harness(vec![route(
			"POST /api/grabs/grab-1/refresh",
			bridge_grab("grab-1", 42, "completed"),
		)])
		.await;
		let conn = harness.conn();
		seed_request(conn, "request-1").await;
		insert_grab(conn, grab("finished", "downloading", minutes(2))).await;

		// No local library yet: the download is complete but cannot be staged.
		refresh_grabs(&harness.services, &["finished".to_owned()])
			.await
			.unwrap();
		let stranded = load_grab(conn, "finished").await;
		assert_eq!(stranded.phase, "completed");
		assert_eq!(stranded.ingest_item_id, None);
		assert_eq!(stranded.handoff_attempts, 1);
		assert_eq!(
			stranded.error.as_deref(),
			Some("create a local library before accepting MAM handoffs")
		);
		let now = Utc::now().fixed_offset();
		assert!(
			due_grab_ids(conn, now, &[]).await.unwrap().is_empty(),
			"a failed handoff rests before its retry"
		);
		assert_eq!(
			due_grab_ids(conn, now + minutes(6), &[]).await.unwrap(),
			["finished"],
			"and is due again afterwards"
		);

		// The operator creates the library; the next pass stages the download.
		seed_library(conn).await;
		refresh_grabs(&harness.services, &["finished".to_owned()])
			.await
			.unwrap();
		let staged = load_grab(conn, "finished").await;
		assert_eq!(staged.phase, "completed");
		assert_eq!(staged.error, None);
		let item_id = staged.ingest_item_id.expect("linked staged item");
		let item = ingest_drop_item::Entity::find_by_id(item_id)
			.one(conn)
			.await
			.unwrap()
			.expect("staged item");
		assert_eq!(item.source_filename, "book.epub");
		let request = book_request::Entity::find_by_id("request-1")
			.one(conn)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(
			item.created_by,
			Some(request.requester_id),
			"the item belongs to the requester (a user), not to the request id"
		);
		assert_eq!(
			item.idempotency_key.as_deref(),
			Some("mam-acquisition-finished")
		);
		assert!(
			due_grab_ids(conn, now + minutes(60), &[])
				.await
				.unwrap()
				.is_empty(),
			"a staged grab is finished work"
		);

		// The last allowed failure ends the grab visibly instead of forever.
		insert_grab(
			conn,
			GrabRow {
				handoff_attempts: HANDOFF_MAX_ATTEMPTS - 1,
				..grab("hopeless", "completed", minutes(6))
			},
		)
		.await;
		std::fs::remove_file(harness._root.path().join("downloads/book.epub")).unwrap();
		refresh_grabs(&harness.services, &["hopeless".to_owned()])
			.await
			.unwrap();
		let hopeless = load_grab(conn, "hopeless").await;
		assert_eq!(hopeless.phase, "error");
		assert_eq!(hopeless.handoff_attempts, HANDOFF_MAX_ATTEMPTS);
		assert_eq!(
			hopeless.error.as_deref(),
			Some(&*format!(
				"handoff failed after {HANDOFF_MAX_ATTEMPTS} attempts: MAM handoff source does not exist"
			))
		);
		assert!(
			due_grab_ids(conn, now + minutes(600), &[])
				.await
				.unwrap()
				.is_empty(),
			"an errored grab is never retried"
		);
	}
}
