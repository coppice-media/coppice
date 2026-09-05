use async_graphql::SimpleObject;
use models::{
	entity::device_pairing::{self, DevicePairingStatus},
	shared::enums::DeviceKind,
};
use sea_orm::prelude::DateTimeWithTimeZone;

/// A device-pairing request as seen by a signed-in user. The code hash and the
/// device's poll nonce are deliberately not exposed: knowing either would let a
/// user approve (or hijack the credential of) somebody else's device.
#[derive(Debug, Clone, SimpleObject)]
pub struct DevicePairing {
	pub id: String,
	pub kind: DeviceKind,
	/// The name the device asked for; the devices service picks a default otherwise
	pub name: Option<String>,
	/// Remote address the device started the pairing from
	pub remote_ip: String,
	/// Effective status at read time; pending rows past `expires_at` read as `EXPIRED`
	pub status: DevicePairingStatus,
	/// The user who approved or denied the pairing
	pub user_id: Option<String>,
	pub failed_attempts: i32,
	pub credential_issued: bool,
	pub created_at: DateTimeWithTimeZone,
	pub expires_at: DateTimeWithTimeZone,
	pub approved_at: Option<DateTimeWithTimeZone>,
}

impl DevicePairing {
	pub fn from_model(model: device_pairing::Model, now: DateTimeWithTimeZone) -> Self {
		Self {
			status: model.effective_status(now),
			id: model.id,
			kind: model.kind,
			name: model.name,
			remote_ip: model.remote_ip,
			user_id: model.user_id,
			failed_attempts: model.failed_attempts,
			credential_issued: model.credential_issued,
			created_at: model.created_at,
			expires_at: model.expires_at,
			approved_at: model.approved_at,
		}
	}
}
