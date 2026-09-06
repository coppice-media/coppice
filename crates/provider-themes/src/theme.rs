//! Plumbing every engine shares: knob resolution, lazy-image attribute
//! precedence, URL building, and the `RemoteSeries`/`RemoteChapter` shapes.
//!
//! Knob names are snake_case renderings of the Kotlin property or
//! single-expression function they come from; each engine's module documents
//! which base class member each one is. Legacy aliases are accepted so a
//! definition generated from an older upstream pin still runs — the alias list
//! lives next to the knob it feeds.

use std::{sync::Arc, time::Duration};

use stump_provider::{
	definition::SourceDefinition, HtmlDocument, RateLimiter, RemoteChapter, RemotePage,
	RemoteSeries, RequestHeaders, SeriesStatus, SourceCapabilities, SourceError,
	SourceHttp, SourceInfo, SourceResult,
};

use crate::{
	dom::{Document, NodeId},
	selector::{Selector, SelectorError},
};

/// Politeness floor for a site nobody measured: two requests a second. Mihon
/// leaves themed sources unlimited and relies on the app being interactive; a
/// server browses on behalf of many users, so it must not.
pub const DEFAULT_REQUESTS_PER_SECOND: u32 = 2;

/// Applied when a definition declares a rate limit that cannot be read. Slower
/// than the default on purpose: the knob's presence means the site pushed back.
pub const CLAMPED_REQUESTS_PER_SECOND: u32 = 1;

/// `X-Requested-With` marker every theme sets on its AJAX calls.
pub const XHR_HEADER: (&str, &str) = ("X-Requested-With", "XMLHttpRequest");

/// Errors raised while turning a definition into an engine. All of them mean
/// the definition is wrong, so they surface at `enableProviderSource` time
/// rather than as an empty browse page.
#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
	#[error("source definition `{id}` has an invalid `{knob}`: {source}")]
	Selector {
		id: String,
		knob: &'static str,
		#[source]
		source: SelectorError,
	},
	#[error("failed to build the HTTP client for `{id}`: {source}")]
	Http {
		id: String,
		#[source]
		source: reqwest::Error,
	},
}

/// The parsed, ready-to-run configuration shared by all three engines.
#[derive(Debug)]
pub struct ThemeContext {
	pub info: SourceInfo,
	pub http: SourceHttp,
	/// Base URL without a trailing slash.
	pub base_url: String,
}

impl ThemeContext {
	pub fn new(
		definition: &SourceDefinition,
		instance_id: &str,
		base_url_override: Option<&str>,
		capabilities: SourceCapabilities,
		headers: RequestHeaders,
	) -> Result<Self, ThemeError> {
		let limiter = rate_limiter(definition);
		let http = SourceHttp::with_limiter(limiter)
			.map_err(|source| ThemeError::Http {
				id: definition.id.clone(),
				source,
			})?
			.with_headers(headers);
		let base_url = base_url_override
			.map(|url| url.trim_end_matches('/').to_string())
			.filter(|url| !url.is_empty())
			.unwrap_or_else(|| definition.base_url().to_string());
		Ok(Self {
			info: SourceInfo {
				id: instance_id.to_string(),
				name: definition.name.clone(),
				lang: definition.lang().to_string(),
				base_url: base_url.clone(),
				capabilities,
			},
			http,
			base_url,
		})
	}

	/// `{base}{path}`, where `path` may be absolute, root-relative, or already
	/// a full URL.
	pub fn url(&self, path: &str) -> String {
		if path.starts_with("http://") || path.starts_with("https://") {
			return path.to_string();
		}
		if path.is_empty() {
			return self.base_url.clone();
		}
		if path.starts_with('/') {
			format!("{}{path}", self.base_url)
		} else {
			format!("{}/{path}", self.base_url)
		}
	}

	pub async fn get(&self, url: &str) -> SourceResult<Document> {
		self.get_with(url, &[]).await
	}

	pub async fn get_with(
		&self,
		url: &str,
		headers: &[(&str, &str)],
	) -> SourceResult<Document> {
		let mut all = vec![("Referer", self.base_url.as_str())];
		all.extend_from_slice(headers);
		let fetched = self.http.get_text(url, &all).await?;
		Ok(parse(fetched))
	}

	pub async fn post_form(
		&self,
		url: &str,
		form: &[(String, String)],
	) -> SourceResult<Document> {
		let headers = [XHR_HEADER, ("Referer", self.base_url.as_str())];
		let fetched = self.http.post_form(url, &headers, form).await?;
		Ok(parse(fetched))
	}

	/// Rate-limited GET of a JSON body (MMRCMS' search suggestions).
	pub async fn get_json<T: serde::de::DeserializeOwned>(
		&self,
		url: &str,
	) -> SourceResult<T> {
		self.http.get_json(url, &[]).await
	}
}

fn parse(fetched: HtmlDocument) -> Document {
	Document::parse(&fetched.body, &fetched.url)
}

/// The limiter a definition asks for, honouring
/// `rate_limit_permits`/`rate_limit_period_seconds` exactly. Upstream's
/// `rateLimit(n)` defaults to one second per window.
pub fn rate_limiter(definition: &SourceDefinition) -> RateLimiter {
	const PERMIT_KEYS: [&str; 2] = ["rate_limit_permits", "rate_limit"];
	const PERIOD_KEYS: [&str; 2] = ["rate_limit_period_seconds", "rate_limit_period"];
	let declared = definition.knob(&PERMIT_KEYS).is_some()
		|| definition.knob(&PERIOD_KEYS).is_some();
	if !declared {
		return RateLimiter::per_second(DEFAULT_REQUESTS_PER_SECOND);
	}
	let permits = definition.int_knob(&PERMIT_KEYS);
	let period = definition.int_knob(&PERIOD_KEYS);
	let Some(permits) = permits.filter(|permits| *permits > 0) else {
		tracing::warn!(
			source = definition.id,
			"Definition declares a rate limit without a usable permit count; \
			 clamping to {CLAMPED_REQUESTS_PER_SECOND} req/s"
		);
		return RateLimiter::per_second(CLAMPED_REQUESTS_PER_SECOND);
	};
	let period = period.filter(|period| *period > 0).unwrap_or(1);
	RateLimiter::new(
		u32::try_from(permits).unwrap_or(CLAMPED_REQUESTS_PER_SECOND),
		Duration::from_secs(period as u64),
	)
}

/// Resolve a selector knob, falling back to the base class default.
pub fn selector(
	definition: &SourceDefinition,
	knob: &'static str,
	aliases: &[&str],
	default: &str,
) -> Result<Selector, ThemeError> {
	let mut keys = Vec::with_capacity(aliases.len() + 1);
	keys.push(knob);
	keys.extend_from_slice(aliases);
	let source = definition.text_knob(&keys).unwrap_or(default);
	Selector::parse(source).map_err(|source| ThemeError::Selector {
		id: definition.id.clone(),
		knob,
		source,
	})
}

/// Resolve an optional selector knob: `None` when neither the knob nor a
/// default is present.
pub fn optional_selector(
	definition: &SourceDefinition,
	knob: &'static str,
	aliases: &[&str],
	default: Option<&str>,
) -> Result<Option<Selector>, ThemeError> {
	let mut keys = Vec::with_capacity(aliases.len() + 1);
	keys.push(knob);
	keys.extend_from_slice(aliases);
	let Some(source) = definition.text_knob(&keys).or(default) else {
		return Ok(None);
	};
	if source == SELF_SELECTOR {
		return Ok(None);
	}
	Selector::parse(source)
		.map(Some)
		.map_err(|source| ThemeError::Selector {
			id: definition.id.clone(),
			knob,
			source,
		})
}

/// Sentinel a definition uses when the matched element *is* the target (the
/// list item is itself the anchor). `optional_selector` yields `None` for it,
/// and the engines then read the item element directly.
pub const SELF_SELECTOR: &str = ":self";

/// Whether a knob is the `:self` sentinel rather than a real selector.
pub fn is_self_selector(definition: &SourceDefinition, keys: &[&str]) -> bool {
	definition.text_knob(keys) == Some(SELF_SELECTOR)
}

/// The lazy-image attribute order a theme's `imgAttr`/`imageFromElement`
/// helper uses. Precedence matters: a lazy-loading site puts a placeholder in
/// `src`, so reading `src` first yields the same 1×1 GIF for every page.
pub struct ImageAttrs(pub &'static [&'static str]);

impl ImageAttrs {
	/// First resolvable attribute in order, with `srcset` reduced to its
	/// highest-descriptor candidate.
	pub fn resolve(&self, document: &Document, node: NodeId) -> Option<String> {
		for name in self.0 {
			if *name == "srcset" {
				if let Some(value) = document
					.attr(node, "srcset")
					.and_then(largest_srcset_candidate)
					.and_then(|candidate| document.resolve(&candidate))
				{
					return Some(value);
				}
				continue;
			}
			if let Some(value) = document.abs_attr(node, name) {
				return Some(value);
			}
		}
		None
	}
}

/// jsoup-side `getSrcSetImage`: the candidate with the largest `w`/`x`
/// descriptor, else the lexicographically largest URL.
fn largest_srcset_candidate(srcset: &str) -> Option<String> {
	let mut best: Option<(f32, &str)> = None;
	let mut fallback: Option<&str> = None;
	for candidate in srcset.split(',') {
		let mut parts = candidate.trim().split_ascii_whitespace();
		let Some(url) = parts.next() else { continue };
		if url.is_empty() {
			continue;
		}
		if fallback.is_none_or(|current| url > current) {
			fallback = Some(url);
		}
		let descriptor = parts.next().and_then(|descriptor| {
			descriptor
				.strip_suffix('w')
				.or_else(|| descriptor.strip_suffix('x'))
				.and_then(|value| value.parse::<f32>().ok())
		});
		if let Some(descriptor) = descriptor {
			if best.is_none_or(|(current, _)| descriptor > current) {
				best = Some((descriptor, url));
			}
		}
	}
	best.map(|(_, url)| url).or(fallback).map(str::to_string)
}

/// Map a status label to [`SeriesStatus`] using the multilingual vocabularies
/// the base classes carry (`completedStatus`/`ongoingStatus`/`hiatusStatus`/
/// `cancelledStatus` in `MadaraBase.kt`, `parseStatus` in `MangaThemesia.kt`,
/// `detailStatus*` in `MMRCMS.kt`), merged into one table.
pub fn parse_status(label: &str) -> SeriesStatus {
	let lower = label.to_lowercase();
	let contains = |words: &[&str]| words.iter().any(|word| lower.contains(word));
	// Cancelled first: "dropped"/"cancelado" would also match "completo".
	if contains(&CANCELLED_STATUS) {
		return SeriesStatus::Cancelled;
	}
	if contains(&HIATUS_STATUS) {
		return SeriesStatus::Hiatus;
	}
	if contains(&COMPLETED_STATUS) {
		return SeriesStatus::Completed;
	}
	if contains(&ONGOING_STATUS) {
		return SeriesStatus::Ongoing;
	}
	SeriesStatus::Unknown
}

const COMPLETED_STATUS: [&str; 27] = [
	"completed",
	"completo",
	"completado",
	"completata",
	"complété",
	"complet",
	"concluído",
	"concluido",
	"finalizado",
	"finished",
	"achevé",
	"terminé",
	"hoàn thành",
	"đã hoàn thành",
	"مكتملة",
	"مكتمل",
	"已完结",
	"完結",
	"tamamlandı",
	"tamamlanan",
	"bitti",
	"bitmiş",
	"tamat",
	"zakończone",
	"завершено",
	"one-shot",
	"مكتملة",
];
const ONGOING_STATUS: [&str; 34] = [
	"ongoing",
	"on going",
	"updating",
	"publishing",
	"em lançamento",
	"em andamento",
	"em postagem",
	"em progresso",
	"lançando",
	"en cours",
	"ativo",
	"activo",
	"đang tiến hành",
	"đang làm",
	"còn nữa",
	"devam ediyor",
	"devam eden",
	"devam etmekte",
	"in corso",
	"in arrivo",
	"güncel",
	"berjalan",
	"prace w toku",
	"продолжается",
	"онгоінг",
	"مستمرة",
	"مستمر",
	"en curso",
	"curso",
	"en marcha",
	"publicandose",
	"publicándose",
	"en emision",
	"连载中",
];
const HIATUS_STATUS: [&str; 12] = [
	"on hold",
	"hiatus",
	"hiato",
	"pausado",
	"en espera",
	"en pause",
	"en attente",
	"durduruldu",
	"beklemede",
	"đang chờ",
	"متوقف",
	"заморожено",
];
const CANCELLED_STATUS: [&str; 11] = [
	"canceled",
	"cancelled",
	"cancelado",
	"cancelados",
	"cancellato",
	"dropped",
	"discontinued",
	"iptal edildi",
	"đã hủy",
	"ملغي",
	"abandonné",
];

/// A `RemoteSeries` with the fields an HTML theme can fill.
pub fn series(
	remote_id: impl Into<String>,
	title: impl Into<String>,
	url: Option<String>,
	thumbnail_url: Option<String>,
	nsfw: bool,
) -> RemoteSeries {
	RemoteSeries {
		remote_id: remote_id.into(),
		title: title.into(),
		url,
		thumbnail_url,
		nsfw,
		..Default::default()
	}
}

/// A `RemoteChapter` with the fields an HTML theme can fill.
///
/// Single construction point on purpose: `RemoteChapter` grows fields in the
/// provider crate, and this keeps the three engines out of that churn.
pub fn chapter(
	remote_id: impl Into<String>,
	title: Option<String>,
	number: Option<f32>,
	uploaded_at: Option<chrono::DateTime<chrono::Utc>>,
	url: Option<String>,
	lang: &str,
) -> RemoteChapter {
	RemoteChapter {
		remote_id: remote_id.into(),
		title,
		number,
		volume: None,
		lang: Some(lang.to_string()),
		scanlator: None,
		uploaded_at,
		url,
		page_count: None,
		..Default::default()
	}
}

/// Pages with the `Referer` every theme's `imageRequest` sets to the chapter
/// URL; without it a Madara or MangaThemesia CDN answers 403.
pub fn pages_with_referer(urls: Vec<String>, referer: &str) -> Vec<RemotePage> {
	urls.into_iter()
		.enumerate()
		.map(|(index, url)| RemotePage {
			index: index as u32,
			url,
			headers: vec![
				("Referer".to_string(), referer.to_string()),
				(
					"Accept".to_string(),
					"image/avif,image/webp,image/png,image/jpeg,*/*".to_string(),
				),
			],
		})
		.collect()
}

/// The chapter number Mihon derives from a chapter name: the first decimal in
/// the label, ignoring a leading volume marker.
pub fn chapter_number(name: &str) -> Option<f32> {
	let lower = name.to_lowercase();
	let after_marker = [
		"chapter",
		"chap",
		"ch.",
		"ch ",
		"capítulo",
		"capitulo",
		"chapitre",
		"bölüm",
		"글",
		"화",
		"話",
	]
	.iter()
	.filter_map(|marker| lower.find(marker).map(|at| at + marker.len()))
	.min();
	let haystack = match after_marker {
		Some(at) => &lower[at..],
		None => lower.as_str(),
	};
	first_decimal(haystack).or_else(|| first_decimal(&lower))
}

fn first_decimal(value: &str) -> Option<f32> {
	let bytes = value.as_bytes();
	let mut index = 0;
	while index < bytes.len() {
		if !bytes[index].is_ascii_digit() {
			index += 1;
			continue;
		}
		let start = index;
		while index < bytes.len() && bytes[index].is_ascii_digit() {
			index += 1;
		}
		if index < bytes.len() && (bytes[index] == b'.' || bytes[index] == b',') {
			let separator = index;
			index += 1;
			let fraction = index;
			while index < bytes.len() && bytes[index].is_ascii_digit() {
				index += 1;
			}
			if index > fraction {
				let mut number = value[start..separator].to_string();
				number.push('.');
				number.push_str(&value[fraction..index]);
				return number.parse().ok();
			}
			index = separator;
		}
		return value[start..index].parse().ok();
	}
	None
}

/// The last non-empty path segment of a URL: the slug every engine uses as its
/// deterministic remote id.
pub fn slug_of(url: &str) -> Option<String> {
	let path = url
		.split(['?', '#'])
		.next()
		.unwrap_or(url)
		.trim_end_matches('/');
	path.rsplit('/')
		.find(|segment| !segment.is_empty())
		.map(|segment| segment.to_string())
}

/// `Unsupported` for a capability a definition switched off.
pub fn unsupported(info: &SourceInfo, operation: &'static str) -> SourceError {
	SourceError::Unsupported {
		source_id: info.id.clone(),
		operation,
	}
}

/// Boxed engine constructor result.
pub type BuildResult = Result<Arc<dyn stump_provider::Source>, ThemeError>;

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::BTreeMap;
	use stump_provider::KnobValue;

	fn definition() -> SourceDefinition {
		SourceDefinition {
			schema: 1,
			id: "en.example".into(),
			name: "Example".into(),
			lang: "en".into(),
			base_url: "https://example.com".into(),
			theme: "madara".into(),
			version: 1,
			nsfw: false,
			knobs: BTreeMap::new(),
			upstream: None,
		}
	}

	#[test]
	fn rate_limits_honour_the_declared_permits_and_period() {
		let limiter = rate_limiter(&definition());
		assert_eq!(limiter.requests(), DEFAULT_REQUESTS_PER_SECOND);
		assert_eq!(limiter.per(), Duration::from_secs(1));

		let mut declared = definition();
		declared
			.knobs
			.insert("rate_limit_permits".into(), KnobValue::Int(3));
		let limiter = rate_limiter(&declared);
		assert_eq!(limiter.requests(), 3);
		assert_eq!(limiter.per(), Duration::from_secs(1));

		declared
			.knobs
			.insert("rate_limit_period_seconds".into(), KnobValue::Int(60));
		let limiter = rate_limiter(&declared);
		assert_eq!(limiter.requests(), 3);
		assert_eq!(limiter.per(), Duration::from_secs(60));

		// A declared-but-unreadable limit means the site pushed back: clamp.
		let mut broken = definition();
		broken
			.knobs
			.insert("rate_limit_permits".into(), KnobValue::Text("many".into()));
		assert_eq!(
			rate_limiter(&broken).requests(),
			CLAMPED_REQUESTS_PER_SECOND
		);
	}

	#[test]
	fn selector_knobs_override_the_base_class_default() {
		let mut definition = definition();
		assert_eq!(
			selector(
				&definition,
				"chapter_list_selector",
				&[],
				"li.wp-manga-chapter"
			)
			.unwrap()
			.source(),
			"li.wp-manga-chapter"
		);
		definition.knobs.insert(
			"chapter_list_selector".into(),
			KnobValue::Text(".custom > a".into()),
		);
		assert_eq!(
			selector(
				&definition,
				"chapter_list_selector",
				&[],
				"li.wp-manga-chapter"
			)
			.unwrap()
			.source(),
			".custom > a"
		);

		definition.knobs.insert(
			"chapter_list_selector".into(),
			KnobValue::Text(":nth-last-of-type(2)".into()),
		);
		let error =
			selector(&definition, "chapter_list_selector", &[], "li").unwrap_err();
		assert!(
			matches!(error, ThemeError::Selector { knob, .. } if knob == "chapter_list_selector"),
			"{error}"
		);
	}

	#[test]
	fn self_selector_sentinel_means_the_item_itself() {
		let mut definition = definition();
		definition
			.knobs
			.insert("manga_url_selector".into(), KnobValue::Text(":self".into()));
		assert!(
			optional_selector(&definition, "manga_url_selector", &[], Some("a"))
				.unwrap()
				.is_none()
		);
		assert!(is_self_selector(&definition, &["manga_url_selector"]));
	}

	#[test]
	fn lazy_image_attributes_beat_placeholder_src() {
		let document = Document::parse(
			r#"<div>
				<img id="lazy" src="/placeholder.gif" data-src="/real/1.jpg">
				<img id="cf" src="/placeholder.gif" data-cfsrc="/real/2.jpg">
				<img id="set" srcset="/small.jpg 320w, /large.jpg 1280w">
				<img id="plain" src="/plain.jpg">
				<img id="none">
			</div>"#,
			"https://site.test/read/",
		);
		let attrs =
			ImageAttrs(&["data-src", "data-lazy-src", "data-cfsrc", "srcset", "src"]);
		let by_id = |id: &str| {
			document
				.ids()
				.find(|node| document.attr(*node, "id") == Some(id))
				.unwrap()
		};
		assert_eq!(
			attrs.resolve(&document, by_id("lazy")).as_deref(),
			Some("https://site.test/real/1.jpg")
		);
		assert_eq!(
			attrs.resolve(&document, by_id("cf")).as_deref(),
			Some("https://site.test/real/2.jpg")
		);
		assert_eq!(
			attrs.resolve(&document, by_id("set")).as_deref(),
			Some("https://site.test/large.jpg")
		);
		assert_eq!(
			attrs.resolve(&document, by_id("plain")).as_deref(),
			Some("https://site.test/plain.jpg")
		);
		assert_eq!(attrs.resolve(&document, by_id("none")), None);
	}

	#[test]
	fn status_labels_map_across_languages() {
		assert_eq!(parse_status("Ongoing"), SeriesStatus::Ongoing);
		assert_eq!(parse_status("Em lançamento"), SeriesStatus::Ongoing);
		assert_eq!(parse_status("Completed"), SeriesStatus::Completed);
		assert_eq!(parse_status("مكتملة"), SeriesStatus::Completed);
		assert_eq!(parse_status("On Hold"), SeriesStatus::Hiatus);
		assert_eq!(parse_status("Dropped"), SeriesStatus::Cancelled);
		assert_eq!(parse_status("Cancelado"), SeriesStatus::Cancelled);
		assert_eq!(parse_status("???"), SeriesStatus::Unknown);
	}

	#[test]
	fn chapter_numbers_come_from_the_label() {
		assert_eq!(chapter_number("Chapter 12"), Some(12.0));
		assert_eq!(chapter_number("Chapter 12.5 - Extra"), Some(12.5));
		assert_eq!(chapter_number("Ch.7"), Some(7.0));
		assert_eq!(chapter_number("Capítulo 103"), Some(103.0));
		// A volume prefix must not win over the chapter number.
		assert_eq!(chapter_number("Vol.2 Chapter 3"), Some(3.0));
		assert_eq!(chapter_number("One Piece 1105"), Some(1105.0));
		assert_eq!(chapter_number("Prologue"), None);
	}

	#[test]
	fn slugs_are_the_last_path_segment() {
		assert_eq!(
			slug_of("https://site.test/manga/foo-bar/").as_deref(),
			Some("foo-bar")
		);
		assert_eq!(
			slug_of("https://site.test/manga/foo-bar/chapter-1?style=list").as_deref(),
			Some("chapter-1")
		);
		assert_eq!(slug_of("https://site.test/").as_deref(), Some("site.test"));
	}

	#[test]
	fn urls_join_absolute_and_relative_paths() {
		let context = ThemeContext {
			info: SourceInfo {
				id: "x".into(),
				name: "X".into(),
				lang: "en".into(),
				base_url: "https://site.test".into(),
				capabilities: SourceCapabilities::default(),
			},
			http: SourceHttp::with_limiter(RateLimiter::unlimited()).unwrap(),
			base_url: "https://site.test".into(),
		};
		assert_eq!(context.url("/manga/"), "https://site.test/manga/");
		assert_eq!(context.url("manga/"), "https://site.test/manga/");
		assert_eq!(context.url(""), "https://site.test");
		assert_eq!(
			context.url("https://cdn.test/x.jpg"),
			"https://cdn.test/x.jpg"
		);
	}
}
