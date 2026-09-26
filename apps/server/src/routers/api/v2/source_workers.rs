//! Source-worker control, inventory reconciliation, and remote byte transfers.
//!
//! Source workers are deliberately separate from compute workers.  Their
//! credential can only authenticate this module, and the control socket carries
//! JSON frames only; a tunnel WebSocket carries bounded binary chunks.

use std::{
	collections::{BTreeMap, BTreeSet, HashMap, HashSet},
	sync::LazyLock,
	time::Duration,
};

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	middleware::auth::auth_middleware,
};
use axum::body::Bytes;
use axum::{
	body::Body,
	extract::{
		ws::{Message, WebSocket, WebSocketUpgrade},
		FromRequestParts, Path, State,
	},
	http::{header, request::Parts},
	middleware,
	response::Response,
	routing::{get, post},
	Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, Stream, StreamExt};
use models::{
	entity::{
		device, media, media_location, media_metadata, remote_source,
		remote_source_import, remote_source_item,
	},
	shared::enums::{DeviceKind, UserPermission},
};
use parking_lot::Mutex;
use sea_orm::{
	prelude::*, ActiveModelTrait, ConnectionTrait, EntityTrait, IntoActiveModel,
	QueryFilter, QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use stump_auth::AuthContext;
use stump_worker::{
	source_protocol::{
		expires_at_millis, parse_source_worker_frame, SourceManifestChunk,
		SourceManifestItem, SourceReadMode, SourceReadRequest, SourceRootHello,
		SourceServerFrame, SourceTransport, SourceWorkerFrame, SourceWorkerHello,
		MAX_SOURCE_MANIFEST_FRAME_BYTES, MAX_SOURCE_MANIFEST_ITEMS,
	},
	SourceHub, SOURCE_TRANSFER_IDLE_TIMEOUT,
};
#[cfg(feature = "ingest")]
use tokio::io::AsyncWriteExt;

/// How long a worker has to answer a read grant with `ReadReady`.  The window
/// bounds acceptance only; an accepted stream is bounded by the hub's idle
/// deadline and its byte budget instead.
const REMOTE_READ_TTL_SECS: i64 = 60;
/// Connect phase of a direct-transport fetch.
const DIRECT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REMOTE_LOCATION_KIND: &str = "remote_source";
const ONLINE: &str = "online";
const OBSERVED: &str = "observed";
const VERIFIED: &str = "source_verified";
const MISSING: &str = "missing";
const PROPOSED: &str = "proposed";
const APPROVED: &str = "approved";
const REJECTED: &str = "rejected";

/// In-flight manifest batches are intentionally server-local.  A disconnected
/// batch is never reconciled and therefore cannot produce omission tombstones.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ManifestKey {
	device_id: String,
	root_id: String,
	batch_id: String,
	revision: u64,
}

#[derive(Default)]
struct ManifestBatch {
	next_sequence: u64,
	fingerprints: HashMap<u64, String>,
	/// Every item id the batch has committed so far; a later chunk repeating
	/// one is a conflicting manifest, not a newer observation.
	worker_ids: HashSet<String>,
}

static MANIFESTS: LazyLock<Mutex<HashMap<ManifestKey, ManifestBatch>>> =
	LazyLock::new(|| Mutex::new(HashMap::new()));

fn manifests() -> &'static Mutex<HashMap<ManifestKey, ManifestBatch>> {
	&MANIFESTS
}

/// Proposals whose materialization this process is applying right now.  A
/// claimed proposal carries no outcome until its bytes are staged; this set
/// tells a concurrent approval apart from one left behind by a crash.
static MATERIALIZING: LazyLock<Mutex<HashSet<String>>> =
	LazyLock::new(|| Mutex::new(HashSet::new()));

/// The direct-transport client: pooled, never follows a redirect off the
/// advertised origin, and fails a fetch that stalls past the idle deadline.
static DIRECT_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
	reqwest::Client::builder()
		.redirect(reqwest::redirect::Policy::none())
		.connect_timeout(DIRECT_CONNECT_TIMEOUT)
		.read_timeout(SOURCE_TRANSFER_IDLE_TIMEOUT)
		.build()
		.expect("direct source client builds without TLS or proxy configuration")
});

pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	let worker = Router::new()
		.route("/source-workers/socket", get(socket))
		.route("/source-workers/streams/{grant_id}", get(tunnel_stream))
		.layer(middleware::from_fn_with_state(
			app_state.clone(),
			auth_middleware,
		));

	let admin = Router::new()
		.route("/remote-sources", get(list_sources))
		.route("/remote-sources/{id}/items", get(list_items))
		.route("/remote-sources/{id}/match", post(match_source))
		.route("/remote-source-imports", get(list_imports))
		.route("/remote-source-imports/{id}/approve", post(approve_import))
		.route("/remote-source-imports/{id}/reject", post(reject_import))
		.route("/remote-source-items/{id}/verify", post(verify_item))
		.route("/remote-source-items/{id}/link", post(link_item))
		.route(
			"/remote-source-items/{id}/materialize",
			post(materialize_item),
		)
		.layer(middleware::from_fn_with_state(app_state, auth_middleware));

	worker.merge(admin)
}

/// The authenticated source-worker device.  This extractor is deliberately
/// distinct from `workers::WorkerDevice`, so a compute key can never enter the
/// source socket (or vice versa).
pub(crate) struct SourceDevice(pub device::Model);

impl FromRequestParts<AppState> for SourceDevice {
	type Rejection = APIError;

	async fn from_request_parts(
		parts: &mut Parts,
		ctx: &AppState,
	) -> Result<Self, Self::Rejection> {
		let auth = parts
			.extensions
			.get::<AuthContext>()
			.ok_or(APIError::Unauthorized)?;
		auth.user_and_enforce_permissions(&[UserPermission::AccessRemoteSource])
			.map_err(|_| {
				APIError::Forbidden(
					"this credential cannot access the remote-source lane".to_string(),
				)
			})?;
		let Some(device_id) = auth.device_id() else {
			return Err(APIError::Forbidden(
				"the source-worker lane requires a device credential".to_string(),
			));
		};
		let device = device::Entity::find_by_id(device_id)
			.one(ctx.conn.as_ref())
			.await?
			.ok_or_else(|| APIError::Forbidden("unknown device".to_string()))?;
		if device.kind != DeviceKind::SourceWorker {
			return Err(APIError::Forbidden(
				"this device is not a source worker".to_string(),
			));
		}
		if device.revoked_at.is_some() {
			return Err(APIError::Forbidden("this device is revoked".to_string()));
		}
		Ok(Self(device))
	}
}

async fn socket(
	State(ctx): State<AppState>,
	SourceDevice(device): SourceDevice,
	upgrade: WebSocketUpgrade,
) -> APIResult<Response> {
	Ok(upgrade.on_upgrade(move |socket| serve_socket(ctx, device, socket)))
}

/// Axum's upgrade callback owns the socket, so the actual socket handler is
/// split out below.  Keeping this small adapter avoids doing any auth work
/// after the WebSocket has been accepted.
async fn serve_socket(ctx: AppState, device: device::Model, mut socket: WebSocket) {
	let Some(Ok(Message::Text(text))) = socket.recv().await else {
		return;
	};
	let frame = match parse_source_worker_frame(text.as_str()) {
		Ok(SourceWorkerFrame::Hello {
			name,
			version,
			roots,
		}) => SourceWorkerHello {
			name,
			version,
			roots,
		},
		Ok(_) => return,
		Err(error) => {
			tracing::warn!(device = %device.id, %error, "invalid source-worker hello");
			return;
		},
	};

	let hub = ctx.source_hub();
	let display_name = frame.name.clone().unwrap_or_else(|| device.name.clone());
	let (mut outbound, epoch) = hub
		.attach(device.id.clone(), display_name, frame.clone())
		.await;
	manifests()
		.lock()
		.retain(|key, _| key.device_id != device.id);
	for root in &frame.roots {
		if let Err(error) = upsert_source(&ctx, &device.id, root).await {
			tracing::warn!(device = %device.id, root = %root.root_id, ?error, "failed to persist source root");
		}
	}

	let (mut writer, mut reader) = socket.split();
	let write_task = tokio::spawn(async move {
		while let Some(text) = outbound.recv().await {
			if writer.send(Message::Text(text.into())).await.is_err() {
				break;
			}
		}
		let _ = writer.close().await;
	});

	while let Some(Ok(message)) = reader.next().await {
		if !hub.is_current(&device.id, epoch).await {
			break;
		}
		let Message::Text(text) = message else {
			continue;
		};
		let Ok(frame) = parse_source_worker_frame(text.as_str()) else {
			continue;
		};
		match frame {
			SourceWorkerFrame::ManifestChunk(chunk) => {
				hub.touch(&device.id).await;
				if let Err(error) =
					reconcile_manifest(&ctx, &device.id, epoch, chunk).await
				{
					tracing::warn!(device = %device.id, ?error, "source manifest rejected");
				}
			},
			other => {
				if let Err(error) = hub.handle_frame(&device.id, other).await {
					tracing::debug!(device = %device.id, ?error, "source frame refused");
				}
			},
		}
	}

	write_task.abort();
	let detached = hub.detach(&device.id, epoch).await;
	if detached {
		manifests()
			.lock()
			.retain(|key, _| key.device_id != device.id);
	}
	if detached && !hub.is_connected(&device.id).await {
		let _ = remote_source::Entity::update_many()
			.filter(remote_source::Column::DeviceId.eq(device.id.clone()))
			.col_expr(remote_source::Column::Health, Expr::value("offline"))
			.exec(ctx.conn.as_ref())
			.await;
	}
}

async fn tunnel_stream(
	State(ctx): State<AppState>,
	SourceDevice(device): SourceDevice,
	Path(grant_id): Path<String>,
	upgrade: WebSocketUpgrade,
) -> APIResult<Response> {
	let hub = ctx.source_hub();
	Ok(upgrade.on_upgrade(move |socket| serve_tunnel(hub, device.id, grant_id, socket)))
}

async fn serve_tunnel(
	hub: std::sync::Arc<SourceHub>,
	device_id: String,
	grant_id: String,
	mut socket: WebSocket,
) {
	while let Some(result) = socket.recv().await {
		let Ok(message) = result else { break };
		match message {
			Message::Binary(bytes) => {
				if hub
					.push_tunnel_chunk(&device_id, &grant_id, bytes.to_vec())
					.await
					.is_err()
				{
					break;
				}
			},
			Message::Close(_) => break,
			Message::Ping(_) | Message::Pong(_) => {},
			Message::Text(_) => break,
		}
	}
	let _ = hub.finish_tunnel(&device_id, &grant_id).await;
}

async fn upsert_source(
	ctx: &AppState,
	device_id: &str,
	root: &SourceRootHello,
) -> APIResult<remote_source::Model> {
	let now = DateTimeWithTimeZone::from(Utc::now());
	let transport = transport_name(root.transport);
	let direct_base_url = validate_direct_origin(root)?;
	let existing = remote_source::Entity::find()
		.filter(remote_source::Column::DeviceId.eq(device_id))
		.filter(remote_source::Column::RootId.eq(root.root_id.clone()))
		.one(ctx.conn.as_ref())
		.await?;
	if let Some(model) = existing {
		let mut active = model.into_active_model();
		active.label = Set(root.label.clone());
		active.kind = Set(root.kind.clone());
		active.privacy_mode = Set(root.privacy_mode.clone());
		active.transport = Set(transport.to_string());
		active.direct_base_url = Set(direct_base_url);
		active.health = Set(ONLINE.to_string());
		active.last_seen_at = Set(now);
		return Ok(active.update(ctx.conn.as_ref()).await?);
	}
	let active = remote_source::ActiveModel {
		id: Set(uuid::Uuid::new_v4().to_string()),
		device_id: Set(device_id.to_string()),
		root_id: Set(root.root_id.clone()),
		label: Set(root.label.clone()),
		kind: Set(root.kind.clone()),
		privacy_mode: Set(root.privacy_mode.clone()),
		transport: Set(transport.to_string()),
		direct_base_url: Set(direct_base_url),
		current_revision: Set(0),
		last_seen_at: Set(now),
		health: Set(ONLINE.to_string()),
		created_at: Set(now),
		updated_at: Set(now),
	};
	Ok(active.insert(ctx.conn.as_ref()).await?)
}

fn transport_name(transport: SourceTransport) -> &'static str {
	match transport {
		SourceTransport::Direct => "direct",
		SourceTransport::Tunnel => "tunnel",
	}
}

fn validate_direct_origin(root: &SourceRootHello) -> APIResult<Option<String>> {
	if root.transport == SourceTransport::Tunnel {
		return Ok(None);
	}
	let base = root.direct_base_url.as_deref().ok_or_else(|| {
		APIError::BadRequest("direct source root requires a base URL".to_string())
	})?;
	let parsed = reqwest::Url::parse(base).map_err(|_| {
		APIError::BadRequest("invalid direct source base URL".to_string())
	})?;
	if !matches!(parsed.scheme(), "http" | "https")
		|| parsed.host_str().is_none()
		|| !parsed.username().is_empty()
		|| parsed.password().is_some()
		|| parsed.query().is_some()
		|| parsed.fragment().is_some()
		|| parsed.path() != "/"
	{
		return Err(APIError::BadRequest(
			"direct source base URL must contain only an HTTP(S) origin".to_string(),
		));
	}
	Ok(Some(parsed.to_string().trim_end_matches('/').to_string()))
}

fn source_transport(value: &str) -> APIResult<SourceTransport> {
	match value {
		"direct" => Ok(SourceTransport::Direct),
		"tunnel" => Ok(SourceTransport::Tunnel),
		_ => Err(APIError::InternalServerError(format!(
			"unknown source transport {value}"
		))),
	}
}

fn sanitize_display_path(path: Option<&str>) -> APIResult<Option<String>> {
	let Some(path) = path else { return Ok(None) };
	if path.len() > 2048
		|| path.contains('\0')
		|| path.starts_with('/')
		|| path.contains('\\')
	{
		return Err(APIError::BadRequest(
			"invalid source relative path".to_string(),
		));
	}
	let mut components = Vec::new();
	for component in path.split('/') {
		if component.is_empty() || component == "." || component == ".." {
			return Err(APIError::BadRequest(
				"invalid source relative path".to_string(),
			));
		}
		components.push(component);
	}
	Ok(Some(components.join("/")))
}

/// Decode an observation's `modified_at`: an RFC 3339 string, or an epoch
/// number read by the protocol's one rule for epoch values.  The built-in
/// source worker sends milliseconds; a magnitude below 10^11 can only be
/// seconds and is scaled up.
fn modified_at(value: Option<&Value>) -> Option<DateTimeWithTimeZone> {
	let value = value?;
	if let Some(text) = value.as_str() {
		return DateTime::parse_from_rfc3339(text)
			.ok()
			.map(DateTimeWithTimeZone::from);
	}
	let millis = i64::try_from(expires_at_millis(value.as_i64()?)).ok()?;
	DateTime::<Utc>::from_timestamp_millis(millis)
		.map(|timestamp| DateTimeWithTimeZone::from(timestamp.fixed_offset()))
}

fn chunk_fingerprint(chunk: &SourceManifestChunk) -> String {
	let encoded = serde_json::to_vec(chunk).unwrap_or_default();
	let mut hash = Sha256::new();
	hash.update(encoded);
	format!("{:x}", hash.finalize())
}

async fn reconcile_manifest(
	ctx: &AppState,
	device_id: &str,
	epoch: u64,
	chunk: SourceManifestChunk,
) -> APIResult<()> {
	if chunk.items.len() > MAX_SOURCE_MANIFEST_ITEMS
		|| chunk.encoded_len() > MAX_SOURCE_MANIFEST_FRAME_BYTES
	{
		return Err(APIError::BadRequest(
			"source manifest chunk exceeds limits".to_string(),
		));
	}
	let source = remote_source::Entity::find()
		.filter(remote_source::Column::DeviceId.eq(device_id))
		.filter(remote_source::Column::RootId.eq(chunk.root_id.clone()))
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| {
			APIError::Conflict("manifest references an unknown source root".to_string())
		})?;

	let key = ManifestKey {
		device_id: device_id.to_string(),
		root_id: chunk.root_id.clone(),
		batch_id: chunk.batch_id.clone(),
		revision: chunk.revision,
	};
	let fingerprint = chunk_fingerprint(&chunk);
	let mut replay = false;
	let chunk_ids = {
		let mut ledger = manifests().lock();
		if chunk.revision < source.current_revision as u64 {
			return Err(APIError::Conflict(
				"manifest revision is older than the committed revision".to_string(),
			));
		}
		if chunk.revision == source.current_revision as u64 && !ledger.contains_key(&key)
		{
			replay = true;
			Vec::new()
		} else {
			if chunk.revision > source.current_revision as u64 + 1 {
				return Err(APIError::Conflict("manifest revision gap".to_string()));
			}
			if ledger.keys().any(|existing| {
				existing.device_id == key.device_id
					&& existing.root_id == key.root_id
					&& existing.revision == key.revision
					&& existing.batch_id != key.batch_id
			}) {
				return Err(APIError::Conflict("conflicting manifest batch".to_string()));
			}

			let batch = ledger.entry(key.clone()).or_default();
			if chunk.sequence < batch.next_sequence {
				if batch.fingerprints.get(&chunk.sequence) == Some(&fingerprint) {
					replay = true;
					Vec::new()
				} else {
					return Err(APIError::Conflict(
						"conflicting manifest replay".to_string(),
					));
				}
			} else {
				if chunk.sequence != batch.next_sequence {
					return Err(APIError::Conflict("manifest sequence gap".to_string()));
				}
				// Check against the batch's committed ids in place; only this
				// chunk's ids are staged, and they join the batch after the
				// database writes succeed.
				let mut seen_ids = HashSet::with_capacity(chunk.items.len());
				for item in &chunk.items {
					if !seen_ids.insert(item.worker_item_id.as_str())
						|| batch.worker_ids.contains(&item.worker_item_id)
					{
						return Err(APIError::Conflict(
							"duplicate item in manifest batch".to_string(),
						));
					}
				}
				chunk
					.items
					.iter()
					.map(|item| item.worker_item_id.clone())
					.collect()
			}
		}
	};

	if replay {
		let _ = ctx
			.source_hub()
			.send(
				device_id,
				&SourceServerFrame::ManifestAck {
					root_id: chunk.root_id,
					revision: chunk.revision,
					sequence: chunk.sequence,
				},
			)
			.await;
		return Ok(());
	}

	for item in &chunk.items {
		upsert_item(ctx, &source, item, chunk.revision).await?;
	}

	if chunk.terminal {
		if !ctx.source_hub().is_current(device_id, epoch).await {
			return Err(APIError::Conflict(
				"source connection was replaced during manifest reconciliation"
					.to_string(),
			));
		}
		remote_source_item::Entity::update_many()
			.filter(remote_source_item::Column::SourceId.eq(source.id.clone()))
			.filter(
				remote_source_item::Column::LastSeenRevision.lt(chunk.revision as i64),
			)
			.filter(remote_source_item::Column::ObservationState.ne("tombstoned"))
			.col_expr(
				remote_source_item::Column::ObservationState,
				Expr::value(MISSING),
			)
			.exec(ctx.conn.as_ref())
			.await?;
		let mut active = source.clone().into_active_model();
		active.current_revision = Set(chunk.revision as i64);
		active.health = Set(ONLINE.to_string());
		active.last_seen_at = Set(DateTimeWithTimeZone::from(Utc::now()));
		active.update(ctx.conn.as_ref()).await?;
	}

	{
		let mut ledger = manifests().lock();
		let batch = ledger.get_mut(&key).ok_or_else(|| {
			APIError::Conflict(
				"manifest connection changed during reconciliation".to_string(),
			)
		})?;
		if batch.next_sequence != chunk.sequence {
			return Err(APIError::Conflict(
				"manifest sequence changed during reconciliation".to_string(),
			));
		}
		batch.worker_ids.extend(chunk_ids);
		batch.fingerprints.insert(chunk.sequence, fingerprint);
		batch.next_sequence = batch.next_sequence.saturating_add(1);
		if chunk.terminal {
			ledger.remove(&key);
		}
	}

	let _ = ctx
		.source_hub()
		.send(
			device_id,
			&SourceServerFrame::ManifestAck {
				root_id: chunk.root_id,
				revision: chunk.revision,
				sequence: chunk.sequence,
			},
		)
		.await;
	Ok(())
}

async fn upsert_item(
	ctx: &AppState,
	source: &remote_source::Model,
	item: &SourceManifestItem,
	revision: u64,
) -> APIResult<()> {
	let relative_path = sanitize_display_path(item.relative_path.as_deref())?;
	let size = i64::try_from(item.size)
		.map_err(|_| APIError::BadRequest("source item is too large".to_string()))?;
	let existing = remote_source_item::Entity::find()
		.filter(remote_source_item::Column::SourceId.eq(source.id.clone()))
		.filter(remote_source_item::Column::WorkerItemId.eq(item.worker_item_id.clone()))
		.one(ctx.conn.as_ref())
		.await?;
	let now = DateTimeWithTimeZone::from(Utc::now());
	if let Some(model) = existing {
		let preserve_verified = model.worker_content_version
			== item.worker_content_version
			&& model.size == size
			&& model.observation_state == VERIFIED;
		let prior_sha256 = model.sha256.clone();
		let mut active = model.into_active_model();
		active.worker_content_version = Set(item.worker_content_version.clone());
		active.relative_path = Set(relative_path);
		active.size = Set(size);
		active.modified_at = Set(modified_at(item.modified_at.as_ref()));
		active.media_type = Set(item.media_type.clone());
		active.quick_fingerprint = Set(item.quick_fingerprint.clone());
		active.sha256 = Set(if preserve_verified {
			prior_sha256
		} else {
			item.sha256.clone()
		});
		active.metadata = Set(item.metadata.clone());
		active.retention = Set(item.retention.clone());
		active.observation_state = Set(if preserve_verified {
			VERIFIED.to_string()
		} else {
			OBSERVED.to_string()
		});
		active.last_seen_revision = Set(revision as i64);
		active.last_seen_at = Set(now);
		active.update(ctx.conn.as_ref()).await?;
	} else {
		remote_source_item::ActiveModel {
			id: Set(uuid::Uuid::new_v4().to_string()),
			source_id: Set(source.id.clone()),
			worker_item_id: Set(item.worker_item_id.clone()),
			worker_content_version: Set(item.worker_content_version.clone()),
			relative_path: Set(relative_path),
			size: Set(size),
			modified_at: Set(modified_at(item.modified_at.as_ref())),
			media_type: Set(item.media_type.clone()),
			quick_fingerprint: Set(item.quick_fingerprint.clone()),
			sha256: Set(item.sha256.clone()),
			metadata: Set(item.metadata.clone()),
			retention: Set(item.retention.clone()),
			observation_state: Set(OBSERVED.to_string()),
			last_seen_revision: Set(revision as i64),
			last_seen_at: Set(now),
			created_at: Set(now),
			updated_at: Set(now),
			imported_media_id: Set(None),
		}
		.insert(ctx.conn.as_ref())
		.await?;
	}
	Ok(())
}

fn enforce_admin(req: &AuthContext) -> APIResult<()> {
	if req.user().is_server_owner
		|| req.user().permissions.iter().any(|permission| {
			*permission == UserPermission::ManageLibrary
				|| *permission == UserPermission::ManageServer
		}) {
		Ok(())
	} else {
		Err(APIError::forbidden_discreet())
	}
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceSummary {
	id: String,
	device_id: String,
	root_id: String,
	label: String,
	kind: String,
	privacy_mode: String,
	transport: String,
	direct_base_url: Option<String>,
	current_revision: i64,
	last_seen_at: DateTimeWithTimeZone,
	health: String,
}

impl From<remote_source::Model> for SourceSummary {
	fn from(source: remote_source::Model) -> Self {
		Self {
			id: source.id,
			device_id: source.device_id,
			root_id: source.root_id,
			label: source.label,
			kind: source.kind,
			privacy_mode: source.privacy_mode,
			transport: source.transport,
			direct_base_url: source.direct_base_url,
			current_revision: source.current_revision,
			last_seen_at: source.last_seen_at,
			health: source.health,
		}
	}
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemSummary {
	id: String,
	source_id: String,
	worker_item_id: String,
	worker_content_version: String,
	relative_path: Option<String>,
	size: i64,
	modified_at: Option<DateTimeWithTimeZone>,
	media_type: Option<String>,
	quick_fingerprint: Option<String>,
	sha256: Option<String>,
	metadata: Option<Value>,
	retention: Option<Value>,
	observation_state: String,
	last_seen_revision: i64,
	last_seen_at: DateTimeWithTimeZone,
	imported_media_id: Option<String>,
}

impl From<remote_source_item::Model> for ItemSummary {
	fn from(item: remote_source_item::Model) -> Self {
		Self {
			id: item.id,
			source_id: item.source_id,
			worker_item_id: item.worker_item_id,
			worker_content_version: item.worker_content_version,
			relative_path: item.relative_path,
			size: item.size,
			modified_at: item.modified_at,
			media_type: item.media_type,
			quick_fingerprint: item.quick_fingerprint,
			sha256: item.sha256,
			metadata: item.metadata,
			retention: item.retention,
			observation_state: item.observation_state,
			last_seen_revision: item.last_seen_revision,
			last_seen_at: item.last_seen_at,
			imported_media_id: item.imported_media_id,
		}
	}
}

async fn list_sources(
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<Vec<SourceSummary>>> {
	enforce_admin(&req)?;
	let sources = remote_source::Entity::find()
		.order_by_asc(remote_source::Column::DeviceId)
		.order_by_asc(remote_source::Column::RootId)
		.all(ctx.conn.as_ref())
		.await?
		.into_iter()
		.map(SourceSummary::from)
		.collect();
	Ok(Json(sources))
}

async fn list_items(
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	Path(source_id): Path<String>,
) -> APIResult<Json<Vec<ItemSummary>>> {
	enforce_admin(&req)?;
	let items = remote_source_item::Entity::find()
		.filter(remote_source_item::Column::SourceId.eq(source_id))
		.order_by_asc(remote_source_item::Column::RelativePath)
		.all(ctx.conn.as_ref())
		.await?
		.into_iter()
		.map(ItemSummary::from)
		.collect();
	Ok(Json(items))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportSummary {
	id: String,
	source_id: String,
	source_item_id: String,
	target_media_id: String,
	worker_item_id: String,
	worker_content_version: String,
	source_sha256: String,
	source_size: i64,
	match_kind: String,
	match_score: i32,
	match_evidence: Value,
	status: String,
	decision_actor_id: Option<String>,
	decision_at: Option<DateTimeWithTimeZone>,
	decision_reason: Option<String>,
	applied_location_id: Option<String>,
	staged_item_id: Option<String>,
	created_at: DateTimeWithTimeZone,
	updated_at: DateTimeWithTimeZone,
}

impl From<remote_source_import::Model> for ImportSummary {
	fn from(value: remote_source_import::Model) -> Self {
		Self {
			id: value.id,
			source_id: value.source_id,
			source_item_id: value.source_item_id,
			target_media_id: value.target_media_id,
			worker_item_id: value.worker_item_id,
			worker_content_version: value.worker_content_version,
			source_sha256: value.source_sha256,
			source_size: value.source_size,
			match_kind: value.match_kind,
			match_score: value.match_score,
			match_evidence: value.match_evidence,
			status: value.status,
			decision_actor_id: value.decision_actor_id,
			decision_at: value.decision_at,
			decision_reason: value.decision_reason,
			applied_location_id: value.applied_location_id,
			staged_item_id: value.staged_item_id,
			created_at: value.created_at,
			updated_at: value.updated_at,
		}
	}
}

#[derive(Debug, PartialEq)]
struct CandidateMatch {
	media_id: String,
	kind: &'static str,
	score: i32,
	evidence: Value,
}
type TypedIdentifier = (String, String);

/// Lookup tables over the local library, built once per match request so a
/// source with thousands of items costs one pass over the local rows rather
/// than one pass per item.
struct CandidateIndex {
	/// Location digest → media ids holding those exact bytes.
	media_by_digest: HashMap<String, BTreeSet<String>>,
	/// Typed identifier → media ids whose metadata carries it.
	media_by_identifier: HashMap<TypedIdentifier, BTreeSet<String>>,
	/// Normalized title → (media id, normalized author set).
	media_by_title: HashMap<String, Vec<(String, BTreeSet<String>)>>,
}

impl CandidateIndex {
	fn build(
		metadata: &[media_metadata::Model],
		locations: &[media_location::Model],
	) -> Self {
		let mut media_by_digest: HashMap<String, BTreeSet<String>> = HashMap::new();
		for location in locations {
			media_by_digest
				.entry(location.sha256.clone())
				.or_default()
				.insert(location.media_id.clone());
		}
		let mut media_by_identifier: HashMap<TypedIdentifier, BTreeSet<String>> =
			HashMap::new();
		let mut media_by_title: HashMap<String, Vec<(String, BTreeSet<String>)>> =
			HashMap::new();
		for value in metadata {
			let Some(media_id) = value.media_id.as_deref() else {
				continue;
			};
			for identifier in local_identifier_values(value) {
				media_by_identifier
					.entry(identifier)
					.or_default()
					.insert(media_id.to_owned());
			}
			if let Some(title) = value.title.as_deref().map(normalize_match_text) {
				let authors = value
					.writers
					.as_deref()
					.into_iter()
					.flat_map(split_authors)
					.collect::<BTreeSet<_>>();
				media_by_title
					.entry(title)
					.or_default()
					.push((media_id.to_owned(), authors));
			}
		}
		Self {
			media_by_digest,
			media_by_identifier,
			media_by_title,
		}
	}
}

async fn match_source(
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	Path(source_id): Path<String>,
) -> APIResult<Json<Vec<ImportSummary>>> {
	enforce_admin(&req)?;
	let source = remote_source::Entity::find_by_id(source_id.clone())
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| APIError::NotFound("remote source not found".to_string()))?;
	if source.privacy_mode.eq_ignore_ascii_case("candidate_only") {
		return Err(APIError::Forbidden(
			"candidate_only sources require an interest-set protocol".to_string(),
		));
	}
	let items = remote_source_item::Entity::find()
		.filter(remote_source_item::Column::SourceId.eq(source.id.clone()))
		.order_by_asc(remote_source_item::Column::Id)
		.all(ctx.conn.as_ref())
		.await?;
	let metadata = media_metadata::Entity::find()
		.order_by_asc(media_metadata::Column::MediaId)
		.all(ctx.conn.as_ref())
		.await?;
	let locations = media_location::Entity::find()
		.order_by_asc(media_location::Column::MediaId)
		.order_by_asc(media_location::Column::Id)
		.all(ctx.conn.as_ref())
		.await?;
	// Indexing the library and normalizing every item is CPU work proportional
	// to the whole catalog; it runs on the blocking pool so this admin request
	// does not stall the request threads.
	let candidates = tokio::task::spawn_blocking(move || {
		let index = CandidateIndex::build(&metadata, &locations);
		items
			.into_iter()
			// Discovery metadata alone is not an import identity. Proposals
			// enter the ledger only after this server has streamed the whole
			// object and persisted its exact SHA-256 verification.
			.filter(|item| {
				item.sha256.is_some()
					&& item.observation_state == VERIFIED
					&& item.size >= 0
			})
			.filter_map(|item| {
				find_candidate(&item, &index).map(|candidate| (item, candidate))
			})
			.collect::<Vec<_>>()
	})
	.await
	.map_err(|error| APIError::InternalServerError(error.to_string()))?;
	let mut proposals = Vec::with_capacity(candidates.len());
	for (item, candidate) in candidates {
		let proposal = upsert_import_proposal(&ctx, &source, &item, candidate).await?;
		proposals.push(ImportSummary::from(proposal));
	}
	proposals.sort_by(|left, right| {
		left.source_item_id
			.cmp(&right.source_item_id)
			.then_with(|| left.target_media_id.cmp(&right.target_media_id))
			.then_with(|| left.id.cmp(&right.id))
	});
	Ok(Json(proposals))
}

/// Match one verified item against the indexed library.  Evidence is tried
/// strongest first, and ambiguous evidence at any tier fails closed rather
/// than falling through to a weaker tier.
fn find_candidate(
	item: &remote_source_item::Model,
	index: &CandidateIndex,
) -> Option<CandidateMatch> {
	let digest = item
		.sha256
		.as_deref()
		.filter(|_| item.observation_state == VERIFIED)
		.unwrap_or_default();
	if !digest.is_empty() {
		if let Some(media_ids) = index.media_by_digest.get(digest) {
			if media_ids.len() > 1 {
				return None;
			}
			if let Some(media_id) = media_ids.iter().next() {
				return Some(CandidateMatch {
					media_id: media_id.clone(),
					kind: "digest",
					score: 100,
					evidence: json!({ "sha256": item.sha256 }),
				});
			}
		}
	}

	let remote_identifiers = metadata_identifier_values(item.metadata.as_ref());
	if !remote_identifiers.is_empty() {
		let mut shared_by_media: BTreeMap<&str, BTreeSet<TypedIdentifier>> =
			BTreeMap::new();
		for identifier in &remote_identifiers {
			for media_id in index
				.media_by_identifier
				.get(identifier)
				.into_iter()
				.flatten()
			{
				shared_by_media
					.entry(media_id.as_str())
					.or_default()
					.insert(identifier.clone());
			}
		}
		if shared_by_media.len() > 1 {
			return None;
		}
		if let Some((media_id, shared)) = shared_by_media.into_iter().next() {
			return Some(CandidateMatch {
				media_id: media_id.to_owned(),
				kind: "identifier",
				score: 90,
				evidence: json!({ "identifiers": identifier_evidence(&shared) }),
			});
		}
	}

	let remote_title = metadata_text_values(item.metadata.as_ref(), &["title", "name"]);
	let remote_authors = metadata_text_values(
		item.metadata.as_ref(),
		&[
			"author", "authors", "creator", "creators", "writer", "writers",
		],
	);
	let remote_title = remote_title
		.into_iter()
		.map(|value| normalize_match_text(&value))
		.find(|value| !value.is_empty())?;
	let remote_authors = remote_authors
		.into_iter()
		.flat_map(|value| split_authors(&value))
		.collect::<BTreeSet<_>>();
	if remote_authors.is_empty() {
		return None;
	}
	let metadata_matches = unique_media_ids(
		index
			.media_by_title
			.get(&remote_title)
			.into_iter()
			.flatten()
			.filter(|(_, authors)| {
				remote_authors.iter().any(|author| authors.contains(author))
			})
			.map(|(media_id, _)| media_id.as_str()),
	);
	if metadata_matches.len() == 1 {
		return Some(CandidateMatch {
			media_id: metadata_matches[0].clone(),
			kind: "metadata",
			score: 70,
			evidence: json!({
				"title": remote_title,
				"authors": remote_authors,
			}),
		});
	}
	None
}

fn unique_media_ids<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
	let mut ids = values
		.into_iter()
		.map(ToOwned::to_owned)
		.collect::<BTreeSet<_>>()
		.into_iter()
		.collect::<Vec<_>>();
	ids.sort();
	ids
}

fn normalize_match_text(value: &str) -> String {
	value
		.chars()
		.filter(|character| character.is_alphanumeric())
		.flat_map(char::to_lowercase)
		.collect()
}

fn split_authors(value: &str) -> Vec<String> {
	value
		.split([',', ';', '|', '&'])
		.map(normalize_match_text)
		.filter(|author| !author.is_empty())
		.collect()
}

fn metadata_text_values(metadata: Option<&Value>, keys: &[&str]) -> Vec<String> {
	let wanted = keys
		.iter()
		.map(|key| key.to_ascii_lowercase())
		.collect::<BTreeSet<_>>();
	let mut values = Vec::new();
	fn visit(value: &Value, wanted: &BTreeSet<String>, values: &mut Vec<String>) {
		match value {
			Value::Object(object) => {
				for (key, value) in object {
					if wanted.contains(&key.to_ascii_lowercase()) {
						match value {
							Value::String(value) => values.push(value.clone()),
							Value::Array(values_array) => values.extend(
								values_array
									.iter()
									.filter_map(Value::as_str)
									.map(ToOwned::to_owned),
							),
							_ => {},
						}
					}
					visit(value, wanted, values);
				}
			},
			Value::Array(array) => {
				for value in array {
					visit(value, wanted, values);
				}
			},
			_ => {},
		}
	}
	if let Some(metadata) = metadata {
		visit(metadata, &wanted, &mut values);
	}
	values.sort();
	values.dedup();
	values
}

fn identifier_scheme(key: &str) -> Option<String> {
	let lowercase = key.to_ascii_lowercase();
	if lowercase == "identifier" || lowercase == "identifiers" {
		return None;
	}
	let compact = lowercase
		.chars()
		.filter(|character| character.is_ascii_alphanumeric())
		.collect::<String>();
	let compact = compact.strip_prefix("identifier").unwrap_or(&compact);
	match compact {
		"isbn" | "isbn10" | "isbn13" => Some("isbn".to_owned()),
		"asin" => Some("asin".to_owned()),
		"amazon" => Some("amazon".to_owned()),
		"mobiasin" => Some("mobi_asin".to_owned()),
		"calibre" => Some("calibre".to_owned()),
		"google" => Some("google".to_owned()),
		"uuid" => Some("uuid".to_owned()),
		_ => None,
	}
}

fn metadata_identifier_values(metadata: Option<&Value>) -> BTreeSet<TypedIdentifier> {
	let mut values = BTreeSet::new();
	fn visit(value: &Value, values: &mut BTreeSet<TypedIdentifier>) {
		match value {
			Value::Object(object) => {
				for (key, value) in object {
					let lowercase = key.to_ascii_lowercase();
					if lowercase == "identifier" || lowercase == "identifiers" {
						collect_generic_identifier_values(value, values);
					} else if let Some(scheme) = identifier_scheme(&lowercase) {
						collect_identifier_values(value, &scheme, values);
					}
					visit(value, values);
				}
			},
			Value::Array(array) => {
				for value in array {
					visit(value, values);
				}
			},
			_ => {},
		}
	}
	if let Some(metadata) = metadata {
		visit(metadata, &mut values);
	}
	values
}

fn collect_generic_identifier_values(
	value: &Value,
	values: &mut BTreeSet<TypedIdentifier>,
) {
	match value {
		Value::Object(object) => {
			for (key, value) in object {
				if let Some(scheme) = identifier_scheme(key) {
					collect_identifier_values(value, &scheme, values);
				}
			}
		},
		Value::Array(array) => {
			for value in array {
				collect_generic_identifier_values(value, values);
			}
		},
		_ => {},
	}
}

fn collect_identifier_values(
	value: &Value,
	scheme: &str,
	values: &mut BTreeSet<TypedIdentifier>,
) {
	match value {
		Value::String(value) => insert_typed_identifier(scheme, value, values),
		Value::Array(array) => {
			for value in array {
				collect_identifier_values(value, scheme, values);
			}
		},
		Value::Object(object) => {
			if let Some(value) = object.get("value") {
				collect_identifier_values(value, scheme, values);
			} else {
				for value in object.values() {
					collect_identifier_values(value, scheme, values);
				}
			}
		},
		_ => {},
	}
}

fn insert_typed_identifier(
	scheme: &str,
	value: &str,
	identifiers: &mut BTreeSet<TypedIdentifier>,
) {
	let value = normalize_match_text(value);
	if !value.is_empty() {
		identifiers.insert((scheme.to_owned(), value));
	}
}

fn identifier_evidence(identifiers: &BTreeSet<TypedIdentifier>) -> Vec<Value> {
	identifiers
		.iter()
		.map(|(scheme, value)| json!({ "scheme": scheme, "value": value }))
		.collect()
}

fn local_identifier_values(value: &media_metadata::Model) -> BTreeSet<TypedIdentifier> {
	let mut identifiers = BTreeSet::new();
	for (scheme, value) in [
		("amazon", value.identifier_amazon.as_deref()),
		("calibre", value.identifier_calibre.as_deref()),
		("google", value.identifier_google.as_deref()),
		("isbn", value.identifier_isbn.as_deref()),
		("mobi_asin", value.identifier_mobi_asin.as_deref()),
		("uuid", value.identifier_uuid.as_deref()),
	] {
		if let Some(value) = value {
			insert_typed_identifier(scheme, value, &mut identifiers);
		}
	}
	identifiers
}

async fn upsert_import_proposal(
	ctx: &AppState,
	source: &remote_source::Model,
	item: &remote_source_item::Model,
	candidate: CandidateMatch,
) -> APIResult<remote_source_import::Model> {
	let source_sha256 = item
		.sha256
		.clone()
		.ok_or_else(|| APIError::Conflict("source item is not verified".to_string()))?;
	let source_size = item.size;
	let txn = models::txn::begin_write(ctx.conn.as_ref()).await?;
	let existing = remote_source_import::Entity::find()
		.filter(remote_source_import::Column::SourceItemId.eq(item.id.clone()))
		.filter(
			remote_source_import::Column::TargetMediaId.eq(candidate.media_id.clone()),
		)
		.filter(
			remote_source_import::Column::WorkerContentVersion
				.eq(item.worker_content_version.clone()),
		)
		.filter(remote_source_import::Column::SourceSha256.eq(source_sha256.clone()))
		.one(&txn)
		.await?;
	if let Some(existing) = existing {
		if existing.status != "proposed" {
			txn.rollback().await?;
			return Ok(existing);
		}
		let mut active = existing.into_active_model();
		active.match_kind = Set(candidate.kind.to_owned());
		active.match_score = Set(candidate.score);
		active.match_evidence = Set(candidate.evidence);
		active.updated_at = Set(DateTimeWithTimeZone::from(Utc::now()));
		let updated = active.update(&txn).await?;
		txn.commit().await?;
		return Ok(updated);
	}
	let now = DateTimeWithTimeZone::from(Utc::now());
	let created = remote_source_import::ActiveModel {
		id: Set(uuid::Uuid::new_v4().to_string()),
		source_id: Set(source.id.clone()),
		source_item_id: Set(item.id.clone()),
		target_media_id: Set(candidate.media_id),
		worker_item_id: Set(item.worker_item_id.clone()),
		worker_content_version: Set(item.worker_content_version.clone()),
		source_sha256: Set(source_sha256),
		source_size: Set(source_size),
		match_kind: Set(candidate.kind.to_owned()),
		match_score: Set(candidate.score),
		match_evidence: Set(candidate.evidence),
		status: Set("proposed".to_owned()),
		decision_actor_id: Set(None),
		decision_at: Set(None),
		decision_reason: Set(None),
		applied_location_id: Set(None),
		staged_item_id: Set(None),
		created_at: Set(now),
		updated_at: Set(now),
	}
	.insert(&txn)
	.await?;
	txn.commit().await?;
	Ok(created)
}

async fn list_imports(
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<Vec<ImportSummary>>> {
	enforce_admin(&req)?;
	let proposals = remote_source_import::Entity::find()
		.order_by_asc(remote_source_import::Column::CreatedAt)
		.order_by_asc(remote_source_import::Column::Id)
		.all(ctx.conn.as_ref())
		.await?
		.into_iter()
		.map(ImportSummary::from)
		.collect();
	Ok(Json(proposals))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportDecisionRequest {
	action: Option<String>,
	library_id: Option<String>,
	filename: Option<String>,
	idempotency_key: Option<String>,
	reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportDecisionResponse {
	proposal: ImportSummary,
	action: String,
	location_id: Option<String>,
	staged_item_id: Option<String>,
	deduplicated: Option<bool>,
}

fn verify_proposal_identity(
	proposal: &remote_source_import::Model,
	item: &remote_source_item::Model,
) -> APIResult<()> {
	if proposal.source_id != item.source_id
		|| proposal.source_item_id != item.id
		|| proposal.worker_item_id != item.worker_item_id
		|| proposal.worker_content_version != item.worker_content_version
		|| proposal.source_sha256.as_str() != item.sha256.as_deref().unwrap_or_default()
		|| proposal.source_size != item.size
		|| item.observation_state != VERIFIED
	{
		return Err(APIError::Conflict(
			"source item changed since this proposal was created".to_string(),
		));
	}
	Ok(())
}

/// Move a `proposed` proposal to `status` with one conditional update.  A
/// proposal another decision already moved is left untouched and reported as
/// `false`, so the caller reads back what won instead of applying anything.
async fn claim_proposal<C: ConnectionTrait>(
	conn: &C,
	proposal_id: &str,
	status: &str,
	actor_id: &str,
	reason: Option<String>,
) -> APIResult<bool> {
	let now = DateTimeWithTimeZone::from(Utc::now());
	let result = remote_source_import::Entity::update_many()
		.col_expr(remote_source_import::Column::Status, Expr::value(status))
		.col_expr(
			remote_source_import::Column::DecisionActorId,
			Expr::value(actor_id),
		)
		.col_expr(remote_source_import::Column::DecisionAt, Expr::value(now))
		.col_expr(
			remote_source_import::Column::DecisionReason,
			Expr::value(reason),
		)
		.col_expr(remote_source_import::Column::UpdatedAt, Expr::value(now))
		.filter(remote_source_import::Column::Id.eq(proposal_id))
		.filter(remote_source_import::Column::Status.eq(PROPOSED))
		.exec(conn)
		.await?;
	Ok(result.rows_affected == 1)
}

/// Hand a claimed approval that produced no outcome back to `proposed`, so it
/// can be decided again.  A proposal that meanwhile recorded an outcome or
/// changed status is left alone.
#[cfg(feature = "ingest")]
async fn release_claim<C: ConnectionTrait>(conn: &C, proposal_id: &str) -> APIResult<()> {
	remote_source_import::Entity::update_many()
		.col_expr(remote_source_import::Column::Status, Expr::value(PROPOSED))
		.col_expr(
			remote_source_import::Column::DecisionActorId,
			Expr::value(Option::<String>::None),
		)
		.col_expr(
			remote_source_import::Column::DecisionAt,
			Expr::value(Option::<DateTimeWithTimeZone>::None),
		)
		.col_expr(
			remote_source_import::Column::DecisionReason,
			Expr::value(Option::<String>::None),
		)
		.col_expr(
			remote_source_import::Column::UpdatedAt,
			Expr::value(DateTimeWithTimeZone::from(Utc::now())),
		)
		.filter(remote_source_import::Column::Id.eq(proposal_id))
		.filter(remote_source_import::Column::Status.eq(APPROVED))
		.filter(remote_source_import::Column::AppliedLocationId.is_null())
		.filter(remote_source_import::Column::StagedItemId.is_null())
		.exec(conn)
		.await?;
	Ok(())
}

async fn load_proposal<C: ConnectionTrait>(
	conn: &C,
	proposal_id: &str,
) -> APIResult<remote_source_import::Model> {
	remote_source_import::Entity::find_by_id(proposal_id.to_owned())
		.one(conn)
		.await?
		.ok_or_else(|| APIError::NotFound("remote import proposal not found".to_string()))
}

/// The answer for an approval that lost its claim: the decision that won.
fn settled_response(
	proposal: remote_source_import::Model,
) -> APIResult<ImportDecisionResponse> {
	match proposal.status.as_str() {
		APPROVED
			if proposal.applied_location_id.is_none()
				&& proposal.staged_item_id.is_none() =>
		{
			// Claimed, but its bytes were never recorded as staged: either a
			// materialization is running right now or the process that
			// claimed it died.  Only the latter may be resumed.
			Err(APIError::Conflict(
				if MATERIALIZING.lock().contains(&proposal.id) {
					"proposal approval is still being applied".to_string()
				} else {
					"proposal was approved but its materialization did not finish; approve it again with action=materialize to resume".to_string()
				},
			))
		},
		APPROVED => Ok(ImportDecisionResponse {
			action: if proposal.applied_location_id.is_some() {
				"link".to_owned()
			} else {
				"materialize".to_owned()
			},
			location_id: proposal.applied_location_id.clone(),
			staged_item_id: proposal.staged_item_id.clone(),
			deduplicated: proposal.staged_item_id.as_ref().map(|_| true),
			proposal: ImportSummary::from(proposal),
		}),
		REJECTED => Err(APIError::Conflict("proposal was rejected".to_string())),
		other => Err(APIError::Conflict(format!("proposal is already {other}"))),
	}
}

async fn approve_import(
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	Path(proposal_id): Path<String>,
	Json(body): Json<ImportDecisionRequest>,
) -> APIResult<Json<ImportDecisionResponse>> {
	enforce_admin(&req)?;
	let response = match body.action.as_deref().unwrap_or("link") {
		"link" => approve_link(&ctx, &proposal_id, &req.user().id, body.reason).await?,
		"materialize" => {
			#[cfg(feature = "ingest")]
			{
				approve_materialize(&ctx, &req, &proposal_id, body).await?
			}
			#[cfg(not(feature = "ingest"))]
			{
				return Err(APIError::NotImplemented);
			}
		},
		_ => {
			return Err(APIError::BadRequest(
				"action must be link or materialize".to_string(),
			));
		},
	};
	Ok(Json(response))
}

/// Approve by linking the verified item to the proposal's target media.  The
/// claim, the identity re-check, and the location upsert commit together: a
/// proposal that lost its claim (rejected or approved meanwhile) applies
/// nothing, and a link that fails leaves the proposal `proposed`.
async fn approve_link(
	ctx: &AppState,
	proposal_id: &str,
	actor_id: &str,
	reason: Option<String>,
) -> APIResult<ImportDecisionResponse> {
	let txn = models::txn::begin_write(ctx.conn.as_ref()).await?;
	if !claim_proposal(&txn, proposal_id, APPROVED, actor_id, reason).await? {
		let current = load_proposal(&txn, proposal_id).await?;
		txn.rollback().await?;
		return settled_response(current);
	}
	let proposal = load_proposal(&txn, proposal_id).await?;
	let (item, source) = load_item(&txn, &proposal.source_item_id).await?;
	if source.id != proposal.source_id {
		return Err(APIError::Conflict(
			"proposal source no longer exists".to_string(),
		));
	}
	verify_proposal_identity(&proposal, &item)?;
	let linked = link_verified_item(&txn, &item, &proposal.target_media_id).await?;
	let mut active = proposal.into_active_model();
	active.applied_location_id = Set(Some(linked.location_id.clone()));
	let updated = active.update(&txn).await?;
	txn.commit().await?;
	Ok(ImportDecisionResponse {
		proposal: ImportSummary::from(updated),
		action: "link".to_owned(),
		location_id: Some(linked.location_id),
		staged_item_id: None,
		deduplicated: None,
	})
}

/// Marks a proposal whose bytes this process is staging right now.  Dropped
/// on every exit, including a cancelled request.
#[cfg(feature = "ingest")]
struct MaterializationGuard(String);

#[cfg(feature = "ingest")]
impl MaterializationGuard {
	fn enter(proposal_id: &str) -> APIResult<Self> {
		if !MATERIALIZING.lock().insert(proposal_id.to_owned()) {
			return Err(APIError::Conflict(
				"proposal approval is still being applied".to_string(),
			));
		}
		Ok(Self(proposal_id.to_owned()))
	}
}

#[cfg(feature = "ingest")]
impl Drop for MaterializationGuard {
	fn drop(&mut self) {
		MATERIALIZING.lock().remove(&self.0);
	}
}

/// Approve by staging the verified bytes.  Staging happens outside the
/// database, so the claim is taken first and the outcome recorded after; a
/// failure hands the proposal back as `proposed`.  A claim left without an
/// outcome by a crash is resumed by the next approval (the staged upload is
/// idempotent on the proposal's key), while one this process is still
/// applying answers 409.
#[cfg(feature = "ingest")]
async fn approve_materialize(
	ctx: &AppState,
	req: &AuthContext,
	proposal_id: &str,
	body: ImportDecisionRequest,
) -> APIResult<ImportDecisionResponse> {
	let library_id = body.library_id.clone().ok_or_else(|| {
		APIError::BadRequest("libraryId is required for materialization".to_string())
	})?;
	let conn = ctx.conn.as_ref();
	let claimed =
		claim_proposal(conn, proposal_id, APPROVED, &req.user().id, body.reason).await?;
	let proposal = load_proposal(conn, proposal_id).await?;
	let unfinished = proposal.status == APPROVED
		&& proposal.applied_location_id.is_none()
		&& proposal.staged_item_id.is_none();
	if !claimed && !unfinished {
		return settled_response(proposal);
	}
	let _in_flight = MaterializationGuard::enter(proposal_id)?;
	let outcome = async {
		let (item, source) = load_item(conn, &proposal.source_item_id).await?;
		if source.id != proposal.source_id {
			return Err(APIError::Conflict(
				"proposal source no longer exists".to_string(),
			));
		}
		verify_proposal_identity(&proposal, &item)?;
		materialize_verified_item(
			ctx,
			req,
			&item,
			&source,
			MaterializeRequest {
				library_id,
				filename: body.filename,
				idempotency_key: Some(
					body.idempotency_key
						.unwrap_or_else(|| format!("remote-source-import:{proposal_id}")),
				),
			},
		)
		.await
	}
	.await;
	match outcome {
		Ok(materialized) => {
			remote_source_import::Entity::update_many()
				.col_expr(
					remote_source_import::Column::StagedItemId,
					Expr::value(materialized.item_id.clone()),
				)
				.col_expr(
					remote_source_import::Column::UpdatedAt,
					Expr::value(DateTimeWithTimeZone::from(Utc::now())),
				)
				.filter(remote_source_import::Column::Id.eq(proposal_id))
				.filter(remote_source_import::Column::Status.eq(APPROVED))
				.exec(conn)
				.await?;
			Ok(ImportDecisionResponse {
				proposal: ImportSummary::from(load_proposal(conn, proposal_id).await?),
				action: "materialize".to_owned(),
				location_id: None,
				staged_item_id: Some(materialized.item_id),
				deduplicated: Some(materialized.deduplicated),
			})
		},
		Err(error) => {
			release_claim(conn, proposal_id).await?;
			Err(error)
		},
	}
}

async fn reject_import(
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	Path(proposal_id): Path<String>,
	Json(body): Json<ImportDecisionRequest>,
) -> APIResult<Json<ImportSummary>> {
	enforce_admin(&req)?;
	let conn = ctx.conn.as_ref();
	if !claim_proposal(conn, &proposal_id, REJECTED, &req.user().id, body.reason).await? {
		let current = load_proposal(conn, &proposal_id).await?;
		if current.status == REJECTED {
			return Ok(Json(ImportSummary::from(current)));
		}
		return Err(APIError::Conflict(format!(
			"proposal is already {}",
			current.status
		)));
	}
	Ok(Json(ImportSummary::from(
		load_proposal(conn, &proposal_id).await?,
	)))
}

async fn load_item<C: ConnectionTrait>(
	conn: &C,
	id: &str,
) -> APIResult<(remote_source_item::Model, remote_source::Model)> {
	let item = remote_source_item::Entity::find_by_id(id.to_string())
		.one(conn)
		.await?
		.ok_or_else(|| APIError::NotFound("remote source item not found".to_string()))?;
	let source = remote_source::Entity::find_by_id(item.source_id.clone())
		.one(conn)
		.await?
		.ok_or_else(|| APIError::NotFound("remote source not found".to_string()))?;
	Ok((item, source))
}

fn source_request(
	source: &remote_source::Model,
	item: &remote_source_item::Model,
	expected_sha256: Option<&str>,
	mode: SourceReadMode,
	offset: u64,
	length: u64,
) -> APIResult<SourceReadRequest> {
	if item.size < 0 {
		return Err(APIError::Conflict(
			"source item has an invalid size".to_string(),
		));
	}
	let transport = source_transport(&source.transport)?;
	Ok(SourceReadRequest {
		root_id: source.root_id.clone(),
		worker_item_id: item.worker_item_id.clone(),
		worker_content_version: item.worker_content_version.clone(),
		expected_sha256: expected_sha256.map(str::to_owned),
		mode,
		offset,
		length,
		transport,
		expires_at: Utc::now().timestamp() + REMOTE_READ_TTL_SECS,
		max_bytes: length,
	})
}

pub(crate) enum RemoteBody {
	Direct(reqwest::Response),
	Tunnel(stump_worker::TunnelReceiver),
}

impl RemoteBody {
	/// The accepted bytes as one stream, whichever transport carries them.  A
	/// transport that stalls past [`SOURCE_TRANSFER_IDLE_TIMEOUT`] yields a
	/// `TimedOut` error and ends, so no consumer waits on a silent worker.
	fn into_stream(
		self,
	) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
		match self {
			Self::Direct(response) => response
				.bytes_stream()
				.map(|result| {
					result.map(Bytes::from).map_err(|error| {
						let kind = if error.is_timeout() {
							std::io::ErrorKind::TimedOut
						} else {
							std::io::ErrorKind::ConnectionAborted
						};
						std::io::Error::new(kind, error)
					})
				})
				.boxed(),
			Self::Tunnel(receiver) => {
				futures_util::stream::unfold(Some(receiver), |receiver| async move {
					let mut receiver = receiver?;
					match receiver.next_chunk().await {
						Ok(Some(chunk)) => Some((Ok(Bytes::from(chunk)), Some(receiver))),
						Ok(None) => None,
						Err(error) => Some((
							Err(std::io::Error::new(std::io::ErrorKind::TimedOut, error)),
							None,
						)),
					}
				})
				.boxed()
			},
		}
	}
}

async fn open_remote_body(
	ctx: &AppState,
	source: &remote_source::Model,
	item: &remote_source_item::Model,
	expected_sha256: Option<&str>,
	mode: SourceReadMode,
	offset: u64,
	length: u64,
) -> APIResult<RemoteBody> {
	let request = source_request(source, item, expected_sha256, mode, offset, length)?;
	let hub = ctx.source_hub();
	let grant = hub
		.issue_grant(&source.device_id, request)
		.await
		.map_err(|error| APIError::ServiceUnavailable(error.to_string()))?;
	match grant.transport {
		SourceTransport::Direct => {
			hub.wait_ready(&grant.grant_id)
				.await
				.map_err(|error| APIError::ServiceUnavailable(error.to_string()))?;
			let grant = hub
				.consume_direct(&source.device_id, &grant.grant_id)
				.await
				.map_err(|error| APIError::ServiceUnavailable(error.to_string()))?;
			let base = match source.direct_base_url.clone() {
				Some(base) => Some(base),
				None => {
					hub.direct_base_url(&source.device_id, &source.root_id)
						.await
				},
			};
			let Some(base) = base else {
				return Err(APIError::ServiceUnavailable(
					"source direct endpoint is unavailable".to_string(),
				));
			};
			let url = format!(
				"{}/v1/source/read/{}",
				base.trim_end_matches('/'),
				grant.grant_id
			);
			let response = DIRECT_CLIENT
				.get(url)
				.send()
				.await
				.map_err(|error| APIError::ServiceUnavailable(error.to_string()))?;
			if !response.status().is_success() {
				return Err(APIError::ServiceUnavailable(
					"source worker is unavailable".to_string(),
				));
			}
			Ok(RemoteBody::Direct(response))
		},
		SourceTransport::Tunnel => {
			hub.wait_ready(&grant.grant_id)
				.await
				.map_err(|error| APIError::ServiceUnavailable(error.to_string()))?;
			let receiver = hub
				.accept_tunnel(&source.device_id, &grant.grant_id)
				.await
				.map_err(|error| APIError::ServiceUnavailable(error.to_string()))?;
			Ok(RemoteBody::Tunnel(receiver))
		},
	}
}

async fn hash_body(
	body: RemoteBody,
	expected_size: u64,
	max_bytes: u64,
) -> APIResult<(String, u64)> {
	let mut hasher = Sha256::new();
	let mut total = 0_u64;
	let mut stream = body.into_stream();
	while let Some(chunk) = stream.next().await {
		let chunk =
			chunk.map_err(|error| APIError::ServiceUnavailable(error.to_string()))?;
		total = total.saturating_add(chunk.len() as u64);
		if total > max_bytes {
			return Err(APIError::BadGateway(
				"source worker exceeded the grant byte budget".to_string(),
			));
		}
		hasher.update(&chunk);
	}
	if total != expected_size {
		return Err(APIError::BadGateway(format!(
			"source returned {total} bytes; expected {expected_size}"
		)));
	}
	Ok((format!("{:x}", hasher.finalize()), total))
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct LinkRequest {
	media_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VerifyResponse {
	item_id: String,
	sha256: String,
	content_version: String,
	size: u64,
	observation_state: String,
}

async fn verify_item(
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	Path(item_id): Path<String>,
) -> APIResult<Json<VerifyResponse>> {
	enforce_admin(&req)?;
	let (item, source) = load_item(ctx.conn.as_ref(), &item_id).await?;
	let expected = u64::try_from(item.size)
		.map_err(|_| APIError::Conflict("source item has an invalid size".to_string()))?;
	let body = open_remote_body(
		&ctx,
		&source,
		&item,
		None,
		SourceReadMode::Full,
		0,
		expected,
	)
	.await?;
	let (sha256, size) = hash_body(body, expected, expected).await?;
	let content_version = format!("sha256:{sha256}");
	let mut active = item.into_active_model();
	active.sha256 = Set(Some(sha256.clone()));
	active.observation_state = Set(VERIFIED.to_string());
	active.updated_at = Set(DateTimeWithTimeZone::from(Utc::now()));
	active.update(ctx.conn.as_ref()).await?;
	Ok(Json(VerifyResponse {
		item_id,
		sha256,
		content_version,
		size,
		observation_state: VERIFIED.to_string(),
	}))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LinkResponse {
	item_id: String,
	media_id: String,
	location_id: String,
	content_version: String,
}

async fn link_item(
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	Path(item_id): Path<String>,
	Json(body): Json<LinkRequest>,
) -> APIResult<Json<LinkResponse>> {
	enforce_admin(&req)?;
	let (item, _source) = load_item(ctx.conn.as_ref(), &item_id).await?;
	Ok(Json(
		link_verified_item(ctx.conn.as_ref(), &item, &body.media_id).await?,
	))
}

/// Upsert the verified remote location for `media_id`.  Runs on whatever
/// connection the caller provides, so an approval can commit it together with
/// the proposal claim.
async fn link_verified_item<C: ConnectionTrait>(
	conn: &C,
	item: &remote_source_item::Model,
	media_id: &str,
) -> APIResult<LinkResponse> {
	if item.observation_state != VERIFIED || item.sha256.is_none() {
		return Err(APIError::Conflict(
			"remote item must be verified before linking".to_string(),
		));
	}
	media::Entity::find_by_id(media_id.to_owned())
		.one(conn)
		.await?
		.ok_or_else(|| APIError::NotFound("media not found".to_string()))?;
	let digest = item.sha256.clone().expect("checked above");
	let content_version = format!("sha256:{digest}");
	let existing = media_location::Entity::find()
		.filter(media_location::Column::MediaId.eq(media_id.to_owned()))
		.filter(media_location::Column::SourceItemId.eq(item.id.clone()))
		.filter(media_location::Column::Kind.eq(REMOTE_LOCATION_KIND))
		.one(conn)
		.await?;
	let now = DateTimeWithTimeZone::from(Utc::now());
	let location = if let Some(model) = existing {
		let id = model.id.clone();
		let mut active = model.into_active_model();
		active.sha256 = Set(digest.clone());
		active.content_version = Set(content_version.clone());
		active.health = Set(ONLINE.to_string());
		active.durability_role = Set(REMOTE_LOCATION_KIND.to_string());
		active.verified_at = Set(Some(now));
		active.last_seen_at = Set(Some(now));
		active.update(conn).await?;
		id
	} else {
		let model = media_location::ActiveModel {
			id: Set(uuid::Uuid::new_v4().to_string()),
			media_id: Set(media_id.to_owned()),
			source_item_id: Set(Some(item.id.clone())),
			kind: Set(REMOTE_LOCATION_KIND.to_string()),
			sha256: Set(digest),
			content_version: Set(content_version.clone()),
			health: Set(ONLINE.to_string()),
			durability_role: Set(REMOTE_LOCATION_KIND.to_string()),
			cache_path: Set(None),
			verified_at: Set(Some(now)),
			last_seen_at: Set(Some(now)),
			created_at: Set(now),
			updated_at: Set(now),
		};
		model.insert(conn).await?.id
	};
	let mut item_active = item.clone().into_active_model();
	item_active.imported_media_id = Set(Some(media_id.to_owned()));
	item_active.update(conn).await?;
	Ok(LinkResponse {
		item_id: item.id.clone(),
		media_id: media_id.to_owned(),
		location_id: location,
		content_version,
	})
}

#[cfg(feature = "ingest")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MaterializeRequest {
	library_id: String,
	filename: Option<String>,
	idempotency_key: Option<String>,
}

#[cfg(feature = "ingest")]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MaterializeResponse {
	item_id: String,
	deduplicated: bool,
}

#[cfg(feature = "ingest")]
struct RemoveMaterializationFile(std::path::PathBuf);

#[cfg(feature = "ingest")]
impl Drop for RemoveMaterializationFile {
	fn drop(&mut self) {
		let _ = std::fs::remove_file(&self.0);
	}
}

#[cfg(feature = "ingest")]
async fn materialize_item(
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	Path(item_id): Path<String>,
	Json(body): Json<MaterializeRequest>,
) -> APIResult<Json<MaterializeResponse>> {
	enforce_admin(&req)?;
	let (item, source) = load_item(ctx.conn.as_ref(), &item_id).await?;
	Ok(Json(
		materialize_verified_item(&ctx, &req, &item, &source, body).await?,
	))
}

#[cfg(feature = "ingest")]
async fn materialize_verified_item(
	ctx: &AppState,
	req: &AuthContext,
	item: &remote_source_item::Model,
	source: &remote_source::Model,
	body: MaterializeRequest,
) -> APIResult<MaterializeResponse> {
	if item.observation_state != VERIFIED || item.sha256.is_none() {
		return Err(APIError::Conflict(
			"remote item must be verified before materializing".to_string(),
		));
	}
	let expected = u64::try_from(item.size)
		.map_err(|_| APIError::Conflict("source item has an invalid size".to_string()))?;
	let body_stream = open_remote_body(
		ctx,
		source,
		item,
		item.sha256.as_deref(),
		SourceReadMode::Full,
		0,
		expected,
	)
	.await?;
	let temp_dir = ctx
		.config
		.get_transform_cache_dir()
		.join("remote-materialize");
	tokio::fs::create_dir_all(&temp_dir).await?;
	let temp_path = temp_dir.join(format!("{}.part", uuid::Uuid::new_v4()));
	let _remove_temp = RemoveMaterializationFile(temp_path.clone());
	let mut file = tokio::fs::File::create(&temp_path).await?;
	let mut total = 0_u64;
	let mut hasher = Sha256::new();
	let mut stream = body_stream.into_stream();
	while let Some(chunk) = stream.next().await {
		let chunk =
			chunk.map_err(|error| APIError::ServiceUnavailable(error.to_string()))?;
		total = total.saturating_add(chunk.len() as u64);
		if total > expected {
			return Err(APIError::BadGateway("source exceeded grant".to_string()));
		}
		hasher.update(&chunk);
		file.write_all(&chunk).await?;
	}
	file.flush().await?;
	if total != expected
		|| format!("{:x}", hasher.finalize()) != item.sha256.clone().unwrap_or_default()
	{
		return Err(APIError::BadGateway(
			"source verification changed during materialization".to_string(),
		));
	}
	drop(file);
	let filename = body
		.filename
		.or_else(|| {
			item.relative_path
				.as_deref()
				.and_then(|path| path.rsplit('/').next().map(str::to_owned))
		})
		.unwrap_or_else(|| format!("{}.bin", item.id));
	let input = tokio::fs::File::open(&temp_path).await?;
	let staged = ctx
		.ingest()
		.store
		.stage_upload(
			&body.library_id,
			Some(&req.user().id),
			None,
			&filename,
			tokio::io::BufReader::new(input),
			body.idempotency_key.as_deref(),
		)
		.await
		.map_err(|error| APIError::InternalServerError(error.to_string()))?;
	Ok(MaterializeResponse {
		item_id: staged.item.id,
		deduplicated: staged.deduplicated,
	})
}

#[cfg(not(feature = "ingest"))]
async fn materialize_item(
	State(_ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	Path(_item_id): Path<String>,
	Json(_body): Json<Value>,
) -> APIResult<Response> {
	enforce_admin(&req)?;
	Err(APIError::NotImplemented)
}

/// Return the catalogued object size even when its source is offline. A
/// verified remote location remains metadata, not a reason to return 404.
pub(crate) async fn remote_media_size(
	ctx: &AppState,
	media_id: &str,
) -> APIResult<Option<u64>> {
	let Some(location) = media_location::Entity::find()
		.filter(media_location::Column::MediaId.eq(media_id.to_string()))
		.filter(media_location::Column::Kind.eq(REMOTE_LOCATION_KIND))
		.filter(media_location::Column::SourceItemId.is_not_null())
		.order_by_desc(media_location::Column::VerifiedAt)
		.one(ctx.conn.as_ref())
		.await?
	else {
		return Ok(None);
	};
	let Some(source_item_id) = location.source_item_id else {
		return Ok(None);
	};
	let size = remote_source_item::Entity::find_by_id(source_item_id)
		.one(ctx.conn.as_ref())
		.await?
		.and_then(|item| u64::try_from(item.size).ok());
	Ok(size)
}

/// Open a remote transfer for `serve_media_file`.  The helper is `pub(crate)`
/// so the existing media route keeps its ACL and local `ServeFile` path.
///
/// Verified copies are tried newest first.  A disconnect marks only the
/// `remote_source` offline, never its locations, so a copy whose source is
/// gone is skipped in favour of an older copy that can still be served; only
/// when no copy can be opened is the media reported unavailable.
pub(crate) async fn open_media_transfer(
	ctx: &AppState,
	media_id: &str,
	offset: u64,
	length: u64,
) -> APIResult<Option<(RemoteBody, remote_source_item::Model)>> {
	let locations = media_location::Entity::find()
		.filter(media_location::Column::MediaId.eq(media_id.to_string()))
		.filter(media_location::Column::Kind.eq(REMOTE_LOCATION_KIND))
		.filter(media_location::Column::Health.eq(ONLINE))
		.filter(media_location::Column::SourceItemId.is_not_null())
		.order_by_desc(media_location::Column::VerifiedAt)
		.order_by_asc(media_location::Column::Id)
		.all(ctx.conn.as_ref())
		.await?;
	for location in locations {
		let Some(source_item_id) = location.source_item_id else {
			continue;
		};
		let Some(item) = remote_source_item::Entity::find_by_id(source_item_id)
			.one(ctx.conn.as_ref())
			.await?
		else {
			continue;
		};
		if item.observation_state != VERIFIED
			|| item.sha256.as_deref() != Some(location.sha256.as_str())
		{
			continue;
		}
		let Some(source) = remote_source::Entity::find_by_id(item.source_id.clone())
			.filter(remote_source::Column::Health.eq(ONLINE))
			.one(ctx.conn.as_ref())
			.await?
		else {
			continue;
		};
		let mode =
			if offset == 0 && length == u64::try_from(item.size).unwrap_or_default() {
				SourceReadMode::Full
			} else {
				SourceReadMode::Range
			};
		match open_remote_body(
			ctx,
			&source,
			&item,
			Some(location.sha256.as_str()),
			mode,
			offset,
			length,
		)
		.await
		{
			Ok(body) => return Ok(Some((body, item))),
			// The stored health is what the socket handler last wrote; the
			// hub's refusal is the live truth, so move on to the next copy.
			Err(APIError::ServiceUnavailable(reason)) => {
				tracing::warn!(
					media_id,
					source_id = %source.id,
					%reason,
					"verified remote copy could not be opened; trying the next"
				);
			},
			Err(error) => return Err(error),
		}
	}
	Ok(None)
}

pub(crate) fn remote_response(
	body: RemoteBody,
	item: &remote_source_item::Model,
	offset: u64,
	length: u64,
	total_size: u64,
	filename: &str,
	media_type: Option<&str>,
) -> Response {
	let mut response = Response::new(Body::from_stream(body.into_stream()));
	*response.status_mut() = if offset == 0 && length == total_size {
		axum::http::StatusCode::OK
	} else {
		axum::http::StatusCode::PARTIAL_CONTENT
	};
	let headers = response.headers_mut();
	headers.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());
	headers.insert(header::CONTENT_LENGTH, length.to_string().parse().unwrap());
	if offset != 0 || length != total_size {
		let end = offset.saturating_add(length).saturating_sub(1);
		let value = format!("bytes {offset}-{end}/{total_size}");
		headers.insert(header::CONTENT_RANGE, value.parse().unwrap());
	}
	if let Some(media_type) = media_type {
		if let Ok(value) = media_type.parse() {
			headers.insert(header::CONTENT_TYPE, value);
		}
	}
	let safe_filename = filename.replace('"', "");
	if let Ok(value) = format!("attachment; filename=\"{safe_filename}\"").parse() {
		headers.insert(header::CONTENT_DISPOSITION, value);
	}
	let _ = item;
	response
}
#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn identifier_matching_preserves_scheme_namespaces() {
		let metadata = json!({
			"isbn": "same-value",
			"asin": "same-value",
			"identifiers": {
				"google": "same-value",
				"unknown": "same-value"
			},
			"identifier_calibre": "same-value",
			"untyped": "same-value"
		});
		let identifiers = metadata_identifier_values(Some(&metadata));
		assert!(identifiers.contains(&("isbn".to_owned(), "samevalue".to_owned())));
		assert!(identifiers.contains(&("asin".to_owned(), "samevalue".to_owned())));
		assert!(identifiers.contains(&("google".to_owned(), "samevalue".to_owned())));
		assert!(identifiers.contains(&("calibre".to_owned(), "samevalue".to_owned())));
		assert!(!identifiers.contains(&("unknown".to_owned(), "samevalue".to_owned())));
		assert!(!identifiers.contains(&("".to_owned(), "samevalue".to_owned())));

		let local = BTreeSet::from([("asin".to_owned(), "samevalue".to_owned())]);
		let shared = identifiers
			.intersection(&local)
			.cloned()
			.collect::<BTreeSet<_>>();
		assert_eq!(
			identifier_evidence(&shared),
			vec![json!({ "scheme": "asin", "value": "samevalue" })]
		);
	}

	#[test]
	fn generic_identifier_map_ignores_untyped_values() {
		let metadata = json!({
			"identifiers": {
				"isbn13": "978-1-4028-9462-6",
				"asin": "B000123",
				"notes": "not-an-identifier"
			}
		});
		let identifiers = metadata_identifier_values(Some(&metadata));
		assert_eq!(
			identifiers,
			BTreeSet::from([
				("asin".to_owned(), "b000123".to_owned()),
				("isbn".to_owned(), "9781402894626".to_owned()),
			])
		);
	}

	/// The built-in source worker serializes `CatalogEntry.modified_at_ms`
	/// as a bare number; it must land as a 2020s date, not one tens of
	/// thousands of years out.  Plain seconds and RFC 3339 keep working.
	#[test]
	fn modified_at_reads_worker_milliseconds_and_seconds() {
		let millis = modified_at(Some(&json!(1_800_000_000_000_i64))).unwrap();
		assert_eq!(millis.to_rfc3339(), "2027-01-15T08:00:00+00:00");
		let seconds = modified_at(Some(&json!(1_700_000_000_i64))).unwrap();
		assert_eq!(seconds.to_rfc3339(), "2023-11-14T22:13:20+00:00");
		let text = modified_at(Some(&json!("2024-02-03T04:05:06+00:00"))).unwrap();
		assert_eq!(text.to_rfc3339(), "2024-02-03T04:05:06+00:00");
		assert!(modified_at(Some(&json!("not a date"))).is_none());
		assert!(modified_at(None).is_none());
	}

	fn location(media_id: &str, sha256: &str) -> media_location::Model {
		let now = DateTimeWithTimeZone::from(Utc::now());
		media_location::Model {
			id: format!("{media_id}-{sha256}"),
			media_id: media_id.to_owned(),
			source_item_id: None,
			kind: REMOTE_LOCATION_KIND.to_owned(),
			sha256: sha256.to_owned(),
			content_version: format!("sha256:{sha256}"),
			health: ONLINE.to_owned(),
			durability_role: REMOTE_LOCATION_KIND.to_owned(),
			cache_path: None,
			verified_at: Some(now),
			last_seen_at: Some(now),
			created_at: now,
			updated_at: now,
		}
	}

	fn verified_item(sha256: &str, metadata: Value) -> remote_source_item::Model {
		let now = DateTimeWithTimeZone::from(Utc::now());
		remote_source_item::Model {
			id: "item".to_owned(),
			source_id: "source".to_owned(),
			worker_item_id: "worker-item".to_owned(),
			worker_content_version: "v1".to_owned(),
			relative_path: Some("book.epub".to_owned()),
			size: 3,
			modified_at: None,
			media_type: None,
			quick_fingerprint: None,
			sha256: Some(sha256.to_owned()),
			metadata: Some(metadata),
			retention: None,
			observation_state: VERIFIED.to_owned(),
			last_seen_revision: 1,
			last_seen_at: now,
			created_at: now,
			updated_at: now,
			imported_media_id: None,
		}
	}

	/// The indexed matcher keeps the tiered semantics: exact digest first,
	/// then typed identifiers, then title plus author, with ambiguity at any
	/// tier failing closed instead of falling through.
	#[test]
	fn candidate_index_matches_by_digest_identifier_and_title() {
		let metadata = vec![
			media_metadata::Model {
				media_id: Some("by-isbn".into()),
				identifier_isbn: Some("978-1-4028-9462-6".into()),
				title: Some("Dune".into()),
				writers: Some("Frank Herbert".into()),
				..Default::default()
			},
			media_metadata::Model {
				media_id: Some("by-asin".into()),
				identifier_amazon: Some("B000123".into()),
				title: Some("Dune".into()),
				writers: Some("Someone Else".into()),
				..Default::default()
			},
			media_metadata::Model {
				media_id: Some("twin-a".into()),
				identifier_uuid: Some("twin".into()),
				..Default::default()
			},
			media_metadata::Model {
				media_id: Some("twin-b".into()),
				identifier_uuid: Some("twin".into()),
				..Default::default()
			},
			media_metadata::Model {
				media_id: None,
				identifier_isbn: Some("orphan".into()),
				..Default::default()
			},
		];
		let locations = vec![
			location("digest-one", "aaa"),
			location("digest-two", "bbb"),
			location("digest-three", "bbb"),
		];
		let index = CandidateIndex::build(&metadata, &locations);

		let digest = find_candidate(&verified_item("aaa", json!({})), &index).unwrap();
		assert_eq!(
			(digest.media_id.as_str(), digest.kind, digest.score),
			("digest-one", "digest", 100)
		);
		assert!(
			find_candidate(
				&verified_item("bbb", json!({ "isbn": "9781402894626" })),
				&index
			)
			.is_none(),
			"two media with the same bytes is ambiguous and must not fall through"
		);

		let identifier = find_candidate(
			&verified_item(
				"zzz",
				json!({ "identifiers": { "isbn13": "978-1-4028-9462-6" } }),
			),
			&index,
		)
		.unwrap();
		assert_eq!(identifier.media_id, "by-isbn");
		assert_eq!(identifier.kind, "identifier");
		assert_eq!(
			identifier.evidence,
			json!({ "identifiers": [{ "scheme": "isbn", "value": "9781402894626" }] })
		);
		assert!(
			find_candidate(&verified_item("zzz", json!({ "uuid": "twin" })), &index)
				.is_none(),
			"an identifier shared by two media is ambiguous"
		);
		assert!(
			find_candidate(&verified_item("zzz", json!({ "isbn": "orphan" })), &index)
				.is_none(),
			"metadata without a media id cannot be a candidate"
		);

		let title = find_candidate(
			&verified_item(
				"zzz",
				json!({ "title": "DUNE", "authors": ["Herbert, Frank", "Frank Herbert"] }),
			),
			&index,
		)
		.unwrap();
		assert_eq!(title.media_id, "by-isbn");
		assert_eq!(title.kind, "metadata");
		assert!(
			find_candidate(&verified_item("zzz", json!({ "title": "Dune" })), &index)
				.is_none(),
			"a title without any author is not evidence"
		);
		assert!(find_candidate(
			&verified_item("zzz", json!({ "title": "Dune", "author": "Nobody" })),
			&index
		)
		.is_none());
	}

	mod db {
		use super::*;
		use ::tests::fake_data;
		use migrations::{Migrator, MigratorTrait};
		use sea_orm::{ConnectOptions, Database};
		use std::sync::Arc;
		use stump_worker::source_protocol::parse_source_server_frame;

		struct Fixture {
			_dir: tempfile::TempDir,
			ctx: AppState,
			user_id: String,
			media_id: String,
			source: remote_source::Model,
			item: remote_source_item::Model,
		}

		const DIGEST: &str =
			"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

		/// A file-backed database with a real pool: the approval race needs
		/// two connections contending for SQLite's write lock, which a
		/// single-connection in-memory pool cannot express.  Migrations run on
		/// a one-connection pool first, as `stump_core::database::connect`
		/// does: sea-orm-migration does not transact SQLite DDL, and the
		/// table-rebuild migrations break when spread over pooled connections.
		async fn fixture() -> Fixture {
			let dir = tempfile::tempdir().unwrap();
			let url = format!("sqlite://{}/source.db?mode=rwc", dir.path().display());
			let mut migrate_options = ConnectOptions::new(url.clone());
			migrate_options.max_connections(1).sqlx_logging(false);
			let migrator = Database::connect(migrate_options).await.unwrap();
			migrator
				.execute_unprepared("PRAGMA journal_mode=WAL")
				.await
				.unwrap();
			Migrator::up(&migrator, None).await.unwrap();
			migrator.close().await.unwrap();
			let mut options = ConnectOptions::new(url);
			options.max_connections(4).sqlx_logging(false);
			let conn = Database::connect(options).await.unwrap();

			let user = fake_data::User::new("operator").insert(&conn).await;
			let library = fake_data::Library::default().insert(&conn).await;
			let series = fake_data::Series {
				library_id: Some(library.id.clone()),
				..Default::default()
			}
			.insert(&conn)
			.await;
			let media = fake_data::Media {
				series_id: series.id.clone(),
				..Default::default()
			}
			.insert(&conn)
			.await;
			let ctx: AppState = Arc::new(stump_core::Ctx::for_testing(conn));
			let (source, item) =
				source_with_item(&ctx, &user.id, "device-a", "root-a").await;
			Fixture {
				_dir: dir,
				ctx,
				user_id: user.id,
				media_id: media.id,
				source,
				item,
			}
		}

		async fn source_with_item(
			ctx: &AppState,
			user_id: &str,
			device_id: &str,
			root_id: &str,
		) -> (remote_source::Model, remote_source_item::Model) {
			let conn = ctx.conn.as_ref();
			device::ActiveModel {
				id: Set(device_id.to_owned()),
				user_id: Set(user_id.to_owned()),
				name: Set(device_id.to_owned()),
				kind: Set(DeviceKind::SourceWorker),
				..Default::default()
			}
			.insert(conn)
			.await
			.unwrap();
			let now = DateTimeWithTimeZone::from(Utc::now());
			let source = remote_source::ActiveModel {
				device_id: Set(device_id.to_owned()),
				root_id: Set(root_id.to_owned()),
				label: Set("Books".to_owned()),
				kind: Set("ebooks".to_owned()),
				privacy_mode: Set("catalog".to_owned()),
				transport: Set("tunnel".to_owned()),
				direct_base_url: Set(None),
				current_revision: Set(1),
				last_seen_at: Set(now),
				health: Set(ONLINE.to_owned()),
				..Default::default()
			}
			.insert(conn)
			.await
			.unwrap();
			let item = remote_source_item::ActiveModel {
				source_id: Set(source.id.clone()),
				worker_item_id: Set(format!("{root_id}-item")),
				worker_content_version: Set("v1".to_owned()),
				relative_path: Set(Some("book.epub".to_owned())),
				size: Set(3),
				modified_at: Set(None),
				media_type: Set(Some("application/epub+zip".to_owned())),
				quick_fingerprint: Set(None),
				sha256: Set(Some(DIGEST.to_owned())),
				metadata: Set(None),
				retention: Set(None),
				observation_state: Set(VERIFIED.to_owned()),
				last_seen_revision: Set(1),
				last_seen_at: Set(now),
				imported_media_id: Set(None),
				..Default::default()
			}
			.insert(conn)
			.await
			.unwrap();
			(source, item)
		}

		impl Fixture {
			async fn proposal(&self) -> remote_source_import::Model {
				remote_source_import::ActiveModel {
					source_id: Set(self.source.id.clone()),
					source_item_id: Set(self.item.id.clone()),
					target_media_id: Set(self.media_id.clone()),
					worker_item_id: Set(self.item.worker_item_id.clone()),
					worker_content_version: Set(self.item.worker_content_version.clone()),
					source_sha256: Set(DIGEST.to_owned()),
					source_size: Set(self.item.size),
					match_kind: Set("digest".to_owned()),
					match_score: Set(100),
					match_evidence: Set(json!({ "sha256": DIGEST })),
					status: Set(PROPOSED.to_owned()),
					..Default::default()
				}
				.insert(self.ctx.conn.as_ref())
				.await
				.unwrap()
			}

			async fn location_count(&self) -> u64 {
				media_location::Entity::find()
					.filter(media_location::Column::MediaId.eq(self.media_id.clone()))
					.count(self.ctx.conn.as_ref())
					.await
					.unwrap()
			}

			async fn attach_responding_worker(&self, device_id: &str, root_id: &str) {
				let hub = self.ctx.source_hub();
				let (mut outbound, _) = hub
					.attach(
						device_id,
						device_id,
						SourceWorkerHello {
							name: None,
							version: None,
							roots: vec![SourceRootHello {
								root_id: root_id.to_owned(),
								label: "Books".to_owned(),
								kind: "ebooks".to_owned(),
								privacy_mode: "catalog".to_owned(),
								transport: SourceTransport::Tunnel,
								direct_base_url: None,
							}],
						},
					)
					.await;
				let device_id = device_id.to_owned();
				tokio::spawn(async move {
					while let Some(frame) = outbound.recv().await {
						if let Ok(SourceServerFrame::Read(grant)) =
							parse_source_server_frame(&frame)
						{
							let _ = hub.read_ready(&device_id, &grant.grant_id).await;
						}
					}
				});
			}
		}

		/// The regression: a rejection that commits while an approval is in
		/// flight must leave nothing applied.  The rejecting transaction holds
		/// the write lock so the approval is genuinely queued behind it.
		#[tokio::test]
		async fn approval_applies_nothing_when_a_rejection_wins_the_claim() {
			let fixture = fixture().await;
			let proposal = fixture.proposal().await;
			let conn = fixture.ctx.conn.as_ref();

			let holder = models::txn::begin_write(conn).await.unwrap();
			assert!(claim_proposal(
				&holder,
				&proposal.id,
				REJECTED,
				&fixture.user_id,
				None
			)
			.await
			.unwrap());

			let approval = {
				let ctx = fixture.ctx.clone();
				let proposal_id = proposal.id.clone();
				let actor = fixture.user_id.clone();
				tokio::spawn(async move {
					approve_link(&ctx, &proposal_id, &actor, None).await
				})
			};
			tokio::time::sleep(std::time::Duration::from_millis(250)).await;
			assert!(
				!approval.is_finished(),
				"the approval waits for the write lock"
			);
			holder.commit().await.unwrap();

			let error = approval.await.unwrap().expect_err("the rejection won");
			assert!(
				matches!(&error, APIError::Conflict(reason) if reason == "proposal was rejected"),
				"{error:?}"
			);
			assert_eq!(fixture.location_count().await, 0, "nothing was linked");
			let settled = load_proposal(conn, &proposal.id).await.unwrap();
			assert_eq!(settled.status, REJECTED);
			assert_eq!(settled.applied_location_id, None);
		}

		/// Two operators approving at once: exactly one link is applied and
		/// both see the same outcome.
		#[tokio::test]
		async fn concurrent_approvals_link_exactly_once() {
			let fixture = fixture().await;
			let proposal = fixture.proposal().await;
			let approvals = (0..4)
				.map(|_| {
					let ctx = fixture.ctx.clone();
					let proposal_id = proposal.id.clone();
					let actor = fixture.user_id.clone();
					tokio::spawn(async move {
						approve_link(&ctx, &proposal_id, &actor, None).await
					})
				})
				.collect::<Vec<_>>();
			let mut location_ids = BTreeSet::new();
			for approval in approvals {
				let response = approval.await.unwrap().unwrap();
				assert_eq!(response.action, "link");
				assert_eq!(response.proposal.status, APPROVED);
				location_ids.insert(response.location_id.unwrap());
			}
			assert_eq!(location_ids.len(), 1, "every approval reports the one link");
			assert_eq!(fixture.location_count().await, 1);
			let settled = load_proposal(fixture.ctx.conn.as_ref(), &proposal.id)
				.await
				.unwrap();
			assert_eq!(settled.applied_location_id, location_ids.into_iter().next());
			let item = remote_source_item::Entity::find_by_id(fixture.item.id.clone())
				.one(fixture.ctx.conn.as_ref())
				.await
				.unwrap()
				.unwrap();
			assert_eq!(item.imported_media_id, Some(fixture.media_id.clone()));
		}

		/// A decision is claimed once: repeating it is idempotent, and the
		/// opposite decision afterwards is a conflict that changes nothing.
		#[tokio::test]
		async fn a_proposal_is_decided_once() {
			let fixture = fixture().await;
			let proposal = fixture.proposal().await;
			let conn = fixture.ctx.conn.as_ref();
			assert!(claim_proposal(
				conn,
				&proposal.id,
				REJECTED,
				&fixture.user_id,
				Some("dup".into())
			)
			.await
			.unwrap());
			assert!(!claim_proposal(
				conn,
				&proposal.id,
				REJECTED,
				&fixture.user_id,
				None
			)
			.await
			.unwrap());
			let error = approve_link(&fixture.ctx, &proposal.id, &fixture.user_id, None)
				.await
				.expect_err("a rejected proposal cannot be approved");
			assert!(matches!(error, APIError::Conflict(_)), "{error:?}");
			assert_eq!(fixture.location_count().await, 0);
			let settled = load_proposal(conn, &proposal.id).await.unwrap();
			assert_eq!(settled.status, REJECTED);
			assert_eq!(settled.decision_reason.as_deref(), Some("dup"));
		}

		/// Two verified copies, the newest on a source that went offline: the
		/// reader is served from the older online copy instead of a 503.
		#[tokio::test]
		async fn remote_transfer_falls_back_to_an_online_copy() {
			let fixture = fixture().await;
			let conn = fixture.ctx.conn.as_ref();
			let (offline_source, offline_item) =
				source_with_item(&fixture.ctx, &fixture.user_id, "device-b", "root-b")
					.await;
			let mut offline = offline_source.into_active_model();
			offline.health = Set("offline".to_owned());
			offline.update(conn).await.unwrap();

			let earlier =
				DateTimeWithTimeZone::from(Utc::now() - chrono::Duration::hours(1));
			let later = DateTimeWithTimeZone::from(Utc::now());
			for (item, verified_at) in [(&fixture.item, earlier), (&offline_item, later)]
			{
				media_location::ActiveModel {
					media_id: Set(fixture.media_id.clone()),
					source_item_id: Set(Some(item.id.clone())),
					kind: Set(REMOTE_LOCATION_KIND.to_owned()),
					sha256: Set(DIGEST.to_owned()),
					content_version: Set(format!("sha256:{DIGEST}")),
					health: Set(ONLINE.to_owned()),
					durability_role: Set(REMOTE_LOCATION_KIND.to_owned()),
					cache_path: Set(None),
					verified_at: Set(Some(verified_at)),
					last_seen_at: Set(Some(verified_at)),
					..Default::default()
				}
				.insert(conn)
				.await
				.unwrap();
			}
			fixture.attach_responding_worker("device-a", "root-a").await;

			let (body, served) = tokio::time::timeout(
				std::time::Duration::from_secs(5),
				open_media_transfer(&fixture.ctx, &fixture.media_id, 0, 3),
			)
			.await
			.expect("the online copy answers promptly")
			.unwrap()
			.expect("an online verified copy exists");
			assert_eq!(
				served.id, fixture.item.id,
				"the older online copy is served"
			);
			assert!(matches!(body, RemoteBody::Tunnel(_)));
			assert_eq!(fixture.ctx.source_hub().pending_grants(), 1);
			drop(body);
			assert_eq!(
				fixture.ctx.source_hub().pending_grants(),
				0,
				"dropping the body releases the grant"
			);
		}
	}
}
