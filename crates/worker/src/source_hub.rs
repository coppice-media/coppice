//! Server-side registry for source-worker sockets and one-use reads.
//!
//! [`SourceHub`] deliberately knows nothing about WebSockets.  The server
//! transport drains [`SourceOutbound`] and feeds source control frames back to
//! [`SourceHub::handle_frame`].  A tunnel is a separate bounded channel so
//! media bytes never share the JSON control socket.
//!
//! A grant lives in the hub only while something can still act on it.  The
//! grant's `expires_at` bounds *acceptance* (the worker's readiness frame and
//! the consumer's claim); an accepted stream is bounded by its byte budget and
//! [`SOURCE_TRANSFER_IDLE_TIMEOUT`] instead.  The consumer releases its grant:
//! `wait_ready` on observing a terminal state, `consume_direct` at
//! consumption, a [`TunnelReceiver`] on drop or idle timeout, and a finished
//! tunnel at its final frame.  A worker-driven failure only marks the grant,
//! so the route that issued it still reads the worker's reason rather than a
//! "not found"; every new registration sweeps whatever no consumer released.

use std::collections::HashMap;
use std::pin::pin;
use std::sync::{
	atomic::{AtomicU64, Ordering},
	Arc,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, FixedOffset, Utc};
use parking_lot::{Mutex, MutexGuard};
use thiserror::Error;
use tokio::sync::{mpsc, Notify, RwLock};
use uuid::Uuid;

use crate::source_protocol::{
	encode_source_frame, expires_at_millis, SourceManifestChunk, SourceProtocolError,
	SourceReadGrant, SourceReadRequest, SourceRootHello, SourceServerFrame,
	SourceTransport, SourceWorkerFrame, SourceWorkerHello,
};

/// Number of control commands a source transport may buffer before the hub
/// reports backpressure to the caller.
pub const SOURCE_OUTBOUND_CAPACITY: usize = 64;
/// Number of binary chunks buffered per pending tunnel.
pub const SOURCE_TUNNEL_CAPACITY: usize = 16;
/// Maximum size of one tunnel WebSocket binary frame.
pub const MAX_SOURCE_TUNNEL_CHUNK_BYTES: usize = 64 * 1024;
/// How long an accepted transfer may stall between chunks before it is
/// failed.  The grant expiry bounds acceptance only; a transfer that is still
/// moving bytes past `expires_at` is legitimate and is bounded by this idle
/// deadline plus its byte budget.
pub const SOURCE_TRANSFER_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// The receiving end of a source worker's bounded outbound command channel.
pub type SourceOutbound = mpsc::Receiver<String>;

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

impl GrantLifecycle {
	fn is_terminal(&self) -> bool {
		matches!(self, Self::Completed | Self::Failed(_) | Self::Expired)
	}
}

#[derive(Debug)]
struct GrantMutable {
	lifecycle: GrantLifecycle,
	tunnel_claimed: bool,
	ready_seen: bool,
	bytes_sent: u64,
	/// Last readiness frame, claim, or delivered chunk.  Bounds accepted
	/// streams, which the fixed grant expiry deliberately does not cover.
	last_activity: Instant,
}

impl GrantMutable {
	/// An accepted tunnel: the worker announced readiness and the tunnel
	/// channel is in use.  Only the idle deadline and the byte budget bound it.
	fn is_accepted_tunnel(&self) -> bool {
		self.lifecycle == GrantLifecycle::Consumed
			&& self.tunnel_claimed
			&& self.ready_seen
	}
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
				last_activity: Instant::now(),
			}),
			notify: Notify::new(),
			tunnel_tx: Mutex::new(Some(tunnel_tx)),
			tunnel_rx: Mutex::new(Some(tunnel_rx)),
		})
	}

	fn lock(&self) -> MutexGuard<'_, GrantMutable> {
		self.mutable.lock()
	}

	/// Drop the tunnel sender so the receiver observes end of stream once the
	/// buffered chunks drain and any later push is refused.
	fn close_tunnel(&self) {
		self.tunnel_tx.lock().take();
	}

	/// Fail a grant that has not reached a terminal state.  Returns whether
	/// this call performed the transition.
	fn fail(&self, error: String) -> bool {
		let transitioned = {
			let mut mutable = self.lock();
			if mutable.lifecycle.is_terminal() {
				false
			} else {
				mutable.lifecycle = GrantLifecycle::Failed(error);
				true
			}
		};
		if transitioned {
			self.close_tunnel();
			self.notify.notify_waiters();
		}
		transitioned
	}

	/// Expire a grant that has not been accepted.  Returns whether the grant is
	/// expired afterwards; an accepted tunnel and terminal grants are left
	/// alone and report `false`.
	fn expire_now(&self) -> bool {
		let outcome = {
			let mut mutable = self.lock();
			match &mutable.lifecycle {
				GrantLifecycle::Expired => return true,
				GrantLifecycle::Pending | GrantLifecycle::Ready => {},
				GrantLifecycle::Consumed
					if mutable.tunnel_claimed && !mutable.ready_seen => {},
				_ => return false,
			}
			mutable.lifecycle = GrantLifecycle::Expired;
			true
		};
		self.close_tunnel();
		self.notify.notify_waiters();
		outcome
	}

	/// Expire the grant when its wall-clock deadline has passed.
	fn expire_if_needed(&self) -> bool {
		self.grant.is_expired() && self.expire_now()
	}

	/// Whether an accepted tunnel that no consumer holds has gone longer than
	/// the idle deadline without a chunk.  A consumer holding the receiver
	/// applies its own per-chunk deadline and releases the grant on drop; a
	/// backpressured stream (channel full, reader slow) must not be mistaken
	/// for a stalled worker.
	fn is_idle(&self, now: Instant) -> bool {
		let mutable = self.lock();
		mutable.is_accepted_tunnel()
			&& self.tunnel_rx.lock().is_some()
			&& now.saturating_duration_since(mutable.last_activity)
				>= SOURCE_TRANSFER_IDLE_TIMEOUT
	}

	/// Time left until `expires_at`, zero once it has passed.
	fn remaining_ttl(&self) -> Duration {
		let now_ms = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.map(|elapsed| elapsed.as_millis() as i128)
			.unwrap_or(i128::MAX);
		let remaining = expires_at_millis(self.grant.expires_at) - now_ms;
		if remaining <= 0 {
			Duration::ZERO
		} else {
			Duration::from_millis(u64::try_from(remaining).unwrap_or(u64::MAX))
		}
	}

	fn status(&self) -> SourceReadStatus {
		match &self.lock().lifecycle {
			GrantLifecycle::Pending => SourceReadStatus::Pending,
			GrantLifecycle::Ready => SourceReadStatus::Ready,
			GrantLifecycle::Consumed => SourceReadStatus::Consumed,
			GrantLifecycle::Completed => SourceReadStatus::Completed,
			GrantLifecycle::Failed(error) => SourceReadStatus::Failed(error.clone()),
			GrantLifecycle::Expired => SourceReadStatus::Expired,
		}
	}
}

/// Every grant the hub still tracks, shared with tunnel receivers so a dropped
/// consumer releases its grant without an explicit call.
#[derive(Default)]
struct PendingRegistry {
	transfers: Mutex<HashMap<String, Arc<PendingTransfer>>>,
}

impl PendingRegistry {
	fn lock(&self) -> MutexGuard<'_, HashMap<String, Arc<PendingTransfer>>> {
		self.transfers.lock()
	}

	fn get(&self, grant_id: &str) -> Option<Arc<PendingTransfer>> {
		self.lock().get(grant_id).cloned()
	}

	fn remove(&self, grant_id: &str) {
		self.lock().remove(grant_id);
	}

	/// Drop everything nothing can act on any more: grants whose acceptance
	/// window closed, accepted tunnels that stalled, and terminal grants left
	/// behind by a consumer that never observed their final state.
	fn reap(&self) {
		let now = Instant::now();
		self.lock().retain(|_, pending| {
			if pending.expire_if_needed() {
				return false;
			}
			if pending.is_idle(now) {
				pending.fail("source transfer stalled past the idle timeout".to_owned());
				return false;
			}
			!pending.lock().lifecycle.is_terminal()
		});
	}
}

/// The consuming end of an accepted tunnel.
///
/// Each chunk is awaited under [`SOURCE_TRANSFER_IDLE_TIMEOUT`]; a stall fails
/// the grant.  Dropping the receiver releases the grant from the hub, so a
/// consumer that abandons a transfer (an HTTP client that went away) leaves
/// nothing behind and any further chunk from the worker is refused.
pub struct TunnelReceiver {
	registry: Arc<PendingRegistry>,
	pending: Arc<PendingTransfer>,
	receiver: mpsc::Receiver<Vec<u8>>,
}

impl TunnelReceiver {
	/// Wait for the next chunk.  `Ok(None)` is a clean end of stream (the
	/// worker finished or the hub closed the tunnel); [`SourceHubError::IdleTimeout`]
	/// means the worker stalled and the grant has been failed.
	pub async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, SourceHubError> {
		match tokio::time::timeout(SOURCE_TRANSFER_IDLE_TIMEOUT, self.receiver.recv())
			.await
		{
			Ok(chunk) => Ok(chunk),
			Err(_) => {
				self.pending
					.fail("source transfer stalled past the idle timeout".to_owned());
				self.registry.remove(&self.pending.grant.grant_id);
				Err(SourceHubError::IdleTimeout)
			},
		}
	}

	/// [`Self::next_chunk`] for consumers that treat a stall like end of
	/// stream and rely on their own byte-count check.
	pub async fn recv(&mut self) -> Option<Vec<u8>> {
		self.next_chunk().await.ok().flatten()
	}
}

impl Drop for TunnelReceiver {
	fn drop(&mut self) {
		self.pending
			.fail("source tunnel receiver was dropped".to_owned());
		self.registry.remove(&self.pending.grant.grant_id);
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
	#[error("source transfer stalled past the idle timeout")]
	IdleTimeout,
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
	pending: Arc<PendingRegistry>,
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
		self.fail_device_grants(&device_id, "source worker connection replaced");
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
			self.fail_device_grants(device_id, "source worker disconnected");
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
			self.fail_pending(&grant.grant_id, "source read command could not be queued");
			return Err(SourceHubError::OutboundFull);
		}
		Ok(grant)
	}

	/// Register a pending grant without sending a control command.  This is
	/// idempotent for the same device and exact grant, which lets a route split
	/// grant creation and request tracking when it needs to do so transactionally.
	///
	/// Every registration also sweeps grants nothing can act on any more, so a
	/// steady read workload keeps the registry bounded without a timer task.
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
		self.pending.reap();
		let pending = PendingTransfer::new(device_id.to_owned(), grant.clone());
		let mut transfers = self.pending.lock();
		if let Some(existing) = transfers.get(&grant.grant_id) {
			if existing.device_id == device_id && existing.grant == grant {
				return Ok(());
			}
			return Err(SourceHubError::GrantConflict);
		}
		transfers.insert(grant.grant_id.clone(), pending);
		Ok(())
	}

	/// Number of grants the hub currently tracks.
	#[must_use]
	pub fn pending_grants(&self) -> usize {
		self.pending.lock().len()
	}

	/// Wait until the worker says it has resolved and prepared the grant, or
	/// until the grant's acceptance window closes.
	pub async fn wait_ready(&self, grant_id: &str) -> Result<(), SourceHubError> {
		let pending = self.pending_transfer(grant_id)?;
		loop {
			// Register for wake-ups before reading the state: `notify_waiters`
			// only reaches futures that are already enabled, so a readiness
			// frame landing between the read and the first poll would
			// otherwise be lost.
			let mut notified = pin!(pending.notify.notified());
			notified.as_mut().enable();
			if pending.expire_if_needed() {
				self.pending.remove(grant_id);
				return Err(SourceHubError::Expired);
			}
			let (ready_seen, lifecycle) = {
				let mutable = pending.lock();
				(mutable.ready_seen, mutable.lifecycle.clone())
			};
			if ready_seen {
				return Ok(());
			}
			match lifecycle {
				GrantLifecycle::Pending
				| GrantLifecycle::Ready
				| GrantLifecycle::Consumed => {
					if tokio::time::timeout(pending.remaining_ttl(), notified)
						.await
						.is_err()
					{
						// Nothing woke us before the deadline: close the
						// acceptance window and let the next pass report it.
						pending.expire_now();
					}
				},
				GrantLifecycle::Failed(error) => {
					self.pending.remove(grant_id);
					return Err(SourceHubError::WorkerFailed(error));
				},
				GrantLifecycle::Expired => {
					self.pending.remove(grant_id);
					return Err(SourceHubError::Expired);
				},
				GrantLifecycle::Completed => {
					self.pending.remove(grant_id);
					return Err(SourceHubError::NotReady);
				},
			}
		}
	}

	/// Snapshot a grant's lifecycle without consuming it.
	pub async fn read_status(
		&self,
		grant_id: &str,
	) -> Result<SourceReadStatus, SourceHubError> {
		let pending = self.pending_transfer(grant_id)?;
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
		let pending = self.pending_transfer(grant_id)?;
		self.check_device_and_expiry(&pending, device_id)?;
		{
			let mut mutable = pending.lock();
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
			mutable.last_activity = Instant::now();
		}
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
		let pending = self.pending_transfer(grant_id)?;
		self.check_device_and_expiry(&pending, device_id)?;
		{
			let mutable = pending.lock();
			match &mutable.lifecycle {
				GrantLifecycle::Pending | GrantLifecycle::Ready => {},
				GrantLifecycle::Consumed
					if mutable.tunnel_claimed && !mutable.ready_seen => {},
				GrantLifecycle::Expired => return Err(SourceHubError::Expired),
				_ => return Err(SourceHubError::Replay),
			}
		}
		// The route that issued the grant observes the failure in `wait_ready`
		// and releases it there; evicting here would race a waiter that has not
		// looked the grant up yet and turn the worker's reason into "not found".
		pending.fail(error.into());
		Ok(())
	}

	/// Consume a direct grant once and return its exact grant details.  The
	/// hub has no further part in a direct transfer, so the grant is released
	/// here; the worker's own one-use store refuses a replayed grant id.
	pub async fn consume_direct(
		&self,
		device_id: &str,
		grant_id: &str,
	) -> Result<SourceReadGrant, SourceHubError> {
		let pending = self.pending_transfer(grant_id)?;
		self.check_device_and_expiry(&pending, device_id)?;
		if pending.grant.transport != SourceTransport::Direct {
			return Err(SourceHubError::TransportMismatch);
		}
		{
			let mut mutable = pending.lock();
			match &mutable.lifecycle {
				GrantLifecycle::Ready => mutable.lifecycle = GrantLifecycle::Consumed,
				GrantLifecycle::Pending => return Err(SourceHubError::NotReady),
				GrantLifecycle::Failed(error) => {
					return Err(SourceHubError::WorkerFailed(error.clone()))
				},
				GrantLifecycle::Expired => return Err(SourceHubError::Expired),
				_ => return Err(SourceHubError::Replay),
			}
		}
		self.pending.remove(grant_id);
		Ok(pending.grant.clone())
	}

	/// Claim the one-use bounded receiver for a tunnel grant.
	pub async fn take_pending_tunnel(
		&self,
		device_id: &str,
		grant_id: &str,
	) -> Result<TunnelReceiver, SourceHubError> {
		let pending = self.pending_transfer(grant_id)?;
		self.check_device_and_expiry(&pending, device_id)?;
		if pending.grant.transport != SourceTransport::Tunnel {
			return Err(SourceHubError::TransportMismatch);
		}
		{
			let mut mutable = pending.lock();
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
			mutable.last_activity = Instant::now();
		}
		let receiver = pending
			.tunnel_rx
			.lock()
			.take()
			.ok_or(SourceHubError::Replay)?;
		Ok(TunnelReceiver {
			registry: Arc::clone(&self.pending),
			pending,
			receiver,
		})
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
		let Ok(pending) = self.pending_transfer(grant_id) else {
			return false;
		};
		if pending.device_id != device_id
			|| pending.grant.transport != SourceTransport::Tunnel
			|| pending.grant.is_expired()
		{
			return false;
		}
		let mutable = pending.lock();
		matches!(
			mutable.lifecycle,
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
		let pending = self.pending_transfer(grant_id)?;
		self.check_device_and_expiry(&pending, device_id)?;
		if chunk.len() > MAX_SOURCE_TUNNEL_CHUNK_BYTES {
			pending.fail(format!(
				"source tunnel chunk is {} bytes; maximum is {}",
				chunk.len(),
				MAX_SOURCE_TUNNEL_CHUNK_BYTES
			));
			return Err(SourceHubError::ChunkTooLarge {
				actual: chunk.len(),
				max: MAX_SOURCE_TUNNEL_CHUNK_BYTES,
			});
		}
		let sender = {
			let mut mutable = pending.lock();
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
				drop(mutable);
				pending.fail(format!(
					"tunnel exceeded {} bytes with {} bytes",
					max, attempted
				));
				return Err(SourceHubError::ExcessBytes { attempted, max });
			}
			mutable.bytes_sent = attempted;
			mutable.last_activity = Instant::now();
			pending
				.tunnel_tx
				.lock()
				.as_ref()
				.cloned()
				.ok_or(SourceHubError::TunnelDisconnected)?
		};
		if sender.send(chunk).await.is_err() {
			pending.fail("source tunnel receiver disconnected".to_owned());
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
	/// Either way the grant is terminal and leaves the hub; the receiver still
	/// drains whatever the worker delivered before this frame.
	pub async fn finish_tunnel(
		&self,
		device_id: &str,
		grant_id: &str,
	) -> Result<(), SourceHubError> {
		let pending = self.pending_transfer(grant_id)?;
		self.check_device_and_expiry(&pending, device_id)?;
		let outcome = {
			let mut mutable = pending.lock();
			if !mutable.tunnel_claimed || mutable.lifecycle != GrantLifecycle::Consumed {
				return Err(SourceHubError::TunnelNotAccepted);
			}
			let actual = mutable.bytes_sent;
			let expected = pending.grant.length;
			if actual == expected {
				mutable.lifecycle = GrantLifecycle::Completed;
				Ok(())
			} else {
				mutable.lifecycle = GrantLifecycle::Failed(format!(
					"tunnel ended at {} bytes; expected {}",
					actual, expected
				));
				Err(SourceHubError::LengthMismatch { actual, expected })
			}
		};
		pending.close_tunnel();
		pending.notify.notify_waiters();
		self.pending.remove(grant_id);
		outcome
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

	fn pending_transfer(
		&self,
		grant_id: &str,
	) -> Result<Arc<PendingTransfer>, SourceHubError> {
		self.pending
			.get(grant_id)
			.ok_or(SourceHubError::GrantNotFound)
	}

	/// Enforce the device binding and the acceptance window.  An accepted
	/// tunnel is not expired by the wall clock; only its idle deadline and
	/// byte budget end it.
	fn check_device_and_expiry(
		&self,
		pending: &PendingTransfer,
		device_id: &str,
	) -> Result<(), SourceHubError> {
		if pending.device_id != device_id {
			return Err(SourceHubError::WrongDevice);
		}
		if pending.expire_if_needed() {
			return Err(SourceHubError::Expired);
		}
		Ok(())
	}

	/// Fail a grant whose issuer already received an error and will not wait.
	fn fail_pending(&self, grant_id: &str, error: &str) {
		if let Ok(pending) = self.pending_transfer(grant_id) {
			pending.fail(error.to_owned());
			self.pending.remove(grant_id);
		}
	}

	/// Fail every grant of a device whose socket went away.  Waiters observe
	/// the failure and release their grant; the rest is swept.
	fn fail_device_grants(&self, device_id: &str, error: &str) {
		let transfers: Vec<_> = self
			.pending
			.lock()
			.values()
			.filter(|pending| pending.device_id == device_id)
			.cloned()
			.collect();
		for pending in transfers {
			pending.fail(error.to_owned());
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::source_protocol::{SourceReadMode, SourceReadRequest};

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

	fn now_ms() -> i64 {
		SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_millis() as i64
	}

	fn request(transport: SourceTransport, length: u64) -> SourceReadRequest {
		request_expiring_in(transport, length, Duration::from_secs(60))
	}

	/// A request whose acceptance window closes after `ttl`.  Millisecond
	/// `expires_at` values keep sub-second windows exact.
	fn request_expiring_in(
		transport: SourceTransport,
		length: u64,
		ttl: Duration,
	) -> SourceReadRequest {
		SourceReadRequest {
			root_id: "root".into(),
			worker_item_id: "item".into(),
			worker_content_version: "version".into(),
			expected_sha256: None,
			mode: SourceReadMode::Range,
			offset: 0,
			length,
			transport,
			expires_at: now_ms() + ttl.as_millis() as i64,
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
		// The hub has no further part in a direct transfer: consumption
		// releases the grant, so a second consume finds nothing.
		assert!(matches!(
			hub.consume_direct("device", &grant.grant_id).await,
			Err(SourceHubError::GrantNotFound)
		));
		assert_eq!(hub.pending_grants(), 0);
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

	/// A grant is one-use: once its transfer finished, failed, or was
	/// consumed, nothing must remain in the hub for it.
	#[tokio::test]
	async fn finished_grants_leave_the_hub() {
		let hub = SourceHub::new();
		let (_outbound, _) = hub
			.attach("device", "Device", hello(SourceTransport::Tunnel))
			.await;

		// Completed tunnel, receiver drained and dropped.
		let grant = hub
			.issue_grant("device", request(SourceTransport::Tunnel, 2))
			.await
			.unwrap();
		hub.read_ready("device", &grant.grant_id).await.unwrap();
		let mut receiver = hub.accept_tunnel("device", &grant.grant_id).await.unwrap();
		hub.push_tunnel_chunk("device", &grant.grant_id, vec![1, 2])
			.await
			.unwrap();
		hub.finish_tunnel("device", &grant.grant_id).await.unwrap();
		assert_eq!(hub.pending_grants(), 0, "completion releases the grant");
		assert_eq!(receiver.recv().await, Some(vec![1, 2]));
		assert!(receiver.recv().await.is_none());
		drop(receiver);

		// Worker-failed grant: the failure is kept for the waiter, which
		// reads the worker's reason and releases the grant.
		let grant = hub
			.issue_grant("device", request(SourceTransport::Tunnel, 2))
			.await
			.unwrap();
		hub.read_failed("device", &grant.grant_id, "storage offline")
			.await
			.unwrap();
		assert_eq!(hub.pending_grants(), 1, "the reason is kept for the waiter");
		assert!(matches!(
			hub.wait_ready(&grant.grant_id).await,
			Err(SourceHubError::WorkerFailed(reason)) if reason == "storage offline"
		));
		assert_eq!(
			hub.pending_grants(),
			0,
			"the waiter releases a failed grant"
		);

		// Consumer that walked away: dropping the receiver releases the grant
		// and a late chunk from the worker is refused.
		let grant = hub
			.issue_grant("device", request(SourceTransport::Tunnel, 2))
			.await
			.unwrap();
		hub.read_ready("device", &grant.grant_id).await.unwrap();
		let receiver = hub.accept_tunnel("device", &grant.grant_id).await.unwrap();
		assert_eq!(hub.pending_grants(), 1);
		drop(receiver);
		assert_eq!(
			hub.pending_grants(),
			0,
			"a dropped receiver releases the grant"
		);
		assert!(matches!(
			hub.push_tunnel_chunk("device", &grant.grant_id, vec![1])
				.await,
			Err(SourceHubError::GrantNotFound)
		));

		// Abandoned before acceptance: swept by the next registration once
		// its window closed.
		let abandoned = hub
			.issue_grant(
				"device",
				request_expiring_in(
					SourceTransport::Tunnel,
					2,
					Duration::from_millis(50),
				),
			)
			.await
			.unwrap();
		assert_eq!(hub.pending_grants(), 1);
		tokio::time::sleep(Duration::from_millis(80)).await;
		let fresh = hub
			.issue_grant("device", request(SourceTransport::Tunnel, 2))
			.await
			.unwrap();
		assert_eq!(hub.pending_grants(), 1, "the expired grant was swept");
		assert!(matches!(
			hub.read_ready("device", &abandoned.grant_id).await,
			Err(SourceHubError::GrantNotFound)
		));
		hub.read_ready("device", &fresh.grant_id).await.unwrap();
	}

	/// A worker that never answers must not hang the route: the wait ends at
	/// the grant's own expiry and reports it.
	#[tokio::test]
	async fn wait_ready_ends_when_the_grant_expires() {
		let hub = SourceHub::new();
		let (_outbound, _) = hub
			.attach("device", "Device", hello(SourceTransport::Tunnel))
			.await;
		let grant = hub
			.issue_grant(
				"device",
				request_expiring_in(
					SourceTransport::Tunnel,
					2,
					Duration::from_millis(200),
				),
			)
			.await
			.unwrap();
		let started = Instant::now();
		let outcome =
			tokio::time::timeout(Duration::from_secs(5), hub.wait_ready(&grant.grant_id))
				.await
				.expect("wait_ready must return once the grant expires");
		assert!(
			matches!(outcome, Err(SourceHubError::Expired)),
			"{outcome:?}"
		);
		assert!(
			started.elapsed() >= Duration::from_millis(150),
			"the wait must last until the deadline, not fail early"
		);
		assert_eq!(hub.pending_grants(), 0, "an expired grant is released");
	}

	/// The readiness frame can land on another thread between the waiter's
	/// state read and its first poll; the wake-up must still be delivered.
	/// (Deterministic reproduction of that window is not possible from the
	/// outside; this bounds the hang if the ordering ever regresses.)
	#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
	async fn wait_ready_never_misses_a_readiness_frame() {
		let hub = Arc::new(SourceHub::new());
		let (mut outbound, _) = hub
			.attach("device", "Device", hello(SourceTransport::Tunnel))
			.await;
		// Drain the control commands the way the socket writer would.
		let drain = tokio::spawn(async move { while outbound.recv().await.is_some() {} });
		for _ in 0..500 {
			let grant = hub
				.issue_grant("device", request(SourceTransport::Tunnel, 1))
				.await
				.unwrap();
			let waiter = {
				let hub = Arc::clone(&hub);
				let grant_id = grant.grant_id.clone();
				tokio::spawn(async move { hub.wait_ready(&grant_id).await })
			};
			let ready = {
				let hub = Arc::clone(&hub);
				let grant_id = grant.grant_id.clone();
				tokio::spawn(async move { hub.read_ready("device", &grant_id).await })
			};
			ready.await.unwrap().unwrap();
			tokio::time::timeout(Duration::from_secs(5), waiter)
				.await
				.expect("a ready grant must wake its waiter")
				.unwrap()
				.unwrap();
			let mut receiver =
				hub.accept_tunnel("device", &grant.grant_id).await.unwrap();
			hub.push_tunnel_chunk("device", &grant.grant_id, vec![7])
				.await
				.unwrap();
			hub.finish_tunnel("device", &grant.grant_id).await.unwrap();
			assert_eq!(receiver.recv().await, Some(vec![7]));
		}
		assert_eq!(hub.pending_grants(), 0);
		drain.abort();
	}

	/// The fixed expiry bounds acceptance only.  A transfer that is still
	/// delivering bytes when `expires_at` passes keeps going to completion.
	#[tokio::test]
	async fn accepted_tunnel_outlives_the_grant_expiry() {
		let hub = SourceHub::new();
		let (_outbound, _) = hub
			.attach("device", "Device", hello(SourceTransport::Tunnel))
			.await;
		let grant = hub
			.issue_grant(
				"device",
				request_expiring_in(
					SourceTransport::Tunnel,
					4,
					Duration::from_millis(150),
				),
			)
			.await
			.unwrap();
		hub.read_ready("device", &grant.grant_id).await.unwrap();
		let mut receiver = hub.accept_tunnel("device", &grant.grant_id).await.unwrap();
		hub.push_tunnel_chunk("device", &grant.grant_id, vec![1, 2])
			.await
			.unwrap();
		tokio::time::sleep(Duration::from_millis(250)).await;
		assert!(grant.is_expired(), "the acceptance window has closed");
		hub.push_tunnel_chunk("device", &grant.grant_id, vec![3, 4])
			.await
			.expect("an accepted stream is not cut off by the acceptance deadline");
		hub.finish_tunnel("device", &grant.grant_id).await.unwrap();
		assert_eq!(receiver.recv().await, Some(vec![1, 2]));
		assert_eq!(receiver.recv().await, Some(vec![3, 4]));
		assert!(receiver.recv().await.is_none());

		// Acceptance itself still closes: a grant nobody made ready expires.
		let late = hub
			.issue_grant(
				"device",
				request_expiring_in(
					SourceTransport::Tunnel,
					1,
					Duration::from_millis(50),
				),
			)
			.await
			.unwrap();
		tokio::time::sleep(Duration::from_millis(80)).await;
		assert!(matches!(
			hub.read_ready("device", &late.grant_id).await,
			Err(SourceHubError::Expired)
		));
	}

	/// An accepted stream is bounded by the idle deadline instead: the
	/// consumer learns about a stalled worker and the grant is released.
	#[tokio::test(start_paused = true)]
	async fn stalled_tunnel_times_out_for_the_consumer() {
		let hub = SourceHub::new();
		let (_outbound, _) = hub
			.attach("device", "Device", hello(SourceTransport::Tunnel))
			.await;
		let grant = hub
			.issue_grant("device", request(SourceTransport::Tunnel, 4))
			.await
			.unwrap();
		hub.read_ready("device", &grant.grant_id).await.unwrap();
		let mut receiver = hub.accept_tunnel("device", &grant.grant_id).await.unwrap();
		hub.push_tunnel_chunk("device", &grant.grant_id, vec![1, 2])
			.await
			.unwrap();
		assert_eq!(receiver.next_chunk().await.unwrap(), Some(vec![1, 2]));
		// No further chunk arrives; the paused clock jumps to the deadline.
		assert!(matches!(
			receiver.next_chunk().await,
			Err(SourceHubError::IdleTimeout)
		));
		assert_eq!(hub.pending_grants(), 0);
		assert!(matches!(
			hub.push_tunnel_chunk("device", &grant.grant_id, vec![3, 4])
				.await,
			Err(SourceHubError::GrantNotFound)
		));
	}
}
