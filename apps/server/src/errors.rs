use axum::{
	extract::multipart::MultipartError,
	http::StatusCode,
	response::{IntoResponse, Response},
	Json,
};
use cli::CliError;
use stump_core::{
	error::CoreError, filesystem::image::ThumbnailGenerateError, opds::v2_0::OPDSV2Error,
	CoreEvent,
};
use stump_media::{EpubSearchError, FileError, ProcessorError};
use tokio::sync::mpsc;
use tower_sessions::session::Error as SessionError;

use std::{net, num::TryFromIntError};
use thiserror::Error;

use crate::config::session::delete_cookie_header;

/// A type alias for the result of a server operation
pub type ServerResult<T> = Result<T, ServerError>;
/// A type alias for the result of an API operation, e.g. the response of an axum handler
pub type APIResult<T> = Result<T, APIError>;

/// The top-level error type for the Stump server binary. The entry is a CLI app which either
/// performs a given command _or_ starts the server.
///
/// Note: If there is an invalid configuration, neither of these can happen, so there is a
/// separate error variant for that.
#[derive(Debug, Error)]
pub enum EntryError {
	#[error("{0}")]
	InvalidConfig(String),
	#[error("{0}")]
	CliError(#[from] CliError),
	#[error("{0}")]
	ServerError(#[from] ServerError),
}

/// A generic error type to encapsulate general server errors, which may include API errors, but
/// also includes other errors such as a failure to boot or bind to a port.
#[derive(Debug, Error)]
pub enum ServerError {
	// TODO: meh
	#[error("{0}")]
	ServerStartError(String),
	#[error("Stump failed to parse the provided address: {0}")]
	InvalidAddress(#[from] net::AddrParseError),
	#[error("{0}")]
	APIError(#[from] APIError),
}

/// Authentication errors, emitted during the authentication process and in instances where a user
/// is not authorized to access a resource/action.
#[derive(Error, Debug)]
pub enum AuthError {
	#[error("Error during the authentication process")]
	BcryptError(#[from] bcrypt::BcryptError),
	#[error("Missing or malformed credentials")]
	BadCredentials,
	#[error("The Authorization header could no be parsed")]
	BadRequest,
	#[error("Unauthorized")]
	Unauthorized,
	#[error("Forbidden")]
	Forbidden,
}

impl From<AuthError> for StatusCode {
	fn from(error: AuthError) -> Self {
		match error {
			AuthError::BcryptError(_) => StatusCode::INTERNAL_SERVER_ERROR,
			AuthError::BadCredentials => StatusCode::UNAUTHORIZED,
			AuthError::BadRequest => StatusCode::BAD_REQUEST,
			AuthError::Unauthorized => StatusCode::UNAUTHORIZED,
			AuthError::Forbidden => StatusCode::FORBIDDEN,
		}
	}
}

impl IntoResponse for AuthError {
	fn into_response(self) -> Response {
		match self {
			AuthError::BcryptError(_) => {
				(StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
			},
			AuthError::BadCredentials => (StatusCode::UNAUTHORIZED, self.to_string()),
			AuthError::BadRequest => (StatusCode::BAD_REQUEST, self.to_string()),
			AuthError::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
			AuthError::Forbidden => (StatusCode::FORBIDDEN, self.to_string()),
		}
		.into_response()
	}
}

/// The error representation for API errors. This is a simple enum which will be converted into a
/// JSON response containing the status code and the error message.
#[allow(unused)]
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
	#[error("Too many requests")]
	TooManyRequests,
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
	Redirect(String),
	#[error("{0}")]
	SessionFetchError(#[from] SessionError),
	#[error("{0}")]
	DbError(sea_orm::error::DbErr),
	/// SQLite write-lock contention. Transient by construction, so it is worth
	/// telling the client to come back rather than reporting a 500 it can only
	/// treat as a hard failure.
	#[error("The database is busy, please retry")]
	DatabaseBusy,
	#[error("OIDC is not enabled")]
	OIDCNotEnabled,
	#[error("OIDC provider is not initialized")]
	OIDCNotInitialized,
	#[error("The provided OIDC configuration is invalid or missing required fields")]
	OIDCConfigurationInvalid,
	#[error("{0}")]
	OIDCConfigurationError(#[from] openidconnect::ConfigurationError),
	#[error("Failed to exchange OIDC token: {0}")]
	OIDCTokenExchangeFailed(String),
	#[error("The OIDC token is missing from the response")]
	OIDCMissingToken,
	#[error("Failed to verify OIDC claims: {0}")]
	OIDCClaimsVerificationFailed(#[from] openidconnect::ClaimsVerificationError),
	#[error("The OIDC token is missing an email claim")]
	OIDCMissingEmail,
}

impl APIError {
	/// A helper function to get the status code for an APIError
	pub fn status_code(&self) -> StatusCode {
		match self {
			APIError::AccountLocked => StatusCode::FORBIDDEN,
			APIError::BadRequest(_) => StatusCode::BAD_REQUEST,
			APIError::CancelledRequest => {
				StatusCode::from_u16(499).expect("499 is a valid HTTP status code")
			},
			APIError::NotFound(_) => StatusCode::NOT_FOUND,
			APIError::InternalServerError(_) => StatusCode::INTERNAL_SERVER_ERROR,
			APIError::Unauthorized => StatusCode::UNAUTHORIZED,
			APIError::Forbidden(_) => StatusCode::FORBIDDEN,
			APIError::Conflict(_) => StatusCode::CONFLICT,
			APIError::NotImplemented => StatusCode::NOT_IMPLEMENTED,
			APIError::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
			APIError::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
			APIError::BadGateway(_) => StatusCode::BAD_GATEWAY,
			APIError::DbError(sea_orm::error::DbErr::RecordNotFound(_)) => {
				StatusCode::NOT_FOUND
			},
			APIError::DbError(_) => StatusCode::INTERNAL_SERVER_ERROR,
			APIError::DatabaseBusy => StatusCode::SERVICE_UNAVAILABLE,
			APIError::Redirect(_) => StatusCode::TEMPORARY_REDIRECT,
			APIError::OIDCConfigurationInvalid => StatusCode::BAD_REQUEST,
			APIError::OIDCNotEnabled => StatusCode::FORBIDDEN,
			_ => StatusCode::INTERNAL_SERVER_ERROR,
		}
	}

	/// How long a client should wait before retrying, for the errors that are
	/// transient by construction.
	pub fn retry_after_seconds(&self) -> Option<u32> {
		matches!(self, APIError::DatabaseBusy).then_some(1)
	}
}

impl From<OPDSV2Error> for APIError {
	fn from(error: OPDSV2Error) -> Self {
		APIError::InternalServerError(error.to_string())
	}
}

impl From<MultipartError> for APIError {
	fn from(error: MultipartError) -> Self {
		APIError::InternalServerError(error.to_string())
	}
}

impl From<prefixed_api_key::BuilderError> for APIError {
	fn from(error: prefixed_api_key::BuilderError) -> Self {
		APIError::InternalServerError(error.to_string())
	}
}

impl From<reqwest::Error> for APIError {
	fn from(error: reqwest::Error) -> Self {
		APIError::InternalServerError(error.to_string())
	}
}

impl From<ThumbnailGenerateError> for APIError {
	fn from(value: ThumbnailGenerateError) -> Self {
		APIError::InternalServerError(value.to_string())
	}
}

impl APIError {
	pub fn forbidden_discreet() -> APIError {
		APIError::Forbidden(String::from(
			"You do not have permission to access this resource.",
		))
	}
}

impl From<TryFromIntError> for APIError {
	fn from(e: TryFromIntError) -> Self {
		APIError::InternalServerError(e.to_string())
	}
}

impl From<sea_orm::error::DbErr> for APIError {
	/// Routes SQLite write-lock contention to [`APIError::DatabaseBusy`] so
	/// every `?` on a database error answers `503` + `Retry-After` instead of a
	/// `500` the client cannot act on. Write transactions take
	/// `BEGIN IMMEDIATE` (`models::txn::begin_write`), so this is the belt to
	/// that braces.
	fn from(error: sea_orm::error::DbErr) -> Self {
		if models::txn::is_write_lock_contention(&error) {
			APIError::DatabaseBusy
		} else {
			APIError::DbError(error)
		}
	}
}

impl From<CoreError> for APIError {
	fn from(err: CoreError) -> Self {
		match err {
			CoreError::FeatureDisabled(feature) => APIError::ServiceUnavailable(format!(
				"{feature} is disabled by server configuration"
			)),
			CoreError::NotFound(message) => APIError::NotFound(message),
			CoreError::BadRequest(message) => APIError::BadRequest(message),
			CoreError::Forbidden(message) => APIError::Forbidden(message),
			CoreError::DBError(db_error) => APIError::from(db_error),
			CoreError::InternalError(err) => APIError::InternalServerError(err),
			CoreError::IoError(err) => APIError::InternalServerError(err.to_string()),
			CoreError::MigrationError(err) => APIError::InternalServerError(err),
			CoreError::Unknown(err) => APIError::InternalServerError(err),
			CoreError::Utf8ConversionError(err) => {
				APIError::InternalServerError(err.to_string())
			},
			CoreError::XmlWriteError(err) => {
				APIError::InternalServerError(err.to_string())
			},
			_ => APIError::InternalServerError(err.to_string()),
		}
	}
}

impl From<AuthError> for APIError {
	fn from(error: AuthError) -> APIError {
		match error {
			AuthError::BcryptError(_) => {
				APIError::InternalServerError("Internal server error".to_string())
			},
			AuthError::BadCredentials => {
				APIError::BadRequest("Missing or malformed credentials".to_string())
			},
			AuthError::BadRequest => APIError::BadRequest(
				"The Authorization header could no be parsed".to_string(),
			),
			AuthError::Unauthorized => APIError::Unauthorized,
			AuthError::Forbidden => APIError::Forbidden("Forbidden".to_string()),
		}
	}
}

impl From<bcrypt::BcryptError> for APIError {
	fn from(error: bcrypt::BcryptError) -> APIError {
		APIError::InternalServerError(error.to_string())
	}
}

impl From<mpsc::error::SendError<CoreEvent>> for APIError {
	fn from(err: mpsc::error::SendError<CoreEvent>) -> Self {
		APIError::InternalServerError(err.to_string())
	}
}

impl From<FileError> for APIError {
	fn from(error: FileError) -> APIError {
		match error {
			// `Unavailable` is a permanent "the source will not serve this"
			// verdict, not a server fault: a provider chapter the remote no
			// longer hosts must read as 404 on every protocol lane.
			FileError::PageNotFound { .. }
			| FileError::ResourceNotFound(_)
			| FileError::Unavailable(_) => APIError::NotFound(error.to_string()),
			_ => APIError::InternalServerError(error.to_string()),
		}
	}
}

impl From<EpubSearchError> for APIError {
	fn from(error: EpubSearchError) -> Self {
		match error {
			EpubSearchError::InvalidQueryLength { .. }
			| EpubSearchError::InvalidCursor
			| EpubSearchError::InvalidLimit { .. } => APIError::BadRequest(error.to_string()),
			EpubSearchError::Cancelled => APIError::CancelledRequest,
			EpubSearchError::File(error) => APIError::from(error),
		}
	}
}

impl From<ProcessorError> for APIError {
	fn from(error: ProcessorError) -> APIError {
		match error {
			ProcessorError::InvalidQuality => APIError::BadRequest(error.to_string()),
			ProcessorError::InvalidSizedImage => APIError::BadRequest(error.to_string()),
			ProcessorError::InvalidConfiguration(err) => {
				APIError::BadRequest(err.to_string())
			},
			_ => APIError::InternalServerError(error.to_string()),
		}
	}
}

impl From<std::io::Error> for APIError {
	fn from(error: std::io::Error) -> APIError {
		APIError::InternalServerError(error.to_string())
	}
}

impl From<stump_devices::DeviceError> for APIError {
	fn from(error: stump_devices::DeviceError) -> Self {
		use stump_devices::DeviceError;
		match error {
			DeviceError::NotFound => APIError::NotFound(error.to_string()),
			DeviceError::Forbidden => APIError::Forbidden(error.to_string()),
			DeviceError::Revoked => APIError::Conflict(error.to_string()),
			DeviceError::InvalidName(_)
			| DeviceError::InvalidScope(_)
			| DeviceError::InvalidEmail(_) => APIError::BadRequest(error.to_string()),
			DeviceError::Credential(_) => {
				APIError::InternalServerError(error.to_string())
			},
			DeviceError::Database(error) => APIError::from(error),
		}
	}
}

/// The Komga provider crate mirrors the server's HTTP error vocabulary so
/// server-local Komga routes can share helpers with the extracted ones.
#[cfg(feature = "komga")]
impl From<stump_komga::errors::APIError> for APIError {
	fn from(error: stump_komga::errors::APIError) -> Self {
		use stump_komga::errors::APIError as Komga;
		match error {
			Komga::AccountLocked => APIError::AccountLocked,
			Komga::BadRequest(message) => APIError::BadRequest(message),
			Komga::CancelledRequest => APIError::CancelledRequest,
			Komga::NotFound(message) => APIError::NotFound(message),
			Komga::InternalServerError(message) => APIError::InternalServerError(message),
			Komga::Unauthorized => APIError::Unauthorized,
			Komga::Forbidden(message) => APIError::Forbidden(message),
			Komga::Conflict(message) => APIError::Conflict(message),
			Komga::NotImplemented => APIError::NotImplemented,
			Komga::NotSupported => APIError::NotSupported,
			Komga::ServiceUnavailable(message) => APIError::ServiceUnavailable(message),
			Komga::BadGateway(message) => APIError::BadGateway(message),
			Komga::Unknown(message) => APIError::Unknown(message),
			Komga::DbError(error) => APIError::from(error),
			Komga::DatabaseBusy => APIError::DatabaseBusy,
		}
	}
}

/// The response body for API errors. This is just a basic JSON response with a status code and a message.
/// Any axum handlers which return a [`Result`] with an Error of [`APIError`] will be converted into this response.
#[derive(Debug)]
pub struct APIErrorResponse {
	status: StatusCode,
	message: String,
	/// Seconds a client should wait before retrying, for transient errors.
	retry_after: Option<u32>,
}

impl From<APIError> for APIErrorResponse {
	fn from(error: APIError) -> Self {
		APIErrorResponse {
			status: error.status_code(),
			message: error.to_string(),
			retry_after: error.retry_after_seconds(),
		}
	}
}

impl IntoResponse for APIErrorResponse {
	fn into_response(self) -> Response {
		let body = serde_json::json!({
			"status": self.status.as_u16(),
			"message": self.message,
		});

		tracing::error!(error = ?self, "API error response");

		let base_response = Json(body).into_response();

		let mut builder = Response::builder()
			.status(self.status)
			.header("Content-Type", "application/json")
			// do not cache error responses
			.header("Cache-Control", "no-store");

		// if the status is 401, we want to encourage the client to delete their
		// session cookie
		if self.status == StatusCode::UNAUTHORIZED {
			let (name, value) = delete_cookie_header();
			builder = builder.header(name, value);
		}

		// transient failures (SQLite write-lock contention) tell the client
		// exactly how long to back off for
		if let Some(seconds) = self.retry_after {
			builder = builder.header("Retry-After", seconds);
		}

		builder
			.body(base_response.into_body())
			.unwrap_or_else(|error| {
				tracing::error!(?error, "Failed to build response");
				(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
			})
	}
}

impl IntoResponse for APIError {
	fn into_response(self) -> Response {
		APIErrorResponse::from(self).into_response()
	}
}

pub mod api_error_message {
	pub const LOCKED_ACCOUNT: &str =
		"Your account is locked. Please contact an administrator to unlock your account.";
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn disabled_feature_is_service_unavailable() {
		let response =
			APIError::from(CoreError::FeatureDisabled("background jobs")).into_response();

		assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
	}
}
