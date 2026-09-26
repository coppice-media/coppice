//! Komf 2.0.1 HTTP compatibility contract for Coppice's Komga lane.
//!
//! The route and DTO surface is pinned to the Komelia-consumed Komf client
//! contract; server-specific persistence and native provider access remain in
//! `stump_server`. See `crates/komf/README.md`.

mod routes;
mod types;

pub use routes::{router, KomfBackend};
pub use types::{
	KomfError, KomfEvent, KomfEventStream, KomfIdentifyRequest, KomfJob, KomfJobPage,
	KomfMediaServerConnectionResponse, KomfMediaServerLibrary, KomfMetadataJobResponse,
	KomfResult, KomfSearchResult, KomfSeriesSearchRequest,
};
