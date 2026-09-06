use serde::{Deserialize, Serialize};

/// Represents a specific metadata field that can be locked or configured
/// for per-field merge strategies
#[cfg_attr(feature = "graphql", derive(async_graphql::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MetadataField {
	Title,
	Summary,
	Genres,
	Tags,
	Artists,
	Publisher,
	Year,
	AgeRating,
	Cover,
	Status,
	VolumeCount,
	PageCount,
	Isbn,
	ReleaseDate,
	Colorists,
	Letterers,
	CoverArtists,
	Writers,
	Format,
	TitleSort,
	Number,
	Series,
	SeriesGroup,
	Notes,
	Language,
	Editors,
	Inkers,
	Teams,
	Links,
	Characters,
	StoryArc,
	StoryArcNumber,
	BookType,
	Imprint,
	PublicationRun,
	Pencillers,
	IdentifierAmazon,
	IdentifierCalibre,
	IdentifierGoogle,
	IdentifierMobiAsin,
	IdentifierUuid,
	ComicId,
	MetaType,
	ComicImage,
	DescriptionFormatted,
	// Appended, never inserted: a client that stores an ordinal (and every
	// generated GraphQL enum) would shift under an insertion.
	/// The readers of an audiobook edition, carried by
	/// [`crate::types::ExternalMediaMetadata::narrators`]. Distinct from
	/// [`Self::Writers`] on purpose -- a narrator is not an author.
	Narrators,
	/// An edition's secondary title, carried by
	/// [`crate::types::ExternalMediaMetadata::subtitle`].
	Subtitle,
}

/// Types of media that can be handled by metadata providers
#[cfg_attr(feature = "graphql", derive(async_graphql::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaType {
	Comic,
	Manga,
	Book,
	LightNovel,
	Manhwa,
	WebNovel,
	Webtoon,
}

#[cfg_attr(feature = "graphql", derive(async_graphql::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublicationStatus {
	Ongoing,
	Completed,
	Hiatus,
	Cancelled,
	Upcoming,
}
