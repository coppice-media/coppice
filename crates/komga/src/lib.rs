//! Komga compatibility profile: the REST/SSE subset used by Komelia, Mihon,
//! Liseur, Komf and Grimmory, served from Stump's own models.
//!
//! This crate owns the Komga DTOs, error mapping, pagination, and every
//! `/api/v1`, `/api/v2`, `/sse/v1` route handler; persistence and platform
//! operations are injected through [`routes::KomgaBackend`]. Identity,
//! settings, auth middleware and mounting stay in `apps/server`.
//!
//! See `crates/komga/README.md` for pinned clients (Komga 1.26.3, komga-client
//! 0.11.0, Komelia `65f92fde`, Mihon `21af65b`, Liseur `31f8182d`), decisions,
//! and how to verify.

pub mod errors;
pub mod routes;

pub mod announcements;
pub mod book;
pub mod collection;
pub mod common;
pub mod filesystem;
pub mod library;
pub mod read_list;
pub mod readium;
pub mod search;
pub mod series;
pub mod settings;
pub mod sse;
pub mod user;

pub use announcements::{
	Author as KomgaAnnouncementAuthor, KomgaAnnouncement, KomgaAnnouncementId,
	KomgaExtension as KomgaAnnouncementExtension, KomgaJsonFeed,
};
pub use book::{
	CopyMode, KomgaBook, KomgaBookId, KomgaBookMetadata, KomgaBookMetadataUpdateRequest,
	KomgaBookPage, KomgaBookQuery, KomgaBookReadProgressUpdateRequest, KomgaBookSearch,
	KomgaBookThumbnail, KomgaMediaStatus, KomgaReadStatus, Media, MediaProfile,
	ReadProgress,
};
pub use collection::{
	KomgaCollection, KomgaCollectionCreateRequest, KomgaCollectionId,
	KomgaCollectionQuery, KomgaCollectionThumbnail, KomgaCollectionUpdateRequest,
};
pub use common::{
	KomgaAuthor, KomgaReadingDirection, KomgaThumbnailId, KomgaWebLink, Page, Pageable,
	PatchValue, Sort,
};
pub use filesystem::{DirectoryListing, DirectoryRequest, Path};
pub use library::{
	KomgaLibrary, KomgaLibraryCreateRequest, KomgaLibraryId, KomgaLibraryUpdateRequest,
	ScanInterval, SeriesCover,
};
pub use read_list::{
	KomgaReadList, KomgaReadListCreateRequest, KomgaReadListId, KomgaReadListQuery,
	KomgaReadListThumbnail, KomgaReadListUpdateRequest,
};
pub use readium::{
	R2Device, R2Location, R2Locator, R2LocatorText, R2Positions, R2Progression,
	ReadiumDevice, ReadiumLocations, ReadiumLocator, ReadiumLocatorText,
	ReadiumPositions, ReadiumProgression,
};
pub use routes::{
	KomgaReadProgressDto, KomgaReadProgressUpdateDto, KomgaSeriesReadProgressDto,
	KomgaSeriesReadProgressUpdateDto,
};
pub use search::{
	AllOfBook, AllOfSeries, AnyOfBook, AnyOfSeries, Author, AuthorMatch, BookCondition,
	BooleanOperator, CollectionId, Date, DateOperator, Deleted, Equality,
	EqualityNullable, Genre, GreaterThan, Is, IsFalse, IsInTheLast, IsNot,
	IsNotInTheLast, IsNotNull, IsNotNullT, IsNull, IsNullT, Language, LessThan,
	LibraryId, MediaProfileCondition, MediaStatus, NumberSort, Numeric, NumericNullable,
	OneShot, Poster, PosterMatch, PosterMatchType, Publisher, ReadListId, ReadStatus,
	ReleaseDate, SeriesCondition, SeriesId, SharingLabel, StringOp, StringOperator, Tag,
	Title, TitleSort,
};
pub use series::{
	KomgaAlternativeTitle, KomgaSeries, KomgaSeriesBookMetadata, KomgaSeriesId,
	KomgaSeriesMetadata, KomgaSeriesMetadataUpdateRequest, KomgaSeriesQuery,
	KomgaSeriesSearch, KomgaSeriesStatus, KomgaSeriesThumbnail, KomgaSeriesThumbnailType,
	SearchField, SearchRegex,
};
pub use settings::{
	KomgaSettings, KomgaSettingsUpdateRequest, KomgaThumbnailSize, SettingMultiSource,
};
pub use sse::{
	BookAdded, BookChanged, BookDeleted, BookImported, CollectionAdded,
	CollectionChanged, CollectionDeleted, KomgaEvent, LibraryAdded, LibraryChanged,
	LibraryDeleted, ReadListAdded, ReadListChanged, ReadListDeleted, ReadProgressChanged,
	ReadProgressDeleted, ReadProgressSeriesChanged, ReadProgressSeriesDeleted,
	SeriesAdded, SeriesChanged, SeriesDeleted, SessionExpired, TaskQueueStatus,
	ThumbnailBookAdded, ThumbnailBookDeleted, ThumbnailReadListAdded,
	ThumbnailReadListDeleted, ThumbnailSeriesAdded, ThumbnailSeriesCollectionAdded,
	ThumbnailSeriesCollectionDeleted, ThumbnailSeriesDeleted, UnknownEvent,
};
pub use user::{
	AllowExclude, KomgaAgeRestriction, KomgaAuthenticationActivity,
	KomgaSharedLibrariesUpdate, KomgaUser, KomgaUserCreateRequest, KomgaUserId,
	KomgaUserUpdateRequest,
};
