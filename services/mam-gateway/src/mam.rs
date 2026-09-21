use crate::{
	config::GatewayConfig,
	error::{GatewayError, GatewayResult},
	http_util::{map_request_error, read_bounded},
};
use bytes::Bytes;
use reqwest::{header, Client};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use url::Url;

const MAM_ALLOWED_HOSTS: &[&str] = &[
	"myanonamouse.net",
	"www.myanonamouse.net",
	"t.myanonamouse.net",
];

#[derive(Clone, Debug)]
pub struct SearchQuery {
	pub query: String,
	pub categories: Vec<u32>,
	pub offset: usize,
	pub limit: usize,
}

/// Fields safe to return to Coppice. The tracker download URL is deliberately
/// not represented here.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicResult {
	pub result_id: String,
	pub title: String,
	pub authors: Vec<String>,
	pub category: u32,
	pub size_bytes: u64,
	pub seeders: u32,
	pub leechers: u32,
	pub published_at: Option<String>,
	pub info_hash: Option<String>,
}

/// A normalized tracker row retained only in the gateway result cache.
#[derive(Clone, Debug)]
pub struct NormalizedResult {
	pub title: String,
	pub authors: Vec<String>,
	pub category: u32,
	pub size_bytes: u64,
	pub seeders: u32,
	pub leechers: u32,
	pub published_at: Option<String>,
	pub info_hash: Option<String>,
	pub(crate) download_url: Url,
}

impl NormalizedResult {
	pub(crate) fn public(&self, result_id: String) -> PublicResult {
		PublicResult {
			result_id,
			title: self.title.clone(),
			authors: self.authors.clone(),
			category: self.category,
			size_bytes: self.size_bytes,
			seeders: self.seeders,
			leechers: self.leechers,
			published_at: self.published_at.clone(),
			info_hash: self.info_hash.clone(),
		}
	}
}

#[derive(Clone)]
pub struct MamClient {
	http: Client,
	config: Arc<GatewayConfig>,
}

impl MamClient {
	pub fn new(config: Arc<GatewayConfig>) -> GatewayResult<Self> {
		let http = Client::builder()
			.redirect(reqwest::redirect::Policy::none())
			.no_proxy()
			.timeout(config.request_timeout)
			.build()
			.map_err(|_| GatewayError::Config)?;
		Ok(Self { http, config })
	}

	pub async fn search(
		&self,
		query: &SearchQuery,
	) -> GatewayResult<Vec<NormalizedResult>> {
		let endpoint = self
			.config
			.mam_base_url
			.join(&self.config.mam_search_path)
			.map_err(|_| GatewayError::Config)?;
		let categories = query
			.categories
			.iter()
			.map(u32::to_string)
			.collect::<Vec<_>>()
			.join(",");
		// These are the only tracker inputs derived from a Torznab request.
		// Cookie/session state is attached by the gateway and never accepted
		// from the caller.
		let form = [
			("tor", "coppice".to_owned()),
			("text", query.query.clone()),
			("category", categories),
			("start", query.offset.to_string()),
			("limit", query.limit.to_string()),
		];
		let response = self
			.http
			.post(endpoint)
			.header(header::ACCEPT, "application/json")
			.header(header::USER_AGENT, "Coppice-Mam-Gateway/0.1")
			.header(header::COOKIE, self.config.mam_cookie.expose())
			.form(&form)
			.send()
			.await
			.map_err(|error| map_request_error(&error))?;
		let status = response.status();
		if status.is_redirection() {
			return Err(GatewayError::RedirectRejected);
		}
		if status == reqwest::StatusCode::UNAUTHORIZED
			|| status == reqwest::StatusCode::FORBIDDEN
		{
			return Err(GatewayError::UpstreamUnauthorized);
		}
		if !status.is_success() {
			return Err(GatewayError::UpstreamUnavailable);
		}
		let body = read_bounded(response, self.config.max_tracker_body_bytes).await?;
		let value: Value =
			serde_json::from_slice(&body).map_err(|_| GatewayError::UpstreamInvalid)?;
		parse_results(&value, &self.config.mam_base_url)
	}

	/// Fetch one tracker-provided torrent artifact. The URL came from a
	/// gateway-held result and checked immediately before use.
	pub async fn fetch_download(
		&self,
		result: &NormalizedResult,
	) -> GatewayResult<Bytes> {
		validate_mam_url(&result.download_url, &self.config.mam_base_url)?;
		let response = self
			.http
			.get(result.download_url.clone())
			.header(
				header::ACCEPT,
				"application/x-bittorrent, application/octet-stream",
			)
			.header(header::USER_AGENT, "Coppice-Mam-Gateway/0.1")
			.header(header::COOKIE, self.config.mam_cookie.expose())
			.send()
			.await
			.map_err(|error| map_request_error(&error))?;
		let status = response.status();
		if status.is_redirection() {
			return Err(GatewayError::RedirectRejected);
		}
		if status == reqwest::StatusCode::UNAUTHORIZED
			|| status == reqwest::StatusCode::FORBIDDEN
		{
			return Err(GatewayError::UpstreamUnauthorized);
		}
		if !status.is_success() {
			return Err(GatewayError::UpstreamUnavailable);
		}
		let bytes = read_bounded(response, self.config.max_torrent_bytes).await?;
		if bytes.is_empty() {
			return Err(GatewayError::UpstreamInvalid);
		}
		Ok(bytes)
	}
}

fn parse_results(value: &Value, base_url: &Url) -> GatewayResult<Vec<NormalizedResult>> {
	let rows = find_rows(value).ok_or(GatewayError::UpstreamInvalid)?;
	let mut results = Vec::with_capacity(rows.len());
	for row in rows {
		let Some(object) = row.as_object() else {
			continue;
		};
		let Some(title) = field_string(object, &["title", "name", "itemName"]) else {
			continue;
		};
		let Some(download) = field_string(
			object,
			&["download", "downloadUrl", "download_url", "torrent", "url"],
		) else {
			continue;
		};
		let Some(download_url) = parse_download_url(&download, base_url) else {
			return Err(GatewayError::UnsafeUrl);
		};
		if validate_mam_url(&download_url, base_url).is_err() {
			return Err(GatewayError::UnsafeUrl);
		}
		let category = field_category(object).unwrap_or(7000);
		let authors = field_authors(object);
		let info_hash = field_string(object, &["infoHash", "info_hash", "hash"])
			.and_then(|hash| normalize_hash(&hash));
		results.push(NormalizedResult {
			title: clean_text(&title, 512),
			authors,
			category,
			size_bytes: field_u64(object, &["size", "sizeBytes", "size_bytes"])
				.unwrap_or(0),
			seeders: field_u64(object, &["seeders", "seed", "peers"])
				.unwrap_or(0)
				.min(u32::MAX as u64) as u32,
			leechers: field_u64(object, &["leechers", "leech", "leechersCount"])
				.unwrap_or(0)
				.min(u32::MAX as u64) as u32,
			published_at: field_string(
				object,
				&["publishedAt", "published_at", "pubDate"],
			)
			.map(|date| clean_text(&date, 128)),
			info_hash,
			download_url,
		});
	}
	if !rows.is_empty() && results.is_empty() {
		return Err(GatewayError::UpstreamInvalid);
	}
	Ok(results)
}

fn find_rows(value: &Value) -> Option<&Vec<Value>> {
	if let Some(rows) = value.as_array() {
		return Some(rows);
	}
	let object = value.as_object()?;
	for key in ["results", "data", "items", "torrents", "rows"] {
		if let Some(rows) = object.get(key).and_then(find_rows) {
			return Some(rows);
		}
	}
	None
}

fn field_string(
	object: &serde_json::Map<String, Value>,
	names: &[&str],
) -> Option<String> {
	names.iter().find_map(|name| {
		let value = object.get(*name)?;
		if let Some(string) = value.as_str() {
			let cleaned = clean_text(string, 4096);
			(!cleaned.is_empty()).then_some(cleaned)
		} else {
			None
		}
	})
}

fn field_authors(object: &serde_json::Map<String, Value>) -> Vec<String> {
	for name in ["authors", "author", "artist", "uploader"] {
		let Some(value) = object.get(name) else {
			continue;
		};
		let mut authors = match value {
			Value::Array(values) => values
				.iter()
				.filter_map(Value::as_str)
				.map(|value| clean_text(value, 128))
				.filter(|value| !value.is_empty())
				.take(16)
				.collect::<Vec<_>>(),
			Value::String(value) => value
				.split(&[',', ';'][..])
				.map(|value| clean_text(value, 128))
				.filter(|value| !value.is_empty())
				.take(16)
				.collect::<Vec<_>>(),
			_ => Vec::new(),
		};
		if !authors.is_empty() {
			authors.shrink_to_fit();
			return authors;
		}
	}
	Vec::new()
}

fn field_u64(object: &serde_json::Map<String, Value>, names: &[&str]) -> Option<u64> {
	names.iter().find_map(|name| {
		let value = object.get(*name)?;
		match value {
			Value::Number(number) => number.as_u64().or_else(|| {
				number
					.as_f64()
					.filter(|value| value.is_finite() && *value >= 0.0)
					.map(|value| value as u64)
			}),
			Value::String(value) => value
				.replace(&[',', '_'][..], "")
				.split_whitespace()
				.next()
				.and_then(|value| value.parse::<u64>().ok()),
			_ => None,
		}
	})
}

fn field_category(object: &serde_json::Map<String, Value>) -> Option<u32> {
	let value = object.get("category").or_else(|| object.get("cat"))?;
	let category = match value {
		Value::Number(number) => number.as_u64(),
		Value::String(value) => value.parse::<u64>().ok().or_else(|| {
			match value.to_ascii_lowercase().as_str() {
				"book" | "books" | "ebook" | "ebooks" => Some(7000),
				"magazine" | "magazines" => Some(7010),
				"comics" | "comic" => Some(7020),
				"audiobook" | "audiobooks" => Some(7030),
				_ => None,
			}
		}),
		_ => None,
	}?;
	u32::try_from(category).ok()
}

fn parse_download_url(value: &str, base_url: &Url) -> Option<Url> {
	if value.bytes().any(|byte| byte == b'\r' || byte == b'\n') {
		return None;
	}
	if value.starts_with('/') {
		base_url.join(value).ok()
	} else {
		Url::parse(value).ok()
	}
}

fn validate_mam_url(url: &Url, base_url: &Url) -> GatewayResult<()> {
	if url.scheme() != base_url.scheme()
		|| url.username() != ""
		|| url.password().is_some()
		|| url.fragment().is_some()
	{
		return Err(GatewayError::UnsafeUrl);
	}
	let Some(host) = url.host_str() else {
		return Err(GatewayError::UnsafeUrl);
	};
	let Some(base_host) = base_url.host_str() else {
		return Err(GatewayError::UnsafeUrl);
	};
	let same_configured_host = host == base_host;
	let same_production_family =
		MAM_ALLOWED_HOSTS.contains(&host) && MAM_ALLOWED_HOSTS.contains(&base_host);
	if !same_configured_host && !same_production_family {
		return Err(GatewayError::UnsafeUrl);
	}
	if url.port_or_known_default() != base_url.port_or_known_default() {
		return Err(GatewayError::UnsafeUrl);
	}
	Ok(())
}

fn normalize_hash(value: &str) -> Option<String> {
	let hash = value.trim().to_ascii_lowercase();
	(hash.len() == 40 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
		.then_some(hash)
}

fn clean_text(value: &str, max_chars: usize) -> String {
	value
		.chars()
		.filter(|character| !character.is_control() || character.is_whitespace())
		.take(max_chars)
		.collect::<String>()
		.trim()
		.to_owned()
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn parser_keeps_only_safe_normalized_fields() {
		let value = json!({
			"results": [{
				"title": "Fixture Book",
				"author": "Ada; Grace",
				"category": "ebooks",
				"size": "1,024",
				"seeders": 5,
				"download": "https://t.myanonamouse.net/tor/download?id=secret"
			}]
		});
		let base = Url::parse("https://t.myanonamouse.net").expect("base");
		let results = parse_results(&value, &base).expect("results");
		assert_eq!(results.len(), 1);
		assert_eq!(results[0].category, 7000);
		assert_eq!(results[0].authors, vec!["Ada", "Grace"]);
		assert_eq!(results[0].size_bytes, 1024);
		assert!(
			!serde_json::to_string(&results[0].public("opaque".to_owned()))
				.expect("public json")
				.contains("secret")
		);
	}

	#[test]
	fn parser_rejects_rows_with_untrusted_download_hosts() {
		let value = json!({
			"results": [{
				"title": "Fixture Book",
				"download": "http://169.254.169.254/latest/meta-data"
			}]
		});
		let base = Url::parse("https://t.myanonamouse.net").expect("base");
		assert!(matches!(
			parse_results(&value, &base),
			Err(GatewayError::UnsafeUrl)
		));
	}
}
