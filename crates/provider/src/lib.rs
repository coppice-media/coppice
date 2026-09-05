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

pub mod cache;
pub mod catalog;
pub mod health;
pub mod host;
pub mod http;
pub mod materialize;
#[cfg(any(test, feature = "mock"))]
pub mod mock;
#[cfg(any(test, feature = "mock"))]
pub mod mock_http;
pub mod rate_limit;
pub mod source;
pub mod virtual_path;

pub use cache::{CacheKey, PageCache};
pub use catalog::{CatalogEntry, CatalogSource, SourceCatalog, SourceTheme};
pub use health::{HealthProbe, HealthStatus};
pub use host::{ProviderError, ProviderHost, ProviderHostConfig, SourceFactory, VirtualArchive};
pub use materialize::{add_series, refresh_series, Materialized};
pub use http::{SourceHttp, USER_AGENT};
pub use rate_limit::RateLimiter;
pub use source::{
	FetchedPage, RemoteChapter, RemotePage, RemoteSeries, SearchFilter,
	SeriesStatus, Source, SourceCapabilities, SourceError, SourceInfo, SourcePage,
	SourceResult,
};
pub use virtual_path::VirtualPath;
