//! The worker lane: the socket a `stump-worker` dials in on, and the route it
//! uploads a job's bytes to.
//!
//! Both routes sit behind the ordinary `auth_middleware`, which is the whole
//! point: a worker authenticates with a device credential like every other
//! client, so the middleware has already resolved the device and narrowed the
//! request before either handler runs. Nothing here re-implements auth; it only
//! adds the one extra rule a worker route has — the device must be of kind
//! [`DeviceKind::Worker`], because a Kobo's API key must not be able to claim
//! jobs and read back other users' books as their inputs.
//!
//! The frame loop is deliberately thin. Everything stateful lives in
//! [`stump_worker::WorkerJobs`]: this module reads text off the socket, hands
//! it over, and writes back whatever the hub queues. That is what lets the
//! crate's own test drive the identical state machine through its own listener.

use std::sync::Arc;

use axum::{
	body::Bytes,
	extract::{
		ws::{Message, WebSocket, WebSocketUpgrade},
		FromRequestParts, Path, State,
	},
	http::request::Parts,
	middleware,
	response::Response,
	routing::{get, put},
	Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use models::{entity::device, shared::enums::DeviceKind};
use sea_orm::EntityTrait;
use serde::Serialize;
use sha2::{Digest, Sha256};
use stump_auth::AuthContext;
use stump_worker::{protocol::parse_worker_frame, WorkerError, WorkerJobs};

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	middleware::auth::auth_middleware,
};

pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	Router::new()
		.route("/workers/socket", get(socket))
		.route("/workers/jobs/{id}/output", put(upload_output))
		.layer(middleware::from_fn_with_state(app_state, auth_middleware))
}

/// The live worker device behind this request.
///
/// An **extractor**, not a check in the handler body, and it is declared before
/// `WebSocketUpgrade` in every signature that takes both. Axum runs extractors
/// in argument order and a rejection short-circuits, so the credential is
/// refused before any upgrade work happens — a caller with the wrong credential
/// gets `403`, never a transport error that hides the real reason.
///
/// A request with no device (a session, a JWT, a password) is refused even for
/// the server owner: the worker lane is addressed *per worker*, and a job
/// offered to "the owner" has nowhere to go.
pub(crate) struct WorkerDevice(pub device::Model);

impl FromRequestParts<AppState> for WorkerDevice {
	type Rejection = APIError;

	async fn from_request_parts(
		parts: &mut Parts,
		ctx: &AppState,
	) -> Result<Self, Self::Rejection> {
		let auth = parts
			.extensions
			.get::<AuthContext>()
			.ok_or(APIError::Unauthorized)?;
		let Some(device_id) = auth.device_id() else {
			return Err(APIError::Forbidden(
				"the worker lane requires a device credential".to_string(),
			));
		};
		let device = device::Entity::find_by_id(device_id)
			.one(ctx.conn.as_ref())
			.await?
			.ok_or_else(|| APIError::Forbidden("unknown device".to_string()))?;
		if device.kind != DeviceKind::Worker {
			return Err(APIError::Forbidden(
				"this device is not a worker".to_string(),
			));
		}
		if device.revoked_at.is_some() {
			return Err(APIError::Forbidden("this device is revoked".to_string()));
		}
		Ok(Self(device))
	}
}

/// `GET /api/v2/workers/socket`
///
/// The worker sends `hello`, then `claim` / `progress` / `result` / `fail`; the
/// server sends `job` and `cancel`. See
/// `docs/content/docs/developer/workers.mdx`.
async fn socket(
	State(ctx): State<AppState>,
	WorkerDevice(device): WorkerDevice,
	upgrade: WebSocketUpgrade,
) -> APIResult<Response> {
	let jobs = ctx.worker_jobs();
	Ok(upgrade.on_upgrade(move |socket| serve(jobs, device, socket)))
}

/// One connection's lifetime.
async fn serve(jobs: Arc<WorkerJobs>, device: device::Model, socket: WebSocket) {
	let mut socket = socket;
	// The first frame must be `hello`: until the hub knows what this worker
	// can do it cannot be offered anything, so there is nothing else to do
	// with a connection that starts any other way.
	let Some(Ok(Message::Text(hello))) = socket.recv().await else {
		tracing::debug!(device = %device.id, "Worker socket closed before hello");
		return;
	};
	let hello = match parse_worker_frame(&hello) {
		Ok(frame) => frame,
		Err(error) => {
			tracing::warn!(%error, device = %device.id, "Unparseable worker hello");
			return;
		},
	};

	let (mut outbound, epoch) =
		match jobs.attach_worker(&device.id, &device.name, hello).await {
			Ok(attached) => attached,
			Err(error) => {
				tracing::warn!(?error, device = %device.id, "Refused a worker connection");
				return;
			},
		};
	tracing::info!(device = %device.id, name = %device.name, "Worker connected");

	let (mut writer, mut reader) = socket.split();
	let writes = tokio::spawn(async move {
		while let Some(text) = outbound.recv().await {
			if writer.send(Message::Text(text.into())).await.is_err() {
				break;
			}
		}
		let _ = writer.close().await;
	});

	while let Some(Ok(message)) = reader.next().await {
		let Message::Text(text) = message else {
			// Ping/pong are answered by axum; a binary frame is not part of
			// the protocol and is ignored rather than fatal.
			continue;
		};
		match parse_worker_frame(&text) {
			Ok(frame) => {
				if let Err(error) = jobs.handle_frame(&device.id, frame).await {
					// A frame for a job this worker no longer holds is a race,
					// not an attack: log it and keep the connection, because
					// dropping it would strand the jobs it does hold.
					tracing::debug!(?error, device = %device.id, "Worker frame refused");
				}
			},
			Err(error) => {
				tracing::warn!(%error, device = %device.id, "Unparseable worker frame");
			},
		}
	}

	writes.abort();
	if let Err(error) = jobs.detach_worker(&device.id, epoch).await {
		tracing::warn!(?error, device = %device.id, "Failed to release a worker");
	}
	tracing::info!(device = %device.id, "Worker disconnected");
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadAccepted {
	pub bytes: u64,
	pub sha256: String,
}

/// `PUT /api/v2/workers/jobs/{id}/output`
///
/// The raw body is the job's output; `x-stump-sha256` is its lowercase-hex
/// SHA-256 and is **required**. Raw rather than multipart because the body is
/// one file and nothing else, and the digest is required rather than optional
/// because these bytes are published straight into the delivery cache — an
/// upload truncated by a dropped connection would otherwise be served to every
/// later request as a valid cache entry.
///
/// The write is staged and renamed, so a failed upload leaves no partial file
/// for the awaiting caller to publish.
async fn upload_output(
	State(ctx): State<AppState>,
	WorkerDevice(device): WorkerDevice,
	Path(id): Path<String>,
	headers: axum::http::HeaderMap,
	body: Bytes,
) -> APIResult<Json<UploadAccepted>> {
	let jobs = ctx.worker_jobs();

	let declared = headers
		.get("x-stump-sha256")
		.and_then(|value| value.to_str().ok())
		.map(str::to_ascii_lowercase)
		.ok_or_else(|| APIError::BadRequest("x-stump-sha256 is required".to_string()))?;

	// An output larger than the whole delivery cache cannot be served from it,
	// so accepting it would only fill the disk.
	let limit = ctx.config.transform.transform_cache_max_bytes;
	if body.len() as u64 > limit {
		return Err(APIError::BadRequest(format!(
			"output is larger than the transform cache budget ({limit} bytes)"
		)));
	}

	let destination =
		jobs.upload_path(&id, &device.id)
			.await
			.map_err(|error| match error {
				WorkerError::NotFound(_) => APIError::NotFound("unknown job".to_string()),
				WorkerError::NotAssigned { .. } => APIError::Forbidden(
					"this job is not assigned to this worker".to_string(),
				),
				other => APIError::InternalServerError(other.to_string()),
			})?;

	let mut hasher = Sha256::new();
	hasher.update(&body);
	let actual =
		hasher
			.finalize()
			.iter()
			.fold(String::with_capacity(64), |mut acc, byte| {
				use std::fmt::Write;
				let _ = write!(acc, "{byte:02x}");
				acc
			});
	if actual != declared {
		return Err(APIError::BadRequest(
			"the uploaded bytes do not match x-stump-sha256".to_string(),
		));
	}

	let staged = destination.with_extension("part");
	tokio::fs::write(&staged, &body).await.map_err(|error| {
		APIError::InternalServerError(format!("failed to stage the output: {error}"))
	})?;
	tokio::fs::rename(&staged, &destination)
		.await
		.map_err(|error| {
			APIError::InternalServerError(format!(
				"failed to publish the output: {error}"
			))
		})?;

	tracing::debug!(job_id = %id, worker = %device.id, bytes = body.len(), "Worker output accepted");
	Ok(Json(UploadAccepted {
		bytes: body.len() as u64,
		sha256: actual,
	}))
}
