//! Narrow Coppice client for the private `mam-gateway` sidecar.
//!
//! Coppice never accepts tracker URLs, magnets, cookies, or arbitrary download
//! URLs. The origin is configured once by an operator; every request below
//! targets a fixed path on that origin and follows no redirects. The sidecar
//! owns tracker credentials and qBittorrent access.

use std::{net::IpAddr, time::Duration};

use quick_xml::de::from_str;
use reqwest::{header, Client, StatusCode, Url};
use serde::{Deserialize, Serialize};

const TORZNAB_PATH: &str = "/torznab/api";
const GRABS_PATH: &str = "/v1/grabs";

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
	#[error("gateway endpoint must be a private http(s) origin")]
	InvalidOrigin,
	#[error("gateway request failed: {0}")]
	Http(#[from] reqwest::Error),
	#[error("gateway returned HTTP {status}: {message}")]
	Status { status: StatusCode, message: String },
	#[error("gateway response was invalid: {0}")]
	InvalidResponse(String),
}

#[derive(Debug, Clone)]
pub struct GatewayClient {
	client: Client,
	origin: Url,
	token: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GrabRequest<'a> {
	pub result_id: &'a str,
	pub idempotency_key: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GrabResponse {
	#[serde(rename = "id", alias = "grabId")]
	pub opaque_id: String,
	pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GrabStatus {
	#[serde(rename = "id", alias = "grabId")]
	pub opaque_id: String,
	pub status: String,
	#[serde(rename = "failureCode", alias = "failure_code", default)]
	pub failure_code: Option<String>,
	#[serde(rename = "failureMessage", alias = "failure_message", default)]
	pub failure_message: Option<String>,
	#[serde(default)]
	pub handoff: Option<GatewayHandoff>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayHandoff {
	/// Relative path below the configured handoff root. Absolute paths are
	/// rejected before any filesystem operation.
	#[serde(rename = "relativePath", alias = "relative_path")]
	pub relative_path: String,
	#[serde(default)]
	pub filename: Option<String>,
	#[serde(default)]
	pub sha256: Option<String>,
	#[serde(rename = "byteSize", alias = "byte_size", default)]
	pub byte_size: Option<i64>,
	#[serde(rename = "mediaKind", alias = "media_kind", default)]
	pub media_kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayRelease {
	pub remote_id: String,
	#[serde(default)]
	pub external_key: Option<String>,
	pub title: String,
	#[serde(default)]
	pub authors: Option<String>,
	#[serde(default)]
	pub format: Option<String>,
	#[serde(default)]
	pub language: Option<String>,
	#[serde(default)]
	pub edition: Option<String>,
	#[serde(default)]
	pub quality: Option<String>,
	#[serde(default)]
	pub size_bytes: Option<i64>,
	#[serde(default)]
	pub seeders: Option<i32>,
	#[serde(default)]
	pub preview_name: Option<String>,
	#[serde(default)]
	pub preview_mime: Option<String>,
	#[serde(default)]
	pub preview_bytes: Option<i64>,
}

impl GatewayClient {
	pub fn new(
		endpoint: &str,
		token: impl Into<String>,
		timeout: Duration,
	) -> Result<Self, GatewayError> {
		let origin = validate_origin(endpoint)?;
		let client = Client::builder()
			.timeout(timeout)
			.redirect(reqwest::redirect::Policy::none())
			.build()?;
		Ok(Self {
			client,
			origin,
			token: token.into(),
		})
	}

	pub fn endpoint(&self) -> &str {
		self.origin.as_str()
	}

	pub async fn health(&self) -> Result<(), GatewayError> {
		let url = self.fixed_url(TORZNAB_PATH)?;
		let response = self
			.client
			.get(url)
			.query(&[("t", "caps")])
			.header(header::AUTHORIZATION, format!("Bearer {}", self.token))
			.send()
			.await?;
		self.ensure_success(response).await
	}

	pub async fn search(&self, query: &str) -> Result<Vec<GatewayRelease>, GatewayError> {
		let query = query.trim();
		if query.is_empty() || query.len() > 256 {
			return Err(GatewayError::InvalidResponse(
				"search query must be 1-256 characters".into(),
			));
		}
		let url = self.fixed_url(TORZNAB_PATH)?;
		let response = self
			.client
			.get(url)
			.query(&[("t", "search"), ("q", query)])
			.header(header::AUTHORIZATION, format!("Bearer {}", self.token))
			.send()
			.await?;
		let response = self.success_body(response).await?;
		parse_torznab(&response)
	}

	pub async fn grab(
		&self,
		result_id: &str,
		idempotency_key: &str,
	) -> Result<GrabResponse, GatewayError> {
		validate_opaque(result_id)?;
		validate_opaque(idempotency_key)?;
		let response = self
			.client
			.post(self.fixed_url(GRABS_PATH)?)
			.header(header::AUTHORIZATION, format!("Bearer {}", self.token))
			.json(&GrabRequest {
				result_id,
				idempotency_key,
			})
			.send()
			.await?;
		let response = self.success_body(response).await?;
		serde_json::from_str(&response)
			.map_err(|error| GatewayError::InvalidResponse(error.to_string()))
	}

	pub async fn status(&self, opaque_id: &str) -> Result<GrabStatus, GatewayError> {
		validate_opaque(opaque_id)?;
		let url = self.fixed_url(&format!("{GRABS_PATH}/{opaque_id}"))?;
		let response = self
			.client
			.get(url)
			.header(header::AUTHORIZATION, format!("Bearer {}", self.token))
			.send()
			.await?;
		let response = self.success_body(response).await?;
		serde_json::from_str(&response)
			.map_err(|error| GatewayError::InvalidResponse(error.to_string()))
	}

	fn fixed_url(&self, path: &str) -> Result<Url, GatewayError> {
		if !path.starts_with('/')
			|| path.contains("..")
			|| path.contains('?')
			|| path.contains('#')
		{
			return Err(GatewayError::InvalidOrigin);
		}
		let mut url = self.origin.clone();
		url.set_path(path);
		url.set_query(None);
		url.set_fragment(None);
		Ok(url)
	}

	async fn success_body(
		&self,
		response: reqwest::Response,
	) -> Result<String, GatewayError> {
		let status = response.status();
		let body = response.text().await?;
		if !status.is_success() {
			return Err(GatewayError::Status {
				status,
				message: "gateway rejected the request".into(),
			});
		}
		Ok(body)
	}

	async fn ensure_success(
		&self,
		response: reqwest::Response,
	) -> Result<(), GatewayError> {
		self.success_body(response).await.map(|_| ())
	}
}

fn validate_origin(endpoint: &str) -> Result<Url, GatewayError> {
	let mut url = Url::parse(endpoint).map_err(|_| GatewayError::InvalidOrigin)?;
	if !matches!(url.scheme(), "http" | "https")
		|| url.host_str().is_none()
		|| url.username() != ""
		|| url.password().is_some()
		|| url.port().is_none()
		|| !matches!(url.path(), "" | "/")
		|| url.query().is_some()
		|| url.fragment().is_some()
	{
		return Err(GatewayError::InvalidOrigin);
	}
	let host = url.host_str().ok_or(GatewayError::InvalidOrigin)?;
	if !is_private_host(host) {
		return Err(GatewayError::InvalidOrigin);
	}
	url.set_path("/");
	Ok(url)
}

fn is_private_host(host: &str) -> bool {
	if host.eq_ignore_ascii_case("localhost")
		|| host.eq_ignore_ascii_case("vpn-gateway")
		|| host.eq_ignore_ascii_case("gluetun")
	{
		return true;
	}
	let Ok(ip) = host.parse::<IpAddr>() else {
		return false;
	};
	match ip {
		IpAddr::V4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
		IpAddr::V6(ip) => {
			ip.is_unique_local() || ip.is_loopback() || ip.is_unicast_link_local()
		},
	}
}

fn validate_opaque(value: &str) -> Result<(), GatewayError> {
	if value.is_empty()
		|| value.len() > 256
		|| !value.bytes().all(|byte| {
			byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
		}) {
		return Err(GatewayError::InvalidResponse(
			"opaque gateway id is invalid".into(),
		));
	}
	Ok(())
}

pub fn safe_failure_code(value: Option<&str>) -> Option<String> {
	let value = value?.trim();
	if value.is_empty()
		|| value.len() > 64
		|| !value
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
	{
		return None;
	}
	Some(value.to_owned())
}

pub fn safe_failure_message(value: Option<&str>) -> Option<String> {
	value.map(|_| "gateway reported a failure".to_owned())
}
#[derive(Debug, Deserialize)]
struct TorznabRss {
	channel: TorznabChannel,
}
#[derive(Debug, Deserialize)]
struct TorznabChannel {
	#[serde(rename = "item", default)]
	items: Vec<TorznabItem>,
}
#[derive(Debug, Deserialize)]
struct TorznabItem {
	guid: Option<String>,
	title: Option<String>,
	author: Option<String>,
	language: Option<String>,
	format: Option<String>,
	edition: Option<String>,
	quality: Option<String>,
	size: Option<i64>,
	seeders: Option<i32>,
	#[serde(rename = "attr", default)]
	attrs: Vec<TorznabAttr>,
	#[serde(rename = "torznab:attr", default)]
	namespaced_attrs: Vec<TorznabAttr>,
	enclosure: Option<TorznabEnclosure>,
}
#[derive(Debug, Deserialize)]
struct TorznabAttr {
	#[serde(rename = "@name")]
	name: String,
	#[serde(rename = "@value")]
	value: String,
}
#[derive(Debug, Deserialize)]
struct TorznabEnclosure {
	#[serde(rename = "@url", default)]
	_url: Option<String>,
	#[serde(rename = "@length")]
	length: Option<i64>,
	#[serde(rename = "@type")]
	type_: Option<String>,
}

fn parse_torznab(body: &str) -> Result<Vec<GatewayRelease>, GatewayError> {
	let rss: TorznabRss = from_str(body)
		.map_err(|error| GatewayError::InvalidResponse(error.to_string()))?;
	Ok(rss
		.channel
		.items
		.into_iter()
		.filter_map(|item| {
			let author_attr = torznab_attr(&item, "author");
			let language_attr = torznab_attr(&item, "language");
			let format_attr =
				torznab_attr(&item, "category").or_else(|| torznab_attr(&item, "format"));
			let edition_attr = torznab_attr(&item, "edition");
			let quality_attr = torznab_attr(&item, "quality");
			let size_attr =
				torznab_attr(&item, "size").and_then(|value| value.parse::<i64>().ok());
			let seeders_attr = torznab_attr(&item, "seeders")
				.and_then(|value| value.parse::<i32>().ok());
			let remote_id = item.guid?.trim().to_string();
			let title = item.title?.trim().to_string();
			if remote_id.is_empty() || title.is_empty() {
				return None;
			}
			let size_bytes = item
				.size
				.or(size_attr)
				.or_else(|| item.enclosure.as_ref().and_then(|e| e.length));
			let preview_mime = item.enclosure.and_then(|e| e.type_);
			Some(GatewayRelease {
				remote_id,
				external_key: None,
				title,
				authors: item.author.or(author_attr),
				format: item.format.or(format_attr),
				language: item.language.or(language_attr),
				edition: item.edition.or(edition_attr),
				quality: item.quality.or(quality_attr),
				size_bytes,
				seeders: item.seeders.or(seeders_attr),
				preview_name: None,
				preview_mime,
				preview_bytes: None,
			})
		})
		.collect())
}

fn torznab_attr(item: &TorznabItem, name: &str) -> Option<String> {
	item.attrs
		.iter()
		.chain(item.namespaced_attrs.iter())
		.find(|attr| attr.name.eq_ignore_ascii_case(name))
		.map(|attr| attr.value.clone())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rejects_public_and_path_origins() {
		assert!(validate_origin("https://example.com:443/").is_err());
		assert!(validate_origin("http://10.0.0.2:8080/v1").is_err());
		assert!(validate_origin("http://vpn-gateway:8080/").is_ok());
		assert!(validate_origin("http://gluetun:3421").is_ok());
	}

	#[test]
	fn parses_safe_torznab_facts_without_enclosure_url() {
		let xml = r#"<rss><channel><item><guid>opaque-1</guid><title>Book EPUB</title><author>A</author><size>42</size><seeders>7</seeders><enclosure url="https://tracker.invalid/raw" length="42" type="application/epub+zip" /></item></channel></rss>"#;
		let rows = parse_torznab(xml).unwrap();
		assert_eq!(rows[0].remote_id, "opaque-1");
		assert_eq!(rows[0].size_bytes, Some(42));
		assert_eq!(
			rows[0].preview_mime.as_deref(),
			Some("application/epub+zip")
		);
	}

	#[test]
	fn decodes_camel_case_grab_handoff_without_tracker_url() {
		let status: GrabStatus = serde_json::from_str(
			r#"{"grabId":"opaque-1","status":"COMPLETED","handoff":{"relativePath":"ready/book.epub","byteSize":42,"mediaKind":"ebook"}}"#,
		)
		.unwrap();
		assert_eq!(status.opaque_id, "opaque-1");
		assert_eq!(status.handoff.unwrap().relative_path, "ready/book.epub");
	}

	#[test]
	fn parses_torznab_namespaced_attributes() {
		let xml = r#"<rss xmlns:torznab="http://torznab.com/schemas/2015/feed"><channel><item><guid>opaque-2</guid><title>Book</title><torznab:attr name="seeders" value="9" /><torznab:attr name="category" value="ebook" /></item></channel></rss>"#;
		let rows = parse_torznab(xml).unwrap();
		assert_eq!(rows[0].seeders, Some(9));
		assert_eq!(rows[0].format.as_deref(), Some("ebook"));
	}

	#[test]
	fn serializes_grab_contract_without_tracker_fields() {
		let payload = serde_json::to_value(GrabRequest {
			result_id: "result-1",
			idempotency_key: "request-1",
		})
		.unwrap();
		assert_eq!(payload["result_id"], "result-1");
		assert_eq!(payload["idempotency_key"], "request-1");
		assert!(payload.get("candidate_id").is_none());
		assert!(payload.get("request_id").is_none());
	}
}
