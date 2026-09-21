//! Device pairing: the unauthenticated half of the flow that lets a companion
//! app (Coppice KOReader/Kobo plugins, Mihon, ...) obtain device credential(s)
//! without ever seeing the user's password.
//!
//! 1. `POST /devices/pair/start` creates a pending pairing and hands the device a
//!    6-digit code plus a random nonce (also embedded in the `stump://pair` QR
//!    payload). The code is stored hashed; the nonce is the device's poll secret.
//! 2. A signed-in user approves the pairing over GraphQL
//!    (`approveDevicePairing`) by entering the code or by scanning the QR, which
//!    binds the pairing to that user.
//! 3. `GET /devices/pair/{id}/status?nonce=` reports the state; the first poll
//!    after approval mints the device credential set through the devices service
//!    and returns every secret exactly once.
//!
//! See `docs/content/docs/developer/device-pairing.mdx` for the sequence diagram
//! and threat model.

use axum::{
	extract::{Path, Query, State},
	routing::{get, post},
	Json, Router,
};
#[cfg(feature = "qr")]
use axum::{
	http::{header, HeaderValue, StatusCode},
	response::IntoResponse,
};
use chrono::{SecondsFormat, Utc};
use models::{
	entity::{
		device_pairing::{self, DevicePairingStatus},
		user::{self, AuthUser, LoginUser},
	},
	shared::enums::DeviceKind,
};
use rand::{rngs::OsRng, TryRngCore};
use sea_orm::{prelude::*, ActiveValue::Set};
use serde::{Deserialize, Serialize};
use stump_api_types::RequestOrigin;
use stump_core::{CoreEvent, DevicePaired, DevicePairingRequested};
use stump_devices::{CredentialKind, Endpoint, IssuedCredential, Protocol};
use subtle::ConstantTimeEq;

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	middleware::{ClientIp, HostExtractor},
	utils::hash_password,
};

/// Suggested delay between two status polls of the same pairing.
pub const POLL_INTERVAL_SECS: u32 = 2;
/// Pending (unexpired) pairings a single remote IP may hold at once.
const MAX_PENDING_PER_IP: u64 = 10;
const MAX_NAME_LEN: usize = 100;
/// Bytes of OS entropy in the nonce (hex-encoded to twice as many characters).
const NONCE_BYTES: usize = 16;

pub(crate) fn mount(_app_state: AppState) -> Router<AppState> {
	let router = Router::new()
		.route("/start", post(start_pairing))
		.route("/{pairing_id}/status", get(pairing_status));

	#[cfg(feature = "qr")]
	let router = router.route("/{pairing_id}/qr.png", get(pairing_qr_png));

	Router::new().nest("/devices/pair", router)
}

#[derive(Debug, Deserialize)]
pub struct StartPairingInput {
	pub kind: DeviceKind,
	#[serde(default)]
	pub name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StartPairingResponse {
	pub pairing_id: String,
	/// Six digits, zero-padded; shown on the device for the user to type
	pub code: String,
	/// Poll secret; also embedded in `qr_payload`
	pub nonce: String,
	/// `stump://pair?host=<origin>&id=<pairing_id>&nonce=<nonce>`
	pub qr_payload: String,
	/// RFC 3339 UTC timestamp with second precision
	pub expires_at: String,
	pub poll_interval_secs: u32,
}

#[derive(Debug, Deserialize)]
pub struct NonceQuery {
	pub nonce: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WirePairingStatus {
	Pending,
	Approved,
	Denied,
	Expired,
}

impl From<DevicePairingStatus> for WirePairingStatus {
	fn from(status: DevicePairingStatus) -> Self {
		match status {
			DevicePairingStatus::Pending => Self::Pending,
			DevicePairingStatus::Approved => Self::Approved,
			DevicePairingStatus::Denied => Self::Denied,
			DevicePairingStatus::Expired => Self::Expired,
		}
	}
}

#[derive(Serialize)]
pub struct PairedDevice {
	pub id: String,
	pub name: String,
	pub kind: DeviceKind,
}

/// The wire form of a minted credential. Owned here so the client contract
/// (`kind ∈ api_key | liseur_token | session`) does not depend on the devices
/// crate's serde attributes.
#[derive(Serialize)]
pub struct PairedCredential {
	pub kind: &'static str,
	pub protocol: Protocol,
	pub credential_ref: String,
	pub secret: String,
}

impl From<&IssuedCredential> for PairedCredential {
	fn from(issued: &IssuedCredential) -> Self {
		Self {
			kind: match issued.kind {
				CredentialKind::ApiKey => "api_key",
				CredentialKind::LiseurToken => "liseur_token",
				CredentialKind::Session => "session",
			},
			protocol: issued.protocol,
			credential_ref: issued.credential_ref.clone(),
			secret: issued.secret.clone(),
		}
	}
}

impl From<IssuedCredential> for PairedCredential {
	fn from(issued: IssuedCredential) -> Self {
		Self::from(&issued)
	}
}

#[derive(Serialize)]
pub struct PairingStatusResponse {
	pub status: WirePairingStatus,
	/// Present once approved. `true` on every approved response, since the
	/// credentials are minted by (and returned from) the first approved poll.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub credential_issued: Option<bool>,
	/// Only on the poll that mints the credentials.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub device: Option<PairedDevice>,
	/// The approved account name, returned with one-time credentials so
	/// clients can configure Basic/KOSync lanes without a password flow.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub username: Option<String>,
	/// Every credential minted in this approval, returned only once. Clients
	/// select by kind and protocol rather than list position.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub credentials: Option<Vec<PairedCredential>>,
	/// The primary credential, retained for older clients that only understand
	/// the singular field.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub credential: Option<PairedCredential>,
	/// Only on the poll that mints the credentials.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub endpoints: Option<Vec<Endpoint>>,
}

impl PairingStatusResponse {
	fn bare(status: DevicePairingStatus) -> Self {
		Self {
			credential_issued: matches!(status, DevicePairingStatus::Approved)
				.then_some(true),
			status: status.into(),
			device: None,
			username: None,
			credentials: None,
			credential: None,
			endpoints: None,
		}
	}
}

/// Build the deep link a companion app opens after scanning the QR code.
pub fn qr_payload(origin: &str, pairing_id: &str, nonce: &str) -> String {
	format!(
		"stump://pair?host={}&id={}&nonce={}",
		urlencoding::encode(origin),
		pairing_id,
		nonce
	)
}

fn os_random_bytes<const N: usize>() -> APIResult<[u8; N]> {
	let mut bytes = [0u8; N];
	OsRng.try_fill_bytes(&mut bytes).map_err(|error| {
		tracing::error!(?error, "OS random number generator unavailable");
		APIError::InternalServerError("Random number generator unavailable".to_string())
	})?;
	Ok(bytes)
}

/// A uniformly distributed 6-digit code (`000000`..=`999999`) from the OS RNG.
/// Rejection sampling keeps the distribution exact instead of biasing the low
/// codes the way a plain modulo would.
fn generate_code() -> APIResult<String> {
	const SPACE: u32 = 1_000_000;
	const LIMIT: u32 = u32::MAX - (u32::MAX % SPACE);
	loop {
		let raw = u32::from_le_bytes(os_random_bytes::<4>()?);
		if raw < LIMIT {
			return Ok(format!("{:06}", raw % SPACE));
		}
	}
}

fn generate_nonce() -> APIResult<String> {
	let bytes = os_random_bytes::<NONCE_BYTES>()?;
	Ok(data_encoding::HEXLOWER.encode(&bytes))
}

fn normalize_name(name: Option<String>) -> APIResult<Option<String>> {
	let Some(name) = name else {
		return Ok(None);
	};
	let trimmed = name.trim();
	if trimmed.is_empty() {
		return Ok(None);
	}
	if trimmed.chars().count() > MAX_NAME_LEN {
		return Err(APIError::BadRequest(format!(
			"Device name must be at most {MAX_NAME_LEN} characters"
		)));
	}
	Ok(Some(trimmed.to_string()))
}

fn rfc3339_secs(at: DateTimeWithTimeZone) -> String {
	at.with_timezone(&Utc)
		.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Start a pairing. Unauthenticated (the device has nothing yet); the outer
/// rate limiter treats this path as an authentication attempt, and each remote
/// IP is additionally capped at [`MAX_PENDING_PER_IP`] open pairings.
async fn start_pairing(
	State(ctx): State<AppState>,
	ClientIp(client_ip): ClientIp,
	HostExtractor(details): HostExtractor,
	Json(input): Json<StartPairingInput>,
) -> APIResult<Json<StartPairingResponse>> {
	let conn = ctx.conn.as_ref();
	let name = normalize_name(input.name)?;
	let remote_ip = client_ip.to_string();
	let now = DateTimeWithTimeZone::from(Utc::now());

	let open_for_ip = device_pairing::Entity::find()
		.filter(
			device_pairing::Column::RemoteIp
				.eq(remote_ip.as_str())
				.and(device_pairing::Column::Status.eq(DevicePairingStatus::Pending))
				.and(device_pairing::Column::ExpiresAt.gt(now)),
		)
		.count(conn)
		.await?;
	if open_for_ip >= MAX_PENDING_PER_IP {
		tracing::warn!(%remote_ip, open_for_ip, "Too many open device pairings");
		return Err(APIError::TooManyRequests);
	}

	let code = generate_code()?;
	let nonce = generate_nonce()?;
	let code_hash = hash_password(&code, &ctx.config)?;

	let pairing = device_pairing::ActiveModel {
		kind: Set(input.kind),
		name: Set(name),
		code_hash: Set(code_hash),
		nonce: Set(nonce),
		remote_ip: Set(remote_ip),
		status: Set(DevicePairingStatus::Pending),
		failed_attempts: Set(0),
		credential_issued: Set(false),
		..Default::default()
	}
	.insert(conn)
	.await?;

	tracing::info!(
		pairing_id = %pairing.id,
		kind = ?pairing.kind,
		name = ?pairing.name,
		remote_ip = %pairing.remote_ip,
		"Device pairing requested"
	);
	ctx.emit_event(CoreEvent::DevicePairingRequested(DevicePairingRequested {
		pairing_id: pairing.id.clone(),
		kind: pairing.kind,
		name: pairing.name.clone(),
		remote_ip: pairing.remote_ip.clone(),
		expires_at: pairing.expires_at,
	}));

	let origin = details.url();
	Ok(Json(StartPairingResponse {
		qr_payload: qr_payload(&origin, &pairing.id, &pairing.nonce),
		pairing_id: pairing.id,
		code,
		nonce: pairing.nonce,
		expires_at: rfc3339_secs(pairing.expires_at),
		poll_interval_secs: POLL_INTERVAL_SECS,
	}))
}

/// Load a pairing for its device. Unknown ids and wrong nonces are
/// indistinguishable so the id space cannot be probed.
async fn find_for_device(
	conn: &DatabaseConnection,
	pairing_id: &str,
	nonce: &str,
) -> APIResult<device_pairing::Model> {
	let pairing = device_pairing::Entity::find_by_id(pairing_id)
		.one(conn)
		.await?
		.ok_or_else(|| APIError::NotFound("Pairing not found".to_string()))?;

	let nonce_matches: bool = pairing.nonce.as_bytes().ct_eq(nonce.as_bytes()).into();
	if !nonce_matches {
		tracing::warn!(%pairing_id, "Device pairing polled with a mismatched nonce");
		return Err(APIError::NotFound("Pairing not found".to_string()));
	}

	Ok(pairing)
}

/// Persist `EXPIRED` for a pending pairing whose deadline passed, so the row
/// reads correctly without recomputing.
async fn persist_expiry(
	conn: &DatabaseConnection,
	pairing: &device_pairing::Model,
) -> APIResult<()> {
	device_pairing::Entity::update_many()
		.col_expr(
			device_pairing::Column::Status,
			Expr::value(DevicePairingStatus::Expired),
		)
		.filter(
			device_pairing::Column::Id
				.eq(pairing.id.as_str())
				.and(device_pairing::Column::Status.eq(DevicePairingStatus::Pending)),
		)
		.exec(conn)
		.await?;
	Ok(())
}

/// Atomically flip `credential_issued` for an approved pairing. Exactly one
/// concurrent poll wins; the others see `false` and get the bare approved body.
async fn claim_credential(
	conn: &DatabaseConnection,
	pairing_id: &str,
) -> APIResult<bool> {
	let result = device_pairing::Entity::update_many()
		.col_expr(device_pairing::Column::CredentialIssued, Expr::value(true))
		.filter(
			device_pairing::Column::Id
				.eq(pairing_id)
				.and(device_pairing::Column::Status.eq(DevicePairingStatus::Approved))
				.and(device_pairing::Column::CredentialIssued.eq(false)),
		)
		.exec(conn)
		.await?;
	Ok(result.rows_affected == 1)
}

async fn release_credential_claim(conn: &DatabaseConnection, pairing_id: &str) {
	let released = device_pairing::Entity::update_many()
		.col_expr(device_pairing::Column::CredentialIssued, Expr::value(false))
		.filter(device_pairing::Column::Id.eq(pairing_id))
		.exec(conn)
		.await;
	if let Err(error) = released {
		tracing::error!(?error, %pairing_id, "Failed to release device pairing credential claim");
	}
}

/// Mint the device and its credential for an approved pairing. Called by the
/// poll that won [`claim_credential`]; on failure the claim is released so the
/// device can retry on its next poll.
async fn issue_credential(
	ctx: &AppState,
	pairing: &device_pairing::Model,
	origin: &RequestOrigin,
) -> APIResult<PairingStatusResponse> {
	let conn = ctx.conn.as_ref();
	let Some(user_id) = pairing.user_id.as_deref() else {
		tracing::error!(pairing_id = %pairing.id, "Approved device pairing has no user");
		release_credential_claim(conn, &pairing.id).await;
		return Err(APIError::InternalServerError(
			"Pairing is approved but not bound to a user".to_string(),
		));
	};

	let approver = LoginUser::find_by_id(user_id.to_string())
		.filter(user::Column::DeletedAt.is_null())
		.into_model::<LoginUser>()
		.one(conn)
		.await?;
	let approver = match approver {
		Some(approver) if !approver.is_locked => AuthUser::from(approver),
		_ => {
			release_credential_claim(conn, &pairing.id).await;
			return Err(APIError::Forbidden(
				"The approving account is no longer available".to_string(),
			));
		},
	};

	let devices = ctx.devices();
	let (device, credential) = match devices
		.create_device(&approver, pairing.kind, pairing.name.clone())
		.await
	{
		Ok(minted) => minted,
		Err(error) => {
			tracing::error!(?error, pairing_id = %pairing.id, "Failed to mint device credential for pairing");
			release_credential_claim(conn, &pairing.id).await;
			return Err(error.into());
		},
	};
	// The secret is already minted; a failure to describe endpoints must not
	// cost the device its one chance to receive it.
	let endpoints = devices
		.endpoints_with_secret(&approver, &device, &credential, origin)
		.await
		.unwrap_or_else(|error| {
			tracing::error!(?error, device_id = %device.id, "Failed to describe device endpoints");
			Vec::new()
		});

	tracing::info!(
		pairing_id = %pairing.id,
		device_id = %device.id,
		user_id = %approver.id,
		"Issued device credential for pairing"
	);

	ctx.emit_event(CoreEvent::DevicePaired(DevicePaired {
		device_id: device.id.clone(),
		user_id: approver.id.clone(),
		device_name: device.name.clone(),
	}));

	let credentials = credential
		.credentials()
		.iter()
		.map(PairedCredential::from)
		.collect();
	let primary = PairedCredential::from(&credential);
	Ok(PairingStatusResponse {
		status: WirePairingStatus::Approved,
		credential_issued: Some(true),
		device: Some(PairedDevice {
			id: device.id,
			name: device.name,
			kind: device.kind,
		}),
		username: Some(approver.username.clone()),
		credentials: Some(credentials),
		credential: Some(primary),
		endpoints: Some(endpoints),
	})
}

/// Poll a pairing. Requires the nonce handed out by `start`; always `200` with
/// a `status` for a known pairing.
async fn pairing_status(
	State(ctx): State<AppState>,
	HostExtractor(details): HostExtractor,
	Path(pairing_id): Path<String>,
	Query(NonceQuery { nonce }): Query<NonceQuery>,
) -> APIResult<Json<PairingStatusResponse>> {
	let conn = ctx.conn.as_ref();
	let pairing = find_for_device(conn, &pairing_id, &nonce).await?;
	let now = DateTimeWithTimeZone::from(Utc::now());

	let status = pairing.effective_status(now);
	if status == DevicePairingStatus::Expired && pairing.status != status {
		persist_expiry(conn, &pairing).await?;
	}

	if status != DevicePairingStatus::Approved || pairing.credential_issued {
		return Ok(Json(PairingStatusResponse::bare(status)));
	}

	if !claim_credential(conn, &pairing.id).await? {
		return Ok(Json(PairingStatusResponse::bare(status)));
	}

	let origin = RequestOrigin::new(details.host, details.scheme);
	issue_credential(&ctx, &pairing, &origin).await.map(Json)
}

/// PNG rendering of the QR payload for devices without a QR library. Same
/// nonce requirement as the status poll (the image contains the nonce).
#[cfg(feature = "qr")]
async fn pairing_qr_png(
	State(ctx): State<AppState>,
	HostExtractor(details): HostExtractor,
	Path(pairing_id): Path<String>,
	Query(NonceQuery { nonce }): Query<NonceQuery>,
) -> APIResult<impl IntoResponse> {
	let pairing = find_for_device(ctx.conn.as_ref(), &pairing_id, &nonce).await?;
	let payload = qr_payload(&details.url(), &pairing.id, &pairing.nonce);
	let png = qr::render_png(&payload)?;

	Ok((
		StatusCode::OK,
		[
			(header::CONTENT_TYPE, HeaderValue::from_static("image/png")),
			(header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
		],
		png,
	))
}

#[cfg(feature = "qr")]
mod qr {
	use std::io::Cursor;

	use image::{ImageFormat, Luma};
	use qrcode::{EcLevel, QrCode};

	use crate::errors::{APIError, APIResult};

	/// Pixels per QR module; 8 keeps a version-4 code readable on e-ink.
	const MODULE_PX: u32 = 8;

	pub fn render_png(payload: &str) -> APIResult<Vec<u8>> {
		let code = QrCode::with_error_correction_level(payload.as_bytes(), EcLevel::M)
			.map_err(|error| {
				tracing::error!(?error, "Failed to encode pairing QR code");
				APIError::InternalServerError("Failed to encode QR code".to_string())
			})?;
		let image = code
			.render::<Luma<u8>>()
			.quiet_zone(true)
			.module_dimensions(MODULE_PX, MODULE_PX)
			.build();

		let mut png = Vec::new();
		image
			.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
			.map_err(|error| {
				tracing::error!(?error, "Failed to encode pairing QR PNG");
				APIError::InternalServerError("Failed to encode QR image".to_string())
			})?;
		Ok(png)
	}

	#[cfg(test)]
	mod tests {
		use super::*;

		#[test]
		fn renders_a_png_for_the_deep_link() {
			let png = render_png(
				"stump://pair?host=http%3A%2F%2F127.0.0.1%3A10801&id=00000000-0000-4000-8000-000000000000&nonce=0123456789abcdef0123456789abcdef",
			)
			.expect("payload fits a QR code");
			assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
			let decoded = image::load_from_memory(&png).expect("valid PNG");
			assert!(decoded.width() >= 33 * MODULE_PX);
			assert_eq!(decoded.width(), decoded.height());
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn code_is_six_zero_padded_digits() {
		for _ in 0..64 {
			let code = generate_code().unwrap();
			assert_eq!(code.len(), 6, "{code}");
			assert!(code.bytes().all(|b| b.is_ascii_digit()), "{code}");
		}
	}

	#[test]
	fn nonce_is_32_lowercase_hex_chars() {
		let nonce = generate_nonce().unwrap();
		assert_eq!(nonce.len(), NONCE_BYTES * 2);
		assert!(nonce
			.bytes()
			.all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
		assert_ne!(nonce, generate_nonce().unwrap());
	}

	#[test]
	fn qr_payload_encodes_the_origin() {
		assert_eq!(
			qr_payload("http://192.168.8.153:25600", "abc", "ff"),
			"stump://pair?host=http%3A%2F%2F192.168.8.153%3A25600&id=abc&nonce=ff"
		);
	}

	#[test]
	fn name_is_trimmed_bounded_and_optional() {
		assert_eq!(normalize_name(None).unwrap(), None);
		assert_eq!(normalize_name(Some("   ".into())).unwrap(), None);
		assert_eq!(
			normalize_name(Some("  Clara  ".into())).unwrap(),
			Some("Clara".into())
		);
		assert!(normalize_name(Some("x".repeat(MAX_NAME_LEN + 1))).is_err());
	}

	#[test]
	fn expires_at_is_rfc3339_utc_seconds() {
		let at =
			DateTimeWithTimeZone::parse_from_rfc3339("2026-09-05T14:03:09.123456+02:00")
				.unwrap();
		assert_eq!(rfc3339_secs(at), "2026-09-05T12:03:09Z");
	}

	#[test]
	fn bare_status_marks_credential_issued_only_when_approved() {
		let approved = PairingStatusResponse::bare(DevicePairingStatus::Approved);
		assert_eq!(approved.credential_issued, Some(true));
		assert!(approved.credential.is_none());
		let pending = PairingStatusResponse::bare(DevicePairingStatus::Pending);
		assert_eq!(pending.credential_issued, None);
		assert_eq!(
			serde_json::to_value(pending).unwrap(),
			serde_json::json!({ "status": "pending" })
		);
	}
}
