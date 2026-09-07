//! The connected-worker registry.
//!
//! The hub knows who is connected, what each one advertises, and how to push a
//! frame at one of them. It knows nothing about WebSockets: a transport
//! [`attach`](WorkerHub::attach)es a worker with an outbound channel and feeds
//! [`WorkerJobs::handle_frame`](crate::WorkerJobs::handle_frame) the text it
//! reads. That is what lets the axum `ws` glue live in `apps/server` (where the
//! auth middleware already resolved the device) while `core` links this crate
//! without linking axum, and it is what lets the crate's own test drive the
//! same hub through a real listener.
//!
//! One device, one connection: a second `hello` from the same device id
//! replaces the first, because a worker that reconnected before the server
//! noticed the old socket die must not be offered each job twice.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, FixedOffset, Utc};
use serde_json::Value;
use tokio::sync::{mpsc, RwLock};

use crate::protocol::{encode, ServerFrame};

/// What one connected worker is and can do.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectedWorker {
	/// The `devices.id` of the paired worker device.
	pub device_id: String,
	/// The device's name, as the console shows it.
	pub name: String,
	/// The worker binary's self-reported version, when it sent one.
	pub version: Option<String>,
	/// The `hello` payload, verbatim.
	pub capabilities: Value,
	pub connected_at: DateTime<FixedOffset>,
	pub last_frame_at: DateTime<FixedOffset>,
}

struct Connection {
	worker: ConnectedWorker,
	outbound: mpsc::UnboundedSender<String>,
	/// Bumped on every attach; a stale transport detaching after it was
	/// replaced must not evict the connection that replaced it.
	epoch: u64,
}

/// The set of workers currently holding a socket.
#[derive(Default)]
pub struct WorkerHub {
	connections: RwLock<HashMap<String, Connection>>,
	epochs: std::sync::atomic::AtomicU64,
}

/// The receiving end of a worker's outbound frames. The transport owns it and
/// writes every string it yields to the socket.
pub type Outbound = mpsc::UnboundedReceiver<String>;

impl WorkerHub {
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Register a worker that just said `hello`, returning the channel its
	/// transport must drain and the attach epoch it must detach with.
	///
	/// Replaces any previous connection for the same device: dropping the old
	/// sender ends the old transport's write loop, so the stale socket closes
	/// on its own without the hub reaching into it.
	pub async fn attach(
		&self,
		device_id: String,
		name: String,
		version: Option<String>,
		capabilities: Value,
	) -> (Outbound, u64) {
		let (tx, rx) = mpsc::unbounded_channel();
		let epoch = self
			.epochs
			.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
			+ 1;
		let now = Utc::now().fixed_offset();
		let mut connections = self.connections.write().await;
		connections.insert(
			device_id.clone(),
			Connection {
				worker: ConnectedWorker {
					device_id,
					name,
					version,
					capabilities,
					connected_at: now,
					last_frame_at: now,
				},
				outbound: tx,
				epoch,
			},
		);
		(rx, epoch)
	}

	/// Remove a worker's connection, but only if it is still the one `epoch`
	/// named. Returns whether anything was removed.
	pub async fn detach(&self, device_id: &str, epoch: u64) -> bool {
		let mut connections = self.connections.write().await;
		match connections.get(device_id) {
			Some(connection) if connection.epoch == epoch => {
				connections.remove(device_id);
				true
			},
			_ => false,
		}
	}

	/// Record that `device_id` sent a frame just now.
	pub async fn touch(&self, device_id: &str) {
		if let Some(connection) = self.connections.write().await.get_mut(device_id) {
			connection.worker.last_frame_at = Utc::now().fixed_offset();
		}
	}

	/// Every connected worker, ordered by device id so the console's list does
	/// not reshuffle between polls.
	pub async fn connected(&self) -> Vec<ConnectedWorker> {
		let connections = self.connections.read().await;
		let mut workers: Vec<ConnectedWorker> = connections
			.values()
			.map(|connection| connection.worker.clone())
			.collect();
		workers.sort_by(|a, b| a.device_id.cmp(&b.device_id));
		workers
	}

	/// Whether `device_id` currently holds a socket.
	pub async fn is_connected(&self, device_id: &str) -> bool {
		self.connections.read().await.contains_key(device_id)
	}

	/// The first connected worker satisfying `requires`, or `None`.
	///
	/// "First" is by device id, which is stable but arbitrary — with one
	/// worker per capability, which is the shape every deployment this is
	/// built for has, any tie-break is the same tie-break. Least-loaded
	/// routing is a change to this function and nothing else.
	pub async fn capable_worker(&self, requires: &Value) -> Option<ConnectedWorker> {
		let connections = self.connections.read().await;
		let mut candidates: Vec<&ConnectedWorker> = connections
			.values()
			.map(|connection| &connection.worker)
			.filter(|worker| crate::kind::satisfies(&worker.capabilities, requires))
			.collect();
		candidates.sort_by(|a, b| a.device_id.cmp(&b.device_id));
		candidates.first().map(|worker| (*worker).clone())
	}

	/// Push a frame at one worker. `false` when it is not connected, which the
	/// dispatcher treats exactly as "no capable worker".
	pub async fn send(&self, device_id: &str, frame: &ServerFrame) -> bool {
		let connections = self.connections.read().await;
		connections
			.get(device_id)
			.is_some_and(|connection| connection.outbound.send(encode(frame)).is_ok())
	}
}

/// A hub shared by the service, the transport and the GraphQL layer.
pub type SharedHub = Arc<WorkerHub>;

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[tokio::test]
	async fn a_second_hello_replaces_the_first_connection() {
		let hub = WorkerHub::new();
		let (mut first, first_epoch) = hub
			.attach(
				"dev-1".into(),
				"gpu".into(),
				None,
				json!({ "transcode": {} }),
			)
			.await;
		let (mut second, second_epoch) = hub
			.attach(
				"dev-1".into(),
				"gpu".into(),
				None,
				json!({ "transcode": {} }),
			)
			.await;

		assert_eq!(hub.connected().await.len(), 1, "one device, one connection");
		// The replaced transport's channel is closed, which is how its write
		// loop learns to stop without the hub touching the socket.
		assert!(first.recv().await.is_none());

		assert!(
			hub.send(
				"dev-1",
				&ServerFrame::Cancel {
					job_id: "j1".into()
				}
			)
			.await
		);
		assert_eq!(
			second.recv().await.as_deref(),
			Some(r#"{"type":"cancel","job_id":"j1"}"#)
		);

		// The stale transport detaching must not evict the live connection.
		assert!(!hub.detach("dev-1", first_epoch).await);
		assert!(hub.is_connected("dev-1").await);
		assert!(hub.detach("dev-1", second_epoch).await);
		assert!(!hub.is_connected("dev-1").await);
	}

	#[tokio::test]
	async fn capable_worker_filters_on_the_advertised_capabilities() {
		let hub = WorkerHub::new();
		hub.attach("b-cpu".into(), "cpu".into(), None, json!({ "align": {} }))
			.await;
		hub.attach(
			"a-gpu".into(),
			"gpu".into(),
			None,
			json!({ "transcode": { "hwaccel": ["nvenc"] } }),
		)
		.await;

		let found = hub
			.capable_worker(&crate::kind::transcode_requires())
			.await
			.expect("the gpu box advertises transcode");
		assert_eq!(found.device_id, "a-gpu");
		assert!(hub.capable_worker(&json!({ "asr": true })).await.is_none());
	}
}
