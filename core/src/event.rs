use std::collections::BTreeMap;

use crate::job::{CoreJobOutput, JobUpdate};
use serde::{Deserialize, Serialize};

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

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct JobQueueStatus {
	pub count: i32,
	pub count_by_type: BTreeMap<String, i32>,
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
}
