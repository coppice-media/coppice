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

/// How a source rates a series' content.
///
/// The variants are MangaDex's `contentRating` vocabulary
/// (`MangaAttributes.contentRating`, enum `safe | suggestive | erotica |
/// pornographic` in <https://api.mangadex.org/docs/static/api.yaml>), which
/// every Mihon-shaped source can be projected onto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContentRating {
	Safe,
	Suggestive,
	Erotica,
	Pornographic,
}

impl ContentRating {
	/// The `series_metadata.age_rating` projection Stump's per-user age
	/// restriction compares against. `Safe` deliberately stores no rating so
	/// an unrestricted title stays visible to users whose restriction does
	/// not `restrict_on_unset`.
	///
	/// | `contentRating` | `age_rating` |
	/// | --- | --- |
	/// | `safe` | `None` |
	/// | `suggestive` | `13` |
	/// | `erotica` | `16` |
	/// | `pornographic` | `18` |
	pub fn age_rating(self) -> Option<i32> {
		match self {
			ContentRating::Safe => None,
			ContentRating::Suggestive => Some(13),
			ContentRating::Erotica => Some(16),
			ContentRating::Pornographic => Some(18),
		}
	}

	/// Whether the rating alone marks the series adult, i.e. what the legacy
	/// [`RemoteSeries::nsfw`] flag reports.
	pub fn is_adult(self) -> bool {
		matches!(self, ContentRating::Erotica | ContentRating::Pornographic)
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
	/// The source's own content rating, when it reports one. Materialisation
	/// projects it onto `series_metadata.age_rating`; see
	/// [`ContentRating::age_rating`].
	#[serde(default)]
	pub content_rating: Option<ContentRating>,
	/// Cross-source ids the source reports for this work, keyed by registry
	/// (`al`, `mal`, `mu`, ...). Used to dedupe the same work across sources;
	/// see [`crate::identity`].
	#[serde(default)]
	pub external_ids: std::collections::BTreeMap<String, String>,
}

/// A chapter as described by a remote source.
///
/// `readable` is the source's verdict on whether the chapter's pages can be
/// fetched at all. Materialisation skips unreadable chapters instead of
/// writing rows whose pages can only ever 404
/// ([`crate::materialize::add_series`]).
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
	/// Whether the source can serve this chapter's pages. `false` for
	/// chapters hosted elsewhere, chapters with no readable images, and
	/// chapters the source marks unavailable.
	#[serde(default = "readable_default")]
	pub readable: bool,
	/// Where the chapter actually lives when it is not hosted by the source
	/// (MangaDex `ChapterAttributes.externalUrl`).
	#[serde(default)]
	pub external_url: Option<String>,
}

fn readable_default() -> bool {
	true
}

/// A chapter is readable unless a source says otherwise, so that sources
/// which cannot tell are not silently skipped by materialisation.
impl Default for RemoteChapter {
	fn default() -> Self {
		Self {
			remote_id: String::new(),
			title: None,
			number: None,
			volume: None,
			lang: None,
			scanlator: None,
			uploaded_at: None,
			url: None,
			page_count: None,
			readable: true,
			external_url: None,
		}
	}
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

/// A fetched HTML document: the decoded body plus the URL it came from after
/// redirects, so relative links and `Referer` headers resolve correctly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlDocument {
	pub body: String,
	pub url: String,
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
	/// The host answered with a Cloudflare managed challenge instead of the
	/// page: it is up, but it will not talk to anything that cannot run the
	/// challenge script until the request carries a browser's `cf_clearance`
	/// cookie (`provider_sources.request_headers`).
	#[error("`{host}` is behind a Cloudflare challenge; configure a `cf_clearance` cookie and matching User-Agent for the source")]
	Challenged { host: String },
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
		let mut request = self.headers().apply(self.client().get(url));
		for (name, value) in headers {
			request = request.header(name.as_str(), value.as_str());
		}
		let response = request.send().await?;
		if let Some(error) = classify(&response, url) {
			return Err(error);
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
		let response = self
			.headers()
			.apply(self.client().get(url))
			.query(query)
			.send()
			.await?;
		if let Some(error) = classify(&response, url) {
			return Err(error);
		}
		let body = response.bytes().await?;
		serde_json::from_slice(&body).map_err(SourceError::decode)
	}

	/// Rate-limited GET of an HTML document. Returns the decoded body and the
	/// URL the response actually came from, which HTML sources need both as the
	/// base for relative links and as the `Referer` for image requests (jsoup's
	/// `Document.location()`).
	pub async fn get_text(
		&self,
		url: &str,
		headers: &[(&str, &str)],
	) -> SourceResult<HtmlDocument> {
		self.limiter().until_ready().await;
		let mut request = self.headers().apply(self.client().get(url));
		for (name, value) in headers {
			request = request.header(*name, *value);
		}
		Self::into_document(request.send().await?, url).await
	}

	/// Rate-limited `application/x-www-form-urlencoded` POST of an HTML
	/// document. Themes that page through `admin-ajax.php` need this.
	pub async fn post_form(
		&self,
		url: &str,
		headers: &[(&str, &str)],
		form: &[(String, String)],
	) -> SourceResult<HtmlDocument> {
		self.limiter().until_ready().await;
		let mut request = self.headers().apply(self.client().post(url).form(form));
		for (name, value) in headers {
			request = request.header(*name, *value);
		}
		Self::into_document(request.send().await?, url).await
	}

	async fn into_document(
		response: reqwest::Response,
		requested: &str,
	) -> SourceResult<HtmlDocument> {
		// `Status` and `Challenged` report the URL the response came from,
		// which is where the challenge actually sits after a redirect.
		let url = response.url().to_string();
		if let Some(error) = classify(&response, &url) {
			return Err(match error {
				// A 404 is about the resource that was asked for.
				SourceError::NotFound(_) => SourceError::NotFound(requested.to_string()),
				other => other,
			});
		}
		Ok(HtmlDocument {
			body: response.text().await?,
			url,
		})
	}
}

/// The error every `SourceHttp` helper agrees a non-success response means, or
/// `None` when the response is usable.
///
/// A Cloudflare managed challenge is told apart from a plain rejection here so
/// that exactly one place decides it (`crate::http::challenge_host`): every
/// caller — page fetch, JSON API call, HTML browse — has to report the same
/// verdict for the same response.
fn classify(response: &reqwest::Response, url: &str) -> Option<SourceError> {
	let status = response.status();
	if status.as_u16() == 429 {
		return Some(SourceError::RateLimited {
			retry_after: retry_after(response.headers()),
		});
	}
	if status.as_u16() == 404 {
		return Some(SourceError::NotFound(url.to_string()));
	}
	if let Some(host) =
		crate::http::challenge_host(status.as_u16(), response.headers(), url)
	{
		return Some(SourceError::Challenged { host });
	}
	(!status.is_success()).then(|| SourceError::Status {
		status: status.as_u16(),
		url: url.to_string(),
	})
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
			..Default::default()
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

	/// The operator's configured headers must reach the wire on every lane a
	/// source uses, and a `User-Agent` among them must replace the Stump one:
	/// a `cf_clearance` cookie is only accepted with the exact user agent it
	/// was issued to.
	#[tokio::test]
	async fn configured_headers_are_attached_to_every_request() {
		use crate::{
			http::RequestHeaders,
			mock_http::{CannedResponse, MockServer},
			rate_limit::RateLimiter,
		};

		let server = MockServer::spawn(vec![
			("/manga", CannedResponse::html("<html>ok</html>")),
			("/api", CannedResponse::json(b"{}".to_vec())),
			(
				"/page.jpg",
				CannedResponse::ok("image/jpeg", b"jpg".to_vec()),
			),
			("/ajax", CannedResponse::html("<html>more</html>")),
		])
		.await;
		let http = SourceHttp::with_limiter(RateLimiter::unlimited())
			.expect("client")
			.with_headers(
				RequestHeaders::from_pairs([
					(
						"Cookie".to_string(),
						"cf_clearance=deadbeefcafe".to_string(),
					),
					(
						"User-Agent".to_string(),
						"Mozilla/5.0 (operator)".to_string(),
					),
				])
				.expect("valid headers"),
			);

		http.get_text(&server.url("/manga"), &[("Referer", "https://site.test/")])
			.await
			.expect("html");
		http.get_json::<serde_json::Value>(&server.url("/api"), &[])
			.await
			.expect("json");
		http.fetch_bytes(&server.url("/page.jpg"), &[])
			.await
			.expect("bytes");
		http.post_form(&server.url("/ajax"), &[], &[])
			.await
			.expect("form");

		let requests = server.requests();
		assert_eq!(requests.len(), 4);
		for request in &requests {
			let header = |name: &str| {
				request
					.headers
					.iter()
					.find(|(key, _)| key == name)
					.map(|(_, value)| value.as_str())
			};
			assert_eq!(
				header("cookie"),
				Some("cf_clearance=deadbeefcafe"),
				"{} {}",
				request.method,
				request.path
			);
			assert_eq!(
				header("user-agent"),
				Some("Mozilla/5.0 (operator)"),
				"the configured user agent must replace the Stump default"
			);
		}
		// A per-request header the engine needs is still sent.
		assert!(requests[0]
			.headers
			.iter()
			.any(|(name, value)| name == "referer" && value == "https://site.test/"));
	}

	/// A Cloudflare challenge is its own error on every lane, so callers can
	/// tell "the site refuses robots" from "the site is broken".
	#[tokio::test]
	async fn a_challenge_response_is_not_reported_as_a_bad_status() {
		use crate::{
			mock_http::{CannedResponse, MockServer},
			rate_limit::RateLimiter,
		};

		let server = MockServer::spawn(vec![
			(
				"/comic",
				CannedResponse::status(403).with_header("cf-mitigated", "challenge"),
			),
			("/blocked", CannedResponse::status(403)),
		])
		.await;
		let http = SourceHttp::with_limiter(RateLimiter::unlimited()).expect("client");

		let host = crate::http::host_of(server.base_url());
		let challenged = http
			.get_text(&server.url("/comic"), &[])
			.await
			.expect_err("challenge");
		assert!(
			matches!(&challenged, SourceError::Challenged { host: reported } if *reported == host),
			"{challenged:?}"
		);
		let challenged = http
			.fetch_bytes(&server.url("/comic"), &[])
			.await
			.expect_err("challenge");
		assert!(matches!(challenged, SourceError::Challenged { .. }));

		// A 403 without the marker stays a plain status failure.
		let refused = http
			.get_text(&server.url("/blocked"), &[])
			.await
			.expect_err("refused");
		assert!(
			matches!(refused, SourceError::Status { status: 403, .. }),
			"{refused:?}"
		);
	}

	/// The documented `contentRating` → `age_rating` table. `Safe` stores no
	/// rating so unrestricted titles stay visible to users whose restriction
	/// does not restrict on unset.
	#[test]
	fn content_rating_projects_to_age_rating() {
		assert_eq!(ContentRating::Safe.age_rating(), None);
		assert_eq!(ContentRating::Suggestive.age_rating(), Some(13));
		assert_eq!(ContentRating::Erotica.age_rating(), Some(16));
		assert_eq!(ContentRating::Pornographic.age_rating(), Some(18));
		assert!(!ContentRating::Safe.is_adult());
		assert!(!ContentRating::Suggestive.is_adult());
		assert!(ContentRating::Erotica.is_adult());
		assert!(ContentRating::Pornographic.is_adult());
	}

	/// A source that cannot tell must not have its chapters skipped, so the
	/// default — and the value a payload without the field decodes to — is
	/// readable.
	#[test]
	fn chapters_default_to_readable() {
		assert!(RemoteChapter::default().readable);
		let decoded: RemoteChapter =
			serde_json::from_str(r#"{"remote_id":"c1"}"#).expect("decode");
		assert!(decoded.readable);
		assert_eq!(decoded.external_url, None);
	}
}
