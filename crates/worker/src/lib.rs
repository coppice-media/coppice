//! Remote compute workers plus root-scoped source inventory and byte serving.
//!
//! The server stays one Rust process. Work that is heavy or hardware-bound is a
//! row in `worker_jobs` that a process somewhere else may claim, and workers
//! dial *out* to the server, so a VPS never needs a route back to the machine
//! with the GPU (or, for `transcode`, the machine with `ffmpeg`).
//!
//! The crate has two halves behind two features, and one shared protocol:
//!
//! | Feature | Who links it | What it is |
//! | --- | --- | --- |
//! | `server` | `stump_core` → `apps/server`, `graphql` | [`WorkerJobs`] / [`WorkerHub`] for compute and [`SourceHub`] for separately authenticated source connections and read grants |
//! | `client` | the `stump-worker` binary | Compute runners plus source catalog/scanner/direct/tunnel clients selected by separate CLI credentials |
//! | `browser` | the same binary, opted in | [`challenge`]: `challenge_solve` in a real, visible Chrome. Default off — a CDP stack is a lot to compile for an operator who only wants Opus |
//! | — | both | Compute [`protocol`] and source [`source_protocol`] frames remain separate; [`kind`] and [`alignment`] own compute-job contracts |
//!
//! The server half deliberately links no HTTP stack. The hub is fed frames and
//! hands back an outbound channel, so the axum `ws` glue lives in
//! `apps/server/src/routers/api/v2/workers.rs`, where the auth middleware has
//! already resolved the device — and `stump_core` links this crate without
//! linking axum.
//!
//! Decisions, layout, and verification commands: `crates/worker/README.md`.
//! Protocol references: `docs/content/docs/developer/workers.mdx` and
//! `docs/content/docs/developer/remote-worker-libraries.mdx`.

pub mod alignment;
pub mod kind;
pub mod protocol;
pub mod source_protocol;

/// Alignment job and SyncMap contracts are shared by server and workers.
pub use alignment::{
	validate_sync_map, validate_sync_map_for_input, AlignExecutionProvider,
	AlignGranularity, AlignInput, AlignPrecision, AlignResult, AudioClip, SyncCue,
	SyncMapProvenance, SyncMapV1, SyncMapValidationContext, SyncMapValidationError,
	TextFragment, TrackDurationsMs, SYNC_MAP_MIME, SYNC_MAP_SCHEMA_VERSION,
};

/// The `worker_jobs` row. Owned by `models` like every other entity; re-exported
/// here because this crate is the only thing that should write it.
#[cfg(feature = "server")]
pub use models::entity::worker_job as entity;
#[cfg(feature = "server")]
pub mod error;
#[cfg(feature = "server")]
pub mod event;
#[cfg(feature = "server")]
pub mod hub;
#[cfg(feature = "server")]
pub mod service;
#[cfg(feature = "server")]
pub mod source_hub;

#[cfg(feature = "browser")]
pub mod challenge;
#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub mod native_align;
#[cfg(feature = "client")]
pub mod source_calibre;
#[cfg(feature = "client")]
pub mod source_catalog;
#[cfg(feature = "client")]
pub mod source_client;
#[cfg(feature = "client")]
pub mod source_direct;
#[cfg(feature = "client")]
pub mod source_scanner;
#[cfg(feature = "client")]
pub mod source_tunnel;
#[cfg(feature = "client")]
pub mod storyteller;
#[cfg(all(test, feature = "server", feature = "client"))]
mod tests;
#[cfg(feature = "client")]
pub mod transcode;

pub use kind::{
	align_requires, challenge_solve_requires, satisfies, transcode_requires,
	ChallengeSolveInput, ChallengeSolveOutput, TranscodeInput, TranscodeOutput,
	TranscodeResult, ALIGN, BROWSER, CHALLENGE_SOLVE, CLEARANCE_COOKIE, TRANSCODE,
};

#[cfg(feature = "server")]
pub use error::{WorkerError, WorkerResult};
#[cfg(feature = "server")]
pub use event::{ChangeListener, WorkerJobChanged};
#[cfg(feature = "server")]
pub use hub::{ConnectedWorker, Outbound, SharedHub, WorkerHub};
#[cfg(feature = "server")]
pub use kind::{KindRegistry, LocalJob, LocalRunner};
#[cfg(feature = "server")]
pub use models::entity::worker_job::Model as WorkerJob;
#[cfg(feature = "server")]
pub use models::shared::enums::WorkerJobStatus;
#[cfg(feature = "server")]
pub use service::{
	JobOutcome, ResultValidator, WorkerJobs, BACKGROUND_PRIORITY, CLAIM_DEADLINE,
	INTERACTIVE_PRIORITY,
};
#[cfg(feature = "server")]
pub use source_hub::{
	ConnectedSourceWorker, SharedSourceHub, SourceHub, SourceHubError, SourceOutbound,
	SourceReadStatus, TunnelReceiver, MAX_SOURCE_TUNNEL_CHUNK_BYTES,
	SOURCE_OUTBOUND_CAPACITY, SOURCE_TRANSFER_IDLE_TIMEOUT, SOURCE_TUNNEL_CAPACITY,
};
pub use source_protocol::{
	encode_source_frame, expires_at_millis, parse_source_server_frame,
	parse_source_worker_frame, SourceManifestChunk, SourceManifestItem,
	SourceProtocolError, SourceReadGrant, SourceReadMode, SourceReadRequest,
	SourceRootHello, SourceServerFrame, SourceTransport, SourceWorkerFrame,
	SourceWorkerHello, MAX_MANIFEST_FRAME_BYTES, MAX_MANIFEST_ITEMS,
	MAX_SOURCE_MANIFEST_FRAME_BYTES, MAX_SOURCE_MANIFEST_ITEMS,
};

#[cfg(feature = "client")]
pub use client::{Assignment, ClientConfig, JobRunner, Progress};
#[cfg(feature = "client")]
pub use native_align::{NativeAlignConfig, NativeAlignRunner};
#[cfg(feature = "client")]
pub use source_calibre::{
	discover as discover_calibre, CalibreBook, CalibreFormat, CalibreSourceError,
	CALIBRE_APPLICATION_ID, CALIBRE_ROOT_KIND, SUPPORTED_SCHEMA_VERSION,
};
#[cfg(feature = "client")]
pub use source_catalog::{
	parse_source_root_config, parse_source_root_spec, sanitize_relative_path,
	CatalogEntry, CatalogSnapshot, ResolvedSourceItem, SourceCatalog, SourceRootConfig,
};
#[cfg(feature = "client")]
pub use source_client::{run_source, SourceClientConfig};
#[cfg(feature = "client")]
pub use source_direct::{start_direct_server, DirectGrantStore};
#[cfg(feature = "client")]
pub use source_scanner::SourceScanner;
#[cfg(feature = "client")]
pub use source_tunnel::handle_tunnel_grant;
#[cfg(feature = "client")]
pub use storyteller::{StorytellerConfig, StorytellerRunner};
#[cfg(feature = "client")]
pub use transcode::{FfmpegCapabilities, TranscodeRunner};

#[cfg(feature = "browser")]
pub use challenge::{ChallengeRunner, SOLVE_BUDGET};
