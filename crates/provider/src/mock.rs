//! An in-memory [`Source`] with two series and three chapters, used by the
//! crate's own tests and (behind the `mock` feature) by server router tests.

use std::sync::{
	atomic::{AtomicUsize, Ordering},
	Arc,
};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};

use crate::{
	http::SourceHttp,
	rate_limit::RateLimiter,
	source::{
		FetchedPage, RemoteChapter, RemotePage, RemoteSeries, SearchFilter, SeriesStatus,
		Source, SourceCapabilities, SourceError, SourceInfo, SourcePage, SourceResult,
	},
};

pub const MOCK_SOURCE_ID: &str = "mock-en";
pub const SERIES_ALPHA: &str = "alpha";
pub const SERIES_BETA: &str = "beta";
/// Chapters of `alpha`, in remote (newest-first) order.
pub const ALPHA_CHAPTERS: [&str; 3] = ["alpha-ch3", "alpha-ch2", "alpha-ch1"];
pub const PAGES_PER_CHAPTER: u32 = 3;

/// A 1x1 PNG so page bytes decode as a real image.
pub const PNG_PIXEL: &[u8] = &[
	0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
	0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
	0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
	0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D,
	0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

pub struct MockSource {
	info: SourceInfo,
	http: SourceHttp,
	page_fetches: AtomicUsize,
	detail_fetches: AtomicUsize,
}

impl MockSource {
	pub fn new() -> Arc<Self> {
		Self::with_id(MOCK_SOURCE_ID)
	}

	/// A second instance of the same catalogue under another id, so tests can
	/// exercise cross-source behaviour (dedupe, merges).
	pub fn with_id(id: &str) -> Arc<Self> {
		Arc::new(Self {
			info: SourceInfo {
				id: id.to_string(),
				name: "Mock".to_string(),
				lang: "en".to_string(),
				base_url: "http://mock.invalid".to_string(),
				capabilities: SourceCapabilities {
					popular: true,
					latest: true,
					search: true,
				},
			},
			http: SourceHttp::with_limiter(RateLimiter::unlimited())
				.expect("reqwest client for mock source"),
			page_fetches: AtomicUsize::new(0),
			detail_fetches: AtomicUsize::new(0),
		})
	}

	pub fn page_fetches(&self) -> usize {
		self.page_fetches.load(Ordering::SeqCst)
	}

	pub fn detail_fetches(&self) -> usize {
		self.detail_fetches.load(Ordering::SeqCst)
	}

	fn series(remote_id: &str) -> Option<RemoteSeries> {
		match remote_id {
			SERIES_ALPHA => Some(RemoteSeries {
				remote_id: SERIES_ALPHA.to_string(),
				title: "Alpha Adventures".to_string(),
				url: Some("http://mock.invalid/title/alpha".to_string()),
				thumbnail_url: Some("http://mock.invalid/covers/alpha.png".to_string()),
				description: Some("Three chapters of alpha.".to_string()),
				authors: vec!["Ada".to_string()],
				artists: vec!["Ada".to_string()],
				genres: vec!["Action".to_string(), "Comedy".to_string()],
				status: SeriesStatus::Ongoing,
				nsfw: false,
				original_language: Some("ja".to_string()),
				external_ids: std::collections::BTreeMap::from([(
					"al".to_string(),
					"30002".to_string(),
				)]),
			}),
			SERIES_BETA => Some(RemoteSeries {
				remote_id: SERIES_BETA.to_string(),
				title: "Beta Blues".to_string(),
				url: None,
				thumbnail_url: None,
				description: None,
				authors: vec![],
				artists: vec![],
				genres: vec![],
				status: SeriesStatus::Completed,
				nsfw: true,
				original_language: None,
				external_ids: Default::default(),
			}),
			_ => None,
		}
	}

	/// Page URL for `(chapter, index)`; the mock never fetches it.
	pub fn page_url(chapter_id: &str, index: u32) -> String {
		format!("http://mock.invalid/pages/{chapter_id}/{}.png", index + 1)
	}

	/// The bytes the mock returns for a page (a PNG with a unique trailer).
	pub fn page_bytes(chapter_id: &str, index: u32) -> Vec<u8> {
		let mut bytes = PNG_PIXEL.to_vec();
		bytes.extend_from_slice(format!("\n{chapter_id}:{index}").as_bytes());
		bytes
	}
}

#[async_trait]
impl Source for MockSource {
	fn info(&self) -> &SourceInfo {
		&self.info
	}

	fn http(&self) -> &SourceHttp {
		&self.http
	}

	async fn popular(&self, page: u32) -> SourceResult<SourcePage<RemoteSeries>> {
		Ok(SourcePage {
			items: if page <= 1 {
				vec![
					Self::series(SERIES_ALPHA).unwrap(),
					Self::series(SERIES_BETA).unwrap(),
				]
			} else {
				Vec::new()
			},
			has_next: false,
		})
	}

	async fn latest(&self, page: u32) -> SourceResult<SourcePage<RemoteSeries>> {
		self.popular(page).await
	}

	async fn search(
		&self,
		query: &str,
		_filters: &[SearchFilter],
		page: u32,
	) -> SourceResult<SourcePage<RemoteSeries>> {
		let query = query.to_ascii_lowercase();
		let mut all = self.popular(page).await?;
		all.items
			.retain(|series| series.title.to_ascii_lowercase().contains(&query));
		Ok(all)
	}

	async fn details(&self, remote_id: &str) -> SourceResult<RemoteSeries> {
		self.detail_fetches.fetch_add(1, Ordering::SeqCst);
		Self::series(remote_id)
			.ok_or_else(|| SourceError::NotFound(remote_id.to_string()))
	}

	async fn chapters(&self, remote_id: &str) -> SourceResult<Vec<RemoteChapter>> {
		match remote_id {
			SERIES_ALPHA => Ok(ALPHA_CHAPTERS
				.iter()
				.enumerate()
				.map(|(position, id)| {
					let number = (ALPHA_CHAPTERS.len() - position) as f32;
					RemoteChapter {
						remote_id: (*id).to_string(),
						title: Some(format!("Chapter {number}")),
						number: Some(number),
						volume: Some("1".to_string()),
						lang: Some("en".to_string()),
						scanlator: Some("Mock Scans".to_string()),
						uploaded_at: Some(
							Utc.with_ymd_and_hms(2026, 1, number as u32, 0, 0, 0)
								.unwrap(),
						),
						url: Some(format!("http://mock.invalid/chapter/{id}")),
						page_count: (number as u32 != 2).then_some(PAGES_PER_CHAPTER),
					}
				})
				.collect()),
			SERIES_BETA => Ok(Vec::new()),
			_ => Err(SourceError::NotFound(remote_id.to_string())),
		}
	}

	async fn pages(&self, chapter_id: &str) -> SourceResult<Vec<RemotePage>> {
		if !ALPHA_CHAPTERS.contains(&chapter_id) {
			return Err(SourceError::NotFound(chapter_id.to_string()));
		}
		Ok((0..PAGES_PER_CHAPTER)
			.map(|index| RemotePage::new(index, Self::page_url(chapter_id, index)))
			.collect())
	}

	async fn fetch_page(&self, page: &RemotePage) -> SourceResult<FetchedPage> {
		self.page_fetches.fetch_add(1, Ordering::SeqCst);
		let chapter = page
			.url
			.strip_prefix("http://mock.invalid/pages/")
			.and_then(|rest| rest.split('/').next())
			.ok_or_else(|| SourceError::NotFound(page.url.clone()))?;
		Ok(FetchedPage {
			bytes: Self::page_bytes(chapter, page.index),
			content_type: Some("image/png".to_string()),
		})
	}

	async fn fetch_image(&self, url: &str) -> SourceResult<FetchedPage> {
		if url.ends_with("/covers/alpha.png") {
			Ok(FetchedPage {
				bytes: PNG_PIXEL.to_vec(),
				content_type: Some("image/png".to_string()),
			})
		} else {
			Err(SourceError::NotFound(url.to_string()))
		}
	}
}
