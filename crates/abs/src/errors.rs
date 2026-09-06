//! Failures of the Audiobookshelf compatibility surface.
//!
//! Audiobookshelf is an Express app that answers almost every failure with
//! `res.sendStatus(code)`, i.e. the canonical reason phrase as
//! `text/plain; charset=utf-8` and no JSON envelope. Captured from
//! `abs-ref` 2.36.0 (`../komga-compat/abs/capture/negatives.txt`):
//! `GET /api/items/nope` → `404 Not Found`, `GET /api/me` unauthenticated →
//! `401 Unauthorized`. Only routes that deliberately return a body (e.g. the
//! login DTOs) send JSON, so the error type here carries a message for the
//! log and serialises the reason phrase for the client.

use axum::{
	http::{header, StatusCode},
	response::{IntoResponse, Response},
};
use sea_orm::DbErr;
use thiserror::Error;

pub type AbsResult<T> = Result<T, AbsError>;

#[derive(Debug, Error)]
pub enum AbsError {
	#[error("{0}")]
	BadRequest(String),
	#[error("{0}")]
	NotFound(String),
	#[error("Unauthorized")]
	Unauthorized,
	#[error("{0}")]
	Forbidden(String),
	#[error("{0}")]
	InternalServerError(String),
	#[error("{0}")]
	DbError(#[from] DbErr),
}

impl AbsError {
	pub fn status_code(&self) -> StatusCode {
		match self {
			Self::BadRequest(_) => StatusCode::BAD_REQUEST,
			Self::NotFound(_) => StatusCode::NOT_FOUND,
			Self::Unauthorized => StatusCode::UNAUTHORIZED,
			Self::Forbidden(_) => StatusCode::FORBIDDEN,
			Self::InternalServerError(_) => StatusCode::INTERNAL_SERVER_ERROR,
			Self::DbError(DbErr::RecordNotFound(_)) => StatusCode::NOT_FOUND,
			Self::DbError(_) => StatusCode::INTERNAL_SERVER_ERROR,
		}
	}
}

impl From<std::num::TryFromIntError> for AbsError {
	fn from(error: std::num::TryFromIntError) -> Self {
		Self::InternalServerError(error.to_string())
	}
}

/// A token that fails to verify is a credential problem, not a server
/// problem: abs-ref answers `401` for both an expired and a forged token
/// (`capture/negatives.txt`). Failing to *mint* one is the server's fault.
impl From<crate::auth::TokenError> for AbsError {
	fn from(error: crate::auth::TokenError) -> Self {
		match error {
			crate::auth::TokenError::Encode(error) => {
				Self::InternalServerError(error.to_string())
			},
			crate::auth::TokenError::Invalid(_) | crate::auth::TokenError::Expired => {
				Self::Unauthorized
			},
		}
	}
}

impl From<serde_json::Error> for AbsError {
	fn from(error: serde_json::Error) -> Self {
		Self::InternalServerError(error.to_string())
	}
}

impl IntoResponse for AbsError {
	fn into_response(self) -> Response {
		let status = self.status_code();
		if status.is_server_error() {
			tracing::error!(error = %self, "Audiobookshelf profile request failed");
		} else {
			tracing::debug!(status = %status, error = %self, "Audiobookshelf profile request refused");
		}
		// `res.sendStatus(code)`: the reason phrase, not the message. The
		// message would leak Stump internals to a client that never reads it.
		let body = status.canonical_reason().unwrap_or("Error").to_owned();
		(
			status,
			[(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
			body,
		)
			.into_response()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// abs-ref answers failures with the reason phrase as plain text; the
	/// internal message must never reach the wire.
	#[tokio::test]
	async fn errors_serialize_as_reason_phrases() {
		for (error, status, body) in [
			(
				AbsError::NotFound("media 1234 is not audio".to_owned()),
				StatusCode::NOT_FOUND,
				"Not Found",
			),
			(
				AbsError::Unauthorized,
				StatusCode::UNAUTHORIZED,
				"Unauthorized",
			),
			(
				AbsError::BadRequest("currentTime must be finite".to_owned()),
				StatusCode::BAD_REQUEST,
				"Bad Request",
			),
			(
				AbsError::Forbidden("no".to_owned()),
				StatusCode::FORBIDDEN,
				"Forbidden",
			),
		] {
			let response = error.into_response();
			assert_eq!(response.status(), status);
			assert_eq!(
				response
					.headers()
					.get(header::CONTENT_TYPE)
					.and_then(|value| value.to_str().ok()),
				Some("text/plain; charset=utf-8")
			);
			let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
				.await
				.expect("body");
			assert_eq!(String::from_utf8_lossy(&bytes), body);
		}
	}

	#[test]
	fn missing_rows_are_not_found() {
		assert_eq!(
			AbsError::DbError(DbErr::RecordNotFound("x".to_owned())).status_code(),
			StatusCode::NOT_FOUND
		);
		assert_eq!(
			AbsError::DbError(DbErr::Custom("x".to_owned())).status_code(),
			StatusCode::INTERNAL_SERVER_ERROR
		);
	}
}
