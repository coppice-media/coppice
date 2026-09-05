//! `POST /api/v2/devices/me/touch`: a paired liseur-sync device reports a
//! free-form sync summary (battery, firmware, reading statistics, ...).
//!
//! The bearer is the device's liseur secret, exactly as on `/v1/*`.  The body
//! is stored verbatim as the device's `last_sync_summary` through the devices
//! service (last writer wins), which also bumps `last_seen_at`/`last_sync_at`.
//! Servers built without `liseur-sync` do not expose the route; clients treat
//! `404` as "summaries unsupported" and keep syncing.

use axum::{
	extract::State,
	http::{header, HeaderMap, StatusCode},
	routing::post,
	Json, Router,
};
use serde_json::Value;
use stump_devices::{CredentialRef, Protocol};
use stump_liseur_sync::{LiseurSyncError, LiseurTokenKind};

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
};

use super::storage;

/// Upper bound on the serialized summary; it is displayed, not processed.
const MAX_SUMMARY_BYTES: usize = 16 * 1024;

pub(crate) fn router() -> Router<AppState> {
	Router::new().route("/api/v2/devices/me/touch", post(touch))
}

fn bearer(headers: &HeaderMap) -> APIResult<&str> {
	headers
		.get(header::AUTHORIZATION)
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.strip_prefix("Bearer "))
		.filter(|value| !value.is_empty())
		.ok_or(APIError::Unauthorized)
}

fn map_auth_error(error: LiseurSyncError) -> APIError {
	match error {
		LiseurSyncError::Unauthorized => APIError::Unauthorized,
		LiseurSyncError::Forbidden(message) => APIError::Forbidden(message),
		other => APIError::InternalServerError(other.to_string()),
	}
}

async fn touch(
	State(ctx): State<AppState>,
	headers: HeaderMap,
	Json(summary): Json<Value>,
) -> APIResult<StatusCode> {
	let secret = bearer(&headers)?;
	let token = storage::authenticate(&ctx, secret)
		.await
		.map_err(map_auth_error)?;
	if token.kind != LiseurTokenKind::Device {
		return Err(APIError::Forbidden(
			"a device token is required".to_string(),
		));
	}
	let token_id = token.token_id.ok_or_else(|| {
		APIError::InternalServerError("liseur token has no identifier".to_string())
	})?;

	if !summary.is_object() {
		return Err(APIError::BadRequest(
			"summary must be a JSON object".to_string(),
		));
	}
	if serde_json::to_vec(&summary)
		.map(|bytes| bytes.len())
		.unwrap_or(usize::MAX)
		> MAX_SUMMARY_BYTES
	{
		return Err(APIError::BadRequest(format!(
			"summary exceeds {MAX_SUMMARY_BYTES} bytes"
		)));
	}

	let seen = ctx
		.devices()
		.touch(
			CredentialRef::LiseurToken(&token_id),
			Protocol::Liseur,
			Some(summary),
		)
		.await?;
	match seen {
		Some(_) => Ok(StatusCode::NO_CONTENT),
		None => Err(APIError::NotFound(
			"this credential is not bound to a device".to_string(),
		)),
	}
}
