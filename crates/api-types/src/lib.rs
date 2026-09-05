//! Transport-neutral request contracts: [`RequestOrigin`] URL building and
//! [`OffsetPagination`] arithmetic shared by HTTP handlers, GraphQL and the
//! protocol crates. No Axum/GraphQL dependency by design.
//! See `crates/api-types/README.md`.

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

/// The origin of the current request.
///
/// This type intentionally contains no web-framework dependencies so it can be
/// shared by HTTP handlers and protocol integrations alike.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestOrigin {
	pub scheme: String,
	pub host: String,
}

impl Default for RequestOrigin {
	fn default() -> Self {
		Self {
			host: "localhost".to_string(),
			scheme: "http".to_string(),
		}
	}
}

impl RequestOrigin {
	/// Construct an origin from a host and scheme.
	pub fn new(host: String, scheme: String) -> Self {
		Self { host, scheme }
	}

	/// Return this origin as a scheme and host URL.
	pub fn url(&self) -> String {
		format!("{}://{}", self.scheme, self.host)
	}

	/// Format a URL using the legacy service-context semantics.
	///
	/// Paths beginning with `/` are appended directly to the origin, strings
	/// beginning with `http` are treated as already-absolute URLs, and all other
	/// paths are separated from the origin with `/`.
	pub fn format_url<A: AsRef<str>>(&self, path: A) -> String {
		let url = path.as_ref();
		if url.starts_with('/') {
			format!("{}{}", self.url(), url)
		} else if url.starts_with("http") {
			url.to_string()
		} else {
			format!("{}/{}", self.url(), url)
		}
	}

	/// Join this origin with an application path without duplicate boundary
	/// slashes.
	pub fn url_for_path(&self, path: &str) -> String {
		format!(
			"{}/{}",
			self.url().trim_end_matches('/'),
			path.trim_start_matches('/')
		)
	}

	/// Add a cache-busting `last_modified` query parameter when a timestamp is
	/// available.
	pub fn cache_friendly_url<A: AsRef<str>>(
		&self,
		path: A,
		timestamp: &Option<DateTime<FixedOffset>>,
	) -> String {
		let base_url = self.format_url(path);

		let Some(datetime) = timestamp else {
			return base_url;
		};

		let ready_for_params = base_url.trim_end_matches('/');

		format!("{ready_for_params}?last_modified={}", datetime.to_rfc3339())
	}
}

fn default_page() -> u64 {
	1
}

fn default_page_size() -> Option<u64> {
	Some(20)
}

fn default_zero_based() -> Option<bool> {
	Some(false)
}

/// A simple offset-based pagination input object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OffsetPagination {
	/// The page to start from. This is 1-based by default, but can be changed to
	/// 0-based by setting `zero_based` to true.
	#[serde(default = "default_page")]
	pub page: u64,
	/// The number of items to return per page. This is 20 by default.
	#[serde(default = "default_page_size")]
	pub page_size: Option<u64>,
	/// Whether or not the page is zero-based. This is false by default.
	#[serde(default = "default_zero_based")]
	pub zero_based: Option<bool>,
}

impl Default for OffsetPagination {
	fn default() -> Self {
		Self {
			page: 1,
			page_size: Some(20),
			zero_based: Some(false),
		}
	}
}

impl OffsetPagination {
	pub fn offset(&self) -> u64 {
		if self.zero_based.unwrap_or(false) {
			self.page * self.page_size.unwrap_or(20)
		} else {
			(self.page - 1) * self.page_size.unwrap_or(20)
		}
	}

	pub fn limit(&self) -> u64 {
		self.page_size.unwrap_or(20)
	}

	pub fn next_page(&self) -> u64 {
		self.page + 1
	}

	pub fn previous_page(&self) -> Option<u64> {
		if self.zero_based.unwrap_or(false) {
			if self.page > 0 {
				Some(self.page - 1)
			} else {
				None
			}
		} else if self.page > 1 {
			Some(self.page - 1)
		} else {
			None
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use chrono::TimeZone;

	mod request_origin {
		use super::*;

		#[test]
		fn defaults_to_local_http_origin() {
			assert_eq!(RequestOrigin::default().url(), "http://localhost");
		}

		#[test]
		fn builds_origin_from_host_and_scheme() {
			let origin =
				RequestOrigin::new("example.com".to_string(), "https".to_string());
			assert_eq!(origin.scheme, "https");
			assert_eq!(origin.host, "example.com");
			assert_eq!(origin.url(), "https://example.com");
		}

		#[test]
		fn preserves_legacy_format_url_behavior() {
			let origin =
				RequestOrigin::new("example.com".to_string(), "https".to_string());
			assert_eq!(
				origin.format_url("/api/v2/media"),
				"https://example.com/api/v2/media"
			);
			assert_eq!(
				origin.format_url("api/v2/media"),
				"https://example.com/api/v2/media"
			);
			assert_eq!(
				origin.format_url("http://other.example/media"),
				"http://other.example/media"
			);
			assert_eq!(origin.format_url("httpsomething"), "httpsomething");
		}

		#[test]
		fn joins_paths_without_duplicate_boundary_slashes() {
			let origin =
				RequestOrigin::new("example.com/".to_string(), "https".to_string());
			assert_eq!(
				origin.url_for_path("/api/v2/media"),
				"https://example.com/api/v2/media"
			);
			assert_eq!(
				origin.url_for_path("api/v2/media"),
				"https://example.com/api/v2/media"
			);
			assert_eq!(
				origin.url_for_path("//api/v2/media"),
				"https://example.com/api/v2/media"
			);
		}

		#[test]
		fn cache_friendly_url_uses_rfc3339_timestamp() {
			let origin = RequestOrigin::default();
			let timestamp = Some(
				FixedOffset::east_opt(5 * 60 * 60)
					.unwrap()
					.with_ymd_and_hms(2026, 9, 2, 3, 4, 5)
					.single()
					.unwrap(),
			);

			assert_eq!(
				origin.cache_friendly_url("/avatar/", &timestamp),
				"http://localhost/avatar?last_modified=2026-09-02T03:04:05+05:00"
			);
			assert_eq!(
				origin.cache_friendly_url("/avatar/", &None),
				"http://localhost/avatar/"
			);
		}
	}

	mod offset_pagination {
		use super::*;

		#[test]
		fn defaults_match_rest_pagination_defaults() {
			let pagination = OffsetPagination::default();
			assert_eq!(pagination.page, 1);
			assert_eq!(pagination.page_size, Some(20));
			assert_eq!(pagination.zero_based, Some(false));
			assert_eq!(pagination.offset(), 0);
			assert_eq!(pagination.limit(), 20);
		}

		#[test]
		fn one_based_pagination_matches_existing_arithmetic() {
			let pagination = OffsetPagination {
				page: 5,
				page_size: Some(10),
				zero_based: Some(false),
			};
			assert_eq!(pagination.offset(), 40);
			assert_eq!(pagination.limit(), 10);
			assert_eq!(pagination.next_page(), 6);
			assert_eq!(pagination.previous_page(), Some(4));
		}

		#[test]
		fn zero_based_pagination_matches_existing_arithmetic() {
			let pagination = OffsetPagination {
				page: 5,
				page_size: Some(10),
				zero_based: Some(true),
			};
			assert_eq!(pagination.offset(), 50);
			assert_eq!(pagination.limit(), 10);
			assert_eq!(pagination.next_page(), 6);
			assert_eq!(pagination.previous_page(), Some(4));
		}

		#[test]
		fn previous_page_is_none_at_each_pagination_origin() {
			let one_based = OffsetPagination {
				page: 1,
				page_size: Some(20),
				zero_based: Some(false),
			};
			assert_eq!(one_based.previous_page(), None);

			let zero_based = OffsetPagination {
				page: 0,
				page_size: Some(20),
				zero_based: Some(true),
			};
			assert_eq!(zero_based.previous_page(), None);
		}
	}
}
