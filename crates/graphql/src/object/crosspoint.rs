//! GraphQL objects for CrossPoint target verification and durable deliveries.

use async_graphql::{Enum, Json, SimpleObject, ID};
use models::entity::{crosspoint_delivery_queue, crosspoint_device_target};
use serde_json::Value;

use stump_crosspoint::{
	profile::CrosspointTransferProfile as ProtocolTransferProfile,
	storage::{DeliveryStatus, TargetFingerprint},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Enum)]
pub enum CrosspointTargetModel {
	Auto,
	X3,
	X4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Enum)]
pub enum CrosspointDeliveryStatus {
	Queued,
	Preparing,
	Transferring,
	Completed,
	Failed,
	Cancelled,
}

#[derive(Clone, Debug, SimpleObject)]
pub struct CrosspointTargetFingerprint {
	pub model: String,
	pub serial: String,
}

#[derive(Clone, Debug, SimpleObject)]
pub struct CrosspointTransferProfile {
	pub optimizer_enabled: bool,
	pub target_model: CrosspointTargetModel,
	pub jpeg_quality: i32,
	pub grayscale: bool,
	pub auto_crop: bool,
	pub split_large_paragraphs: bool,
	pub remove_fonts: bool,
	pub chunk_bytes: i32,
	pub retry_count: i32,
	pub retry_delay_seconds: i64,
	pub timeout_seconds: i64,
	pub max_upload_bytes: i64,
}

#[derive(Clone, Debug, SimpleObject)]
pub struct CrosspointTarget {
	pub device_id: ID,
	pub user_id: ID,
	pub host_or_ip: String,
	pub http_port: i32,
	pub ws_port: i32,
	pub root_path: String,
	pub discovery_method: String,
	pub verified_at: Option<String>,
	pub fingerprint: Option<CrosspointTargetFingerprint>,
	pub profile: CrosspointTransferProfile,
	pub profile_json: Json<Value>,
	pub profile_digest: String,
	pub revoked_at: Option<String>,
}

#[derive(Clone, Debug, SimpleObject)]
pub struct CrosspointTargetVerification {
	pub device_id: ID,
	pub host_or_ip: String,
	pub http_port: i32,
	pub ws_port: i32,
	pub model: String,
	pub serial: String,
	pub verified: bool,
}

#[derive(Clone, Debug, SimpleObject)]
pub struct CrosspointDelivery {
	pub id: ID,
	pub user_id: ID,
	pub device_id: ID,
	pub media_id: ID,
	pub source_revision: String,
	pub profile_digest: String,
	pub profile_json: String,
	pub destination_path: String,
	pub idempotency_key: String,
	pub status: CrosspointDeliveryStatus,
	pub attempts: i32,
	pub max_attempts: i32,
	pub next_attempt_at: Option<String>,
	pub last_error: Option<String>,
	pub queued_at: String,
	pub started_at: Option<String>,
	pub completed_at: Option<String>,
	pub updated_at: String,
}

#[derive(Clone, Debug, SimpleObject)]
pub struct CrosspointDeliveryAttempt {
	pub id: ID,
	pub queue_id: ID,
	pub attempt_no: i32,
	pub status: CrosspointDeliveryStatus,
	pub error: Option<String>,
	pub bytes: i64,
	pub started_at: String,
	pub finished_at: Option<String>,
}

impl From<stump_crosspoint::profile::CrosspointTargetModel> for CrosspointTargetModel {
	fn from(value: stump_crosspoint::profile::CrosspointTargetModel) -> Self {
		match value {
			stump_crosspoint::profile::CrosspointTargetModel::Auto => Self::Auto,
			stump_crosspoint::profile::CrosspointTargetModel::X3 => Self::X3,
			stump_crosspoint::profile::CrosspointTargetModel::X4 => Self::X4,
		}
	}
}

impl From<DeliveryStatus> for CrosspointDeliveryStatus {
	fn from(value: DeliveryStatus) -> Self {
		match value {
			stump_crosspoint::storage::DeliveryStatus::Queued => Self::Queued,
			stump_crosspoint::storage::DeliveryStatus::Preparing => Self::Preparing,
			stump_crosspoint::storage::DeliveryStatus::Transferring => Self::Transferring,
			stump_crosspoint::storage::DeliveryStatus::Completed => Self::Completed,
			stump_crosspoint::storage::DeliveryStatus::Failed => Self::Failed,
			stump_crosspoint::storage::DeliveryStatus::Cancelled => Self::Cancelled,
		}
	}
}

impl From<&str> for CrosspointDeliveryStatus {
	fn from(value: &str) -> Self {
		match value {
			"PREPARING" => Self::Preparing,
			"TRANSFERRING" => Self::Transferring,
			"COMPLETED" => Self::Completed,
			"FAILED" => Self::Failed,
			"CANCELLED" => Self::Cancelled,
			_ => Self::Queued,
		}
	}
}
impl From<ProtocolTransferProfile> for CrosspointTransferProfile {
	fn from(value: ProtocolTransferProfile) -> Self {
		Self {
			optimizer_enabled: value.optimizer_enabled,
			target_model: value.target_model.into(),
			jpeg_quality: value.jpeg_quality as i32,
			grayscale: value.grayscale,
			auto_crop: value.auto_crop,
			split_large_paragraphs: value.split_large_paragraphs,
			remove_fonts: value.remove_fonts,
			chunk_bytes: value.chunk_bytes as i32,
			retry_count: value.retry_count as i32,
			retry_delay_seconds: value.retry_delay_seconds as i64,
			timeout_seconds: value.timeout_seconds as i64,
			max_upload_bytes: value.max_upload_bytes as i64,
		}
	}
}

impl CrosspointTarget {
	/// Convert a persisted target only after validating its typed profile.
	/// Corrupt or legacy rows must not be presented as an apparently usable
	/// target to a caller.
	pub fn from_model(value: crosspoint_device_target::Model) -> Result<Self, String> {
		let profile: ProtocolTransferProfile =
			serde_json::from_value(value.profile_json.clone())
				.map_err(|error| format!("invalid CrossPoint target profile: {error}"))?;
		let profile = profile
			.validate()
			.map_err(|error| format!("invalid CrossPoint target profile: {error}"))?;
		if profile.digest() != value.profile_digest {
			return Err(
				"CrossPoint target profile digest does not match its JSON".to_string()
			);
		}
		let fingerprint = match value.fingerprint {
			Some(value) => {
				let fingerprint: TargetFingerprint = serde_json::from_value(value)
					.map_err(|error| {
						format!("invalid CrossPoint target fingerprint: {error}")
					})?;
				Some(CrosspointTargetFingerprint {
					model: fingerprint.model,
					serial: fingerprint.serial,
				})
			},
			None => None,
		};

		Ok(Self {
			device_id: ID(value.device_id),
			user_id: ID(value.user_id),
			host_or_ip: value.host_or_ip,
			http_port: value.http_port,
			ws_port: value.ws_port,
			root_path: value.root_path,
			discovery_method: value.discovery_method,
			verified_at: value.verified_at.map(|value| value.to_rfc3339()),
			fingerprint,
			profile: profile.into(),
			profile_json: Json(value.profile_json),
			profile_digest: value.profile_digest,
			revoked_at: value.revoked_at.map(|value| value.to_rfc3339()),
		})
	}
}

impl From<crosspoint_delivery_queue::Model> for CrosspointDelivery {
	fn from(value: crosspoint_delivery_queue::Model) -> Self {
		Self {
			id: ID(value.id),
			user_id: ID(value.user_id),
			device_id: ID(value.device_id),
			media_id: ID(value.media_id),
			source_revision: value.source_revision,
			profile_digest: value.profile_digest,
			profile_json: value.profile_json,
			destination_path: value.destination_path,
			idempotency_key: value.idempotency_key,
			status: CrosspointDeliveryStatus::from(value.status.as_str()),
			attempts: value.attempts,
			max_attempts: value.max_attempts,
			next_attempt_at: value.next_attempt_at.map(|value| value.to_rfc3339()),
			last_error: value.last_error,
			queued_at: value.queued_at.to_rfc3339(),
			started_at: value.started_at.map(|value| value.to_rfc3339()),
			completed_at: value.completed_at.map(|value| value.to_rfc3339()),
			updated_at: value.updated_at.to_rfc3339(),
		}
	}
}
