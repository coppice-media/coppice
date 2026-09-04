use std::fmt::Debug;

use serde::{de, Deserialize, Serialize};

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

/// A trait to extend the output type for a job with a common interface. Job output starts
/// in an 'empty' state (Default) and is frequently updated during execution.
///
/// The state is also serialized and stored in the DB, so it must implement [Serialize] and [`de::DeserializeOwned`].
pub trait JobOutputExt: Serialize + de::DeserializeOwned + Debug {
	/// Update the state with new data. By default, the implementation is a full replacement
	fn update(&mut self, updated: Self) {
		*self = updated;
	}

	/// Serialize the state to JSON. If serialization fails, the error is logged and None is returned.
	fn into_json(self) -> Option<serde_json::Value> {
		serde_json::to_value(&self).map_or_else(
			|error| {
				tracing::error!(?error, job_data = ?self, "Failed to serialize job data!");
				None
			},
			Some,
		)
	}
}
