use crate::error::{GatewayError, GatewayResult};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use reqwest::Response;

/// Read an upstream body without trusting its `Content-Length` header.
///
/// The stream is stopped as soon as the configured limit is crossed. This is
/// used for both tracker JSON and qBittorrent API responses, and therefore
/// prevents a hostile or broken upstream from growing gateway memory without
/// bound.
pub async fn read_bounded(response: Response, max_bytes: usize) -> GatewayResult<Bytes> {
	if response
		.content_length()
		.is_some_and(|length| length > max_bytes as u64)
	{
		return Err(GatewayError::BodyLimit);
	}

	let mut body = BytesMut::with_capacity(
		response.content_length().unwrap_or(0).min(max_bytes as u64) as usize,
	);
	let mut stream = response.bytes_stream();
	while let Some(chunk) = stream.next().await {
		let chunk = chunk.map_err(|error| {
			if error.is_timeout() {
				GatewayError::Timeout
			} else {
				GatewayError::UpstreamUnavailable
			}
		})?;
		if body.len().saturating_add(chunk.len()) > max_bytes {
			return Err(GatewayError::BodyLimit);
		}
		body.extend_from_slice(&chunk);
	}
	Ok(body.freeze())
}

pub fn map_request_error(error: &reqwest::Error) -> GatewayError {
	if error.is_timeout() {
		GatewayError::Timeout
	} else {
		GatewayError::UpstreamUnavailable
	}
}
