use models::shared::enums::DeviceProtocol;
use serde::{Deserialize, Serialize};

/// Emitted when a device credential authenticates a request and the device's
/// last-seen state was updated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct DeviceSeen {
	pub device_id: String,
	pub user_id: String,
	/// The protocol the request arrived on, which may differ from the protocol
	/// the credential was minted for (one key works on every path).
	pub protocol: DeviceProtocol,
}
