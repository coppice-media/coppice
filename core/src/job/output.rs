use serde::{Deserialize, Serialize};

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
