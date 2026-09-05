//! MangaDex (`https://api.mangadex.org`) as a [`stump_provider::Source`].
//!
//! Browse and search use `/manga`, details `/manga/{id}`, chapters
//! `/manga/{id}/feed` (paged, filtered to the instance language), and pages
//! `/at-home/server/{chapterId}` whose `baseUrl/data/{hash}/{file}` links
//! expire after roughly fifteen minutes. Image fetches from MangaDex@Home
//! nodes are reported to `https://api.mangadex.network/report` as the network
//! rules require. API calls share one limiter at MangaDex's 5 requests/second.

use std::{collections::HashMap, sync::Arc, time::Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use models::entity::provider_source;
use serde::Deserialize;
use stump_provider::{
	FetchedPage, ProviderError, RateLimiter, RemoteChapter, RemotePage, RemoteSeries,
	SearchFilter, SeriesStatus, Source, SourceCapabilities, SourceError, SourceFactory,
	SourceHttp, SourceInfo, SourcePage, SourceResult,
};

pub const IMPLEMENTATION: &str = "mangadex";
pub const API_URL: &str = "https://api.mangadex.org";
pub const SITE_URL: &str = "https://mangadex.org";
pub const COVER_URL: &str = "https://uploads.mangadex.org/covers";
pub const REPORT_URL: &str = "https://api.mangadex.network/report";
pub const CATALOG_PKG: &str = "eu.kanade.tachiyomi.extension.all.mangadex";
/// MangaDex's documented global limit.
pub const REQUESTS_PER_SECOND: u32 = 5;
const BROWSE_PAGE_SIZE: u32 = 20;
const FEED_PAGE_SIZE: u32 = 500;
const CONTENT_RATINGS: [&str; 4] = ["safe", "suggestive", "erotica", "pornographic"];

/// The factory registered with the provider host.
pub fn factory() -> SourceFactory {
	SourceFactory {
		implementation: IMPLEMENTATION,
		name: "MangaDex",
		catalog_pkg: CATALOG_PKG,
		base_url: SITE_URL,
		build: |row| {
			MangaDexSource::new(&row.id, &row.lang)
				.map(|source| Arc::new(source) as Arc<dyn Source>)
				.map_err(ProviderError::from)
		},
	}
}

pub struct MangaDexSource {
	info: SourceInfo,
	http: SourceHttp,
	api_url: String,
	report_url: Option<String>,
}

impl MangaDexSource {
	/// A source instance whose chapters are filtered to `lang` (`all` for
	/// every language).
	pub fn new(instance_id: &str, lang: &str) -> Result<Self, reqwest::Error> {
		Ok(Self::with_http(
			instance_id,
			lang,
			SourceHttp::with_limiter(RateLimiter::per_second(REQUESTS_PER_SECOND))?,
			API_URL,
			Some(REPORT_URL),
		))
	}

	/// Construct against a custom API base (tests point this at a mock server).
	pub fn with_http(
		instance_id: &str,
		lang: &str,
		http: SourceHttp,
		api_url: &str,
		report_url: Option<&str>,
	) -> Self {
		let lang = lang.trim().to_ascii_lowercase();
		Self {
			info: SourceInfo {
				id: instance_id.to_string(),
				name: "MangaDex".to_string(),
				lang: if lang.is_empty() { "all".to_string() } else { lang },
				base_url: SITE_URL.to_string(),
				capabilities: SourceCapabilities {
					popular: true,
					latest: true,
					search: true,
				},
			},
			http,
			api_url: api_url.trim_end_matches('/').to_string(),
			report_url: report_url.map(str::to_string),
		}
	}

	fn language_filter(&self) -> Option<&str> {
		(self.info.lang != "all").then_some(self.info.lang.as_str())
	}

	fn manga_query(&self, page: u32, order: (&str, &str)) -> Vec<(&'static str, String)> {
		let mut query = vec![
			("limit", BROWSE_PAGE_SIZE.to_string()),
			("offset", (page.saturating_sub(1) * BROWSE_PAGE_SIZE).to_string()),
			("includes[]", "cover_art".to_string()),
			("includes[]", "author".to_string()),
			("includes[]", "artist".to_string()),
		];
		if let Some(lang) = self.language_filter() {
			query.push(("availableTranslatedLanguage[]", lang.to_string()));
		}
		for rating in CONTENT_RATINGS {
			query.push(("contentRating[]", rating.to_string()));
		}
		query.push((order_key(order.0), order.1.to_string()));
		query
	}

	async fn manga_list(
		&self,
		query: Vec<(&'static str, String)>,
	) -> SourceResult<SourcePage<RemoteSeries>> {
		let url = format!("{}/manga", self.api_url);
		let response: MangaListResponse = self.http.get_json(&url, &query).await?;
		let has_next = response.offset + response.data.len() as u32 < response.total;
		Ok(SourcePage {
			items: response.data.into_iter().map(RemoteSeries::from).collect(),
			has_next,
		})
	}

	fn is_home_node(&self, url: &str) -> bool {
		url.split('/')
			.nth(2)
			.is_some_and(|host| host != "uploads.mangadex.org")
	}

	/// Report an image fetch to the MangaDex@Home network (fire and forget).
	fn report(&self, url: &str, success: bool, bytes: usize, duration_ms: u128, cached: bool) {
		let Some(report_url) = self.report_url.clone() else {
			return;
		};
		if !self.is_home_node(url) {
			return;
		}
		let client = self.http.client().clone();
		let body = serde_json::json!({
			"url": url,
			"success": success,
			"bytes": bytes,
			"duration": duration_ms,
			"cached": cached,
		});
		tokio::spawn(async move {
			if let Err(error) = client.post(&report_url).json(&body).send().await {
				tracing::debug!(?error, "MangaDex@Home report failed");
			}
		});
	}
}

fn order_key(field: &str) -> &'static str {
	match field {
		"followedCount" => "order[followedCount]",
		"latestUploadedChapter" => "order[latestUploadedChapter]",
		"relevance" => "order[relevance]",
		_ => "order[followedCount]",
	}
}

#[async_trait]
impl Source for MangaDexSource {
	fn info(&self) -> &SourceInfo {
		&self.info
	}

	fn http(&self) -> &SourceHttp {
		&self.http
	}

	async fn popular(&self, page: u32) -> SourceResult<SourcePage<RemoteSeries>> {
		self.manga_list(self.manga_query(page, ("followedCount", "desc")))
			.await
	}

	async fn latest(&self, page: u32) -> SourceResult<SourcePage<RemoteSeries>> {
		self.manga_list(self.manga_query(page, ("latestUploadedChapter", "desc")))
			.await
	}

	async fn search(
		&self,
		query: &str,
		filters: &[SearchFilter],
		page: u32,
	) -> SourceResult<SourcePage<RemoteSeries>> {
		let mut params = self.manga_query(page, ("relevance", "desc"));
		let title: String = query.chars().take(400).collect();
		if !title.trim().is_empty() {
			params.push(("title", title.trim().to_string()));
		}
		for filter in filters {
			match filter.key.as_str() {
				"author" => params.push(("authorOrArtist", filter.value.clone())),
				"year" => params.push(("year", filter.value.clone())),
				"status" => params.push(("status[]", filter.value.clone())),
				"tag" => params.push(("includedTags[]", filter.value.clone())),
				_ => {},
			}
		}
		self.manga_list(params).await
	}

	async fn details(&self, remote_id: &str) -> SourceResult<RemoteSeries> {
		let url = format!("{}/manga/{remote_id}", self.api_url);
		let query = [
			("includes[]", "cover_art".to_string()),
			("includes[]", "author".to_string()),
			("includes[]", "artist".to_string()),
		];
		let response: MangaResponse = self.http.get_json(&url, &query).await?;
		Ok(RemoteSeries::from(response.data))
	}

	async fn chapters(&self, remote_id: &str) -> SourceResult<Vec<RemoteChapter>> {
		let url = format!("{}/manga/{remote_id}/feed", self.api_url);
		let mut chapters = Vec::new();
		let mut offset = 0u32;
		loop {
			let mut query = vec![
				("limit", FEED_PAGE_SIZE.to_string()),
				("offset", offset.to_string()),
				("includes[]", "scanlation_group".to_string()),
				("order[volume]", "desc".to_string()),
				("order[chapter]", "desc".to_string()),
			];
			if let Some(lang) = self.language_filter() {
				query.push(("translatedLanguage[]", lang.to_string()));
			}
			for rating in CONTENT_RATINGS {
				query.push(("contentRating[]", rating.to_string()));
			}
			let response: ChapterListResponse = self.http.get_json(&url, &query).await?;
			let received = response.data.len() as u32;
			chapters.extend(
				response
					.data
					.into_iter()
					// Externally hosted chapters cannot be read through the API.
					.filter(|chapter| chapter.attributes.external_url.is_none())
					.map(|chapter| chapter.into_remote(SITE_URL)),
			);
			offset += received;
			if received == 0 || offset >= response.total {
				break;
			}
		}
		Ok(chapters)
	}

	async fn pages(&self, chapter_id: &str) -> SourceResult<Vec<RemotePage>> {
		let url = format!("{}/at-home/server/{chapter_id}", self.api_url);
		let response: AtHomeResponse = self.http.get_json(&url, &[]).await?;
		let base = response.base_url.trim_end_matches('/');
		Ok(response
			.chapter
			.data
			.into_iter()
			.enumerate()
			.map(|(index, file)| {
				RemotePage::new(
					index as u32,
					format!("{base}/data/{}/{file}", response.chapter.hash),
				)
			})
			.collect())
	}

	async fn fetch_page(&self, page: &RemotePage) -> SourceResult<FetchedPage> {
		let started = Instant::now();
		let result = self.http.fetch_bytes(&page.url, &page.headers).await;
		let duration_ms = started.elapsed().as_millis();
		match &result {
			Ok(fetched) => self.report(&page.url, true, fetched.bytes.len(), duration_ms, false),
			Err(_) => self.report(&page.url, false, 0, duration_ms, false),
		}
		result
	}
}

// --- DTOs -------------------------------------------------------------------

#[derive(Deserialize)]
struct MangaListResponse {
	#[serde(default)]
	data: Vec<Manga>,
	#[serde(default)]
	offset: u32,
	#[serde(default)]
	total: u32,
}

#[derive(Deserialize)]
struct MangaResponse {
	data: Manga,
}

#[derive(Deserialize)]
struct Manga {
	id: String,
	#[serde(default)]
	attributes: MangaAttributes,
	#[serde(default)]
	relationships: Vec<Relationship>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct MangaAttributes {
	#[serde(default)]
	title: HashMap<String, String>,
	#[serde(default)]
	alt_titles: Vec<HashMap<String, String>>,
	#[serde(default)]
	description: HashMap<String, String>,
	#[serde(default)]
	original_language: Option<String>,
	#[serde(default)]
	status: Option<String>,
	#[serde(default)]
	content_rating: Option<String>,
	#[serde(default)]
	tags: Vec<Tag>,
}

#[derive(Deserialize)]
struct Tag {
	#[serde(default)]
	attributes: TagAttributes,
}

#[derive(Deserialize, Default)]
struct TagAttributes {
	#[serde(default)]
	name: HashMap<String, String>,
	#[serde(default)]
	group: String,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum Relationship {
	#[serde(rename = "author")]
	Author {
		#[serde(default)]
		attributes: Option<NamedAttributes>,
	},
	#[serde(rename = "artist")]
	Artist {
		#[serde(default)]
		attributes: Option<NamedAttributes>,
	},
	#[serde(rename = "cover_art")]
	CoverArt {
		#[serde(default)]
		attributes: Option<CoverAttributes>,
	},
	#[serde(rename = "scanlation_group")]
	ScanlationGroup {
		#[serde(default)]
		attributes: Option<NamedAttributes>,
	},
	#[serde(other)]
	Other,
}

#[derive(Deserialize, Default)]
struct NamedAttributes {
	#[serde(default)]
	name: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CoverAttributes {
	#[serde(default)]
	file_name: Option<String>,
}

fn preferred(map: &HashMap<String, String>, lang: &str) -> Option<String> {
	map.get(lang)
		.or_else(|| map.get("en"))
		.or_else(|| map.get("ja-ro"))
		.or_else(|| map.values().next())
		.cloned()
		.filter(|value| !value.trim().is_empty())
}

impl From<Manga> for RemoteSeries {
	fn from(manga: Manga) -> Self {
		let attributes = manga.attributes;
		let title = preferred(&attributes.title, "en")
			.or_else(|| {
				attributes
					.alt_titles
					.iter()
					.find_map(|alt| preferred(alt, "en"))
			})
			.unwrap_or_else(|| manga.id.clone());
		let mut authors = Vec::new();
		let mut artists = Vec::new();
		let mut cover = None;
		for relationship in manga.relationships {
			match relationship {
				Relationship::Author { attributes } => {
					if let Some(name) = attributes.and_then(|a| a.name) {
						authors.push(name);
					}
				},
				Relationship::Artist { attributes } => {
					if let Some(name) = attributes.and_then(|a| a.name) {
						artists.push(name);
					}
				},
				Relationship::CoverArt { attributes } => {
					if cover.is_none() {
						cover = attributes.and_then(|a| a.file_name);
					}
				},
				_ => {},
			}
		}
		let genres = attributes
			.tags
			.iter()
			.filter(|tag| matches!(tag.attributes.group.as_str(), "genre" | "theme" | ""))
			.filter_map(|tag| preferred(&tag.attributes.name, "en"))
			.collect();
		RemoteSeries {
			url: Some(format!("{SITE_URL}/title/{}", manga.id)),
			thumbnail_url: cover
				.map(|file| format!("{COVER_URL}/{}/{file}.512.jpg", manga.id)),
			description: preferred(&attributes.description, "en"),
			authors,
			artists,
			genres,
			status: match attributes.status.as_deref() {
				Some("ongoing") => SeriesStatus::Ongoing,
				Some("completed") => SeriesStatus::Completed,
				Some("hiatus") => SeriesStatus::Hiatus,
				Some("cancelled") => SeriesStatus::Cancelled,
				_ => SeriesStatus::Unknown,
			},
			nsfw: matches!(
				attributes.content_rating.as_deref(),
				Some("erotica") | Some("pornographic")
			),
			original_language: attributes.original_language,
			remote_id: manga.id,
			title,
		}
	}
}

#[derive(Deserialize)]
struct ChapterListResponse {
	#[serde(default)]
	data: Vec<Chapter>,
	#[serde(default)]
	total: u32,
}

#[derive(Deserialize)]
struct Chapter {
	id: String,
	#[serde(default)]
	attributes: ChapterAttributes,
	#[serde(default)]
	relationships: Vec<Relationship>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ChapterAttributes {
	#[serde(default)]
	volume: Option<String>,
	#[serde(default)]
	chapter: Option<String>,
	#[serde(default)]
	title: Option<String>,
	#[serde(default)]
	translated_language: Option<String>,
	#[serde(default)]
	pages: Option<u32>,
	#[serde(default)]
	publish_at: Option<DateTime<Utc>>,
	#[serde(default)]
	external_url: Option<String>,
}

impl Chapter {
	fn into_remote(self, site_url: &str) -> RemoteChapter {
		let scanlator = self.relationships.into_iter().find_map(|relationship| match relationship {
			Relationship::ScanlationGroup { attributes } => attributes.and_then(|a| a.name),
			_ => None,
		});
		RemoteChapter {
			url: Some(format!("{site_url}/chapter/{}", self.id)),
			title: self.attributes.title.filter(|title| !title.trim().is_empty()),
			number: self
				.attributes
				.chapter
				.as_deref()
				.and_then(|chapter| chapter.trim().parse::<f32>().ok()),
			volume: self.attributes.volume.filter(|volume| !volume.trim().is_empty()),
			lang: self.attributes.translated_language,
			scanlator,
			uploaded_at: self.attributes.publish_at,
			page_count: self.attributes.pages.filter(|pages| *pages > 0),
			remote_id: self.id,
		}
	}
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtHomeResponse {
	base_url: String,
	chapter: AtHomeChapter,
}

#[derive(Deserialize)]
struct AtHomeChapter {
	hash: String,
	#[serde(default)]
	data: Vec<String>,
}

#[cfg(test)]
mod tests {
	use super::*;
	use stump_provider::mock_http::{CannedResponse, MockServer};

	const MANGA_ID: &str = "32d76d19-8a05-4db0-9fc2-e0b0648fe9d0";
	const CHAPTER_ID: &str = "0f1b5f6c-1c25-4a97-bd91-ca4a5b6f3e2d";

	fn manga_json(id: &str) -> serde_json::Value {
		serde_json::json!({
			"id": id,
			"type": "manga",
			"attributes": {
				"title": {"en": "Solo Leveling"},
				"altTitles": [{"ja-ro": "Na Honjaman Level Up"}, {"ko": "나 혼자만 레벨업"}],
				"description": {"en": "Ten years ago, gates opened.", "fr": "Il y a dix ans"},
				"originalLanguage": "ko",
				"status": "completed",
				"contentRating": "safe",
				"year": 2018,
				"tags": [
					{"id": "t1", "type": "tag", "attributes": {"name": {"en": "Action"}, "group": "genre"}},
					{"id": "t2", "type": "tag", "attributes": {"name": {"en": "Monsters"}, "group": "theme"}},
					{"id": "t3", "type": "tag", "attributes": {"name": {"en": "Long Strip"}, "group": "format"}}
				]
			},
			"relationships": [
				{"id": "a1", "type": "author", "attributes": {"name": "Chugong"}},
				{"id": "a2", "type": "artist", "attributes": {"name": "DUBU"}},
				{"id": "c1", "type": "cover_art", "attributes": {"fileName": "cover.jpg", "volume": "1"}},
				{"id": "x1", "type": "manga", "related": "sequel"}
			]
		})
	}

	fn server_source(server: &MockServer) -> MangaDexSource {
		MangaDexSource::with_http(
			"mangadex-en",
			"en",
			SourceHttp::with_limiter(RateLimiter::unlimited()).unwrap(),
			server.base_url(),
			Some(&server.url("/report")),
		)
	}

	#[test]
	fn factory_builds_language_scoped_instances() {
		let factory = factory();
		assert_eq!(factory.implementation, "mangadex");
		assert_eq!(factory.catalog_pkg, CATALOG_PKG);
		let row = provider_source::Model {
			id: "mangadex-ja".to_string(),
			implementation: "mangadex".to_string(),
			catalog_id: None,
			name: "MangaDex".to_string(),
			lang: "ja".to_string(),
			base_url: SITE_URL.to_string(),
			enabled: true,
			created_by: None,
			created_at: Utc::now().into(),
			updated_at: None,
		};
		let source = (factory.build)(&row).unwrap();
		assert_eq!(source.info().id, "mangadex-ja");
		assert_eq!(source.info().lang, "ja");
		assert_eq!(source.info().base_url, SITE_URL);
		assert!(source.info().capabilities.search);
	}

	#[tokio::test]
	async fn search_maps_manga_list_and_pagination() {
		let body = serde_json::json!({
			"result": "ok",
			"data": [manga_json(MANGA_ID)],
			"limit": 20,
			"offset": 0,
			"total": 21
		});
		let server = MockServer::spawn(vec![]).await;
		let query = format!(
			"/manga?limit=20&offset=0&includes%5B%5D=cover_art&includes%5B%5D=author&includes%5B%5D=artist&availableTranslatedLanguage%5B%5D=en&contentRating%5B%5D=safe&contentRating%5B%5D=suggestive&contentRating%5B%5D=erotica&contentRating%5B%5D=pornographic&order%5Brelevance%5D=desc&title=solo+leveling"
		);
		server.set_route(&query, CannedResponse::json(body.to_string()));
		let source = server_source(&server);

		let page = source.search("solo leveling", &[], 1).await.unwrap();
		assert!(page.has_next);
		assert_eq!(page.items.len(), 1);
		let series = &page.items[0];
		assert_eq!(series.remote_id, MANGA_ID);
		assert_eq!(series.title, "Solo Leveling");
		assert_eq!(series.authors, vec!["Chugong"]);
		assert_eq!(series.artists, vec!["DUBU"]);
		assert_eq!(series.genres, vec!["Action", "Monsters"]);
		assert_eq!(series.status, SeriesStatus::Completed);
		assert!(!series.nsfw);
		assert_eq!(series.original_language.as_deref(), Some("ko"));
		assert_eq!(
			series.thumbnail_url.as_deref(),
			Some(&*format!("{COVER_URL}/{MANGA_ID}/cover.jpg.512.jpg"))
		);
		assert_eq!(
			series.url.as_deref(),
			Some(&*format!("{SITE_URL}/title/{MANGA_ID}"))
		);
		assert_eq!(series.description.as_deref(), Some("Ten years ago, gates opened."));
		assert_eq!(server.request_count(&query), 1);
	}

	#[tokio::test]
	async fn chapters_follow_feed_pagination_and_skip_external() {
		let chapter = |id: &str, number: &str, external: Option<&str>| {
			serde_json::json!({
				"id": id,
				"type": "chapter",
				"attributes": {
					"volume": "1",
					"chapter": number,
					"title": format!("Chapter {number}"),
					"translatedLanguage": "en",
					"pages": 12,
					"publishAt": "2024-01-02T03:04:05+00:00",
					"externalUrl": external
				},
				"relationships": [
					{"id": "g1", "type": "scanlation_group", "attributes": {"name": "Asura"}}
				]
			})
		};
		let base = format!(
			"/manga/{MANGA_ID}/feed?limit=500&offset={{offset}}&includes%5B%5D=scanlation_group&order%5Bvolume%5D=desc&order%5Bchapter%5D=desc&translatedLanguage%5B%5D=en&contentRating%5B%5D=safe&contentRating%5B%5D=suggestive&contentRating%5B%5D=erotica&contentRating%5B%5D=pornographic"
		);
		let first = serde_json::json!({
			"result": "ok",
			"data": [chapter("c2", "2", None), chapter("cx", "1.5", Some("https://external"))],
			"limit": 500,
			"offset": 0,
			"total": 3
		});
		let second = serde_json::json!({
			"result": "ok",
			"data": [chapter("c1", "1", None)],
			"limit": 500,
			"offset": 2,
			"total": 3
		});
		let server = MockServer::spawn(vec![]).await;
		server.set_route(&base.replace("{offset}", "0"), CannedResponse::json(first.to_string()));
		server.set_route(&base.replace("{offset}", "2"), CannedResponse::json(second.to_string()));
		let source = server_source(&server);

		let chapters = source.chapters(MANGA_ID).await.unwrap();
		assert_eq!(chapters.len(), 2);
		assert_eq!(chapters[0].remote_id, "c2");
		assert_eq!(chapters[0].number, Some(2.0));
		assert_eq!(chapters[0].volume.as_deref(), Some("1"));
		assert_eq!(chapters[0].scanlator.as_deref(), Some("Asura"));
		assert_eq!(chapters[0].page_count, Some(12));
		assert_eq!(chapters[0].lang.as_deref(), Some("en"));
		assert_eq!(
			chapters[0].uploaded_at.map(|at| at.to_rfc3339()),
			Some("2024-01-02T03:04:05+00:00".to_string())
		);
		assert_eq!(chapters[0].display_name(), "Vol. 1 Ch. 2 - Chapter 2");
		assert_eq!(chapters[1].remote_id, "c1");
	}

	#[tokio::test]
	async fn pages_build_at_home_urls_and_fetch_reports_to_network() {
		let server = MockServer::spawn(vec![]).await;
		let at_home = serde_json::json!({
			"result": "ok",
			"baseUrl": server.base_url(),
			"chapter": {"hash": "abc123", "data": ["1-x.png", "2-y.jpg"], "dataSaver": ["1-x.jpg"]}
		});
		server.set_route(
			&format!("/at-home/server/{CHAPTER_ID}"),
			CannedResponse::json(at_home.to_string()),
		);
		server.set_route("/data/abc123/1-x.png", CannedResponse::ok("image/png", b"png".to_vec()));
		server.set_route("/report", CannedResponse::json(b"{}".to_vec()));
		let source = server_source(&server);

		let pages = source.pages(CHAPTER_ID).await.unwrap();
		assert_eq!(pages.len(), 2);
		assert_eq!(pages[0].index, 0);
		assert_eq!(pages[0].url, server.url("/data/abc123/1-x.png"));
		assert_eq!(pages[1].url, server.url("/data/abc123/2-y.jpg"));

		let fetched = source.fetch_page(&pages[0]).await.unwrap();
		assert_eq!(fetched.bytes, b"png");
		assert_eq!(fetched.content_type.as_deref(), Some("image/png"));

		// The mock server is not uploads.mangadex.org, so the fetch is reported.
		for _ in 0..50 {
			if server.request_count("/report") == 1 {
				break;
			}
			tokio::time::sleep(std::time::Duration::from_millis(20)).await;
		}
		let report = server
			.requests()
			.into_iter()
			.find(|request| request.path == "/report")
			.expect("at-home report");
		assert_eq!(report.method, "POST");
		let body: serde_json::Value = serde_json::from_slice(&report.body).unwrap();
		assert_eq!(body["success"], true);
		assert_eq!(body["bytes"], 3);
		assert_eq!(body["url"], server.url("/data/abc123/1-x.png"));

		let missing = RemotePage::new(1, server.url("/data/abc123/2-y.jpg"));
		assert!(matches!(
			source.fetch_page(&missing).await,
			Err(SourceError::NotFound(_))
		));
	}

	#[tokio::test]
	async fn rate_limited_and_decode_errors_are_typed() {
		let server = MockServer::spawn(vec![
			(
				&format!("/manga/{MANGA_ID}?includes%5B%5D=cover_art&includes%5B%5D=author&includes%5B%5D=artist"),
				CannedResponse::status(429).with_header("Retry-After", "7"),
			),
			(
				"/manga/bad?includes%5B%5D=cover_art&includes%5B%5D=author&includes%5B%5D=artist",
				CannedResponse::json(b"not json".to_vec()),
			),
		])
		.await;
		let source = server_source(&server);
		match source.details(MANGA_ID).await {
			Err(SourceError::RateLimited { retry_after }) => {
				assert_eq!(retry_after, Some(std::time::Duration::from_secs(7)));
			},
			other => panic!("expected rate limit, got {other:?}"),
		}
		assert!(matches!(source.details("bad").await, Err(SourceError::Decode(_))));
	}

	#[test]
	fn all_language_instances_do_not_filter() {
		let source = MangaDexSource::with_http(
			"mangadex-all",
			"all",
			SourceHttp::with_limiter(RateLimiter::unlimited()).unwrap(),
			API_URL,
			None,
		);
		assert!(source.language_filter().is_none());
		let query = source.manga_query(2, ("followedCount", "desc"));
		assert!(query.iter().any(|(key, value)| *key == "offset" && value == "20"));
		assert!(!query.iter().any(|(key, _)| *key == "availableTranslatedLanguage[]"));
		assert!(source.is_home_node("https://cmdxd98sb0x3yprd.mangadex.network/data/x/1.png"));
		assert!(!source.is_home_node("https://uploads.mangadex.org/data/x/1.png"));
	}

	/// Requires network access; exercised manually against the live API.
	#[tokio::test]
	#[ignore]
	async fn live_search_details_chapters_and_first_page() {
		let source = MangaDexSource::new("mangadex-en", "en").unwrap();
		let results = source.search("one piece", &[], 1).await.unwrap();
		let first = results.items.first().expect("live search result");
		let details = source.details(&first.remote_id).await.unwrap();
		assert_eq!(details.remote_id, first.remote_id);
		let chapters = source.chapters(&first.remote_id).await.unwrap();
		let chapter = chapters.last().expect("live chapter");
		let pages = source.pages(&chapter.remote_id).await.unwrap();
		assert!(!pages.is_empty());
		let fetched = source.fetch_page(&pages[0]).await.unwrap();
		assert!(!fetched.bytes.is_empty());
	}
}
