use std::pin::Pin;

use axum::{http::StatusCode, response::IntoResponse, Json};
use chrono::{DateTime, Utc};
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type KomfResult<T> = Result<T, KomfError>;
pub type KomfEventStream = Pin<Box<dyn Stream<Item = KomfEvent> + Send + 'static>>;

#[derive(Debug, thiserror::Error)]
pub enum KomfError {
	#[error("{0}")]
	BadRequest(String),
	#[error("{0}")]
	Forbidden(String),
	#[error("{0}")]
	NotFound(String),
	#[error("{0}")]
	Unprocessable(String),
	#[error("{0}")]
	Internal(String),
}

impl IntoResponse for KomfError {
	fn into_response(self) -> axum::response::Response {
		let status = match self {
			Self::BadRequest(_) => StatusCode::BAD_REQUEST,
			Self::Forbidden(_) => StatusCode::FORBIDDEN,
			Self::NotFound(_) => StatusCode::NOT_FOUND,
			Self::Unprocessable(_) => StatusCode::UNPROCESSABLE_ENTITY,
			Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
		};
		let body = serde_json::json!({ "message": self.to_string() });
		(status, Json(body)).into_response()
	}
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KomfSeriesSearchRequest {
	pub name: Option<String>,
	pub library_id: Option<String>,
	pub series_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KomfIdentifyRequest {
	pub series_id: String,
	pub library_id: Option<String>,
	pub provider: String,
	pub provider_series_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomfSearchResult {
	pub url: String,
	pub image_url: Option<String>,
	pub title: String,
	pub provider: String,
	pub result_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomfMetadataJobResponse {
	pub job_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomfMediaServerConnectionResponse {
	pub success: bool,
	pub http_status_code: Option<u16>,
	pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomfMediaServerLibrary {
	pub id: String,
	pub name: String,
	pub roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomfJob {
	pub series_id: String,
	pub id: String,
	pub status: String,
	pub message: String,
	pub started_at: DateTime<Utc>,
	pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomfJobPage {
	pub content: Vec<KomfJob>,
	pub total_pages: i32,
	pub current_page: i32,
}

#[derive(Debug, Clone)]
pub struct KomfEvent {
	pub name: String,
	pub data: Option<Value>,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn captured_komf_response_fixtures_match_public_dtos() {
		let fixtures: Value =
			serde_json::from_str(include_str!("../tests/fixtures/komf-responses.json"))
				.unwrap();
		let providers: Vec<String> =
			serde_json::from_value(fixtures["providers"].clone()).unwrap();
		assert_eq!(providers, ["MAL"]);

		let metadata_job: KomfMetadataJobResponse =
			serde_json::from_value(fixtures["metadataJobResponse"].clone()).unwrap();
		assert_eq!(
			serde_json::to_value(metadata_job).unwrap(),
			fixtures["metadataJobResponse"]
		);

		let search: Vec<KomfSearchResult> =
			serde_json::from_value(fixtures["searchResults"].clone()).unwrap();
		assert_eq!(search[0].provider, "MANGADEX");
		assert_eq!(
			serde_json::to_value(search).unwrap(),
			fixtures["searchResults"]
		);

		let connection: KomfMediaServerConnectionResponse =
			serde_json::from_value(fixtures["mediaServerConnection"].clone()).unwrap();
		assert!(connection.success);
		assert_eq!(connection.http_status_code, Some(200));
		assert_eq!(
			serde_json::to_value(connection).unwrap(),
			fixtures["mediaServerConnection"]
		);

		let libraries: Vec<KomfMediaServerLibrary> =
			serde_json::from_value(fixtures["mediaServerLibraries"].clone()).unwrap();
		assert_eq!(libraries[0].roots, ["/data/library"]);
		assert_eq!(
			serde_json::to_value(libraries).unwrap(),
			fixtures["mediaServerLibraries"]
		);

		let jobs: KomfJobPage = serde_json::from_value(fixtures["jobs"].clone()).unwrap();
		assert_eq!(jobs.content[0].status, "FAILED");
		assert_eq!(jobs.current_page, 0);
		assert_eq!(serde_json::to_value(jobs).unwrap(), fixtures["jobs"]);
	}
}
