use serde::{Deserialize, Serialize};

use stump_jobs::JobOutputExt;

use crate::filesystem::{
	image::{PlaceholderGenerationOutput, ThumbnailGenerationOutput},
	media::analysis::AnalyzeMediaOutput,
	metadata::MetadataFetchJobOutput,
	scanner::{LibraryScanOutput, SeriesScanOutput},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "graphql", derive(async_graphql::Union))]
#[serde(untagged, rename_all = "camelCase")]
pub enum CoreJobOutput {
	LibraryScan(LibraryScanOutput),
	SeriesScan(SeriesScanOutput),
	ThumbnailGeneration(ThumbnailGenerationOutput),
	PlaceholderGeneration(PlaceholderGenerationOutput),
	MetadataFetch(MetadataFetchJobOutput),
	AnalyzeMedia(AnalyzeMediaOutput),
	NotificationDispatch(NotificationDispatchOutput),
	AnnotationSync(crate::annotation_sync::AnnotationSyncOutput),
}

#[cfg(not(feature = "graphql"))]
impl From<LibraryScanOutput> for CoreJobOutput {
	fn from(output: LibraryScanOutput) -> Self {
		Self::LibraryScan(output)
	}
}

#[cfg(not(feature = "graphql"))]
impl From<SeriesScanOutput> for CoreJobOutput {
	fn from(output: SeriesScanOutput) -> Self {
		Self::SeriesScan(output)
	}
}

#[cfg(not(feature = "graphql"))]
impl From<ThumbnailGenerationOutput> for CoreJobOutput {
	fn from(output: ThumbnailGenerationOutput) -> Self {
		Self::ThumbnailGeneration(output)
	}
}

#[cfg(not(feature = "graphql"))]
impl From<PlaceholderGenerationOutput> for CoreJobOutput {
	fn from(output: PlaceholderGenerationOutput) -> Self {
		Self::PlaceholderGeneration(output)
	}
}

#[cfg(not(feature = "graphql"))]
impl From<MetadataFetchJobOutput> for CoreJobOutput {
	fn from(output: MetadataFetchJobOutput) -> Self {
		Self::MetadataFetch(output)
	}
}

#[cfg(not(feature = "graphql"))]
impl From<AnalyzeMediaOutput> for CoreJobOutput {
	fn from(output: AnalyzeMediaOutput) -> Self {
		Self::AnalyzeMedia(output)
	}
}

#[cfg(not(feature = "graphql"))]
impl From<crate::annotation_sync::AnnotationSyncOutput> for CoreJobOutput {
	fn from(output: crate::annotation_sync::AnnotationSyncOutput) -> Self {
		Self::AnnotationSync(output)
	}
}

/// Summary of one notification dispatch job's deliveries.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct NotificationDispatchOutput {
	/// Deliveries that succeeded (possibly after retries).
	pub sent: u32,
	/// Deliveries that exhausted their attempts and were dropped.
	pub failed: u32,
}

impl JobOutputExt for NotificationDispatchOutput {
	fn update(&mut self, updated: Self) {
		self.sent += updated.sent;
		self.failed += updated.failed;
	}
}

#[cfg(not(feature = "graphql"))]
impl From<NotificationDispatchOutput> for CoreJobOutput {
	fn from(output: NotificationDispatchOutput) -> Self {
		Self::NotificationDispatch(output)
	}
}
