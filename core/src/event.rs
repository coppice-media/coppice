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
/// A device pairing completed and its credential was minted for `user_id`.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct DevicePaired {
	pub device_id: String,
	pub user_id: String,
	pub device_name: String,
}

/// A staged ingest item finished analysis and is waiting for a decision.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct IngestAwaitingReview {
	pub library_id: String,
	pub drop_item_id: String,
	pub source_filename: String,
	pub created_by: Option<String>,
}

/// A quality report contains at least one failed check.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct QualityFailed {
	pub library_id: String,
	/// Set for staged items; media-rework reports carry `media_id` instead.
	pub drop_item_id: Option<String>,
	pub media_id: Option<String>,
	pub created_by: Option<String>,
	pub score: u8,
	/// The check ids that returned [`QualityStatus::Fail`](crate::ingest::contract::QualityStatus).
	pub failed_checks: Vec<String>,
}

/// A staged analysis job failed.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct AnalysisJobFailed {
	pub analysis_job_id: String,
	pub error: String,
}

/// Provider identify/lookup finished for a staged item and candidates were
/// stored.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct ProviderMatchDone {
	pub library_id: String,
	pub drop_item_id: String,
	pub created_by: Option<String>,
	pub candidate_count: usize,
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
	/// A device pairing completed and the credential was minted
	DevicePaired(DevicePaired),
	/// A staged ingest item finished analysis and needs a human decision
	IngestAwaitingReview(IngestAwaitingReview),
	/// A quality report contains at least one failed check
	QualityFailed(QualityFailed),
	/// A staged analysis job failed
	AnalysisJobFailed(AnalysisJobFailed),
	/// Provider lookup finished for a staged item
	ProviderMatchDone(ProviderMatchDone),
	/// A shelf collection was created (any protocol, incl. Kobo write-back)
	CollectionAdded(CollectionAdded),
	/// A shelf collection's name, ordering, or membership changed
	CollectionChanged(CollectionChanged),
	/// A shelf collection was deleted
	CollectionDeleted(CollectionDeleted),
	/// A reading list was created (any protocol, incl. Kobo write-back)
	ReadListAdded(ReadListAdded),
	/// A reading list's name, ordering, or membership changed
	ReadListChanged(ReadListChanged),
	/// A reading list was deleted
	ReadListDeleted(ReadListDeleted),
	/// A library was created; the shared library service emits it after commit
	LibraryCreated(LibraryCreated),
	/// A library's name, path, or configuration changed
	LibraryUpdated(LibraryUpdated),
	/// A library (and all of its series/media) was deleted
	LibraryDeleted(LibraryDeleted),
}

/// Container events share one payload shape per container kind: the id plus
/// the visible member ids, so protocol adapters can gate visibility and echo
/// the Komga wire events without re-querying.

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct CollectionAdded {
	pub id: String,
	pub series_ids: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct CollectionChanged {
	pub id: String,
	pub series_ids: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct CollectionDeleted {
	pub id: String,
	pub series_ids: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct ReadListAdded {
	pub id: String,
	pub book_ids: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct ReadListChanged {
	pub id: String,
	pub book_ids: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct ReadListDeleted {
	pub id: String,
	pub book_ids: Vec<String>,
}
/// Library lifecycle events carry the identity triple the Komga SSE surface
/// needs to announce changes (payload keeps the root path so protocol adapters
/// can log or resolve the library without re-querying).

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct LibraryCreated {
	pub id: String,
	pub name: String,
	pub path: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct LibraryUpdated {
	pub id: String,
	pub name: String,
	pub path: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct LibraryDeleted {
	pub id: String,
	pub name: String,
	pub path: String,
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
