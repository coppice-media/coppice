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
	AnnotationSync(crate::job::annotation_sync::AnnotationSyncOutput),
	#[cfg(feature = "providers")]
	ProviderSourceHealth(ProviderSourceHealthOutput),
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
impl From<crate::job::annotation_sync::AnnotationSyncOutput> for CoreJobOutput {
	fn from(output: crate::job::annotation_sync::AnnotationSyncOutput) -> Self {
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

/// Summary of one provider source-health run.
#[cfg(feature = "providers")]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct ProviderSourceHealthOutput {
	/// Base URLs probed (one per host, not per source).
	pub probed_urls: u32,
	/// `source_health` rows written.
	pub updated_sources: u32,
	pub ok: u32,
	pub degraded: u32,
	/// Sources that reached `provider_health_dead_after` failures.
	pub dead: u32,
}

#[cfg(feature = "providers")]
impl JobOutputExt for ProviderSourceHealthOutput {
	fn update(&mut self, updated: Self) {
		self.probed_urls += updated.probed_urls;
		self.updated_sources += updated.updated_sources;
		self.ok += updated.ok;
		self.degraded += updated.degraded;
		self.dead += updated.dead;
	}
}

#[cfg(all(feature = "providers", not(feature = "graphql")))]
impl From<ProviderSourceHealthOutput> for CoreJobOutput {
	fn from(output: ProviderSourceHealthOutput) -> Self {
		Self::ProviderSourceHealth(output)
	}
}
