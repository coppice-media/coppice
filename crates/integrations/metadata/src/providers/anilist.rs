//! AniList (https://anilist.co) metadata provider client.
//!
//! Behaviour mirrors Komf's AniList provider
//! (`komf-core/src/commonMain/kotlin/snd/komf/providers/anilist/`, master
//! 428dac6028cc22db47b41ff9f979d253d2095715): a single GraphQL endpoint,
//! one search query (manga formats only) and one media query, title
//! preference `english → romaji → native`, HTML-stripped descriptions,
//! tag filtering by rank, and staff-role expansion into writer/artist roles.
//!
//! AniList requires no API key. The official limit is 30 requests/minute
//! (90/minute when degraded); the shared rate limiter is per-second, so the
//! default here is 2 req/s -- comfortably under both windows.

use reqwest_middleware::ClientWithMiddleware;
use serde::Deserialize;
use serde_json::json;

use crate::{
	client::{build_client_with_retry, RetryClientConfig},
	error::MetadataProviderError,
	provider::ProviderCredentialVerification,
	types::{
		ExternalMediaMetadata, ExternalSeriesMetadata, MatchCandidate, MediaType,
		PublicationStatus, SearchOutcome, SearchQuery,
	},
	ExternalMetadata, MetadataProvider, RateLimiter,
};

/// AniList allows 30 req/min (90 req/min degraded). The shared limiter is
/// per-second, so 2 req/s keeps us under the stricter window.
const ANILIST_DEFAULT_RATE_LIMIT: u32 = 2;

/// Komf's default `tagsScoreThreshold` (percent-based tag rank).
const ANILIST_DEFAULT_TAGS_SCORE_THRESHOLD: i32 = 60;
/// Komf's default `tagsSizeLimit`.
const ANILIST_DEFAULT_TAGS_SIZE_LIMIT: usize = 15;

const MANGA_FORMATS: &[&str] = &["MANGA", "ONE_SHOT"];
const NOVEL_FORMATS: &[&str] = &["NOVEL"];

/// Komf's allowed staff roles, split by the side they map to.
const STORY_AND_ART_ROLE: &str = "Story & Art";
const STORY_ROLES: &[&str] = &["Story", "Original Story", "Original Creator"];
const ART_ROLES: &[&str] = &["Art", "Illustration"];

/// Fields requested for every media (Komf's `mangaFragment`, minus the
/// unused `type`/`format`/`idMal`/`countryOfOrigin` echo fields and the
/// per-tag `description`/`category` payload, none of which have a carrier in
/// the shared metadata types).
const MANGA_FIELDS: &str = "
	id
	title {
		romaji
		english
		native
	}
	status
	description(asHtml: false)
	chapters
	volumes
	coverImage {
		extraLarge
	}
	startDate {
		year
		month
		day
	}
	genres
	synonyms
	tags {
		name
		rank
	}
	staff {
		edges {
			node {
				name {
					full
				}
			}
			role
		}
	}
";

fn search_query() -> String {
	format!(
		r#"query Search($search: String!, $perPage: Int!, $formats: [MediaFormat!]!) {{
	mediaSearch: Page(page: 1, perPage: $perPage) {{
		media(type: MANGA, format_in: $formats, search: $search) {{
			{MANGA_FIELDS}
		}}
	}}
}}"#
	)
}

fn media_query() -> String {
	format!(
		r#"query Media($id: Int!) {{
	media: Media(id: $id) {{
		{MANGA_FIELDS}
	}}
}}"#
	)
}

fn formats_for(media_type: MediaType) -> &'static [&'static str] {
	match media_type {
		MediaType::LightNovel | MediaType::Book | MediaType::WebNovel => NOVEL_FORMATS,
		// Komf rejects the COMIC media type outright for AniList; we fall back
		// to the manga formats so a misconfigured media type still yields
		// usable results.
		MediaType::Manga | MediaType::Manhwa | MediaType::Webtoon | MediaType::Comic => {
			MANGA_FORMATS
		},
	}
}

pub struct AniListClient {
	client: ClientWithMiddleware,
	api_url: String,
	rate_limiter: RateLimiter,
	media_type: MediaType,
	tags_score_threshold: i32,
	tags_size_limit: usize,
}

impl AniListClient {
	const API_URL: &'static str = "https://graphql.anilist.co";

	pub fn new(media_type: Option<MediaType>, rate_limit: Option<u32>) -> Self {
		let inner = reqwest::Client::builder()
			.user_agent(concat!(
				"stump/",
				env!("CARGO_PKG_VERSION"),
				" (+https://github.com/stumpapp/stump)"
			))
			.build()
			.expect("Failed to build AniList HTTP client"); // this should never really happen

		Self {
			client: build_client_with_retry(inner, RetryClientConfig::default()),
			api_url: Self::API_URL.to_string(),
			rate_limiter: RateLimiter::new(
				rate_limit.unwrap_or(ANILIST_DEFAULT_RATE_LIMIT),
			),
			media_type: media_type.unwrap_or(MediaType::Manga),
			tags_score_threshold: ANILIST_DEFAULT_TAGS_SCORE_THRESHOLD,
			tags_size_limit: ANILIST_DEFAULT_TAGS_SIZE_LIMIT,
		}
	}

	/// Test-only override of the API base URL, used to point the client at a
	/// local mock server.
	#[cfg(test)]
	fn with_api_url(mut self, api_url: impl Into<String>) -> Self {
		self.api_url = api_url.into();
		self
	}

	/// Test-only override of Komf's `tagsScoreThreshold` setting.
	#[cfg(test)]
	fn with_tags_score_threshold(mut self, threshold: i32) -> Self {
		self.tags_score_threshold = threshold;
		self
	}

	/// Test-only override of Komf's `tagsSizeLimit` setting.
	#[cfg(test)]
	fn with_tags_size_limit(mut self, limit: usize) -> Self {
		self.tags_size_limit = limit;
		self
	}

	async fn execute<T: serde::de::DeserializeOwned>(
		&self,
		query: &str,
		variables: serde_json::Value,
	) -> Result<T, MetadataProviderError> {
		self.rate_limiter.until_ready().await;

		let body = self
			.client
			.post(&self.api_url)
			.json(&json!({ "query": query, "variables": variables }))
			.send()
			.await?
			.error_for_status()?
			// Parse the body explicitly so malformed JSON surfaces as a
			// `ParseError` instead of reqwest's `Decode` wrapper.
			.text()
			.await?;
		let response: GraphQLResponse<T> = serde_json::from_str(&body)?;

		if let Some(errors) = response.errors {
			if !errors.is_empty() {
				let messages: Vec<_> =
					errors.iter().map(|e| e.message.as_str()).collect();
				return Err(MetadataProviderError::Other(format!(
					"GraphQL errors: {}",
					messages.join("; ")
				)));
			}
		}

		response.data.ok_or(MetadataProviderError::EmptyResponse)
	}

	/// Search AniList's manga catalogue. Variables keep user input out of the
	/// query string entirely.
	#[tracing::instrument(skip(self))]
	async fn search(
		&self,
		query: &SearchQuery,
	) -> Result<Vec<AniListMedia>, MetadataProviderError> {
		// AniList truncates overly long search terms; Komf caps input at 400
		// characters for the same reason.
		let text: String = query.title.chars().take(400).collect();
		let data: SearchData = self
			.execute(
				&search_query(),
				json!({
					"search": text,
					"perPage": query.limit.unwrap_or(10),
					"formats": formats_for(self.media_type),
				}),
			)
			.await?;
		Ok(data.media_search.media)
	}

	async fn fetch_media(&self, id: i64) -> Result<AniListMedia, MetadataProviderError> {
		let data: MediaData = self.execute(&media_query(), json!({ "id": id })).await?;
		data.media
			.ok_or_else(|| MetadataProviderError::NotFound(format!("Media {}", id)))
	}
}

#[async_trait::async_trait]
impl MetadataProvider for AniListClient {
	fn id(&self) -> &'static str {
		"anilist"
	}

	fn name(&self) -> &'static str {
		"AniList"
	}

	fn supported_media_types(&self) -> Vec<MediaType> {
		match self.media_type {
			MediaType::LightNovel | MediaType::Book | MediaType::WebNovel => {
				vec![MediaType::LightNovel, MediaType::Book, MediaType::WebNovel]
			},
			MediaType::Manga
			| MediaType::Manhwa
			| MediaType::Webtoon
			| MediaType::Comic => vec![MediaType::Manga, MediaType::Manhwa, MediaType::Webtoon],
		}
	}

	/// AniList has no series/book distinction -- every entry is a `Media`.
	/// Series search returns the same entries modelled as series metadata.
	#[tracing::instrument(skip(self))]
	async fn search_series(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		tracing::trace!("Searching for series on AniList");
		let results = self.search(query).await?;
		let requested = results.len();

		let mut candidates = Vec::with_capacity(results.len());
		for media in results {
			candidates.push(MatchCandidate {
				external_id: media.id.to_string(),
				metadata: ExternalMetadata::Series(
					media.to_series_metadata(
						self.tags_score_threshold,
						self.tags_size_limit,
					),
				),
				provider: self.id().to_string(),
				confidence: 0.0,
				confidence_factors: Vec::new(),
			});
		}

		Ok(SearchOutcome {
			candidates: self.score_search(query, candidates),
			requested,
		})
	}

	/// Search AniList's manga catalogue and score the results against the query.
	#[tracing::instrument(skip(self))]
	async fn search_media(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		tracing::trace!("Searching for media on AniList");
		let results = self.search(query).await?;
		let requested = results.len();

		let mut candidates = Vec::with_capacity(results.len());
		for media in results {
			candidates.push(MatchCandidate {
				external_id: media.id.to_string(),
				metadata: ExternalMetadata::Media(
					media.to_media_metadata(
						self.tags_score_threshold,
						self.tags_size_limit,
					),
				),
				provider: self.id().to_string(),
				confidence: 0.0,
				confidence_factors: Vec::new(),
			});
		}

		Ok(SearchOutcome {
			candidates: self.score_search(query, candidates),
			requested,
		})
	}

	async fn fetch_series_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalSeriesMetadata, MetadataProviderError> {
		let id: i64 = external_id.parse().map_err(|_| {
			MetadataProviderError::Other(format!(
				"Invalid AniList media ID: {}",
				external_id
			))
		})?;

		Ok(self
			.fetch_media(id)
			.await?
			.to_series_metadata(self.tags_score_threshold, self.tags_size_limit))
	}

	async fn fetch_media_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalMediaMetadata, MetadataProviderError> {
		let id: i64 = external_id.parse().map_err(|_| {
			MetadataProviderError::Other(format!(
				"Invalid AniList media ID: {}",
				external_id
			))
		})?;

		Ok(self
			.fetch_media(id)
			.await?
			.to_media_metadata(self.tags_score_threshold, self.tags_size_limit))
	}

	/// AniList is keyless, so "verification" is a health check: the GraphQL
	/// endpoint must answer a trivial query without errors.
	#[tracing::instrument(skip(self))]
	async fn verify_credentials(
		&self,
	) -> Result<ProviderCredentialVerification, MetadataProviderError> {
		match self
			.execute::<PageData>(
				"query HealthCheck { Page(page: 1, perPage: 1) { media(type: MANGA) { id } } }",
				json!({}),
			)
			.await
		{
			Ok(_) => Ok(ProviderCredentialVerification {
				response_status: 200,
				is_valid: true,
				error: None,
			}),
			Err(error) => Ok(ProviderCredentialVerification {
				response_status: 200,
				is_valid: false,
				error: Some(error.to_string()),
			}),
		}
	}
}

#[derive(Debug, Deserialize)]
pub struct GraphQLResponse<T> {
	pub data: Option<T>,
	pub errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Deserialize)]
pub struct GraphQLError {
	pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchData {
	#[serde(rename = "mediaSearch")]
	pub media_search: MediaPage,
}

#[derive(Debug, Deserialize)]
pub struct MediaPage {
	pub media: Vec<AniListMedia>,
}

#[derive(Debug, Deserialize)]
pub struct MediaData {
	pub media: Option<AniListMedia>,
}

#[derive(Debug, Deserialize)]
pub struct PageData {
	#[serde(default)]
	#[allow(dead_code)]
	pub page: Option<serde_json::Value>,
}

/// See https://docs.anilist.co/guide/media
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AniListMedia {
	pub id: i64,
	pub title: Option<AniListTitle>,
	pub status: Option<AniListMediaStatus>,
	pub description: Option<String>,
	pub chapters: Option<i32>,
	pub volumes: Option<i32>,
	pub cover_image: Option<AniListCoverImage>,
	pub start_date: Option<AniListFuzzyDate>,
	pub genres: Option<Vec<String>>,
	pub synonyms: Option<Vec<String>>,
	pub tags: Option<Vec<AniListMediaTag>>,
	pub staff: Option<AniListStaffConnection>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AniListTitle {
	pub romaji: Option<String>,
	pub english: Option<String>,
	pub native: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AniListMediaStatus {
	Finished,
	Releasing,
	NotYetReleased,
	Cancelled,
	Hiatus,
	/// Future-proofing: AniList may introduce new statuses.
	#[serde(other)]
	Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AniListCoverImage {
	pub extra_large: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AniListFuzzyDate {
	pub year: Option<i32>,
	pub month: Option<i32>,
	pub day: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AniListMediaTag {
	pub name: String,
	pub rank: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AniListStaffConnection {
	pub edges: Vec<AniListStaffEdge>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AniListStaffEdge {
	pub node: Option<AniListStaff>,
	pub role: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AniListStaff {
	pub name: Option<AniListStaffName>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AniListStaffName {
	pub full: Option<String>,
}

impl AniListMedia {
	/// Komf's title preference: english, then romaji, then native.
	fn title_preferred(&self) -> Option<&str> {
		let title = self.title.as_ref()?;
		[
			title.english.as_deref(),
			title.romaji.as_deref(),
			title.native.as_deref(),
		]
		.into_iter()
		.flatten()
		.map(str::trim)
		.find(|title| !title.is_empty())
	}

	/// The romaji (falling back to native) title, used as the series name.
	fn title_romaji_or_native(&self) -> Option<&str> {
		let title = self.title.as_ref()?;
		[title.romaji.as_deref(), title.native.as_deref()]
			.into_iter()
			.flatten()
			.map(str::trim)
			.find(|title| !title.is_empty())
	}

	fn publication_status(&self) -> Option<PublicationStatus> {
		match self.status {
			Some(AniListMediaStatus::Finished) => Some(PublicationStatus::Completed),
			Some(AniListMediaStatus::Releasing) => Some(PublicationStatus::Ongoing),
			// Komf maps NOT_YET_RELEASED to ONGOING; Stump's status enum has a
			// dedicated Upcoming variant, which is the faithful translation.
			Some(AniListMediaStatus::NotYetReleased) => Some(PublicationStatus::Upcoming),
			Some(AniListMediaStatus::Cancelled) => Some(PublicationStatus::Cancelled),
			Some(AniListMediaStatus::Hiatus) => Some(PublicationStatus::Hiatus),
			_ => None,
		}
	}

	/// Komf's tag filter: keep tags ranked at or above the threshold, sort by
	/// rank descending (stable, ties keep AniList order), cap at the size limit.
	fn filtered_tags(&self, threshold: i32, limit: usize) -> Vec<String> {
		let mut ranked: Vec<(i32, &str)> = self
			.tags
			.as_deref()
			.unwrap_or(&[])
			.iter()
			.filter_map(|tag| tag.rank.map(|rank| (rank, tag.name.as_str())))
			.filter(|(rank, _)| *rank >= threshold)
			.collect();
		ranked.sort_by(|left, right| right.0.cmp(&left.0));
		ranked
			.into_iter()
			.take(limit)
			.map(|(_, name)| name.to_string())
			.collect()
	}

	/// Expand staff edges into `(writers, artists)` following Komf's role
	/// mapping: "Story & Art" contributes to both, Story-family roles to
	/// writers, Art-family roles to artists. Anything else is ignored.
	fn staff_roles(&self) -> (Vec<String>, Vec<String>) {
		let mut writers = Vec::new();
		let mut artists = Vec::new();

		let Some(edges) = self.staff.as_ref().map(|staff| &staff.edges) else {
			return (writers, artists);
		};

		for edge in edges {
			let Some(name) = edge
				.node
				.as_ref()
				.and_then(|node| node.name.as_ref())
				.and_then(|name| name.full.as_deref())
				.map(str::trim)
				.filter(|name| !name.is_empty())
			else {
				continue;
			};
			// Roles carry a parenthesised qualifier, e.g. "Story (retired)".
			let role = strip_parenthetical(edge.role.as_deref().unwrap_or(""));

			if role == STORY_AND_ART_ROLE {
				writers.push(name.to_string());
				artists.push(name.to_string());
			} else if STORY_ROLES.contains(&role.as_str()) {
				writers.push(name.to_string());
			} else if ART_ROLES.contains(&role.as_str()) {
				artists.push(name.to_string());
			}
		}

		(writers, artists)
	}

	fn to_series_metadata(
		&self,
		tags_score_threshold: i32,
		tags_size_limit: usize,
	) -> ExternalSeriesMetadata {
		let (writers, artists) = self.staff_roles();
		let title = self.title_preferred().unwrap_or_default();

		// Alternative titles: the romaji/native forms plus AniList synonyms,
		// deduplicated and without the preferred title itself.
		let mut alternative_titles: Vec<String> = Vec::new();
		let mut push_alt = |value: Option<&str>| {
			if let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) {
				if value != title && !alternative_titles.iter().any(|t| t == value) {
					alternative_titles.push(value.to_string());
				}
			}
		};
		push_alt(self.title.as_ref().and_then(|t| t.romaji.as_deref()));
		push_alt(self.title.as_ref().and_then(|t| t.native.as_deref()));
		for synonym in self.synonyms.as_deref().unwrap_or(&[]) {
			push_alt(Some(synonym.as_str()));
		}

		ExternalSeriesMetadata {
			provider: "anilist".to_string(),
			external_id: self.id.to_string(),
			title: title.to_string(),
			alternative_titles,
			summary: self.description.as_deref().map(strip_html),
			status: self.publication_status(),
			year: self.start_date.as_ref().and_then(|date| date.year),
			end_year: None,
			genres: self.genres.clone().or_else(|| Some(Vec::new())),
			tags: Some(self.filtered_tags(tags_score_threshold, tags_size_limit)),
			age_rating: None,
			authors: Some(writers),
			artists: Some(artists),
			publisher: None,
			cover_url: self
				.cover_image
				.as_ref()
				.and_then(|cover| cover.extra_large.clone()),
			volume_count: self.volumes,
		}
	}

	fn to_media_metadata(
		&self,
		tags_score_threshold: i32,
		tags_size_limit: usize,
	) -> ExternalMediaMetadata {
		let (writers, artists) = self.staff_roles();
		let start_date = self.start_date.as_ref();

		ExternalMediaMetadata {
			provider: "anilist".to_string(),
			external_id: self.id.to_string(),
			title: self.title_preferred().map(str::to_string),
			// AniList descriptions are HTML-ish markdown; strip the markup.
			summary: self.description.as_deref().map(strip_html),
			page_count: self.chapters,
			// An AniList `Media` is the series itself, so the series name is
			// the romaji (or native) title.
			series_name: self.title_romaji_or_native().map(str::to_string),
			series_external_id: Some(self.id.to_string()),
			number: None,
			day: start_date.and_then(|date| date.day),
			month: start_date.and_then(|date| date.month),
			year: start_date.and_then(|date| date.year),
			genres: self.genres.clone(),
			tags: Some(self.filtered_tags(tags_score_threshold, tags_size_limit)),
			writers: Some(writers),
			artists: Some(artists),
			cover_url: self
				.cover_image
				.as_ref()
				.and_then(|cover| cover.extra_large.clone()),
			provider_url: Some(format!("https://anilist.co/manga/{}", self.id)),
			..Default::default()
		}
	}
}

/// Remove a trailing parenthesised qualifier, e.g. `"Story (design)"` becomes
/// `"Story"` (Komf: `role.replace("\\([^)]*\\)".toRegex(), "").trim()`).
fn strip_parenthetical(role: &str) -> String {
	let trimmed = role.trim();
	match (trimmed.find('('), trimmed.rfind(')')) {
		(Some(open), Some(close)) if close > open => {
			let without = format!("{}{}", &trimmed[..open], &trimmed[close + 1..]);
			without.trim().to_string()
		},
		_ => trimmed.to_string(),
	}
}

/// Strip HTML markup and decode the entities AniList descriptions actually
/// use (Komf runs the description through Ksoup and takes `wholeText()`).
/// Tags are removed, entities are decoded, and whitespace runs collapse.
fn strip_html(input: &str) -> String {
	let mut out = String::with_capacity(input.len());
	let mut chars = input.chars().peekable();

	while let Some(ch) = chars.next() {
		match ch {
			'<' => {
				// Block separators like <br> contribute a space so adjacent
				// text nodes do not fuse ("ahoy<br/>&amp;" -> "ahoy &").
				let mut tag_name = String::new();
				let mut is_block = false;
				for skipped in chars.by_ref() {
					if skipped == '>' {
						break;
					}
					if skipped.is_ascii_alphanumeric() && tag_name.len() < 4 {
						tag_name.push(skipped.to_ascii_lowercase());
					} else {
						is_block |= tag_name == "br";
					}
				}
				is_block |= tag_name == "br";
				if is_block {
					out.push(' ');
				}
			},
			'&' => {
				let lookahead: String = chars.clone().take(12).collect();
				match decode_entity(&lookahead) {
					Some((decoded, consumed)) => {
						for _ in 0..consumed {
							chars.next();
						}
						out.push(decoded);
					},
					None => out.push('&'),
				}
			},
			_ => out.push(ch),
		}
	}

	out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Decode one entity from `rest` (the text after `&`). Returns the decoded
/// character and the number of chars consumed (excluding the leading `&`).
fn decode_entity(rest: &str) -> Option<(char, usize)> {
	let end = rest.find(';')?;
	let name = &rest[..end];
	let decoded = match name {
		"amp" => '&',
		"lt" => '<',
		"gt" => '>',
		"quot" => '"',
		"apos" => '\'',
		"nbsp" => '\u{00a0}',
		"ndash" => '\u{2013}',
		"mdash" => '\u{2014}',
		"hellip" => '\u{2026}',
		other => {
			// Hex forms must be checked before the decimal prefix, since
			// "#x42" also strips under '#'.
			let code = if let Some(hex) = other
				.strip_prefix("#x")
				.or_else(|| other.strip_prefix("#X"))
			{
				u32::from_str_radix(hex, 16).ok()?
			} else {
				u32::from_str_radix(other.strip_prefix('#')?, 10).ok()?
			};
			char::from_u32(code)?
		},
	};
	Some((decoded, end + 1))
}

#[cfg(test)]
mod tests {
	use super::*;

	use crate::mock_http::{render_ok, MockServer};

	fn get_test_client() -> AniListClient {
		dotenvy::dotenv().ok();
		// No token required for AniList; live tests only need network access.
		AniListClient::new(None, Some(u32::MAX))
	}

	fn sample_media_json(id: i64, title_english: &str) -> serde_json::Value {
		serde_json::json!({
			"id": id,
			"title": {
				"romaji": "One Piece",
				"english": title_english,
				"native": "ONE PIECE"
			},
			"status": "RELEASING",
			"description": "<p>Pirates <b>ahoy</b><br/>&amp; adventure</p>",
			"chapters": 1100,
			"volumes": 105,
			"coverImage": { "extraLarge": "https://s4.anilist.co/cover.jpg" },
			"startDate": { "year": 1997, "month": 7, "day": 22 },
			"genres": ["Action", "Adventure"],
			"synonyms": ["OP"],
			"tags": [
				{ "name": "Shounen", "rank": 95 },
				{ "name": "Pirates", "rank": 80 },
				{ "name": "Ensemble Cast", "rank": 60 },
				{ "name": "Episodic", "rank": 50 }
			],
			"staff": {
				"edges": [
					{
						"node": { "name": { "full": "Eiichiro Oda" } },
						"role": "Story & Art"
					},
					{
						"node": { "name": { "full": "Someone Else" } },
						"role": "Color (digital)"
					}
				]
			}
		})
	}

	fn search_response_body(media: serde_json::Value) -> String {
		serde_json::json!({
			"data": {
				"mediaSearch": { "media": [media] }
			}
		})
		.to_string()
	}

	fn media_response_body(media: serde_json::Value) -> String {
		serde_json::json!({ "data": { "media": media } }).to_string()
	}

	#[tokio::test]
	async fn search_media_maps_mocked_response() {
		let media = sample_media_json(30013, "One Piece");
		let server = MockServer::spawn(vec![
			render_ok(&search_response_body(media.clone())),
			render_ok(&media_response_body(media)),
		]);
		let client = get_test_client().with_api_url(server.url.clone());

		let query = SearchQuery {
			title: "One Piece".to_string(),
			limit: Some(5),
			..Default::default()
		};
		let outcome = client
			.search_media(&query)
			.await
			.expect("search should succeed against the mock");

		assert_eq!(outcome.requested, 1);
		assert_eq!(outcome.candidates.len(), 1);
		let candidate = &outcome.candidates[0];
		assert_eq!(candidate.provider, "anilist");
		assert_eq!(candidate.external_id, "30013");

		let media = candidate.metadata.as_media().unwrap();
		assert_eq!(media.title.as_deref(), Some("One Piece"));
		assert_eq!(media.series_name.as_deref(), Some("One Piece"));
		assert_eq!(media.series_external_id.as_deref(), Some("30013"));
		assert_eq!(media.year, Some(1997));
		assert_eq!(media.month, Some(7));
		assert_eq!(media.day, Some(22));
		assert_eq!(media.page_count, Some(1100));
		assert_eq!(
			media.cover_url.as_deref(),
			Some("https://s4.anilist.co/cover.jpg")
		);
		assert_eq!(media.summary.as_deref(), Some("Pirates ahoy & adventure"));
		assert_eq!(media.number, None);
		// "Story & Art" maps to both writers and artists; unlisted roles drop.
		assert_eq!(
			media.writers.as_deref(),
			Some(["Eiichiro Oda".to_string()].as_slice())
		);
		assert_eq!(
			media.artists.as_deref(),
			Some(["Eiichiro Oda".to_string()].as_slice())
		);
		// Default threshold is 60: the rank-50 tag is excluded.
		assert_eq!(
			media.tags.as_deref(),
			Some(
				[
					"Shounen".to_string(),
					"Pirates".to_string(),
					"Ensemble Cast".to_string()
				]
				.as_slice()
			)
		);
		assert_eq!(
			media.provider_url.as_deref(),
			Some("https://anilist.co/manga/30013")
		);

		let requests = server.requests();
		// A single search request; results are mapped straight from the
		// search response without a per-hit detail fetch.
		assert_eq!(requests.len(), 1);
		// The search POSTs a GraphQL query with variables and the current package
		// version in the Stump user agent.
		assert!(requests[0].starts_with("POST /"));
		let expected_user_agent = format!(
			"user-agent: stump/{} (+https://github.com/stumpapp/stump)",
			env!("CARGO_PKG_VERSION")
		);
		assert!(requests[0].to_lowercase().contains(&expected_user_agent));
		assert!(requests[0].contains("query Search"));
		assert!(requests[0].contains("format_in"));
		assert!(requests[0].contains(r#""formats":["MANGA","ONE_SHOT"]"#));
		assert!(requests[0].contains(r#""search":"One Piece""#));
	}

	#[tokio::test]
	async fn fetch_media_metadata_maps_mocked_response() {
		let server = MockServer::spawn(vec![render_ok(&media_response_body(
			sample_media_json(30013, "One Piece"),
		))]);
		let client = get_test_client().with_api_url(server.url.clone());

		let media = client
			.fetch_media_metadata("30013")
			.await
			.expect("fetch should succeed against the mock");

		assert_eq!(media.provider, "anilist");
		assert_eq!(media.external_id, "30013");
		assert_eq!(media.title.as_deref(), Some("One Piece"));
		assert_eq!(media.year, Some(1997));
	}

	#[tokio::test]
	async fn search_series_maps_mocked_response() {
		let mut media = sample_media_json(30013, "One Piece");
		media["status"] = serde_json::json!("FINISHED");
		media["synonyms"] = serde_json::json!(["OP", "One Piece", "  "]);
		media["title"]["english"] = serde_json::json!(null);

		let server =
			MockServer::spawn(vec![render_ok(&search_response_body(media.clone()))]);
		let client = get_test_client().with_api_url(server.url.clone());

		let query = SearchQuery {
			title: "One Piece".to_string(),
			limit: Some(5),
			..Default::default()
		};
		let outcome = client
			.search_series(&query)
			.await
			.expect("search should succeed against the mock");

		assert_eq!(outcome.requested, 1);
		let series = outcome.candidates[0].metadata.as_series().unwrap();
		// English title is absent: falls back to romaji.
		assert_eq!(series.title, "One Piece");
		assert_eq!(series.status, Some(PublicationStatus::Completed));
		assert_eq!(series.volume_count, Some(105));
		// Synonyms dedupe against the title and drop empties.
		assert_eq!(
			series.alternative_titles,
			vec!["ONE PIECE".to_string(), "OP".to_string()]
		);
		assert_eq!(
			series.cover_url.as_deref(),
			Some("https://s4.anilist.co/cover.jpg")
		);
	}

	#[tokio::test]
	async fn tag_threshold_and_limit_settings_apply() {
		let server = MockServer::spawn(vec![render_ok(&media_response_body(
			sample_media_json(30013, "One Piece"),
		))]);
		let client = get_test_client()
			.with_api_url(server.url.clone())
			.with_tags_score_threshold(70)
			.with_tags_size_limit(1);

		let media = client
			.fetch_media_metadata("30013")
			.await
			.expect("fetch should succeed against the mock");

		// Threshold 70 excludes the rank-60 tag; limit 1 keeps only the top tag.
		assert_eq!(
			media.tags.as_deref(),
			Some(["Shounen".to_string()].as_slice())
		);
	}

	#[tokio::test]
	async fn staff_roles_follow_komf_mapping() {
		let media = AniListMedia {
			id: 1,
			title: None,
			status: None,
			description: None,
			chapters: None,
			volumes: None,
			cover_image: None,
			start_date: None,
			genres: None,
			synonyms: None,
			tags: None,
			staff: Some(AniListStaffConnection {
				edges: vec![
					edge(Some("Story Author"), Some("Story")),
					edge(Some("Art Person"), Some("Illustration")),
					edge(Some("Both Person"), Some("Story & Art (double-sided)")),
					edge(Some("Original Person"), Some("Original Creator")),
					edge(Some("Letterer Person"), Some("Lettering")),
					edge(None, Some("Art")),
					edge(Some("No Role Person"), None),
				],
			}),
		};

		let (writers, artists) = media.staff_roles();
		assert_eq!(
			writers,
			vec![
				"Story Author".to_string(),
				"Both Person".to_string(),
				"Original Person".to_string()
			]
		);
		assert_eq!(
			artists,
			vec!["Art Person".to_string(), "Both Person".to_string()]
		);
	}

	fn edge(name: Option<&str>, role: Option<&str>) -> AniListStaffEdge {
		AniListStaffEdge {
			node: name.map(|name| AniListStaff {
				name: Some(AniListStaffName {
					full: Some(name.to_string()),
				}),
			}),
			role: role.map(str::to_string),
		}
	}

	#[tokio::test]
	async fn rate_limited_response_maps_to_rate_limited() {
		// The retry middleware retries 429s up to `max_retries` times, so the
		// mock needs one response per attempt (initial + 3 retries).
		let server = MockServer::spawn(vec![
			render_rate_limited(),
			render_rate_limited(),
			render_rate_limited(),
			render_rate_limited(),
		]);
		let client = get_test_client().with_api_url(server.url.clone());

		let query = SearchQuery {
			title: "One Piece".to_string(),
			limit: Some(5),
			..Default::default()
		};
		let error = client
			.search_media(&query)
			.await
			.expect_err("search should fail after exhausting retries");

		assert!(
			error.is_rate_limited(),
			"expected a rate-limited classification, got: {error:?}"
		);
	}

	fn render_rate_limited() -> String {
		"HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_string()
	}

	#[tokio::test]
	async fn malformed_json_maps_to_parse_error() {
		let server = MockServer::spawn(vec![render_ok("not json at all")]);
		let client = get_test_client().with_api_url(server.url.clone());

		let query = SearchQuery {
			title: "One Piece".to_string(),
			limit: Some(5),
			..Default::default()
		};
		let error = client
			.search_media(&query)
			.await
			.expect_err("malformed JSON should fail");

		assert!(
			matches!(error, MetadataProviderError::ParseError(_)),
			"expected a parse error, got: {error:?}"
		);
		assert!(!error.is_rate_limited());
	}

	#[tokio::test]
	async fn graphql_errors_are_surfaced() {
		let body = serde_json::json!({
			"errors": [{ "message": "Not Found." }]
		})
		.to_string();
		let server = MockServer::spawn(vec![render_ok(&body)]);
		let client = get_test_client().with_api_url(server.url.clone());

		let error = client
			.fetch_media_metadata("1")
			.await
			.expect_err("GraphQL errors should fail");

		assert!(
			matches!(&error, MetadataProviderError::Other(message) if message.contains("Not Found")),
			"expected the GraphQL error to be surfaced, got: {error:?}"
		);
	}

	#[ignore = "Requires network access; set ANILIST_LIVE_TESTS=1"]
	#[tokio::test]
	async fn live_search_media() {
		if std::env::var("ANILIST_LIVE_TESTS").as_deref() != Ok("1") {
			return;
		}
		let client = get_test_client();
		let query = SearchQuery {
			title: "One Piece".to_string(),
			limit: Some(5),
			..Default::default()
		};
		let outcome = client.search_media(&query).await.expect("live search");
		assert!(!outcome.candidates.is_empty());
		println!("Found {} media candidates", outcome.candidates.len());
	}

	#[ignore = "Requires network access; set ANILIST_LIVE_TESTS=1"]
	#[tokio::test]
	async fn live_fetch_media_metadata() {
		if std::env::var("ANILIST_LIVE_TESTS").as_deref() != Ok("1") {
			return;
		}
		let client = get_test_client();
		// Berserk, a stable long-running AniList entry.
		let media = client
			.fetch_media_metadata("30013")
			.await
			.expect("live fetch");
		println!("Fetched: {:#?}", media);
		assert_eq!(media.provider, "anilist");
	}

	#[test]
	fn strip_html_handles_anilist_descriptions() {
		assert_eq!(
			strip_html("<p>Pirates <b>ahoy</b><br/>&amp; adventure</p>"),
			"Pirates ahoy & adventure"
		);
		assert_eq!(
			strip_html("Line one.<br />Line two.&mdash;done &#65;&#x42;"),
			"Line one. Line two.\u{2014}done AB"
		);
		assert_eq!(strip_html(""), "");
		assert_eq!(strip_html("plain text"), "plain text");
		// Unterminated tags and unknown entities pass through safely.
		assert_eq!(strip_html("oops <div and gone"), "oops");
		assert_eq!(strip_html("5 & 6 &amp; 7"), "5 & 6 & 7");
	}

	#[test]
	fn strip_parenthetical_matches_komf() {
		assert_eq!(strip_parenthetical("Story & Art"), "Story & Art");
		assert_eq!(strip_parenthetical("Story (design)"), "Story");
		assert_eq!(strip_parenthetical("  Art  "), "Art");
	}
}
