use std::{convert::Infallible, sync::Arc};

use async_trait::async_trait;
use axum::{
	body::Bytes,
	extract::{Extension, Path, Query},
	http::{header, StatusCode},
	response::{IntoResponse, Response, Sse},
	routing::{delete, get, post},
	Json, Router,
};
use futures_util::{stream, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use stump_auth::AuthContext;

use crate::types::{
	KomfError, KomfEvent, KomfEventStream, KomfIdentifyRequest, KomfJob, KomfJobPage,
	KomfMediaServerLibrary, KomfMetadataJobResponse, KomfResult, KomfSearchResult,
	KomfSeriesSearchRequest,
};

const JOB_COMPLETED_EVENT: &str = "__KomfJobCompleted";
const JOB_NOT_FOUND_EVENT: &str = "EventStreamNotFoundEvent";

#[async_trait]
pub trait KomfBackend: Send + Sync + 'static {
	async fn providers(
		&self,
		auth: &AuthContext,
		library_id: Option<&str>,
	) -> KomfResult<Vec<String>>;

	async fn search_series(
		&self,
		auth: &AuthContext,
		request: KomfSeriesSearchRequest,
	) -> KomfResult<Vec<KomfSearchResult>>;

	async fn series_cover(
		&self,
		auth: &AuthContext,
		library_id: &str,
		provider: &str,
		provider_series_id: &str,
	) -> KomfResult<Option<(String, Bytes)>>;

	async fn identify_series(
		&self,
		auth: &AuthContext,
		request: KomfIdentifyRequest,
	) -> KomfResult<KomfMetadataJobResponse>;

	async fn match_series(
		&self,
		auth: &AuthContext,
		library_id: &str,
		series_id: &str,
	) -> KomfResult<KomfMetadataJobResponse>;

	async fn match_library(&self, auth: &AuthContext, library_id: &str)
		-> KomfResult<()>;

	async fn reset_series(
		&self,
		auth: &AuthContext,
		library_id: &str,
		series_id: &str,
		remove_comic_info: bool,
	) -> KomfResult<()>;

	async fn reset_library(
		&self,
		auth: &AuthContext,
		library_id: &str,
		remove_comic_info: bool,
	) -> KomfResult<()>;

	async fn config(&self, auth: &AuthContext) -> KomfResult<Value>;

	async fn update_config(&self, auth: &AuthContext, patch: Value) -> KomfResult<()>;

	async fn jobs(
		&self,
		auth: &AuthContext,
		status: Option<&str>,
		page: i64,
		page_size: i64,
	) -> KomfResult<KomfJobPage>;

	async fn job(&self, auth: &AuthContext, job_id: &str) -> KomfResult<Option<KomfJob>>;

	async fn delete_all_jobs(&self, auth: &AuthContext) -> KomfResult<()>;

	fn job_events(&self, auth: &AuthContext, job_id: &str) -> Option<KomfEventStream>;

	async fn libraries(
		&self,
		auth: &AuthContext,
	) -> KomfResult<Vec<KomfMediaServerLibrary>>;
}

/// Komelia's Komf client calls shared config/job endpoints at the origin root;
/// the server-specific metadata and media operations live under `/api/komga`.
pub fn router<S>(backend: Arc<dyn KomfBackend>) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::new()
		.route("/api/komga/metadata/providers", get(get_providers))
		.route("/api/config", get(get_config).patch(update_config))
		.route("/api/jobs", get(get_jobs))
		.route("/api/jobs/all", delete(delete_all_jobs))
		.route("/api/jobs/{job_id}/events", get(job_events))
		.route("/api/jobs/{job_id}", get(get_job))
		.route("/api/komga/metadata/search", get(search_series))
		.route("/api/komga/metadata/series-cover", get(series_cover))
		.route("/api/komga/metadata/identify", post(identify_series))
		.route(
			"/api/komga/metadata/match/library/{library_id}/series/{series_id}",
			post(match_series),
		)
		.route(
			"/api/komga/metadata/match/library/{library_id}",
			post(match_library),
		)
		.route(
			"/api/komga/metadata/reset/library/{library_id}/series/{series_id}",
			post(reset_series),
		)
		.route(
			"/api/komga/metadata/reset/library/{library_id}",
			post(reset_library),
		)
		.route(
			"/api/komga/media-server/connected",
			get(media_server_connected),
		)
		.route(
			"/api/komga/media-server/libraries",
			get(media_server_libraries),
		)
		.layer(Extension(backend))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderListQuery {
	library_id: Option<String>,
}

async fn get_providers(
	Extension(backend): Extension<Arc<dyn KomfBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ProviderListQuery>,
) -> Result<Json<Vec<String>>, KomfError> {
	backend
		.providers(&auth, query.library_id.as_deref())
		.await
		.map(Json)
}

async fn get_config(
	Extension(backend): Extension<Arc<dyn KomfBackend>>,
	Extension(auth): Extension<AuthContext>,
) -> Result<Json<Value>, KomfError> {
	backend.config(&auth).await.map(Json)
}

async fn update_config(
	Extension(backend): Extension<Arc<dyn KomfBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(patch): Json<Value>,
) -> Result<StatusCode, KomfError> {
	backend.update_config(&auth, patch).await?;
	Ok(StatusCode::NO_CONTENT)
}

async fn search_series(
	Extension(backend): Extension<Arc<dyn KomfBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(request): Query<KomfSeriesSearchRequest>,
) -> Response {
	if request.name.as_deref().is_none_or(str::is_empty) {
		return StatusCode::BAD_REQUEST.into_response();
	}
	match backend.search_series(&auth, request).await {
		Ok(results) => Json(results).into_response(),
		Err(error) => error.into_response(),
	}
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeriesCoverQuery {
	library_id: Option<String>,
	provider: Option<String>,
	provider_series_id: Option<String>,
}

async fn series_cover(
	Extension(backend): Extension<Arc<dyn KomfBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesCoverQuery>,
) -> Response {
	let (Some(library_id), Some(provider), Some(provider_series_id)) = (
		query.library_id.as_deref(),
		query.provider.as_deref(),
		query.provider_series_id.as_deref(),
	) else {
		return StatusCode::BAD_REQUEST.into_response();
	};
	match backend
		.series_cover(&auth, library_id, provider, provider_series_id)
		.await
	{
		Ok(Some((content_type, bytes))) => {
			let mut response = bytes.into_response();
			if let Ok(value) = content_type.parse() {
				response.headers_mut().insert(header::CONTENT_TYPE, value);
			}
			response
		},
		Ok(None) => StatusCode::NOT_FOUND.into_response(),
		Err(error) => error.into_response(),
	}
}

async fn identify_series(
	Extension(backend): Extension<Arc<dyn KomfBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(request): Json<KomfIdentifyRequest>,
) -> Result<Json<KomfMetadataJobResponse>, KomfError> {
	backend.identify_series(&auth, request).await.map(Json)
}

async fn match_series(
	Extension(backend): Extension<Arc<dyn KomfBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path((library_id, series_id)): Path<(String, String)>,
) -> Result<Json<KomfMetadataJobResponse>, KomfError> {
	backend
		.match_series(&auth, &library_id, &series_id)
		.await
		.map(Json)
}

async fn match_library(
	Extension(backend): Extension<Arc<dyn KomfBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(library_id): Path<String>,
) -> Result<StatusCode, KomfError> {
	backend.match_library(&auth, &library_id).await?;
	Ok(StatusCode::ACCEPTED)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResetQuery {
	remove_comic_info: Option<bool>,
}

async fn reset_series(
	Extension(backend): Extension<Arc<dyn KomfBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path((library_id, series_id)): Path<(String, String)>,
	Query(query): Query<ResetQuery>,
) -> Result<StatusCode, KomfError> {
	backend
		.reset_series(
			&auth,
			&library_id,
			&series_id,
			query.remove_comic_info.unwrap_or(false),
		)
		.await?;
	Ok(StatusCode::NO_CONTENT)
}

async fn reset_library(
	Extension(backend): Extension<Arc<dyn KomfBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(library_id): Path<String>,
	Query(query): Query<ResetQuery>,
) -> Result<StatusCode, KomfError> {
	backend
		.reset_library(&auth, &library_id, query.remove_comic_info.unwrap_or(false))
		.await?;
	Ok(StatusCode::NO_CONTENT)
}

async fn media_server_connected(
	Extension(_auth): Extension<AuthContext>,
) -> Json<crate::KomfMediaServerConnectionResponse> {
	Json(crate::KomfMediaServerConnectionResponse {
		success: true,
		http_status_code: Some(StatusCode::OK.as_u16()),
		error_message: None,
	})
}

async fn media_server_libraries(
	Extension(backend): Extension<Arc<dyn KomfBackend>>,
	Extension(auth): Extension<AuthContext>,
) -> Result<Json<Vec<KomfMediaServerLibrary>>, KomfError> {
	backend.libraries(&auth).await.map(Json)
}

#[derive(Debug, Deserialize)]
struct JobsQuery {
	status: Option<String>,
	page: Option<String>,
	#[serde(rename = "pageSize")]
	page_size: Option<String>,
}

async fn get_jobs(
	Extension(backend): Extension<Arc<dyn KomfBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<JobsQuery>,
) -> Response {
	if query
		.status
		.as_deref()
		.is_some_and(|status| !matches!(status, "RUNNING" | "FAILED" | "COMPLETED"))
	{
		return StatusCode::BAD_REQUEST.into_response();
	}
	let page = match query.page.as_deref().map(str::parse).transpose() {
		Ok(page) => page.unwrap_or(0),
		Err(_) => return StatusCode::BAD_REQUEST.into_response(),
	};
	let page_size = match query.page_size.as_deref().map(str::parse).transpose() {
		Ok(page_size) => page_size.unwrap_or(1000),
		Err(_) => return StatusCode::BAD_REQUEST.into_response(),
	};
	if page_size <= 0 {
		return StatusCode::BAD_REQUEST.into_response();
	}
	match backend
		.jobs(&auth, query.status.as_deref(), page, page_size)
		.await
	{
		Ok(page) => Json(page).into_response(),
		Err(error) => error.into_response(),
	}
}

async fn get_job(
	Extension(backend): Extension<Arc<dyn KomfBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(job_id): Path<String>,
) -> Response {
	match backend.job(&auth, &job_id).await {
		Ok(Some(job)) => Json(job).into_response(),
		Ok(None) => StatusCode::NOT_FOUND.into_response(),
		Err(error) => error.into_response(),
	}
}

async fn delete_all_jobs(
	Extension(backend): Extension<Arc<dyn KomfBackend>>,
	Extension(auth): Extension<AuthContext>,
) -> Result<StatusCode, KomfError> {
	backend.delete_all_jobs(&auth).await?;
	Ok(StatusCode::NO_CONTENT)
}

async fn job_events(
	Extension(backend): Extension<Arc<dyn KomfBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(job_id): Path<String>,
) -> Sse<impl futures_util::Stream<Item = Result<axum::response::sse::Event, Infallible>>>
{
	let events = match backend.job_events(&auth, &job_id) {
		Some(events) => events
			.filter_map(|event: KomfEvent| async move {
				if event.name == JOB_COMPLETED_EVENT {
					return None;
				}
				let data = event
					.data
					.map(|value| value.to_string())
					.unwrap_or_default();
				Some(Ok(axum::response::sse::Event::default()
					.event(event.name)
					.data(data)))
			})
			.left_stream(),
		None => stream::once(async {
			Ok(axum::response::sse::Event::default()
				.event(JOB_NOT_FOUND_EVENT)
				.data(""))
		})
		.right_stream(),
	};
	Sse::new(events)
}
