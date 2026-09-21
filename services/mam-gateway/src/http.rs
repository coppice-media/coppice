use crate::{
	config::Secret,
	error::{GatewayError, GatewayResult},
	handoff::{self, HandoffInspection},
	mam::{MamClient, SearchQuery},
	qbit::{QbitClient, SubmitResult},
	store::{now_seconds, GrabReservation, GrabState},
	AppState,
};
use axum::{
	extract::{DefaultBodyLimit, Path, Query, State},
	http::{header, HeaderMap, HeaderValue, StatusCode},
	response::{IntoResponse, Response},
	routing::{get, post},
	Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::{collections::HashMap, sync::Arc};
use subtle::ConstantTimeEq;

const ALLOWED_TORZNAB_PARAMS: &[&str] = &["t", "q", "cat", "limit", "offset", "apikey"];
const ALLOWED_CATEGORIES: &[u32] = &[7000, 7010, 7020, 7030, 7040, 7050, 7060];

pub fn router(state: AppState) -> Router {
	Router::new()
		.route("/torznab/api", get(torznab_api))
		.route("/v1/health", get(health))
		.route("/v1/results/{id}", get(get_result))
		.route("/v1/grabs", post(create_grab))
		.route("/v1/grabs/{id}", get(get_grab))
		.layer(DefaultBodyLimit::max(state.config.max_grab_body_bytes()))
		.with_state(state)
}

async fn torznab_api(
	State(state): State<AppState>,
	headers: HeaderMap,
	Query(params): Query<HashMap<String, String>>,
) -> Response {
	match torznab_api_inner(&state, &headers, params).await {
		Ok((status, body)) => xml_response(status, body),
		Err(error) => torznab_error_response(error),
	}
}

async fn torznab_api_inner(
	state: &AppState,
	headers: &HeaderMap,
	params: HashMap<String, String>,
) -> GatewayResult<(StatusCode, String)> {
	if params
		.keys()
		.any(|key| !ALLOWED_TORZNAB_PARAMS.contains(&key.as_str()))
	{
		return Err(GatewayError::BadRequest);
	}
	if let Some(api_key) = params.get("apikey") {
		authenticate_value(api_key, &state.config.gateway_token)?;
		if headers.contains_key(header::AUTHORIZATION)
			|| headers.contains_key("x-api-key")
		{
			authenticate_headers(headers, &state.config.gateway_token)?;
		}
	} else {
		authenticate_headers(headers, &state.config.gateway_token)?;
	}
	let request_type = params.get("t").map(String::as_str).unwrap_or("search");
	match request_type {
		"caps" => Ok((StatusCode::OK, caps_xml())),
		"search" => {
			let query = parse_search_query(&params)?;
			let normalized = state.mam.search(&query).await?;
			let mut store = state.store.lock().await;
			let results =
				store.insert_results(normalized, state.config.result_ttl, now_seconds());
			Ok((StatusCode::OK, search_xml(&results)))
		},
		_ => Err(GatewayError::BadRequest),
	}
}

async fn health(State(_state): State<AppState>) -> Response {
	Json(json!({
		"status": "ok",
	}))
	.into_response()
}

async fn get_result(
	State(state): State<AppState>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> Response {
	if let Err(error) = authenticate_headers(&headers, &state.config.gateway_token) {
		return error.into_response();
	}
	let mut store = state.store.lock().await;
	match store.get_result(&id, now_seconds()) {
		Ok(result) => Json(result.public).into_response(),
		Err(error) => error.into_response(),
	}
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GrabRequest {
	result_id: String,
	idempotency_key: String,
}

async fn create_grab(
	State(state): State<AppState>,
	headers: HeaderMap,
	Json(input): Json<GrabRequest>,
) -> Response {
	if let Err(error) = authenticate_headers(&headers, &state.config.gateway_token) {
		return error.into_response();
	}
	if let Some(value) = headers.get("idempotency-key") {
		let header_key = match value.to_str() {
			Ok(value) => value,
			Err(_) => return GatewayError::BadRequest.into_response(),
		};
		if header_key != input.idempotency_key.as_str() {
			return GatewayError::BadRequest.into_response();
		}
	}
	let reservation = {
		let mut store = state.store.lock().await;
		store.reserve_grab(
			&input.result_id,
			&input.idempotency_key,
			state.config.result_ttl,
			now_seconds(),
		)
	};
	let reservation = match reservation {
		Ok(reservation) => reservation,
		Err(error) => return error.into_response(),
	};
	match reservation {
		GrabReservation::Existing(record) => {
			let status = if record.state == GrabState::Failed {
				StatusCode::BAD_GATEWAY
			} else {
				StatusCode::OK
			};
			(status, Json(record.public())).into_response()
		},
		GrabReservation::New { record, result } => {
			let grab_id = record.id.clone();
			let tag = record.tag.clone();
			let outcome =
				submit_grab(&state.mam, &state.qbit, &result.normalized, &tag).await;
			match outcome {
				Ok(SubmitResult::Submitted | SubmitResult::AlreadyPresent) => {
					let mut store = state.store.lock().await;
					match store.mark_downloading(&grab_id, 0.0, now_seconds()) {
						Ok(record) => {
							(StatusCode::CREATED, Json(record.public())).into_response()
						},
						Err(error) => error.into_response(),
					}
				},
				Err(error) => {
					let error_code = error.code();
					let mut store = state.store.lock().await;
					let _ = store.mark_failed(&grab_id, error_code, now_seconds());
					error.into_response()
				},
			}
		},
	}
}

async fn submit_grab(
	mam: &Arc<MamClient>,
	qbit: &Arc<QbitClient>,
	result: &crate::mam::NormalizedResult,
	tag: &str,
) -> GatewayResult<SubmitResult> {
	let torrent = mam.fetch_download(result).await?;
	qbit.submit(torrent, tag).await
}

async fn get_grab(
	State(state): State<AppState>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> Response {
	if let Err(error) = authenticate_headers(&headers, &state.config.gateway_token) {
		return error.into_response();
	}
	let record = {
		let mut store = state.store.lock().await;
		match store.get_grab(&id, now_seconds()) {
			Ok(record) => record,
			Err(error) => return error.into_response(),
		}
	};
	if matches!(record.state, GrabState::Completed | GrabState::Failed) {
		return Json(record.public()).into_response();
	}
	let snapshot = match state.qbit.find_by_tag(&record.tag).await {
		Ok(snapshot) => snapshot,
		Err(error) => return error.into_response(),
	};
	let updated = match snapshot {
		Some(snapshot) if snapshot.failed => {
			let mut store = state.store.lock().await;
			store.mark_failed(&id, "download_client_failed", now_seconds())
		},
		Some(snapshot) if snapshot.completed => {
			let progress = snapshot.progress;
			let inspection = handoff::inspect(
				state.config.handoff_root.clone(),
				snapshot.content_path,
				state.config.max_handoff_bytes,
			)
			.await;
			let mut store = state.store.lock().await;
			match inspection {
				HandoffInspection::Ready(handoff) => {
					store.mark_completed(&id, handoff, now_seconds())
				},
				HandoffInspection::NotReady => {
					store.mark_downloading(&id, progress, now_seconds())
				},
				HandoffInspection::Invalid(code) => {
					store.mark_failed(&id, code, now_seconds())
				},
			}
		},
		Some(snapshot) => {
			let mut store = state.store.lock().await;
			store.mark_downloading(&id, snapshot.progress, now_seconds())
		},
		None => {
			let mut store = state.store.lock().await;
			store.get_grab(&id, now_seconds())
		},
	};
	match updated {
		Ok(record) => Json(record.public()).into_response(),
		Err(error) => error.into_response(),
	}
}

fn parse_search_query(params: &HashMap<String, String>) -> GatewayResult<SearchQuery> {
	let query = params.get("q").ok_or(GatewayError::BadRequest)?;
	if query.trim().is_empty() || query.len() > 256 || query.chars().any(char::is_control)
	{
		return Err(GatewayError::BadRequest);
	}
	let categories = match params.get("cat") {
		Some(value) => value
			.split(',')
			.map(|part| part.parse::<u32>().map_err(|_| GatewayError::BadRequest))
			.collect::<GatewayResult<Vec<_>>>()?,
		None => Vec::new(),
	};
	if categories
		.iter()
		.any(|category| !ALLOWED_CATEGORIES.contains(category))
	{
		return Err(GatewayError::BadRequest);
	}
	let offset = parse_bounded_usize(params.get("offset"), 0, 10_000)?;
	let limit = parse_bounded_usize(params.get("limit"), 50, 1..=100)?;
	Ok(SearchQuery {
		query: query.trim().to_owned(),
		categories,
		offset,
		limit,
	})
}

fn parse_bounded_usize(
	value: Option<&String>,
	default: usize,
	bounds: impl Into<Bounds>,
) -> GatewayResult<usize> {
	let bounds = bounds.into();
	let value = match value {
		Some(value) => value
			.parse::<usize>()
			.map_err(|_| GatewayError::BadRequest)?,
		None => default,
	};
	if !bounds.contains(value) {
		return Err(GatewayError::BadRequest);
	}
	Ok(value)
}

struct Bounds {
	min: usize,
	max: usize,
}

impl Bounds {
	fn contains(&self, value: usize) -> bool {
		(self.min..=self.max).contains(&value)
	}
}

impl From<usize> for Bounds {
	fn from(value: usize) -> Self {
		Self { min: 0, max: value }
	}
}

impl From<std::ops::RangeInclusive<usize>> for Bounds {
	fn from(range: std::ops::RangeInclusive<usize>) -> Self {
		Self {
			min: *range.start(),
			max: *range.end(),
		}
	}
}

fn authenticate_headers(headers: &HeaderMap, expected: &Secret) -> GatewayResult<()> {
	if let Some(value) = headers
		.get(header::AUTHORIZATION)
		.and_then(|value| value.to_str().ok())
	{
		if let Some(token) = value.strip_prefix("Bearer ") {
			authenticate_value(token, expected)?;
			return Ok(());
		}
	}
	if let Some(value) = headers
		.get("x-api-key")
		.and_then(|value| value.to_str().ok())
	{
		authenticate_value(value, expected)?;
		return Ok(());
	}
	Err(GatewayError::Unauthorized)
}

fn authenticate_value(value: &str, expected: &Secret) -> GatewayResult<()> {
	if bool::from(value.as_bytes().ct_eq(expected.expose().as_bytes())) {
		Ok(())
	} else {
		Err(GatewayError::Unauthorized)
	}
}

fn xml_response(status: StatusCode, body: String) -> Response {
	(
		status,
		[(
			header::CONTENT_TYPE,
			HeaderValue::from_static("application/xml; charset=utf-8"),
		)],
		body,
	)
		.into_response()
}

fn torznab_error_response(error: GatewayError) -> Response {
	let status = error.status();
	let code = match &error {
		GatewayError::Unauthorized | GatewayError::UpstreamUnauthorized => "100",
		GatewayError::BadRequest => "200",
		GatewayError::Timeout => "503",
		GatewayError::RedirectRejected | GatewayError::UnsafeUrl => "502",
		GatewayError::BodyLimit | GatewayError::PayloadTooLarge => "413",
		_ => "500",
	};
	xml_response(
		status,
		format!(
			"<?xml version=\"1.0\" encoding=\"UTF-8\"?><error code=\"{code}\" description=\"{}\" />",
			escape_xml(error.public_message())
		),
	)
}

fn caps_xml() -> String {
	"<?xml version=\"1.0\" encoding=\"UTF-8\"?><caps><server title=\"Coppice MAM gateway\"/>\
<limits max=\"100\" default=\"50\"/><searching><search available=\"yes\" supportedParams=\"q,cat\"/><book available=\"yes\" supportedParams=\"q,cat\"/></searching><categories><category id=\"7000\" name=\"Books\"/><category id=\"7010\" name=\"Magazines\"/><category id=\"7020\" name=\"Comics\"/><category id=\"7030\" name=\"Audio books\"/></categories></caps>".to_owned()
}

fn search_xml(results: &[crate::mam::PublicResult]) -> String {
	let mut xml = String::from(
		"<?xml version=\"1.0\" encoding=\"UTF-8\"?><rss version=\"2.0\" xmlns:torznab=\"http://torznab.com/schemas/2015/feed\"><channel><title>Coppice MAM gateway</title>",
	);
	for result in results {
		xml.push_str("<item><title>");
		xml.push_str(&escape_xml(&result.title));
		xml.push_str("</title><guid isPermaLink=\"false\">");
		xml.push_str(&escape_xml(&result.result_id));
		xml.push_str("</guid><link>urn:coppice:mam-result:");
		xml.push_str(&escape_xml(&result.result_id));
		xml.push_str("</link>");
		if !result.authors.is_empty() {
			xml.push_str("<author>");
			xml.push_str(&escape_xml(&result.authors.join(", ")));
			xml.push_str("</author>");
		}
		if let Some(date) = &result.published_at {
			xml.push_str("<pubDate>");
			xml.push_str(&escape_xml(date));
			xml.push_str("</pubDate>");
		}
		xml.push_str("<size>");
		xml.push_str(&result.size_bytes.to_string());
		xml.push_str("</size><torznab:attr name=\"seeders\" value=\"");
		xml.push_str(&result.seeders.to_string());
		xml.push_str("\"/><torznab:attr name=\"leechers\" value=\"");
		xml.push_str(&result.leechers.to_string());
		xml.push_str("\"/><torznab:attr name=\"category\" value=\"");
		xml.push_str(&result.category.to_string());
		xml.push_str("\"/><torznab:attr name=\"coppice-result-id\" value=\"");
		xml.push_str(&escape_xml(&result.result_id));
		xml.push_str("\"/></item>");
	}
	xml.push_str("</channel></rss>");
	xml
}

fn escape_xml(value: &str) -> String {
	let mut escaped = String::with_capacity(value.len());
	for character in value.chars() {
		match character {
			'&' => escaped.push_str("&amp;"),
			'<' => escaped.push_str("&lt;"),
			'>' => escaped.push_str("&gt;"),
			'\'' => escaped.push_str("&apos;"),
			'"' => escaped.push_str("&quot;"),
			character => escaped.push(character),
		}
	}
	escaped
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn torznab_output_contains_opaque_ids_but_no_tracker_url() {
		let result = crate::mam::PublicResult {
			result_id: "0123456789abcdef0123456789abcdef".to_owned(),
			title: "Book & <Edition>".to_owned(),
			authors: vec![],
			category: 7000,
			size_bytes: 123,
			seeders: 2,
			leechers: 1,
			published_at: None,
			info_hash: None,
		};
		let xml = search_xml(&[result]);
		assert!(xml.contains("coppice-result-id"));
		assert!(xml.contains("Book &amp; &lt;Edition&gt;"));
		assert!(!xml.contains("https://t.myanonamouse.net"));
		assert!(!xml.contains("download"));
	}

	#[test]
	fn unknown_torznab_parameters_are_rejected() {
		let mut params = HashMap::new();
		params.insert("apikey".to_owned(), "token".to_owned());
		params.insert("url".to_owned(), "http://169.254.169.254".to_owned());
		assert!(params
			.keys()
			.any(|key| !ALLOWED_TORZNAB_PARAMS.contains(&key.as_str())));
	}
}
