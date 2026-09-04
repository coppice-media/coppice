use axum::{
	http::StatusCode,
	response::{IntoResponse, Response},
	Json,
};
use sea_orm::DbErr;
use std::num::TryFromIntError;
use thiserror::Error;

pub type APIResult<T> = Result<T, APIError>;

#[derive(Debug, Error)]
pub enum APIError {
	#[error("Your account has been locked by an administrator")]
	AccountLocked,
	#[error("{0}")]
	BadRequest(String),
	#[error("Request cancelled")]
	CancelledRequest,
	#[error("{0}")]
	NotFound(String),
	#[error("{0}")]
	InternalServerError(String),
	#[error("Unauthorized")]
	Unauthorized,
	#[error("{0}")]
	Forbidden(String),
	#[error("{0}")]
	Conflict(String),
	#[error("This functionality has not been implemented yet")]
	NotImplemented,
	#[error("This functionality is not supported")]
	NotSupported,
	#[error("{0}")]
	ServiceUnavailable(String),
	#[error("{0}")]
	BadGateway(String),
	#[error("{0}")]
	Unknown(String),
	#[error("{0}")]
	DbError(#[from] DbErr),
}

impl APIError {
	pub fn status_code(&self) -> StatusCode {
		match self {
			Self::AccountLocked | Self::Forbidden(_) => StatusCode::FORBIDDEN,
			Self::BadRequest(_) => StatusCode::BAD_REQUEST,
			Self::CancelledRequest => StatusCode::from_u16(499).expect("499 is valid"),
			Self::NotFound(_) => StatusCode::NOT_FOUND,
			Self::InternalServerError(_) => StatusCode::INTERNAL_SERVER_ERROR,
			Self::Unauthorized => StatusCode::UNAUTHORIZED,
			Self::Conflict(_) => StatusCode::CONFLICT,
			Self::NotImplemented => StatusCode::NOT_IMPLEMENTED,
			Self::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
			Self::BadGateway(_) => StatusCode::BAD_GATEWAY,
			Self::DbError(DbErr::RecordNotFound(_)) => StatusCode::NOT_FOUND,
			Self::DbError(_) => StatusCode::INTERNAL_SERVER_ERROR,
			Self::NotSupported | Self::Unknown(_) => StatusCode::INTERNAL_SERVER_ERROR,
		}
	}

	pub fn forbidden_discreet() -> Self {
		Self::Forbidden("You do not have permission to access this resource.".to_owned())
	}
}

impl From<TryFromIntError> for APIError {
	fn from(error: TryFromIntError) -> Self {
		Self::InternalServerError(error.to_string())
	}
}

impl From<std::io::Error> for APIError {
	fn from(error: std::io::Error) -> Self {
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
		let body = Json(serde_json::json!({
			"status": status.as_u16(),
			"message": self.to_string(),
		}));
		let mut response = (status, body).into_response();
		if status == StatusCode::UNAUTHORIZED {
			response.headers_mut().insert(
				"Set-Cookie",
				"stump_session=; HttpOnly; SameSite=Lax; Path=/; Domain=; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=0"
					.parse()
					.expect("static cookie header is valid"),
			);
		}
		response
	}
}
