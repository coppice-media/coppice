use axum::{
	http::StatusCode,
	response::{IntoResponse, Response},
	Json,
};
use serde::Serialize;
use thiserror::Error;

/// Errors deliberately contain no upstream response bodies, cookies, or URLs.
#[derive(Debug, Error)]
pub enum GatewayError {
	#[error("configuration is invalid")]
	Config,
	#[error("request is invalid")]
	BadRequest,
	#[error("authentication is required")]
	Unauthorized,
	#[error("request is not allowed")]
	Forbidden,
	#[error("resource was not found")]
	NotFound,
	#[error("result has expired")]
	Expired,
	#[error("request body is too large")]
	PayloadTooLarge,
	#[error("upstream authentication failed")]
	UpstreamUnauthorized,
	#[error("upstream redirected the request")]
	RedirectRejected,
	#[error("upstream request timed out")]
	Timeout,
	#[error("upstream response exceeded the body limit")]
	BodyLimit,
	#[error("upstream response was invalid")]
	UpstreamInvalid,
	#[error("upstream service is unavailable")]
	UpstreamUnavailable,
	#[error("upstream URL was rejected")]
	UnsafeUrl,
	#[error("download client rejected the request")]
	DownloadClientRejected,
	#[error("an idempotency key was already used for another result")]
	IdempotencyConflict,
	#[error("internal gateway error")]
	Internal,
}

impl GatewayError {
	pub fn code(&self) -> &'static str {
		match self {
			Self::Config => "configuration_error",
			Self::BadRequest => "invalid_request",
			Self::Unauthorized => "authentication_required",
			Self::Forbidden => "forbidden",
			Self::NotFound => "not_found",
			Self::Expired => "result_expired",
			Self::PayloadTooLarge | Self::BodyLimit => "body_limit_exceeded",
			Self::UpstreamUnauthorized => "upstream_authentication_failed",
			Self::RedirectRejected => "redirect_rejected",
			Self::Timeout => "upstream_timeout",
			Self::UpstreamInvalid => "upstream_invalid_response",
			Self::UpstreamUnavailable => "upstream_unavailable",
			Self::UnsafeUrl => "unsafe_upstream_url",
			Self::DownloadClientRejected => "download_client_rejected",
			Self::IdempotencyConflict => "idempotency_conflict",
			Self::Internal => "internal_error",
		}
	}
	pub fn status(&self) -> StatusCode {
		match self {
			Self::Config | Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
			Self::BadRequest => StatusCode::BAD_REQUEST,
			Self::UnsafeUrl => StatusCode::BAD_GATEWAY,
			Self::Unauthorized => StatusCode::UNAUTHORIZED,
			Self::Forbidden => StatusCode::FORBIDDEN,
			Self::NotFound => StatusCode::NOT_FOUND,
			Self::Expired => StatusCode::GONE,
			Self::PayloadTooLarge | Self::BodyLimit => StatusCode::PAYLOAD_TOO_LARGE,
			Self::UpstreamUnauthorized
			| Self::RedirectRejected
			| Self::UpstreamInvalid
			| Self::UpstreamUnavailable
			| Self::DownloadClientRejected => StatusCode::BAD_GATEWAY,
			Self::Timeout => StatusCode::GATEWAY_TIMEOUT,
			Self::IdempotencyConflict => StatusCode::CONFLICT,
		}
	}

	pub fn public_message(&self) -> &'static str {
		match self {
			Self::Config => "gateway configuration is invalid",
			Self::BadRequest => "request is invalid",
			Self::Unauthorized => "authentication required",
			Self::Forbidden => "request is not allowed",
			Self::NotFound => "resource not found",
			Self::Expired => "result expired",
			Self::PayloadTooLarge | Self::BodyLimit => {
				"request or upstream body exceeds the configured limit"
			},
			Self::UpstreamUnauthorized => "upstream authentication failed",
			Self::RedirectRejected => "upstream redirects are not accepted",
			Self::Timeout => "upstream request timed out",
			Self::UpstreamInvalid => "upstream response was invalid",
			Self::UpstreamUnavailable => "upstream service is unavailable",
			Self::UnsafeUrl => "upstream URL was rejected",
			Self::DownloadClientRejected => "download client rejected the request",
			Self::IdempotencyConflict => "idempotency key is bound to another result",
			Self::Internal => "internal gateway error",
		}
	}
}

#[derive(Debug, Serialize)]
struct ErrorBody {
	error: ErrorValue,
}

#[derive(Debug, Serialize)]
struct ErrorValue {
	code: &'static str,
	message: &'static str,
}

impl IntoResponse for GatewayError {
	fn into_response(self) -> Response {
		let status = self.status();
		(
			status,
			Json(ErrorBody {
				error: ErrorValue {
					code: self.code(),
					message: self.public_message(),
				},
			}),
		)
			.into_response()
	}
}

pub type GatewayResult<T> = Result<T, GatewayError>;
