//! Server-side registry for source-worker sockets and one-use reads.
//!
//! [`SourceHub`] deliberately knows nothing about WebSockets.  The server
//! transport drains [`SourceOutbound`] and feeds source control frames back to
//! [`SourceHub::handle_frame`].  A tunnel is a separate bounded channel so
//! media bytes never share the JSON control socket.

use std::collections::HashMap;
use std::sync::{
	atomic::{AtomicU64, Ordering},
	Arc, Mutex,
};

use chrono::{DateTime, FixedOffset, Utc};
use thiserror::Error;
use tokio::sync::{mpsc, Notify, RwLock};
use uuid::Uuid;

use crate::source_protocol::{
	encode_source_frame, SourceManifestChunk, SourceProtocolError, SourceReadGrant,
	SourceReadRequest, SourceRootHello, SourceServerFrame, SourceTransport,
	SourceWorkerFrame, SourceWorkerHello,
};

/// Number of control commands a source transport may buffer before the hub
/// reports backpressure to the caller.
pub const SOURCE_OUTBOUND_CAPACITY: usize = 64;
/// Number of binary chunks buffered per pending tunnel.
pub const SOURCE_TUNNEL_CAPACITY: usize = 16;
/// Maximum size of one tunnel WebSocket binary frame.
pub const MAX_SOURCE_TUNNEL_CHUNK_BYTES: usize = 64 * 1024;

/// The receiving end of a source worker's bounded outbound command channel.
pub type SourceOutbound = mpsc::Receiver<String>;
/// The receiving end of a bounded tunnel data channel.
pub type TunnelReceiver = mpsc::Receiver<Vec<u8>>;

/// A connected source worker and its root advertisements.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectedSourceWorker {
	pub device_id: String,
	pub name: String,
	pub version: Option<String>,
	pub roots: Vec<SourceRootHello>,
	pub connected_at: DateTime<FixedOffset>,
	pub last_frame_at: DateTime<FixedOffset>,
}

struct Connection {
	worker: ConnectedSourceWorker,
	outbound: mpsc::Sender<String>,
	epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GrantLifecycle {
	Pending,
	Ready,
	Consumed,
	Completed,
	Failed(String),
	Expired,
}

#[derive(Debug)]
struct GrantMutable {
	lifecycle: GrantLifecycle,
	tunnel_claimed: bool,
	ready_seen: bool,
	bytes_sent: u64,
}

struct PendingTransfer {
	grant: SourceReadGrant,
	device_id: String,
	mutable: Mutex<GrantMutable>,
	notify: Notify,
	tunnel_tx: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
	tunnel_rx: Mutex<Option<mpsc::Receiver<Vec<u8>>>>,
}

impl PendingTransfer {
	fn new(device_id: String, grant: SourceReadGrant) -> Arc<Self> {
		let (tunnel_tx, tunnel_rx) = mpsc::channel(SOURCE_TUNNEL_CAPACITY);
		Arc::new(Self {
			grant,
			device_id,
			mutable: Mutex::new(GrantMutable {
				lifecycle: GrantLifecycle::Pending,
				tunnel_claimed: false,
				ready_seen: false,
				bytes_sent: 0,
			}),
			notify: Notify::new(),
			tunnel_tx: Mutex::new(Some(tunnel_tx)),
			tunnel_rx: Mutex::new(Some(tunnel_rx)),
		})
	}

	fn fail(&self, error: String) {
		let mut mutable = self.mutable.lock().expect("source grant mutex poisoned");
		let should_fail = matches!(
			&mutable.lifecycle,
			GrantLifecycle::Pending | GrantLifecycle::Ready
		) || (matches!(&mutable.lifecycle, GrantLifecycle::Consumed)
			&& mutable.tunnel_claimed
			&& !mutable.ready_seen);
		if should_fail {
			mutable.lifecycle = GrantLifecycle::Failed(error);
			self.notify.notify_waiters();
		}
	}

	fn expire_if_needed(&self) -> bool {
		if !self.grant.is_expired() {
			return false;
		}
		let mut mutable = self.mutable.lock().expect("source grant mutex poisoned");
		let should_expire = matches!(
			&mutable.lifecycle,
			GrantLifecycle::Pending | GrantLifecycle::Ready
		) || (matches!(&mutable.lifecycle, GrantLifecycle::Consumed)
			&& mutable.tunnel_claimed
			&& !mutable.ready_seen);
		if should_expire {
			mutable.lifecycle = GrantLifecycle::Expired;
			self.notify.notify_waiters();
		}
		true
	}

	fn status(&self) -> SourceReadStatus {
		let mutable = self.mutable.lock().expect("source grant mutex poisoned");
		match &mutable.lifecycle {
			GrantLifecycle::Pending => SourceReadStatus::Pending,
			GrantLifecycle::Ready => SourceReadStatus::Ready,
			GrantLifecycle::Consumed => SourceReadStatus::Consumed,
			GrantLifecycle::Completed => SourceReadStatus::Completed,
			GrantLifecycle::Failed(error) => SourceReadStatus::Failed(error.clone()),
			GrantLifecycle::Expired => SourceReadStatus::Expired,
		}
	}
}

/// Public state used by a route that is waiting for the worker's readiness
/// response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceReadStatus {
	Pending,
	Ready,
	Consumed,
	Completed,
	Failed(String),
	Expired,
}

/// Errors returned by source connection and transfer operations.
#[derive(Debug, Error)]
pub enum SourceHubError {
	#[error("source worker is offline")]
	Offline,
	#[error("source worker outbound channel is full")]
	OutboundFull,
	#[error("source worker outbound channel is closed")]
	OutboundClosed,
	#[error("source root is not advertised")]
	RootNotFound,
	#[error("source root transport does not match the requested transport")]
	TransportMismatch,
	#[error("source direct root has no base URL")]
	DirectBaseUrlMissing,
	#[error("source read grant is not found")]
	GrantNotFound,
	#[error("source read grant conflicts with an existing grant")]
	GrantConflict,
	#[error("source read grant belongs to another device")]
	WrongDevice,
	#[error("source read grant was already consumed")]
	Replay,
	#[error("source read grant has expired")]
	Expired,
	#[error("source read grant is not ready")]
	NotReady,
	#[error("source worker failed the read: {0}")]
	WorkerFailed(String),
	#[error("source tunnel has not been accepted")]
	TunnelNotAccepted,
	#[error("source tunnel chunk is {actual} bytes; maximum is {max}")]
	ChunkTooLarge { actual: usize, max: usize },
	#[error(
		"source tunnel exceeds its byte budget (attempted {attempted}, maximum {max})"
	)]
	ExcessBytes { attempted: u64, max: u64 },
	#[error("source tunnel ended at {actual} bytes; expected {expected}")]
	LengthMismatch { actual: u64, expected: u64 },
	#[error("source tunnel receiver disconnected")]
	TunnelDisconnected,
	#[error("source frame is not accepted after attach")]
	UnexpectedFrame,
	#[error(transparent)]
	Protocol(#[from] SourceProtocolError),
}

/// The source-worker connection and transfer registry.
#[derive(Default)]
pub struct SourceHub {
	connections: RwLock<HashMap<String, Connection>>,
	pending: RwLock<HashMap<String, Arc<PendingTransfer>>>,
	epochs: AtomicU64,
}

/// A hub shared by the source socket, tunnel route, and reader proxy.
pub type SharedSourceHub = Arc<SourceHub>;

impl SourceHub {
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Attach one source worker, replacing any older socket for the same device.
	/// Replacing a socket invalidates its outstanding reads; the old outbound
	/// receiver observes channel closure when the old sender is dropped.
	pub async fn attach(
		&self,
		device_id: impl Into<String>,
		device_name: impl Into<String>,
		hello: SourceWorkerHello,
	) -> (SourceOutbound, u64) {
		let device_id = device_id.into();
		let fallback_name = device_name.into();
		let name = hello.name.unwrap_or(fallback_name);
		let now = Utc::now().fixed_offset();
		let (outbound, receiver) = mpsc::channel(SOURCE_OUTBOUND_CAPACITY);
		let epoch = self.epochs.fetch_add(1, Ordering::Relaxed) + 1;
		// Invalidate grants before installing the replacement, so a concurrent
		// request cannot be accidentally failed after it targets the new socket.
		self.fail_device_grants(&device_id, "source worker connection replaced")
			.await;
		let connection = Connection {
			worker: ConnectedSourceWorker {
				device_id: device_id.clone(),
				name,
				version: hello.version,
				roots: hello.roots,
				connected_at: now,
				last_frame_at: now,
			},
			outbound,
			epoch,
		};
		let mut connections = self.connections.write().await;
		connections.insert(device_id, connection);
		(receiver, epoch)
	}

	/// Attach a decoded hello frame, useful for transports that do not want to
	/// construct [`SourceWorkerHello`] themselves.
	pub async fn attach_frame(
		&self,
		device_id: impl Into<String>,
		device_name: impl Into<String>,
		frame: SourceWorkerFrame,
	) -> Result<(SourceOutbound, u64), SourceHubError> {
		let hello = frame.into_hello()?;
		Ok(self.attach(device_id, device_name, hello).await)
	}

	/// Detach only the socket named by `epoch`.
	pub async fn detach(&self, device_id: &str, epoch: u64) -> bool {
		let removed = {
			let mut connections = self.connections.write().await;
			matches!(
				connections.get(device_id),
				Some(connection) if connection.epoch == epoch
			) && connections.remove(device_id).is_some()
		};
		if removed {
			self.fail_device_grants(device_id, "source worker disconnected")
				.await;
		}
		removed
	}

	/// Record activity from a source control frame.
	pub async fn touch(&self, device_id: &str) {
		if let Some(connection) = self.connections.write().await.get_mut(device_id) {
			connection.worker.last_frame_at = Utc::now().fixed_offset();
		}
	}

	/// Return all connected source workers in stable device-id order.
	pub async fn connected(&self) -> Vec<ConnectedSourceWorker> {
		let connections = self.connections.read().await;
		let mut workers: Vec<_> = connections
			.values()
			.map(|connection| connection.worker.clone())
			.collect();
		workers.sort_by(|left, right| left.device_id.cmp(&right.device_id));
		workers
	}

	/// Return one connected source worker.
	pub async fn worker(&self, device_id: &str) -> Option<ConnectedSourceWorker> {
		self.connections
			.read()
			.await
			.get(device_id)
			.map(|connection| connection.worker.clone())
	}

	/// Whether the device currently holds a source socket.
	pub async fn is_connected(&self, device_id: &str) -> bool {
		self.connections.read().await.contains_key(device_id)
	}

	/// Whether `epoch` still names the active control socket for this device.
	pub async fn is_current(&self, device_id: &str, epoch: u64) -> bool {
		matches!(
			self.connections.read().await.get(device_id),
			Some(connection) if connection.epoch == epoch
		)
	}

	/// Look up an advertised root without exposing any worker filesystem path.
	pub async fn root(&self, device_id: &str, root_id: &str) -> Option<SourceRootHello> {
		self.worker(device_id).await.and_then(|worker| {
			worker
				.roots
				.into_iter()
				.find(|root| root.root_id == root_id)
		})
	}
	/// Alias used by route code that treats root lookup as an explicit
	/// operation rather than a property accessor.
	pub async fn lookup_root(
		&self,
		device_id: &str,
		root_id: &str,
	) -> Option<SourceRootHello> {
		self.root(device_id, root_id).await
	}

	/// Alias for the direct transport lookup.
	pub async fn lookup_direct_base_url(
		&self,
		device_id: &str,
		root_id: &str,
	) -> Option<String> {
		self.direct_base_url(device_id, root_id).await
	}

	/// Look up a direct root's advertised base URL.
	pub async fn direct_base_url(
		&self,
		device_id: &str,
		root_id: &str,
	) -> Option<String> {
		self.root(device_id, root_id).await.and_then(|root| {
			(root.transport == SourceTransport::Direct)
				.then_some(root.direct_base_url)
				.flatten()
		})
	}

	/// Try to enqueue a server command without waiting while holding the
	/// connection registry lock.  `false` means offline, closed, or full.
	pub async fn send(&self, device_id: &str, frame: &SourceServerFrame) -> bool {
		let outbound = self
			.connections
			.read()
			.await
			.get(device_id)
			.map(|connection| connection.outbound.clone());
		let Some(outbound) = outbound else {
			return false;
		};
		outbound.try_send(encode_source_frame(frame)).is_ok()
	}

	/// Enqueue a command and preserve the reason a bounded channel refused it.
	pub async fn send_wait(
		&self,
		device_id: &str,
		frame: &SourceServerFrame,
	) -> Result<(), SourceHubError> {
		let outbound = self
			.connections
			.read()
			.await
			.get(device_id)
			.map(|connection| connection.outbound.clone())
			.ok_or(SourceHubError::Offline)?;
		outbound
			.send(encode_source_frame(frame))
			.await
			.map_err(|_| SourceHubError::OutboundClosed)
	}

	/// Issue and queue one unpredictable, one-use read grant.
	pub async fn issue_grant<R>(
		&self,
		device_id: &str,
		request: R,
	) -> Result<SourceReadGrant, SourceHubError>
	where
		R: Into<SourceReadRequest>,
	{
		let request = request.into();
		request.validate()?;
		let grant = SourceReadGrant {
			grant_id: Uuid::new_v4().to_string(),
			root_id: request.root_id,
			worker_item_id: request.worker_item_id,
			worker_content_version: request.worker_content_version,
			expected_sha256: request.expected_sha256,
			mode: request.mode,
			offset: request.offset,
			length: request.length,
			transport: request.transport,
			expires_at: request.expires_at,
			max_bytes: request.max_bytes,
		};
		grant.validate()?;
		if grant.is_expired() {
			return Err(SourceHubError::Expired);
		}
		self.validate_root_for_grant(device_id, &grant).await?;
		self.register_pending_read(device_id, grant.clone()).await?;
		if !self
			.send(device_id, &SourceServerFrame::Read(grant.clone()))
			.await
		{
			self.fail_pending(&grant.grant_id, "source read command could not be queued")
				.await;
			return Err(SourceHubError::OutboundFull);
		}
		Ok(grant)
	}

	/// Register a pending grant without sending a control command.  This is
	/// idempotent for the same device and exact grant, which lets a route split
	/// grant creation and request tracking when it needs to do so transactionally.
	pub async fn register_pending_read(
		&self,
		device_id: &str,
		grant: SourceReadGrant,
	) -> Result<(), SourceHubError> {
		grant.validate()?;
		if grant.is_expired() {
			return Err(SourceHubError::Expired);
		}
		self.validate_root_for_grant(device_id, &grant).await?;
		let pending = PendingTransfer::new(device_id.to_owned(), grant.clone());
		let mut transfers = self.pending.write().await;
		if let Some(existing) = transfers.get(&grant.grant_id) {
			if existing.device_id == device_id && existing.grant == grant {
				return Ok(());
			}
			return Err(SourceHubError::GrantConflict);
		}
		transfers.insert(grant.grant_id.clone(), pending);
		Ok(())
	}

	/// Wait until the worker says it has resolved and prepared the grant.
	pub async fn wait_ready(&self, grant_id: &str) -> Result<(), SourceHubError> {
		let pending = self.pending_transfer(grant_id).await?;
		loop {
			if pending.grant.is_expired() {
				pending.expire_if_needed();
			}
			let notified = pending.notify.notified();
			let state = {
				let mutable =
					pending.mutable.lock().expect("source grant mutex poisoned");
				(mutable.ready_seen, mutable.lifecycle.clone())
			};
			if state.0 {
				return Ok(());
			}
			match state.1 {
				GrantLifecycle::Pending
				| GrantLifecycle::Ready
				| GrantLifecycle::Consumed => notified.await,
				GrantLifecycle::Failed(error) => {
					return Err(SourceHubError::WorkerFailed(error))
				},
				GrantLifecycle::Expired => return Err(SourceHubError::Expired),
				GrantLifecycle::Completed => return Err(SourceHubError::NotReady),
			}
		}
	}
	/// Snapshot a grant's lifecycle without consuming it.
	pub async fn read_status(
		&self,
		grant_id: &str,
	) -> Result<SourceReadStatus, SourceHubError> {
		let pending = self.pending_transfer(grant_id).await?;
		if pending.expire_if_needed() {
			return Ok(SourceReadStatus::Expired);
		}
		Ok(pending.status())
	}

	/// Mark a grant ready, enforcing its device binding and expiry.
	pub async fn read_ready(
		&self,
		device_id: &str,
		grant_id: &str,
	) -> Result<(), SourceHubError> {
		let pending = self.pending_transfer(grant_id).await?;
		self.check_device_and_expiry(&pending, device_id)?;
		let mut mutable = pending.mutable.lock().expect("source grant mutex poisoned");
		if mutable.ready_seen {
			return Err(SourceHubError::Replay);
		}
		match &mutable.lifecycle {
			GrantLifecycle::Pending => mutable.lifecycle = GrantLifecycle::Ready,
			GrantLifecycle::Ready => {},
			GrantLifecycle::Consumed if mutable.tunnel_claimed => {},
			GrantLifecycle::Failed(error) => {
				return Err(SourceHubError::WorkerFailed(error.clone()))
			},
			GrantLifecycle::Expired => return Err(SourceHubError::Expired),
			_ => return Err(SourceHubError::Replay),
		}
		mutable.ready_seen = true;
		pending.notify.notify_waiters();
		Ok(())
	}

	/// Mark a grant failed, enforcing its device binding and expiry.
	pub async fn read_failed(
		&self,
		device_id: &str,
		grant_id: &str,
		error: impl Into<String>,
	) -> Result<(), SourceHubError> {
		let pending = self.pending_transfer(grant_id).await?;
		self.check_device_and_expiry(&pending, device_id)?;
		let mut mutable = pending.mutable.lock().expect("source grant mutex poisoned");
		match &mutable.lifecycle {
			GrantLifecycle::Pending | GrantLifecycle::Ready => {
				mutable.lifecycle = GrantLifecycle::Failed(error.into());
				pending.notify.notify_waiters();
				Ok(())
			},
			GrantLifecycle::Consumed if mutable.tunnel_claimed && !mutable.ready_seen => {
				mutable.lifecycle = GrantLifecycle::Failed(error.into());
				pending.notify.notify_waiters();
				Ok(())
			},
			GrantLifecycle::Expired => Err(SourceHubError::Expired),
			_ => Err(SourceHubError::Replay),
		}
	}

	/// Consume a direct grant once and return its exact grant details.
	pub async fn consume_direct(
		&self,
		device_id: &str,
		grant_id: &str,
	) -> Result<SourceReadGrant, SourceHubError> {
		let pending = self.pending_transfer(grant_id).await?;
		self.check_device_and_expiry(&pending, device_id)?;
		if pending.grant.transport != SourceTransport::Direct {
			return Err(SourceHubError::TransportMismatch);
		}
		let mut mutable = pending.mutable.lock().expect("source grant mutex poisoned");
		match &mutable.lifecycle {
			GrantLifecycle::Ready => {
				mutable.lifecycle = GrantLifecycle::Consumed;
				Ok(pending.grant.clone())
			},
			GrantLifecycle::Pending => Err(SourceHubError::NotReady),
			GrantLifecycle::Failed(error) => {
				Err(SourceHubError::WorkerFailed(error.clone()))
			},
			GrantLifecycle::Expired => Err(SourceHubError::Expired),
			_ => Err(SourceHubError::Replay),
		}
	}

	/// Claim the one-use bounded receiver for a tunnel grant.
	pub async fn take_pending_tunnel(
		&self,
		device_id: &str,
		grant_id: &str,
	) -> Result<TunnelReceiver, SourceHubError> {
		let pending = self.pending_transfer(grant_id).await?;
		self.check_device_and_expiry(&pending, device_id)?;
		if pending.grant.transport != SourceTransport::Tunnel {
			return Err(SourceHubError::TransportMismatch);
		}
		{
			let mut mutable =
				pending.mutable.lock().expect("source grant mutex poisoned");
			match &mutable.lifecycle {
				GrantLifecycle::Pending | GrantLifecycle::Ready => {
					mutable.lifecycle = GrantLifecycle::Consumed;
					mutable.tunnel_claimed = true;
				},
				GrantLifecycle::Consumed if mutable.tunnel_claimed => {},
				GrantLifecycle::Failed(error) => {
					return Err(SourceHubError::WorkerFailed(error.clone()))
				},
				GrantLifecycle::Expired => return Err(SourceHubError::Expired),
				_ => return Err(SourceHubError::Replay),
			}
		}
		let receiver = pending
			.tunnel_rx
			.lock()
			.expect("source tunnel receiver mutex poisoned")
			.take()
			.ok_or(SourceHubError::Replay);
		receiver
	}

	/// Alias with the transport terminology used by the stream route.
	pub async fn accept_tunnel(
		&self,
		device_id: &str,
		grant_id: &str,
	) -> Result<TunnelReceiver, SourceHubError> {
		self.take_pending_tunnel(device_id, grant_id).await
	}

	/// Whether an authenticated device may claim a pending tunnel grant.
	pub async fn has_pending_tunnel(&self, device_id: &str, grant_id: &str) -> bool {
		let Ok(pending) = self.pending_transfer(grant_id).await else {
			return false;
		};
		if pending.device_id != device_id
			|| pending.grant.transport != SourceTransport::Tunnel
			|| pending.grant.is_expired()
		{
			return false;
		}
		let mutable = pending.mutable.lock().expect("source grant mutex poisoned");
		matches!(
			&mutable.lifecycle,
			GrantLifecycle::Pending | GrantLifecycle::Ready
		)
	}

	/// Push one bounded binary chunk into an accepted tunnel.
	pub async fn push_tunnel_chunk(
		&self,
		device_id: &str,
		grant_id: &str,
		chunk: Vec<u8>,
	) -> Result<(), SourceHubError> {
		let pending = self.pending_transfer(grant_id).await?;
		self.check_device_and_expiry(&pending, device_id)?;
		if chunk.len() > MAX_SOURCE_TUNNEL_CHUNK_BYTES {
			pending.fail(format!(
				"source tunnel chunk is {} bytes; maximum is {}",
				chunk.len(),
				MAX_SOURCE_TUNNEL_CHUNK_BYTES
			));
			pending
				.tunnel_tx
				.lock()
				.expect("source tunnel sender mutex poisoned")
				.take();
			return Err(SourceHubError::ChunkTooLarge {
				actual: chunk.len(),
				max: MAX_SOURCE_TUNNEL_CHUNK_BYTES,
			});
		}
		let sender = {
			let mut mutable =
				pending.mutable.lock().expect("source grant mutex poisoned");
			match &mutable.lifecycle {
				GrantLifecycle::Pending | GrantLifecycle::Ready => {
					mutable.lifecycle = GrantLifecycle::Consumed;
					mutable.tunnel_claimed = true;
				},
				GrantLifecycle::Consumed if mutable.tunnel_claimed => {},
				_ => return Err(SourceHubError::TunnelNotAccepted),
			}
			let attempted = mutable.bytes_sent.saturating_add(chunk.len() as u64);
			let max = pending.grant.max_bytes.min(pending.grant.length);
			if attempted > max {
				mutable.lifecycle = GrantLifecycle::Failed(format!(
					"tunnel exceeded {} bytes with {} bytes",
					max, attempted
				));
				pending.notify.notify_waiters();
				pending
					.tunnel_tx
					.lock()
					.expect("source tunnel sender mutex poisoned")
					.take();
				return Err(SourceHubError::ExcessBytes { attempted, max });
			}
			mutable.bytes_sent = attempted;
			pending
				.tunnel_tx
				.lock()
				.expect("source tunnel sender mutex poisoned")
				.as_ref()
				.cloned()
				.ok_or(SourceHubError::TunnelDisconnected)?
		};
		if sender.send(chunk).await.is_err() {
			pending.fail("source tunnel receiver disconnected".to_owned());
			pending
				.tunnel_tx
				.lock()
				.expect("source tunnel sender mutex poisoned")
				.take();
			return Err(SourceHubError::TunnelDisconnected);
		}
		Ok(())
	}

	/// Alias for route code that calls tunnel payloads data rather than chunks.
	pub async fn push_tunnel_data(
		&self,
		device_id: &str,
		grant_id: &str,
		data: Vec<u8>,
	) -> Result<(), SourceHubError> {
		self.push_tunnel_chunk(device_id, grant_id, data).await
	}

	/// Finish an accepted tunnel and require the exact authorized byte count.
	pub async fn finish_tunnel(
		&self,
		device_id: &str,
		grant_id: &str,
	) -> Result<(), SourceHubError> {
		let pending = self.pending_transfer(grant_id).await?;
		self.check_device_and_expiry(&pending, device_id)?;
		let (actual, expected, complete) = {
			let mut mutable =
				pending.mutable.lock().expect("source grant mutex poisoned");
			if !mutable.tunnel_claimed || mutable.lifecycle != GrantLifecycle::Consumed {
				return Err(SourceHubError::TunnelNotAccepted);
			}
			let actual = mutable.bytes_sent;
			let expected = pending.grant.length;
			if actual == expected {
				mutable.lifecycle = GrantLifecycle::Completed;
				pending.notify.notify_waiters();
				(actual, expected, true)
			} else {
				mutable.lifecycle = GrantLifecycle::Failed(format!(
					"tunnel ended at {} bytes; expected {}",
					actual, expected
				));
				pending.notify.notify_waiters();
				(actual, expected, false)
			}
		};
		pending
			.tunnel_tx
			.lock()
			.expect("source tunnel sender mutex poisoned")
			.take();
		if complete {
			Ok(())
		} else {
			Err(SourceHubError::LengthMismatch { actual, expected })
		}
	}

	/// Handle one decoded source control frame.  Manifest chunks are returned to
	/// the server for persistence; readiness/failure frames are consumed here.
	pub async fn handle_frame(
		&self,
		device_id: &str,
		frame: SourceWorkerFrame,
	) -> Result<Option<SourceManifestChunk>, SourceHubError> {
		self.touch(device_id).await;
		match frame {
			SourceWorkerFrame::Hello { .. } => Err(SourceHubError::UnexpectedFrame),
			SourceWorkerFrame::ManifestChunk(chunk) => {
				chunk.validate()?;
				Ok(Some(chunk))
			},
			SourceWorkerFrame::ReadReady { grant_id } => {
				self.read_ready(device_id, &grant_id).await?;
				Ok(None)
			},
			SourceWorkerFrame::ReadFailed { grant_id, error } => {
				self.read_failed(device_id, &grant_id, error).await?;
				Ok(None)
			},
		}
	}

	async fn validate_root_for_grant(
		&self,
		device_id: &str,
		grant: &SourceReadGrant,
	) -> Result<(), SourceHubError> {
		let connected = self.is_connected(device_id).await;
		let root = self.root(device_id, &grant.root_id).await;
		let root = match root {
			Some(root) => root,
			None if connected => return Err(SourceHubError::RootNotFound),
			None => return Err(SourceHubError::Offline),
		};
		if root.transport != grant.transport {
			return Err(SourceHubError::TransportMismatch);
		}
		if grant.transport == SourceTransport::Direct && root.direct_base_url.is_none() {
			return Err(SourceHubError::DirectBaseUrlMissing);
		}
		Ok(())
	}

	async fn pending_transfer(
		&self,
		grant_id: &str,
	) -> Result<Arc<PendingTransfer>, SourceHubError> {
		self.pending
			.read()
			.await
			.get(grant_id)
			.cloned()
			.ok_or(SourceHubError::GrantNotFound)
	}

	fn check_device_and_expiry(
		&self,
		pending: &PendingTransfer,
		device_id: &str,
	) -> Result<(), SourceHubError> {
		if pending.device_id != device_id {
			return Err(SourceHubError::WrongDevice);
		}
		if pending.expire_if_needed() {
			pending
				.tunnel_tx
				.lock()
				.expect("source tunnel sender mutex poisoned")
				.take();
			return Err(SourceHubError::Expired);
		}
		Ok(())
	}

	async fn fail_pending(&self, grant_id: &str, error: &str) {
		if let Ok(pending) = self.pending_transfer(grant_id).await {
			pending.fail(error.to_owned());
			pending
				.tunnel_tx
				.lock()
				.expect("source tunnel sender mutex poisoned")
				.take();
		}
	}

	async fn fail_device_grants(&self, device_id: &str, error: &str) {
		let transfers: Vec<_> = self
			.pending
			.read()
			.await
			.values()
			.filter(|pending| pending.device_id == device_id)
			.cloned()
			.collect();
		for pending in transfers {
			pending.fail(error.to_owned());
			pending
				.tunnel_tx
				.lock()
				.expect("source tunnel sender mutex poisoned")
				.take();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::source_protocol::{SourceReadMode, SourceReadRequest};
	use std::time::SystemTime;

	fn hello(transport: SourceTransport) -> SourceWorkerHello {
		SourceWorkerHello {
			name: Some("home".into()),
			version: Some("1".into()),
			roots: vec![SourceRootHello {
				root_id: "root".into(),
				label: "Books".into(),
				kind: "folder".into(),
				privacy_mode: "catalog".into(),
				transport,
				direct_base_url: (transport == SourceTransport::Direct)
					.then(|| "http://127.0.0.1:1234".into()),
			}],
		}
	}

	fn request(transport: SourceTransport, length: u64) -> SourceReadRequest {
		SourceReadRequest {
			root_id: "root".into(),
			worker_item_id: "item".into(),
			worker_content_version: "version".into(),
			expected_sha256: None,
			mode: SourceReadMode::Range,
			offset: 0,
			length,
			transport,
			expires_at: (SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_secs() as i64)
				+ 60,
			max_bytes: length,
		}
	}

	#[tokio::test]
	async fn grant_is_bound_to_device_and_consumed_once() {
		let hub = SourceHub::new();
		let (_outbound, _) = hub
			.attach("device", "Device", hello(SourceTransport::Direct))
			.await;
		let grant = hub
			.issue_grant("device", request(SourceTransport::Direct, 4))
			.await
			.unwrap();
		hub.read_ready("device", &grant.grant_id).await.unwrap();
		assert!(matches!(
			hub.consume_direct("other", &grant.grant_id).await,
			Err(SourceHubError::WrongDevice)
		));
		hub.consume_direct("device", &grant.grant_id).await.unwrap();
		assert!(matches!(
			hub.consume_direct("device", &grant.grant_id).await,
			Err(SourceHubError::Replay)
		));
	}

	#[tokio::test]
	async fn tunnel_rejects_excess_and_wrong_device() {
		let hub = SourceHub::new();
		let (_outbound, _) = hub
			.attach("device", "Device", hello(SourceTransport::Tunnel))
			.await;
		let grant = hub
			.issue_grant("device", request(SourceTransport::Tunnel, 3))
			.await
			.unwrap();
		hub.read_ready("device", &grant.grant_id).await.unwrap();
		assert!(matches!(
			hub.accept_tunnel("other", &grant.grant_id).await,
			Err(SourceHubError::WrongDevice)
		));
		let mut receiver = hub.accept_tunnel("device", &grant.grant_id).await.unwrap();
		hub.push_tunnel_chunk("device", &grant.grant_id, vec![1, 2, 3, 4])
			.await
			.unwrap_err();
		assert!(receiver.recv().await.is_none());
	}

	#[tokio::test]
	async fn tunnel_delivers_exact_bytes_and_closes() {
		let hub = SourceHub::new();
		let (_outbound, _) = hub
			.attach("device", "Device", hello(SourceTransport::Tunnel))
			.await;
		let grant = hub
			.issue_grant("device", request(SourceTransport::Tunnel, 5))
			.await
			.unwrap();
		hub.read_ready("device", &grant.grant_id).await.unwrap();
		let mut receiver = hub.accept_tunnel("device", &grant.grant_id).await.unwrap();
		hub.push_tunnel_chunk("device", &grant.grant_id, b"hello".to_vec())
			.await
			.unwrap();
		hub.finish_tunnel("device", &grant.grant_id).await.unwrap();
		assert_eq!(receiver.recv().await.as_deref(), Some(b"hello".as_slice()));
		assert!(receiver.recv().await.is_none());
	}
}
