//! HTTP plumbing shared by sources, the catalog fetcher, and the health
//! checker: the client builder, the per-source rate-limited client, the
//! operator-configured [`RequestHeaders`] sent with every request of a source,
//! and the Cloudflare managed-challenge classifier.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::rate_limit::RateLimiter;

/// User agent sent to every remote source, catalog, and health probe.
pub const USER_AGENT: &str = concat!(
	"Coppice/",
	env!("CARGO_PKG_VERSION"),
	" (+https://github.com/stumpapp/stump)"
);

/// Default per-request timeout for source API calls and page downloads.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Header Cloudflare sets on a response it produced itself; `challenge` means
/// the request was answered with an interstitial rather than by the origin.
pub const CF_MITIGATED_HEADER: &str = "cf-mitigated";

/// Prefix of the `cf-chl-*` headers a challenge response carries
/// (`cf-chl-bypass`, `cf-chl-out`, …).
pub const CF_CHALLENGE_PREFIX: &str = "cf-chl";

/// Build a reqwest client with the Coppice user agent.
///
/// `user_agent` overrides the default for sources that must impersonate a
/// specific client (MangaDex only requires a descriptive one).
pub fn build_client(
	user_agent: Option<&str>,
	timeout: Duration,
) -> Result<reqwest::Client, reqwest::Error> {
	reqwest::Client::builder()
		.user_agent(user_agent.unwrap_or(USER_AGENT))
		.timeout(timeout)
		.build()
}

/// `Some(host)` when `status` plus `headers` is Cloudflare's managed-challenge
/// interstitial rather than the origin refusing the request.
///
/// Since 2026-06 `readcomicsonline.ru` answers every HTML path with
/// `403` + `cf-mitigated: challenge` to any client that cannot run the
/// challenge script. That is not a broken site and not an authorisation
/// failure: the host is up and will answer the moment the request carries a
/// browser's `cf_clearance` cookie, which is why it gets its own error instead
/// of being folded into a generic status failure.
pub fn challenge_host(status: u16, headers: &HeaderMap, url: &str) -> Option<String> {
	if !matches!(status, 403 | 503) {
		return None;
	}
	let mitigated = headers
		.get(CF_MITIGATED_HEADER)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|value| value.trim().eq_ignore_ascii_case("challenge"));
	let marker = headers
		.keys()
		.any(|name| name.as_str().starts_with(CF_CHALLENGE_PREFIX));
	(mitigated || marker).then(|| host_of(url))
}

/// The host of `url`, or the whole string when it does not parse.
pub fn host_of(url: &str) -> String {
	reqwest::Url::parse(url)
		.ok()
		.and_then(|parsed| parsed.host_str().map(str::to_string))
		.unwrap_or_else(|| url.to_string())
}

/// A header name/value pair rejected by [`RequestHeaders::from_pairs`]. The
/// value is never part of the message: an invalid `Cookie` is still a secret.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HeaderError {
	#[error("`{0}` is not a valid HTTP header name")]
	Name(String),
	#[error("the value configured for `{0}` is not a valid HTTP header value")]
	Value(String),
}

/// One configured header as the API is allowed to describe it: the name in
/// full, the value only as a fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaskedHeader {
	pub name: String,
	/// `Mozi…9537` for a long value, `…` for a short one.
	pub preview: String,
	/// Characters in the configured value.
	pub length: usize,
}

/// Operator-configured request headers for one source instance
/// (`provider_sources.request_headers`), sent with every request that source
/// makes.
///
/// The values are credentials — a `cf_clearance` cookie is a session token
/// bound to one User-Agent and IP — so they are never logged and never
/// returned in full by the API. `Debug` prints names and lengths only, so a
/// `?row`-style log line cannot leak one.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct RequestHeaders {
	headers: Arc<Vec<(HeaderName, HeaderValue)>>,
}

impl std::fmt::Debug for RequestHeaders {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let mut map = f.debug_map();
		for (name, value) in self.headers.iter() {
			map.entry(&name.as_str(), &format_args!("<{} bytes>", value.len()));
		}
		map.finish()
	}
}

impl RequestHeaders {
	/// Read a stored `request_headers` JSON object. Anything that is not a
	/// string-valued object entry, or not a valid header, is dropped with a
	/// warning naming the key: the mutation validates on the way in, so a bad
	/// row is a hand-edited database, not an operator mistake to surface.
	pub fn parse(raw: Option<&str>) -> Self {
		let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
			return Self::default();
		};
		let decoded: BTreeMap<String, serde_json::Value> = match serde_json::from_str(raw)
		{
			Ok(decoded) => decoded,
			Err(error) => {
				tracing::warn!(
					%error,
					"provider_sources.request_headers is not a JSON object of strings; ignoring it"
				);
				return Self::default();
			},
		};
		let pairs = decoded.into_iter().filter_map(|(name, value)| match value {
			serde_json::Value::String(value) => Some((name, value)),
			_ => {
				tracing::warn!(header = name, "Ignoring non-string request header");
				None
			},
		});
		let mut headers = Vec::new();
		for (name, value) in pairs {
			match parse_pair(&name, &value) {
				Ok(pair) => headers.push(pair),
				Err(error) => tracing::warn!(%error, "Ignoring request header"),
			}
		}
		Self {
			headers: Arc::new(headers),
		}
	}

	/// Validate an operator-supplied map. Empty names and empty values are
	/// dropped (clearing a header is how one is removed), everything else must
	/// be a legal header or the whole call fails.
	pub fn from_pairs(
		pairs: impl IntoIterator<Item = (String, String)>,
	) -> Result<Self, HeaderError> {
		let mut sorted: BTreeMap<String, String> = BTreeMap::new();
		for (name, value) in pairs {
			let name = name.trim().to_string();
			let value = value.trim().to_string();
			if name.is_empty() || value.is_empty() {
				continue;
			}
			sorted.insert(name, value);
		}
		let mut headers = Vec::with_capacity(sorted.len());
		for (name, value) in &sorted {
			headers.push(parse_pair(name, value)?);
		}
		Ok(Self {
			headers: Arc::new(headers),
		})
	}

	/// These headers with `overrides` applied: an entry replaces the header of
	/// the same name, adds one that was not configured, and — empty, the way
	/// [`RequestHeaders::from_pairs`] treats an empty value — removes one.
	///
	/// Merging is offered instead of a value getter on purpose. A caller that
	/// has to rebuild the map by hand needs to read the stored `cf_clearance`
	/// back out, and a getter for that is a getter anything else can call
	/// too. The only caller is the clearance a browser worker earned
	/// (`crate::challenge`), which replaces `Cookie` and `User-Agent` and must
	/// not drop whatever else the operator configured.
	pub fn merged_with(
		&self,
		overrides: impl IntoIterator<Item = (String, String)>,
	) -> Result<Self, HeaderError> {
		let mut merged: BTreeMap<String, (HeaderName, HeaderValue)> = self
			.headers
			.iter()
			.map(|(name, value)| {
				(name.as_str().to_string(), (name.clone(), value.clone()))
			})
			.collect();
		for (name, value) in overrides {
			let name = name.trim();
			let value = value.trim();
			if name.is_empty() {
				continue;
			}
			let pair = parse_pair(name, value)?;
			let key = pair.0.as_str().to_string();
			if value.is_empty() {
				merged.remove(&key);
			} else {
				merged.insert(key, pair);
			}
		}
		Ok(Self {
			headers: Arc::new(merged.into_values().collect()),
		})
	}

	/// The JSON object to store, or `None` when nothing is configured so the
	/// column goes back to `NULL`.
	pub fn to_json(&self) -> Option<String> {
		if self.is_empty() {
			return None;
		}
		let map: BTreeMap<&str, &str> = self
			.headers
			.iter()
			.filter_map(|(name, value)| Some((name.as_str(), value.to_str().ok()?)))
			.collect();
		serde_json::to_string(&map).ok()
	}

	pub fn is_empty(&self) -> bool {
		self.headers.is_empty()
	}

	pub fn len(&self) -> usize {
		self.headers.len()
	}

	/// What the API returns: names in full, values as fingerprints.
	pub fn masked(&self) -> Vec<MaskedHeader> {
		self.headers
			.iter()
			.map(|(name, value)| {
				let value = value.to_str().unwrap_or_default();
				MaskedHeader {
					name: name.as_str().to_string(),
					preview: mask(value),
					length: value.chars().count(),
				}
			})
			.collect()
	}

	/// Attach the configured headers to a request. Applied before the
	/// caller's own headers so a per-request `Referer` still wins, and after
	/// the client's default user agent so an operator-supplied `User-Agent`
	/// replaces it (reqwest replaces default headers by name).
	pub fn apply(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
		let mut request = request;
		for (name, value) in self.headers.iter() {
			request = request.header(name.clone(), value.clone());
		}
		request
	}
}

fn parse_pair(name: &str, value: &str) -> Result<(HeaderName, HeaderValue), HeaderError> {
	let parsed_name = HeaderName::from_bytes(name.as_bytes())
		.map_err(|_| HeaderError::Name(name.to_string()))?;
	let parsed_value =
		HeaderValue::from_str(value).map_err(|_| HeaderError::Value(name.to_string()))?;
	Ok((parsed_name, parsed_value))
}

/// `abcd…wxyz` for a value long enough that its ends identify it, `…` for one
/// short enough that its ends would be most of it.
fn mask(value: &str) -> String {
	const KEEP: usize = 4;
	let chars: Vec<char> = value.chars().collect();
	if chars.len() <= KEEP * 3 {
		return "…".to_string();
	}
	let head: String = chars[..KEEP].iter().collect();
	let tail: String = chars[chars.len() - KEEP..].iter().collect();
	format!("{head}…{tail}")
}

/// A rate-limited HTTP client owned by one source instance.
#[derive(Clone, Debug)]
pub struct SourceHttp {
	client: reqwest::Client,
	limiter: RateLimiter,
	headers: RequestHeaders,
}

impl SourceHttp {
	pub fn new(client: reqwest::Client, limiter: RateLimiter) -> Self {
		Self {
			client,
			limiter,
			headers: RequestHeaders::default(),
		}
	}

	/// Client with the Stump user agent, default timeout, and the given limiter.
	pub fn with_limiter(limiter: RateLimiter) -> Result<Self, reqwest::Error> {
		Ok(Self::new(build_client(None, DEFAULT_TIMEOUT)?, limiter))
	}

	/// Send the operator's configured headers with every request.
	pub fn with_headers(mut self, headers: RequestHeaders) -> Self {
		self.headers = headers;
		self
	}

	pub fn client(&self) -> &reqwest::Client {
		&self.client
	}

	pub fn limiter(&self) -> &RateLimiter {
		&self.limiter
	}

	pub fn headers(&self) -> &RequestHeaders {
		&self.headers
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_cloudflare_challenge_is_told_apart_from_a_real_rejection() {
		let mut headers = HeaderMap::new();
		headers.insert("cf-mitigated", HeaderValue::from_static("challenge"));
		assert_eq!(
			challenge_host(403, &headers, "https://readcomicsonline.ru/comic/x"),
			Some("readcomicsonline.ru".to_string())
		);
		// The same marker on a 503 (Cloudflare's "just a moment" variant).
		assert_eq!(
			challenge_host(503, &headers, "https://site.test/"),
			Some("site.test".to_string())
		);
		// A plain 403 from the origin is not a challenge.
		assert!(challenge_host(403, &HeaderMap::new(), "https://site.test/").is_none());
		// Nor is a challenge marker on a status that is not a block.
		assert!(challenge_host(200, &headers, "https://site.test/").is_none());
		// `cf-chl-*` alone is enough: not every deployment sets cf-mitigated.
		let mut chl = HeaderMap::new();
		chl.insert("cf-chl-bypass", HeaderValue::from_static("1"));
		assert_eq!(
			challenge_host(403, &chl, "https://site.test/comic"),
			Some("site.test".to_string())
		);
		// An unrelated Cloudflare header is not a challenge.
		let mut cache = HeaderMap::new();
		cache.insert("cf-cache-status", HeaderValue::from_static("DYNAMIC"));
		assert!(challenge_host(403, &cache, "https://site.test/").is_none());
	}

	#[test]
	fn stored_headers_round_trip_and_are_masked_never_echoed() {
		let headers = RequestHeaders::from_pairs([
			(
				"Cookie".to_string(),
				"cf_clearance=abcdefghijklmnop".to_string(),
			),
			("User-Agent".to_string(), "Mozilla/5.0".to_string()),
			// Cleared entries are how a header is removed.
			("X-Gone".to_string(), "  ".to_string()),
		])
		.expect("valid headers");
		assert_eq!(headers.len(), 2);
		let json = headers.to_json().expect("stored json");
		// Header names normalise to lower case: HTTP/2 (what reqwest speaks to
		// Cloudflare) forbids anything else, so the operator's casing cannot
		// be preserved and is not pretended to be.
		assert_eq!(
			json,
			r#"{"cookie":"cf_clearance=abcdefghijklmnop","user-agent":"Mozilla/5.0"}"#
		);
		assert_eq!(RequestHeaders::parse(Some(&json)), headers);

		let masked = headers.masked();
		assert_eq!(masked[0].name, "cookie");
		assert_eq!(masked[0].preview, "cf_c…mnop");
		assert_eq!(masked[0].length, 29);
		// A short value gives nothing away at all.
		assert_eq!(masked[1].preview, "…");
		assert!(!format!("{headers:?}").contains("cf_clearance"));

		// Nothing configured stores NULL rather than `{}`.
		assert!(RequestHeaders::default().to_json().is_none());
		assert!(RequestHeaders::parse(None).is_empty());
		assert!(RequestHeaders::parse(Some("not json")).is_empty());
		assert!(RequestHeaders::parse(Some(r#"{"Cookie":12}"#)).is_empty());

		assert_eq!(
			RequestHeaders::from_pairs([("Bad Name".to_string(), "x".to_string())]),
			Err(HeaderError::Name("Bad Name".to_string()))
		);
		let rejected =
			RequestHeaders::from_pairs([("Cookie".to_string(), "a\nb".to_string())])
				.expect_err("newline is not a header value");
		assert!(!rejected.to_string().contains("a\nb"));
	}
}
