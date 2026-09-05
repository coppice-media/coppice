use crate::job::CoreJobOutput;
use models::shared::enums::DeviceKind;
use sea_orm::prelude::DateTimeWithTimeZone;
use serde::{Deserialize, Serialize};
use stump_devices::DeviceSeen;
use stump_jobs::{JobEvent, JobQueueStatus, JobUpdate};

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
pub struct JobStarted {
	pub id: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
pub struct JobOutput {
	pub id: String,
	pub output: CoreJobOutput,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
pub struct DiscoveredMissingLibrary {
	pub id: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct CreatedMedia {
	pub id: String,
	pub series_id: String,
	pub library_id: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct MediaDeleted {
	pub id: String,
	pub series_id: String,
	pub library_id: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct SeriesDeleted {
	pub id: String,
	pub library_id: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct CreatedManySeries {
	pub count: u64,
	pub library_id: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct CreatedOrUpdatedManyMedia {
	pub count: u64,
	pub series_id: String,
	pub library_id: String,
}

/// An unauthenticated device asked to be paired; a signed-in user has until
/// `expires_at` to approve it with the code shown on the device (or its QR).
#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct DevicePairingRequested {
	pub pairing_id: String,
	pub kind: DeviceKind,
	pub name: Option<String>,
	pub remote_ip: String,
	pub expires_at: DateTimeWithTimeZone,
}

/// An event that is emitted by the core and consumed by a client
#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::Union))]
#[serde(tag = "__typename")]
pub enum CoreEvent {
	JobStarted(JobStarted),
	JobUpdate(JobUpdate),
	JobOutput(JobOutput),
	JobQueueStatus(JobQueueStatus),
	DiscoveredMissingLibrary(DiscoveredMissingLibrary),
	CreatedMedia(CreatedMedia),
	MediaDeleted(MediaDeleted),
	SeriesDeleted(SeriesDeleted),
	CreatedManySeries(CreatedManySeries),
	CreatedOrUpdatedManyMedia(CreatedOrUpdatedManyMedia),
	/// A registered device authenticated a request; see [`stump_devices::DeviceService::touch`]
	DeviceSeen(DeviceSeen),
	/// A device started the pairing flow; see `apps/server/src/routers/api/v2/device_pairing.rs`
	DevicePairingRequested(DevicePairingRequested),
}

impl From<JobEvent<CoreJobOutput>> for CoreEvent {
	fn from(event: JobEvent<CoreJobOutput>) -> Self {
		match event {
			JobEvent::Started { id } => Self::JobStarted(JobStarted { id }),
			JobEvent::Progress(update) => Self::JobUpdate(update),
			JobEvent::Output { id, output } => Self::JobOutput(JobOutput { id, output }),
			JobEvent::QueueStatus(status) => Self::JobQueueStatus(status),
		}
	}
}
