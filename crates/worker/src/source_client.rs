//! Source-worker control client.
//!
//! This module owns the source-specific credential/socket, catalog scans, and
//! grant dispatch. It is intentionally separate from the compute worker client:
//! source credentials cannot authenticate to the compute socket and vice versa.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use tokio::sync::{mpsc, Semaphore};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::header, Message};

use crate::source_catalog::{CatalogError, SourceCatalog, SourceRootConfig};
use crate::source_direct::{start_direct_server, DirectGrantStore};
use crate::source_protocol::{
	encode_source_frame, parse_source_server_frame, SourceManifestChunk, SourceReadGrant,
	SourceServerFrame, SourceTransport, SourceWorkerFrame,
	MAX_SOURCE_MANIFEST_FRAME_BYTES, MAX_SOURCE_MANIFEST_ITEMS,
};
use crate::source_scanner::{ScanError, SourceScanner};
use crate::source_tunnel::handle_tunnel_grant;

pub const SOURCE_CONTROL_QUEUE_CAPACITY: usize = 64;
pub const SOURCE_TUNNEL_TASK_LIMIT: usize = 8;
pub const SOURCE_BACKOFF_MIN: Duration = Duration::from_secs(1);
pub const SOURCE_BACKOFF_MAX: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct SourceClientConfig {
	pub server: String,
	pub api_key: String,
	pub name: Option<String>,
	pub roots: Vec<SourceRootConfig>,
	pub state_dir: std::path::PathBuf,
	pub scan_interval: Duration,
	pub listen: Option<SocketAddr>,
}

impl SourceClientConfig {
	pub fn validate(&self) -> Result<(), SourceClientError> {
		if self.server.trim().is_empty() || self.api_key.trim().is_empty() {
			return Err(SourceClientError::Config(
				"server and source API key are required".into(),
			));
		}
		if self.roots.is_empty() {
			return Err(SourceClientError::Config(
				"at least one --source-root is required".into(),
			));
		}
		if self.scan_interval.is_zero() {
			return Err(SourceClientError::Config(
				"source scan interval must be positive".into(),
			));
		}
		let mut ids = HashSet::new();
		let mut direct = false;
		for root in &self.roots {
			root.validate()?;
			if !ids.insert(root.root_id.clone()) {
				return Err(SourceClientError::Config("duplicate source root_id".into()));
			}
			direct |= root.transport == SourceTransport::Direct;
		}
		if direct && self.listen.is_none() {
			return Err(SourceClientError::Config(
				"direct source transport requires --source-listen".into(),
			));
		}
		Ok(())
	}

	#[must_use]
	pub fn socket_url(&self) -> String {
		websocket_base(&self.server, "/api/v2/source-workers/socket")
	}
}

#[derive(Debug, thiserror::Error)]
pub enum SourceClientError {
	#[error("invalid source client configuration: {0}")]
	Config(String),
	#[error(transparent)]
	Catalog(#[from] CatalogError),
	#[error(transparent)]
	Scan(#[from] ScanError),
	#[error(transparent)]
	Io(#[from] std::io::Error),
	#[error("source control URL is invalid: {0}")]
	InvalidUrl(String),
	#[error("source control connection failed: {0}")]
	Connect(String),
	#[error("source control writer closed")]
	WriterClosed,
	#[error("source control task failed: {0}")]
	Task(String),
}

/// Start the source worker and reconnect forever after transient socket
/// failures. The source API key is used only on source control and tunnel
/// sockets; it is never passed to the compute client.
pub async fn run_source(config: SourceClientConfig) -> Result<(), SourceClientError> {
	config.validate()?;
	let catalog = Arc::new(SourceCatalog::open(
		&config.state_dir,
		config.roots.clone(),
	)?);
	let scanner = SourceScanner::new(catalog.clone());
	// Scan before the first hello so a newly started worker never advertises an
	// empty catalog merely because it connected before its first timer tick.
	scanner.scan_once().await?;
	let direct_grants = DirectGrantStore::new();
	let direct_server = if let Some(listen) = config.listen {
		Some(start_direct_server(catalog.clone(), direct_grants.clone(), listen).await?)
	} else {
		None
	};
	let mut backoff = SOURCE_BACKOFF_MIN;
	loop {
		let result = run_source_connection(
			&config,
			catalog.clone(),
			scanner.clone(),
			direct_grants.clone(),
		)
		.await;
		match result {
			Ok(()) => {
				backoff = SOURCE_BACKOFF_MIN;
				tracing::warn!("source control socket closed; reconnecting");
			},
			Err(error) => {
				tracing::warn!(%error, ?backoff, "source control socket failed; reconnecting");
			},
		}
		tokio::time::sleep(backoff).await;
		backoff = (backoff * 2).min(SOURCE_BACKOFF_MAX);
		// Keep the direct listener alive across control reconnects. The server
		// will issue fresh grants once the source control socket is attached.
		let _ = &direct_server;
	}
}

/// Build bounded manifest chunks. A chunk is never emitted until it satisfies
/// both the item and encoded-byte protocol limits.
pub fn manifest_chunks(
	root_id: &str,
	revision: u64,
	items: Vec<crate::source_protocol::SourceManifestItem>,
) -> Result<Vec<SourceManifestChunk>, SourceClientError> {
	let batch_id = format!("{root_id}-{revision}");
	let mut chunks = Vec::new();
	let mut current = Vec::new();
	for item in items {
		if current.len() >= MAX_SOURCE_MANIFEST_ITEMS {
			chunks.push(SourceManifestChunk {
				batch_id: batch_id.clone(),
				root_id: root_id.to_owned(),
				revision,
				sequence: chunks.len() as u64,
				terminal: false,
				items: std::mem::take(&mut current),
			});
		}
		current.push(item);
		let candidate = SourceManifestChunk {
			batch_id: batch_id.clone(),
			root_id: root_id.to_owned(),
			revision,
			sequence: chunks.len() as u64,
			terminal: false,
			items: current.clone(),
		};
		if candidate.encoded_len() > MAX_SOURCE_MANIFEST_FRAME_BYTES {
			let oversized = current.pop().expect("candidate item was just pushed");
			if current.is_empty() {
				return Err(SourceClientError::Config(format!(
					"manifest item `{}` exceeds the 1 MiB frame budget",
					oversized.worker_item_id
				)));
			}
			chunks.push(SourceManifestChunk {
				batch_id: batch_id.clone(),
				root_id: root_id.to_owned(),
				revision,
				sequence: chunks.len() as u64,
				terminal: false,
				items: std::mem::take(&mut current),
			});
			current.push(oversized);
			if (SourceManifestChunk {
				batch_id: batch_id.clone(),
				root_id: root_id.to_owned(),
				revision,
				sequence: chunks.len() as u64,
				terminal: false,
				items: current.clone(),
			})
			.encoded_len()
				> MAX_SOURCE_MANIFEST_FRAME_BYTES
			{
				return Err(SourceClientError::Config(
					"manifest item exceeds the 1 MiB frame budget".into(),
				));
			}
		}
	}
	if !current.is_empty() || chunks.is_empty() {
		chunks.push(SourceManifestChunk {
			batch_id,
			root_id: root_id.to_owned(),
			revision,
			sequence: chunks.len() as u64,
			terminal: true,
			items: current,
		});
	} else if let Some(last) = chunks.last_mut() {
		last.terminal = true;
	}
	let chunk_count = chunks.len();
	for (sequence, chunk) in chunks.iter_mut().enumerate() {
		chunk.sequence = sequence as u64;
		chunk.terminal = sequence + 1 == chunk_count;
		chunk
			.validate()
			.map_err(|error| SourceClientError::Config(error.to_string()))?;
	}
	Ok(chunks)
}

async fn run_source_connection(
	config: &SourceClientConfig,
	catalog: Arc<SourceCatalog>,
	scanner: SourceScanner,
	direct_grants: DirectGrantStore,
) -> Result<(), SourceClientError> {
	let mut request = config
		.socket_url()
		.into_client_request()
		.map_err(|error| SourceClientError::InvalidUrl(error.to_string()))?;
	request.headers_mut().insert(
		header::AUTHORIZATION,
		format!("Bearer {}", config.api_key)
			.parse()
			.map_err(|_| SourceClientError::Config("source API key is invalid".into()))?,
	);
	let (socket, _) = tokio_tungstenite::connect_async(request)
		.await
		.map_err(|error| SourceClientError::Connect(error.to_string()))?;
	let (mut writer, mut reader) = socket.split();
	let (tx, mut rx) = mpsc::channel::<String>(SOURCE_CONTROL_QUEUE_CAPACITY);
	let _writer_task = tokio::spawn(async move {
		while let Some(frame) = rx.recv().await {
			writer
				.send(Message::Text(frame.into()))
				.await
				.map_err(|error| error.to_string())?;
		}
		Ok::<(), String>(())
	});
	let roots = config
		.roots
		.iter()
		.map(SourceRootConfig::hello)
		.collect::<Result<Vec<_>, _>>()?;
	tx.send(encode_source_frame(&SourceWorkerFrame::hello(
		config.name.clone(),
		Some(env!("CARGO_PKG_VERSION").to_owned()),
		roots,
	)))
	.await
	.map_err(|_| SourceClientError::WriterClosed)?;
	send_catalog(&catalog, &tx).await?;

	let mut ticker = tokio::time::interval(config.scan_interval);
	ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
	let inflight = Arc::new(Mutex::new(HashSet::new()));
	let tunnel_permits = Arc::new(Semaphore::new(SOURCE_TUNNEL_TASK_LIMIT));
	let mut tunnel_tasks = tokio::task::JoinSet::new();
	loop {
		tokio::select! {
			Some(joined) = tunnel_tasks.join_next() => {
				if let Err(error) = joined {
					tracing::debug!(%error, "source tunnel task ended unexpectedly");
				}
			}
			_ = ticker.tick() => {
				direct_grants.reap_expired();
				let snapshots = scanner.scan_once().await?;
				for snapshot in snapshots {
					let chunks = manifest_chunks(&snapshot.root.root_id, snapshot.revision, SourceCatalog::manifest_items(&snapshot))?;
					for chunk in chunks {
						tx.send(encode_source_frame(&SourceWorkerFrame::ManifestChunk(chunk)))
							.await
							.map_err(|_| SourceClientError::WriterClosed)?;
					}
				}
			}
			message = reader.next() => {
				let message = match message {
					Some(Ok(message)) => message,
					Some(Err(error)) => return Err(SourceClientError::Connect(error.to_string())),
					None => return Ok(()),
				};
				let text = match message {
					Message::Text(text) => text.to_string(),
					Message::Close(_) => return Ok(()),
					Message::Ping(_) | Message::Pong(_) => continue,
					Message::Binary(_) => {
						tracing::warn!("ignoring binary frame on source control socket");
						continue;
					}
					_ => continue,
				};
				match parse_source_server_frame(&text) {
					Ok(SourceServerFrame::ManifestAck { .. }) => {}
					Ok(SourceServerFrame::Read(grant)) => {
						dispatch_grant(config, catalog.clone(), direct_grants.clone(), tx.clone(), inflight.clone(), tunnel_permits.clone(), &mut tunnel_tasks, grant).await?;
					}
					Err(error) => tracing::warn!(%error, "ignoring invalid source server frame"),
				}
			}
		}
	}
	// The loop returns on reader close; keep cleanup explicit for cancellation
	// and for future alternate shutdown paths.
	#[allow(unreachable_code)]
	{
		tunnel_tasks.abort_all();
		while tunnel_tasks.join_next().await.is_some() {}
		_writer_task.abort();
	}
	Ok(())
}

async fn send_catalog(
	catalog: &SourceCatalog,
	tx: &mpsc::Sender<String>,
) -> Result<(), SourceClientError> {
	for root in catalog.roots() {
		let snapshot = catalog.snapshot(&root.root_id)?;
		let chunks = manifest_chunks(
			&root.root_id,
			snapshot.revision,
			SourceCatalog::manifest_items(&snapshot),
		)?;
		for chunk in chunks {
			tx.send(encode_source_frame(&SourceWorkerFrame::ManifestChunk(
				chunk,
			)))
			.await
			.map_err(|_| SourceClientError::WriterClosed)?;
		}
	}
	Ok(())
}

/// Validate and dispatch one read grant.  A tunnel grant id enters `inflight`
/// only once its transfer task exists: every earlier refusal (invalid grant,
/// stale item, transport mismatch, full tunnel capacity) answers `ReadFailed`
/// and leaves nothing behind, so repeated refusals cannot grow the set.
#[allow(clippy::too_many_arguments)]
async fn dispatch_grant(
	config: &SourceClientConfig,
	catalog: Arc<SourceCatalog>,
	direct_grants: DirectGrantStore,
	tx: mpsc::Sender<String>,
	inflight: Arc<Mutex<HashSet<String>>>,
	tunnel_permits: Arc<Semaphore>,
	tunnel_tasks: &mut tokio::task::JoinSet<Result<(), String>>,
	grant: SourceReadGrant,
) -> Result<(), SourceClientError> {
	let grant_id = grant.grant_id.clone();
	if let Err(error) = grant.validate() {
		send_read_failed(&tx, &grant_id, &error.to_string()).await?;
		return Ok(());
	}
	let resolved = match catalog.resolve_grant(&grant) {
		Ok(resolved) => resolved,
		Err(error) => {
			send_read_failed(&tx, &grant_id, &error.to_string()).await?;
			return Ok(());
		},
	};
	if resolved.root.transport != grant.transport {
		send_read_failed(&tx, &grant_id, "source grant transport does not match root")
			.await?;
		return Ok(());
	}
	match grant.transport {
		SourceTransport::Direct => {
			if let Err(error) = direct_grants.insert(grant) {
				send_read_failed(&tx, &grant_id, &error.to_string()).await?;
			} else {
				tx.send(encode_source_frame(&SourceWorkerFrame::ReadReady {
					grant_id,
				}))
				.await
				.map_err(|_| SourceClientError::WriterClosed)?;
			}
		},
		SourceTransport::Tunnel => {
			let permit = match tunnel_permits.try_acquire_owned() {
				Ok(permit) => permit,
				Err(_) => {
					send_read_failed(&tx, &grant_id, "source tunnel capacity is full")
						.await?;
					return Ok(());
				},
			};
			let is_new = inflight.lock().insert(grant_id.clone());
			if !is_new {
				send_read_failed(&tx, &grant_id, "source grant replay").await?;
				return Ok(());
			}
			tx.send(encode_source_frame(&SourceWorkerFrame::ReadReady {
				grant_id: grant_id.clone(),
			}))
			.await
			.map_err(|_| SourceClientError::WriterClosed)?;
			let server = config.server.clone();
			let api_key = config.api_key.clone();
			let tx_done = tx.clone();
			let inflight_done = inflight.clone();
			tunnel_tasks.spawn(async move {
				let _permit = permit;
				let result = handle_tunnel_grant(catalog, &server, &api_key, grant).await;
				if let Err(error) = &result {
					let _ = tx_done
						.send(encode_source_frame(&SourceWorkerFrame::ReadFailed {
							grant_id: grant_id.clone(),
							error: error.to_string(),
						}))
						.await;
				}
				inflight_done.lock().remove(&grant_id);
				result.map_err(|error| error.to_string())
			});
		},
	}
	Ok(())
}

async fn send_read_failed(
	tx: &mpsc::Sender<String>,
	grant_id: &str,
	error: &str,
) -> Result<(), SourceClientError> {
	tx.send(encode_source_frame(&SourceWorkerFrame::ReadFailed {
		grant_id: grant_id.to_owned(),
		error: error.to_owned(),
	}))
	.await
	.map_err(|_| SourceClientError::WriterClosed)
}

fn websocket_base(server: &str, suffix: &str) -> String {
	let base = server.trim_end_matches('/');
	let base = if let Some(rest) = base.strip_prefix("https://") {
		format!("wss://{rest}")
	} else if let Some(rest) = base.strip_prefix("http://") {
		format!("ws://{rest}")
	} else {
		base.to_owned()
	};
	format!("{base}{suffix}")
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::source_protocol::SourceManifestItem;

	fn item(id: usize) -> SourceManifestItem {
		SourceManifestItem {
			worker_item_id: format!("i{id}"),
			worker_content_version: "v".into(),
			relative_path: Some(format!("{id}.epub")),
			size: id as u64,
			modified_at: None,
			media_type: None,
			quick_fingerprint: None,
			sha256: None,
			metadata: None,
			retention: None,
		}
	}

	#[test]
	fn manifests_chunk_at_item_limit_with_terminal_marker() {
		let chunks = manifest_chunks("root", 4, (0..513).map(item).collect()).unwrap();
		assert_eq!(chunks.len(), 3);
		assert!(chunks.last().unwrap().terminal);
		assert!(chunks
			.iter()
			.enumerate()
			.all(|(sequence, chunk)| chunk.sequence == sequence as u64));
	}

	/// A refused tunnel grant must not stay in the in-flight set: the set
	/// exists to catch replays of grants whose transfer task is running, and
	/// a worker that refuses many stale grants must not grow without bound.
	#[tokio::test]
	async fn refused_tunnel_grants_do_not_stay_inflight() {
		use crate::source_catalog::PRIVACY_CATALOG;
		use crate::source_protocol::{parse_source_worker_frame, SourceReadMode};

		let dir = tempfile::tempdir().unwrap();
		let root_path = dir.path().join("root");
		std::fs::create_dir_all(&root_path).unwrap();
		let root = SourceRootConfig {
			root_id: "root".into(),
			label: "Root".into(),
			kind: "library".into(),
			privacy_mode: PRIVACY_CATALOG.into(),
			path: root_path,
			transport: SourceTransport::Tunnel,
			direct_base_url: None,
		};
		let catalog = Arc::new(
			SourceCatalog::open(dir.path().join("state"), vec![root.clone()]).unwrap(),
		);
		let config = SourceClientConfig {
			server: "http://127.0.0.1:1".into(),
			api_key: "secret".into(),
			name: None,
			roots: vec![root],
			state_dir: dir.path().join("state"),
			scan_interval: Duration::from_secs(60),
			listen: None,
		};
		let (tx, mut rx) = mpsc::channel(8);
		let inflight = Arc::new(Mutex::new(HashSet::new()));
		let permits = Arc::new(Semaphore::new(SOURCE_TUNNEL_TASK_LIMIT));
		let mut tasks = tokio::task::JoinSet::new();
		let grant = |id: &str| SourceReadGrant {
			grant_id: id.into(),
			root_id: "root".into(),
			worker_item_id: "missing".into(),
			worker_content_version: "v".into(),
			expected_sha256: None,
			mode: SourceReadMode::Full,
			offset: 0,
			length: 1,
			transport: SourceTransport::Tunnel,
			expires_at: std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_secs() as i64
				+ 60,
			max_bytes: 1,
		};

		for id in ["g1", "g2", "g3"] {
			dispatch_grant(
				&config,
				catalog.clone(),
				DirectGrantStore::new(),
				tx.clone(),
				inflight.clone(),
				permits.clone(),
				&mut tasks,
				grant(id),
			)
			.await
			.unwrap();
			let frame = parse_source_worker_frame(&rx.recv().await.unwrap()).unwrap();
			assert!(
				matches!(frame, SourceWorkerFrame::ReadFailed { ref grant_id, .. } if grant_id == id),
				"{frame:?}"
			);
		}
		assert!(tasks.is_empty(), "no transfer task was started");
		assert!(
			inflight.lock().is_empty(),
			"refused grants must not be retained: {:?}",
			inflight.lock()
		);
		assert_eq!(permits.available_permits(), SOURCE_TUNNEL_TASK_LIMIT);
	}
}
