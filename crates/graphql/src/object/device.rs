use async_graphql::{ComplexObject, Context, Result, SimpleObject};
use chrono::{DateTime, Utc};
use models::{
	entity::device,
	shared::enums::{DeviceCredentialKind, DeviceProtocol},
};
use stump_devices::{
	secret_hint, DeviceTelemetryCounters as DeviceTelemetryCountersSnapshot,
	DeviceTelemetrySnapshot, Endpoint, IssuedCredential,
};

use crate::data::CoreContext;

/// A registered client (Kobo, KOReader, Coppice, Mihon, Komelia, Liseur, an
/// OPDS reader, a script, or a browser) owned by a user.
#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct Device {
	#[graphql(flatten)]
	pub model: device::Model,
}

impl From<device::Model> for Device {
	fn from(model: device::Model) -> Self {
		Self { model }
	}
}

/// Typed, merge-safe device activity and battery telemetry.
#[derive(Debug, Clone, SimpleObject)]
pub struct DeviceTelemetry {
	pub battery_percent: Option<i32>,
	pub charging: Option<bool>,
	pub battery_source: Option<String>,
	pub battery_observed_at: Option<DateTime<Utc>>,
	pub sync_status: Option<String>,
	pub sync_protocol: Option<DeviceProtocol>,
	pub synced_at: Option<DateTime<Utc>>,
	pub counters: DeviceTelemetryCounters,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct DeviceTelemetryCounters {
	pub progress: Option<i64>,
	pub highlights: Option<i64>,
	pub notes: Option<i64>,
	pub bookmarks: Option<i64>,
	pub sessions: Option<i64>,
	pub items: Option<i64>,
}

impl From<DeviceTelemetrySnapshot> for DeviceTelemetry {
	fn from(value: DeviceTelemetrySnapshot) -> Self {
		Self {
			battery_percent: value.battery_percent,
			charging: value.charging,
			battery_source: value.battery_source,
			battery_observed_at: value.battery_observed_at,
			sync_status: value.sync_status,
			sync_protocol: value.sync_protocol,
			synced_at: value.synced_at,
			counters: DeviceTelemetryCounters::from(value.counters),
		}
	}
}

impl From<DeviceTelemetryCountersSnapshot> for DeviceTelemetryCounters {
	fn from(value: DeviceTelemetryCountersSnapshot) -> Self {
		Self {
			progress: value.progress,
			highlights: value.highlights,
			notes: value.notes,
			bookmarks: value.bookmarks,
			sessions: value.sessions,
			items: value.items,
		}
	}
}

/// The credential a device authenticates with, redacted.
#[derive(Debug, Clone, SimpleObject)]
pub struct DeviceCredentialSummary {
	pub kind: DeviceCredentialKind,
	/// The protocol the credential was minted for
	pub protocol: DeviceProtocol,
	/// The visible part of the secret, e.g. `stump_abcdefgh_…`
	pub secret_hint: String,
}

#[ComplexObject]
impl Device {
	/// The device's primary credential, or `null` once it has been revoked or
	/// when the device was registered by a protocol and never minted one.
	async fn credential(
		&self,
		ctx: &Context<'_>,
	) -> Result<Option<DeviceCredentialSummary>> {
		let core = ctx.data::<CoreContext>()?;
		Ok(core
			.devices()
			.credential(&self.model.id)
			.await?
			.map(summary))
	}

	/// Every credential currently attached to the device, redacted. Clients
	/// must select by `kind` and `protocol`; list order is not a contract.
	async fn credentials(
		&self,
		ctx: &Context<'_>,
	) -> Result<Vec<DeviceCredentialSummary>> {
		let core = ctx.data::<CoreContext>()?;
		Ok(core
			.devices()
			.credentials(&self.model.id)
			.await?
			.into_iter()
			.map(summary)
			.collect())
	}

	/// The libraries this device may see, or `null` when the device inherits
	/// its user's visibility. The scope is intersected with that visibility,
	/// so it only ever narrows; an empty list is a device that sees nothing.
	async fn library_scope(&self) -> Option<Vec<String>> {
		self.model.library_scope_ids()
	}

	/// Typed battery, sync and cumulative activity fields. It is nullable until
	/// a protocol reports its first summary.
	async fn telemetry(&self, ctx: &Context<'_>) -> Result<Option<DeviceTelemetry>> {
		let core = ctx.data::<CoreContext>()?;
		Ok(core
			.devices()
			.telemetry(&self.model.id)
			.await?
			.map(DeviceTelemetry::from))
	}
}

fn summary(
	credential: models::entity::device_credential::Model,
) -> DeviceCredentialSummary {
	DeviceCredentialSummary {
		kind: credential.credential_kind,
		protocol: credential.protocol,
		secret_hint: secret_hint(&credential),
	}
}

/// A device together with freshly minted credentials. The secrets inside
/// `credential`, `credentials`, and the endpoint URLs are shown exactly once.
#[derive(Debug, Clone, SimpleObject)]
pub struct DeviceWithCredential {
	pub device: Device,
	/// The primary credential retained for existing clients.
	pub credential: IssuedCredential,
	/// Every credential minted in this operation, including `credential`.
	pub credentials: Vec<IssuedCredential>,
	pub endpoints: Vec<Endpoint>,
}
