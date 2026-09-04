use axum::{
	body::Body,
	http::{header, HeaderMap, HeaderValue, Response, StatusCode},
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::errors::{APIError, APIResult};

const PRIVATE_REVALIDATE: &str = "max-age=0, must-revalidate, private";

pub fn cached_json<T: Serialize>(
	request_headers: &HeaderMap,
	value: &T,
) -> APIResult<Response<Body>> {
	let body = serde_json::to_vec(value)
		.map_err(|error| APIError::InternalServerError(error.to_string()))?;
	cached_bytes(request_headers, "application/json", body)
}

pub fn cached_bytes(
	request_headers: &HeaderMap,
	content_type: &str,
	body: Vec<u8>,
) -> APIResult<Response<Body>> {
	let digest = Sha256::digest(&body);
	let etag = format!("\"{digest:x}\"");
	let not_modified = request_headers
		.get(header::IF_NONE_MATCH)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|value| {
			value
				.split(',')
				.map(str::trim)
				.any(|candidate| candidate == "*" || candidate == etag)
		});

	let mut builder = Response::builder()
		.status(if not_modified {
			StatusCode::NOT_MODIFIED
		} else {
			StatusCode::OK
		})
		.header(header::CACHE_CONTROL, PRIVATE_REVALIDATE)
		.header(
			header::ETAG,
			HeaderValue::from_str(&etag)
				.map_err(|error| APIError::InternalServerError(error.to_string()))?,
		);

	if !not_modified {
		builder = builder.header(
			header::CONTENT_TYPE,
			HeaderValue::from_str(content_type)
				.map_err(|error| APIError::InternalServerError(error.to_string()))?,
		);
	}

	builder
		.body(if not_modified {
			Body::empty()
		} else {
			Body::from(body)
		})
		.map_err(|error| APIError::InternalServerError(error.to_string()))
}

#[cfg(test)]
mod tests {
	use axum::http::{header, HeaderMap, HeaderValue, StatusCode};

	use super::*;

	#[test]
	fn cached_response_revalidates_matching_etag() {
		let initial =
			cached_bytes(&HeaderMap::new(), "text/plain", b"hello".to_vec()).unwrap();
		let etag = initial.headers()[header::ETAG].clone();
		assert_eq!(initial.status(), StatusCode::OK);

		let mut headers = HeaderMap::new();
		headers.insert(header::IF_NONE_MATCH, etag.clone());
		let revalidated =
			cached_bytes(&headers, "text/plain", b"hello".to_vec()).unwrap();
		assert_eq!(revalidated.status(), StatusCode::NOT_MODIFIED);
		assert_eq!(revalidated.headers()[header::ETAG], etag);
		assert!(revalidated.headers().get(header::CONTENT_TYPE).is_none());
	}

	#[test]
	fn changed_body_changes_validator() {
		let mut headers = HeaderMap::new();
		headers.insert(header::IF_NONE_MATCH, HeaderValue::from_static("\"stale\""));
		let response = cached_bytes(&headers, "text/plain", b"new".to_vec()).unwrap();
		assert_eq!(response.status(), StatusCode::OK);
		assert_eq!(response.headers()[header::CONTENT_TYPE], "text/plain");
	}
}
