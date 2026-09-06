//! Remote source host for Stump.
//!
//! `stump_provider` mirrors Mihon's source model: a [`Source`] can browse,
//! search, describe a series, list its chapters, list a chapter's pages, and
//! download page bytes. Nothing but MangaDex is compiled in; the
//! [`catalog::SourceCatalog`] lists the Keiyoushi extension index at runtime so
//! operators can see which sources exist and how healthy they are.
//!
//! Remote series are materialised into ordinary `series`/`media` rows tagged
//! with `source_provider`/`remote_id`/`remote_chapter_id` and a
//! [`virtual_path::VirtualPath`] instead of a file, so every existing route
//! (native, Komga, OPDS, Kobo) serves them unchanged. Pages flow through the
//! bounded on-disk [`cache::PageCache`] and are resolved by the
//! [`host::ProviderHost`], which implements
//! [`stump_media::virtual_media::VirtualMediaResolver`].
//!
//! Virtual libraries ("Mode B", [`browse`], [`virtual_library`], [`gc`])
//! browse a source live under deterministic ids and materialise lazily.
//! Design decisions and verification steps: `crates/provider/README.md`.

pub mod browse;
pub mod cache;
pub mod catalog;
pub mod definition;
pub mod event;
pub mod gc;
pub mod health;
pub mod host;
pub mod http;
pub mod identity;
pub mod materialize;
#[cfg(any(test, feature = "mock"))]
pub mod mock;
#[cfg(any(test, feature = "mock"))]
pub mod mock_http;
pub mod rate_limit;
pub mod source;
pub mod virtual_library;
pub mod virtual_path;

pub use browse::{BrowseKind, RemoteOrigin, VirtualBrowseCache};
pub use cache::{CacheKey, PageCache};
pub use catalog::{CatalogEntry, CatalogSource, SourceCatalog, SourceTheme};
pub use definition::{
	DefinitionEngine, DefinitionError, DefinitionIndex, DefinitionIndexEntry,
	DefinitionLoader, KnobValue, SourceDefinition,
};
pub use event::{ProviderEvent, ProviderEventSink};
pub use gc::{gc_materialised_series, GcReport};
pub use health::{HealthProbe, HealthStatus, HealthTarget};
pub use host::{
	ProviderError, ProviderHost, ProviderHostConfig, SourceFactory, VirtualArchive,
};
pub use http::{
	challenge_host, HeaderError, MaskedHeader, RequestHeaders, SourceHttp, USER_AGENT,
};
pub use identity::{merge_series, normalise_title, MergeReport, SeriesDuplicate};
pub use materialize::{add_series, refresh_series, Materialized, SkippedChapter};
pub use rate_limit::RateLimiter;
pub use source::{
	ContentRating, FetchedPage, HtmlDocument, RemoteChapter, RemotePage, RemoteSeries,
	SearchFilter, SeriesStatus, Source, SourceCapabilities, SourceError, SourceInfo,
	SourcePage, SourceResult,
};
pub use virtual_library::create_virtual_library;
pub use virtual_path::VirtualPath;
