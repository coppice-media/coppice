use axum::{
	http::StatusCode,
	response::{IntoResponse, Response},
	Json,
};
use sea_orm::DbErr;
use thiserror::Error;

pub type APIResult<T> = Result<T, APIError>;

/// Errors raised by the Kavita compatibility surface.
///
/// Kavita answers most failures with an `ApiException` body of the shape
/// `{"status": <code>, "message": <text>, "details": <text|null>}`; `Unauthorized`
/// mirrors the empty-bodied 401 that Kavita's JWT/API-key handlers produce.
#[derive(Debug, Error)]
pub enum APIError {
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

impl APIError {
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

impl From<std::num::TryFromIntError> for APIError {
	fn from(error: std::num::TryFromIntError) -> Self {
		Self::InternalServerError(error.to_string())
	}
}

impl From<serde_json::Error> for APIError {
	fn from(error: serde_json::Error) -> Self {
		Self::InternalServerError(error.to_string())
	}
}

impl IntoResponse for APIError {
	fn into_response(self) -> Response {
		let status = self.status_code();
		if status == StatusCode::UNAUTHORIZED {
			return (status, [(axum::http::header::WWW_AUTHENTICATE, "Bearer")])
				.into_response();
		}
		let body = Json(serde_json::json!({
			"status": status.as_u16(),
			"message": self.to_string(),
			"details": serde_json::Value::Null,
		}));
		(status, body).into_response()
	}
}
