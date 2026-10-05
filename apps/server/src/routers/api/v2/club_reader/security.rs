use axum::{
	extract::{FromRequestParts, Request, State},
	http::{
		header::{
			CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, EXPIRES, HOST, ORIGIN, PRAGMA,
			REFERRER_POLICY,
		},
		request::Parts,
		uri::{Authority, Uri},
		HeaderMap, HeaderValue,
	},
	middleware::Next,
	response::{IntoResponse, Response},
	Json,
};
use axum_extra::extract::Host;
use chrono::{DateTime, Utc};
use data_encoding::{BASE64URL_NOPAD, HEXLOWER};
use models::{
	entity::{book_club_member, user::AuthUser},
	shared::{
		book_club::BookClubMemberRole, enums::UserPermission,
		permission_set::user_has_all_permissions,
	},
};
use rand::{rngs::OsRng, TryRngCore};
use sea_orm::prelude::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{config::state::AppState, middleware::HostExtractor};

pub(super) const READER_COOKIE_NAME: &str = "coppice_club_reader";
pub(super) const EXPECTED_PARTICIPANT_HEADER: &str = "x-club-reader-participant";

/// Enforce same-origin writes and keep every capability response out of caches.
pub(super) async fn origin_guard(
	State(ctx): State<AppState>,
	request: Request,
	next: Next,
) -> Response {
	let writes = matches!(
		*request.method(),
		axum::http::Method::POST
			| axum::http::Method::PUT
			| axum::http::Method::PATCH
			| axum::http::Method::DELETE
	);

	let (mut parts, body) = request.into_parts();
	let origin_ok = !writes || same_origin(&mut parts, &ctx).await;
	let mut response = if origin_ok {
		next.run(Request::from_parts(parts, body)).await
	} else {
		crate::errors::APIError::Forbidden("same-origin request required".to_string())
			.into_response()
	};

	response.headers_mut().insert(
		CACHE_CONTROL,
		HeaderValue::from_static("private, no-store, no-cache, max-age=0"),
	);
	response
		.headers_mut()
		.insert(PRAGMA, HeaderValue::from_static("no-cache"));
	response
		.headers_mut()
		.insert(EXPIRES, HeaderValue::from_static("0"));
	response
		.headers_mut()
		.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
	response.headers_mut().insert(
		"x-content-type-options",
		HeaderValue::from_static("nosniff"),
	);
	response
}

/// Route-local error mapping for the capability-scoped guest reader.
///
/// Guest requests never authenticate with the normal Stump session, so a guest
/// 401 must not carry the shared [`APIErrorResponse`](crate::errors::APIErrorResponse)
/// `stump_session` clear: an invalid reader link would otherwise sign the user
/// out of the app in the same browser. Extractor rejections and method
/// mismatches (plain-text or empty axum bodies) are rewritten as JSON errors so
/// every guest failure has one shape.
pub(super) async fn guest_response_guard(request: Request, next: Next) -> Response {
	let mut response = next.run(request).await;
	crate::middleware::auth::strip_session_cookie_clears(response.headers_mut());

	let status = response.status();
	let is_json = response
		.headers()
		.get(CONTENT_TYPE)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|value| value.starts_with("application/json"));
	if (status.is_client_error() || status.is_server_error()) && !is_json {
		let body = Json(json!({
			"status": status.as_u16(),
			"message": status.canonical_reason().unwrap_or("Request failed"),
		}))
		.into_response()
		.into_body();
		let headers = response.headers_mut();
		headers.remove(CONTENT_LENGTH);
		headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
		*response.body_mut() = body;
	}
	response
}

async fn same_origin(parts: &mut Parts, ctx: &AppState) -> bool {
	let Some(origin) = parts
		.headers
		.get(ORIGIN)
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.parse::<Uri>().ok())
	else {
		return false;
	};
	let Some(origin_scheme) = origin.scheme_str() else {
		return false;
	};
	if !matches!(origin_scheme, "http" | "https")
		|| origin.path() != "/"
		|| origin.query().is_some()
	{
		return false;
	}
	let Some(origin_authority) = origin.authority() else {
		return false;
	};

	if parts.headers.get("sec-fetch-site").is_some_and(|value| {
		value
			.to_str()
			.map(|site| site.eq_ignore_ascii_case("cross-site"))
			.unwrap_or(true)
	}) {
		return false;
	}

	let Ok(HostExtractor(host_details)) =
		HostExtractor::from_request_parts(parts, ctx).await
	else {
		return false;
	};
	if !origin_scheme.eq_ignore_ascii_case(&host_details.scheme) {
		return false;
	}
	// Forwarded host headers are only honoured behind a configured trusted proxy;
	// otherwise the raw `Host` header (or HTTP/2 authority) is the only authority
	// a browser origin can match.
	let expected_host = if ctx.config.server.trust_proxy_headers {
		match Host::from_request_parts(parts, ctx).await {
			Ok(Host(host)) => Some(host),
			Err(_) => return false,
		}
	} else {
		parts
			.headers
			.get(HOST)
			.and_then(|value| value.to_str().ok())
			.map(str::to_owned)
			.or_else(|| parts.uri.authority().map(ToString::to_string))
	};
	let Some(expected_authority) =
		expected_host.and_then(|value| value.parse::<Authority>().ok())
	else {
		return false;
	};

	origin_authority
		.host()
		.trim_end_matches('.')
		.eq_ignore_ascii_case(expected_authority.host().trim_end_matches('.'))
		&& effective_port(origin_authority, origin_scheme)
			== effective_port(&expected_authority, &host_details.scheme)
}

fn effective_port(authority: &Authority, scheme: &str) -> Option<u16> {
	authority.port_u16().or_else(|| match scheme {
		"http" => Some(80),
		"https" => Some(443),
		_ => None,
	})
}

pub(super) fn require_expected_participant(
	headers: &HeaderMap,
	participant_id: &str,
) -> crate::errors::APIResult<()> {
	let matches = headers
		.get(EXPECTED_PARTICIPANT_HEADER)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|value| value == participant_id);
	if matches {
		Ok(())
	} else {
		Err(crate::errors::APIError::Conflict(
			"Reader identity changed; refresh the reader before saving".to_string(),
		))
	}
}

pub(super) fn cookie_token(headers: &HeaderMap) -> crate::errors::APIResult<String> {
	let mut tokens = headers
		.get_all(axum::http::header::COOKIE)
		.iter()
		.filter_map(|value| value.to_str().ok())
		.flat_map(|cookies| cookies.split(';'))
		.filter_map(|cookie| {
			let (name, value) = cookie.trim().split_once('=')?;
			(name == READER_COOKIE_NAME).then(|| value.trim().to_owned())
		});
	let token = tokens.next().ok_or(crate::errors::APIError::Unauthorized)?;
	if tokens.next().is_some() || token.is_empty() || token.len() > 128 {
		return Err(crate::errors::APIError::Unauthorized);
	}
	Ok(token)
}

pub(super) fn secret_digest(secret: &str) -> String {
	HEXLOWER.encode(&Sha256::digest(secret.as_bytes()))
}

pub(super) fn digest_matches(secret: &str, digest: &str) -> bool {
	let candidate = secret_digest(secret);
	bool::from(candidate.as_bytes().ct_eq(digest.as_bytes()))
}

pub(super) fn generate_secret() -> crate::errors::APIResult<String> {
	let mut bytes = [0u8; 32];
	OsRng.try_fill_bytes(&mut bytes).map_err(|error| {
		tracing::error!(?error, "OS random number generator unavailable");
		crate::errors::APIError::InternalServerError(
			"Random number generator unavailable".to_string(),
		)
	})?;
	Ok(BASE64URL_NOPAD.encode(&bytes))
}

pub(super) fn credential_cookie(
	session_id: &str,
	secret: &str,
	expires_at: Option<DateTime<Utc>>,
	secure: bool,
) -> crate::errors::APIResult<HeaderValue> {
	if !session_id.chars().all(|character| {
		character.is_ascii_alphanumeric() || character == '-' || character == '_'
	}) {
		return Err(crate::errors::APIError::InternalServerError(
			"Invalid reader session identifier".to_string(),
		));
	}
	let mut cookie = format!(
		"{READER_COOKIE_NAME}={secret}; Path=/api/v2/club-reader/sessions/{session_id}; HttpOnly; SameSite=Strict"
	);
	if secure {
		cookie.push_str("; Secure");
	}
	if let Some(deadline) = expires_at {
		let max_age = (deadline - Utc::now()).num_seconds().max(0);
		cookie.push_str(&format!("; Max-Age={max_age}"));
	}
	HeaderValue::from_str(&cookie).map_err(|_| {
		crate::errors::APIError::InternalServerError(
			"Unable to create reader credential cookie".to_string(),
		)
	})
}

pub(super) async fn authorize_club_member(
	ctx: &AppState,
	user: &AuthUser,
	club_id: &str,
	required_role: Option<BookClubMemberRole>,
	require_reader_permission: bool,
) -> crate::errors::APIResult<bool> {
	if user.is_locked {
		return Err(crate::errors::APIError::Forbidden(
			"You do not have access to this book club".to_string(),
		));
	}
	if user.is_server_owner {
		return Ok(true);
	}
	let permissions = if require_reader_permission {
		&[
			UserPermission::AccessBookClub,
			UserPermission::ShareBookClubReader,
		][..]
	} else {
		&[UserPermission::AccessBookClub][..]
	};
	if !user_has_all_permissions(user, permissions) {
		return Err(crate::errors::APIError::Forbidden(
			"You do not have access to this book club".to_string(),
		));
	}
	let membership = book_club_member::Entity::find_by_club_for_user(user, club_id)
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| {
			crate::errors::APIError::Forbidden(
				"You do not have access to this book club".to_string(),
			)
		})?;
	let required_role = required_role.unwrap_or(BookClubMemberRole::Member);
	if membership.role < required_role {
		return Err(crate::errors::APIError::Forbidden(
			"You do not have permission to manage this reader session".to_string(),
		));
	}
	Ok(membership.role >= BookClubMemberRole::Admin && require_reader_permission)
}

pub(super) fn is_expired(deadline: Option<DateTimeWithTimeZone>) -> bool {
	deadline.is_some_and(|deadline| deadline.with_timezone(&Utc) <= Utc::now())
}
