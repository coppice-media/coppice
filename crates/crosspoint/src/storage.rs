//! Transport-neutral durable-storage vocabulary for CrossPoint targets and
//! delivery queues. The SeaORM entities live in `stump_models`; these values
//! document what a storage/service implementation must persist.

use serde::{Deserialize, Serialize};

/// CrossPoint's verified LAN identity snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetFingerprint {
	pub model: String,
	pub serial: String,
}

impl TargetFingerprint {
	pub fn is_supported_model(&self) -> bool {
		matches!(self.model.as_str(), "X3" | "X4")
	}

	pub fn is_present(&self) -> bool {
		!self.model.is_empty() && !self.serial.is_empty()
	}
}

/// How a target's host was learned. Discovery is still followed by `/api/status`
/// verification before a row can be used for transfer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryMethod {
	#[default]
	Manual,
	Udp,
}

/// Durable queue lifecycle. Terminal rows are retained as transfer history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeliveryStatus {
	Queued,
	Preparing,
	Transferring,
	Completed,
	Failed,
	Cancelled,
}

impl DeliveryStatus {
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Queued => "QUEUED",
			Self::Preparing => "PREPARING",
			Self::Transferring => "TRANSFERRING",
			Self::Completed => "COMPLETED",
			Self::Failed => "FAILED",
			Self::Cancelled => "CANCELLED",
		}
	}

	pub const fn is_terminal(self) -> bool {
		matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
	}
}

/// A verified target snapshot returned to a management UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetSnapshot {
	pub device_id: String,
	pub user_id: String,
	pub host_or_ip: String,
	pub http_port: u16,
	pub ws_port: u16,
	pub root_path: String,
	pub discovery_method: DiscoveryMethod,
	pub verified_at: Option<String>,
	pub fingerprint: Option<TargetFingerprint>,
	pub profile_json: serde_json::Value,
	pub profile_digest: String,
	pub revoked_at: Option<String>,
}

/// Durable queue row. `source_revision` and `profile_digest` are immutable
/// idempotency inputs; `profile_json` is the normalized snapshot used after a
/// restart rather than rereading mutable device defaults.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliverySnapshot {
	pub id: String,
	pub user_id: String,
	pub device_id: String,
	pub media_id: String,
	pub source_revision: String,
	pub profile_digest: String,
	pub profile_json: serde_json::Value,
	pub destination_path: String,
	pub idempotency_key: String,
	pub status: DeliveryStatus,
	pub attempts: u32,
	pub next_attempt_at: Option<String>,
	pub last_error: Option<String>,
	pub queued_at: String,
	pub started_at: Option<String>,
	pub completed_at: Option<String>,
}

/// One attempt audit row. Attempts never overwrite one another.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryAttemptSnapshot {
	pub id: String,
	pub queue_id: String,
	pub attempt_no: u32,
	pub status: DeliveryStatus,
	pub error: Option<String>,
	pub bytes: u64,
	pub started_at: String,
	pub finished_at: Option<String>,
}

/// Fixed ports from the pinned firmware. A stored target can never redirect
/// the server's client to an arbitrary port.
pub const CROSSPOINT_HTTP_PORT: u16 = 80;
pub const CROSSPOINT_WS_PORT: u16 = 81;
pub const CROSSPOINT_DISCOVERY_PORT: u16 = 8134;
/// The firmware parses upload sizes through a signed long; callers must reject
/// larger values before opening a socket.
pub const MAX_FIRMWARE_UPLOAD_BYTES: u64 = i32::MAX as u64;
/// Application policy default; this is not an upstream firmware limit.
pub const DEFAULT_MAX_UPLOAD_BYTES: u64 = 512 * 1024 * 1024;
