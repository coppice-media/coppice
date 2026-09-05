//! The [`Source`] contract mirrors Mihon's `CatalogueSource`/`HttpSource`:
//! browse (popular/latest/search), series details, chapter list, page list,
//! and page bytes. Everything a source returns is a plain remote description;
//! the host owns persistence, caching, and rate limiting policy.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::http::SourceHttp;

/// Identity and advertised capabilities of one source instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceInfo {
	/// Stable instance identifier, e.g. `mangadex-en`. Used as the
	/// `source_provider` column and in virtual media paths.
	pub id: String,
	pub name: String,
	/// BCP-47 language of the chapters this instance yields (`all` when
	/// the source is not language scoped).
	pub lang: String,
	pub base_url: String,
	pub capabilities: SourceCapabilities,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SourceCapabilities {
	pub popular: bool,
	pub latest: bool,
	pub search: bool,
}

/// One page of browse/search results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePage<T> {
	pub items: Vec<T>,
	pub has_next: bool,
}

impl<T> Default for SourcePage<T> {
	fn default() -> Self {
		Self {
			items: Vec::new(),
			has_next: false,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SeriesStatus {
	#[default]
	Unknown,
	Ongoing,
	Completed,
	Hiatus,
	Cancelled,
}

impl SeriesStatus {
	/// The Komga/Stump `series_metadata.status` projection.
	pub fn as_metadata_status(self) -> &'static str {
		match self {
			SeriesStatus::Ongoing | SeriesStatus::Hiatus | SeriesStatus::Unknown => {
				"Continuing"
			},
			SeriesStatus::Completed | SeriesStatus::Cancelled => "Ended",
		}
	}
}

/// A series as described by a remote source.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RemoteSeries {
	pub remote_id: String,
	pub title: String,
	pub url: Option<String>,
	pub thumbnail_url: Option<String>,
	pub description: Option<String>,
	pub authors: Vec<String>,
	pub artists: Vec<String>,
	pub genres: Vec<String>,
	pub status: SeriesStatus,
	pub nsfw: bool,
	pub original_language: Option<String>,
}

/// A chapter as described by a remote source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteChapter {
	pub remote_id: String,
	pub title: Option<String>,
	pub number: Option<f32>,
	pub volume: Option<String>,
	pub lang: Option<String>,
	pub scanlator: Option<String>,
	pub uploaded_at: Option<DateTime<Utc>>,
	pub url: Option<String>,
	/// Known page count, when the source reports it without a page fetch.
	pub page_count: Option<u32>,
}

impl RemoteChapter {
	/// Human readable chapter name used for the `media.name` column, e.g.
	/// `Vol. 2 Ch. 12 - Title`.
	pub fn display_name(&self) -> String {
		let mut name = String::new();
		if let Some(volume) = self.volume.as_deref().filter(|v| !v.is_empty()) {
			name.push_str("Vol. ");
			name.push_str(volume);
			name.push(' ');
		}
		match self.number {
			Some(number) if number.fract() == 0.0 => {
				name.push_str(&format!("Ch. {}", number as i64));
			},
			Some(number) => name.push_str(&format!("Ch. {number}")),
			None => {},
		}
		if let Some(title) = self.title.as_deref().filter(|t| !t.trim().is_empty()) {
			if !name.is_empty() {
				name.push_str(" - ");
			}
			name.push_str(title.trim());
		}
		if name.is_empty() {
			name.push_str(&self.remote_id);
		}
		name
	}
}

/// A page of a chapter: where to fetch it and which headers the host must send.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotePage {
	/// Zero-based page index.
	pub index: u32,
	pub url: String,
	#[serde(default)]
	pub headers: Vec<(String, String)>,
}

impl RemotePage {
	pub fn new(index: u32, url: impl Into<String>) -> Self {
		Self {
			index,
			url: url.into(),
			headers: Vec::new(),
		}
	}
}

/// Fetched page bytes plus the content type the origin reported, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedPage {
	pub bytes: Vec<u8>,
	pub content_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchFilter {
	pub key: String,
	pub value: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
	#[error("HTTP request failed: {0}")]
	Http(#[from] reqwest::Error),
	#[error("Unexpected HTTP status {status} from {url}")]
	Status { status: u16, url: String },
	#[error("Rate limited by the remote source")]
	RateLimited { retry_after: Option<Duration> },
	#[error("Failed to decode a source response: {0}")]
	Decode(String),
	#[error("Remote resource not found: {0}")]
	NotFound(String),
	#[error("Source `{source_id}` does not support {operation}")]
	Unsupported {
		source_id: String,
		operation: &'static str,
	},
	#[error("{0}")]
	Other(String),
}

impl SourceError {
	pub fn decode(error: impl std::fmt::Display) -> Self {
		SourceError::Decode(error.to_string())
	}
}

pub type SourceResult<T> = Result<T, SourceError>;

/// A remote manga/comic source. Implementations are cheap to clone behind an
/// `Arc` and must be safe to share across tasks.
#[async_trait]
pub trait Source: Send + Sync {
	fn info(&self) -> &SourceInfo;

	/// The shared HTTP client and limiter used by the default page/image
	/// fetchers. Implementations that fetch images from a different host may
	/// override [`Source::fetch_page`] and [`Source::fetch_image`] instead.
	fn http(&self) -> &SourceHttp;

	async fn popular(&self, page: u32) -> SourceResult<SourcePage<RemoteSeries>>;

	async fn latest(&self, page: u32) -> SourceResult<SourcePage<RemoteSeries>>;

	async fn search(
		&self,
		query: &str,
		filters: &[SearchFilter],
		page: u32,
	) -> SourceResult<SourcePage<RemoteSeries>>;

	async fn details(&self, remote_id: &str) -> SourceResult<RemoteSeries>;

	async fn chapters(&self, remote_id: &str) -> SourceResult<Vec<RemoteChapter>>;

	async fn pages(&self, chapter_id: &str) -> SourceResult<Vec<RemotePage>>;

	/// Download one page. The default honours the page's headers and the
	/// source limiter.
	async fn fetch_page(&self, page: &RemotePage) -> SourceResult<FetchedPage> {
		self.http().fetch_bytes(&page.url, &page.headers).await
	}

	/// Download an arbitrary image belonging to the source (covers).
	async fn fetch_image(&self, url: &str) -> SourceResult<FetchedPage> {
		self.http().fetch_bytes(url, &[]).await
	}
}

impl SourceHttp {
	/// Rate-limited GET returning the response bytes and reported content type.
	pub async fn fetch_bytes(
		&self,
		url: &str,
		headers: &[(String, String)],
	) -> SourceResult<FetchedPage> {
		self.limiter().until_ready().await;
		let mut request = self.client().get(url);
		for (name, value) in headers {
			request = request.header(name.as_str(), value.as_str());
		}
		let response = request.send().await?;
		let status = response.status();
		if status.as_u16() == 429 {
			return Err(SourceError::RateLimited {
				retry_after: retry_after(response.headers()),
			});
		}
		if status.as_u16() == 404 {
			return Err(SourceError::NotFound(url.to_string()));
		}
		if !status.is_success() {
			return Err(SourceError::Status {
				status: status.as_u16(),
				url: url.to_string(),
			});
		}
		let content_type = response
			.headers()
			.get(reqwest::header::CONTENT_TYPE)
			.and_then(|value| value.to_str().ok())
			.map(|value| value.split(';').next().unwrap_or(value).trim().to_string());
		let bytes = response.bytes().await?.to_vec();
		Ok(FetchedPage {
			bytes,
			content_type,
		})
	}

	/// Rate-limited GET decoding a JSON body.
	pub async fn get_json<T: serde::de::DeserializeOwned>(
		&self,
		url: &str,
		query: &[(&str, String)],
	) -> SourceResult<T> {
		self.limiter().until_ready().await;
		let response = self.client().get(url).query(query).send().await?;
		let status = response.status();
		if status.as_u16() == 429 {
			return Err(SourceError::RateLimited {
				retry_after: retry_after(response.headers()),
			});
		}
		if status.as_u16() == 404 {
			return Err(SourceError::NotFound(url.to_string()));
		}
		if !status.is_success() {
			return Err(SourceError::Status {
				status: status.as_u16(),
				url: url.to_string(),
			});
		}
		let body = response.bytes().await?;
		serde_json::from_slice(&body).map_err(SourceError::decode)
	}
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
	headers
		.get(reqwest::header::RETRY_AFTER)
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.trim().parse::<u64>().ok())
		.map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn chapter_display_name_combines_volume_number_and_title() {
		let chapter = RemoteChapter {
			remote_id: "c1".into(),
			title: Some("The Beginning".into()),
			number: Some(12.0),
			volume: Some("2".into()),
			lang: None,
			scanlator: None,
			uploaded_at: None,
			url: None,
			page_count: None,
		};
		assert_eq!(chapter.display_name(), "Vol. 2 Ch. 12 - The Beginning");

		let decimal = RemoteChapter {
			number: Some(12.5),
			volume: None,
			title: None,
			..chapter.clone()
		};
		assert_eq!(decimal.display_name(), "Ch. 12.5");

		let bare = RemoteChapter {
			number: None,
			volume: None,
			title: None,
			..chapter
		};
		assert_eq!(bare.display_name(), "c1");
	}

	#[test]
	fn series_status_projects_to_metadata_status() {
		assert_eq!(SeriesStatus::Ongoing.as_metadata_status(), "Continuing");
		assert_eq!(SeriesStatus::Completed.as_metadata_status(), "Ended");
	}
}
