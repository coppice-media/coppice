//! Remote workers: the `worker_jobs` queue, the socket protocol, and the
//! `stump-worker` client.
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
//! | `server` | `stump_core` → `apps/server`, `graphql` | [`WorkerJobs`] (the queue and dispatcher), [`WorkerHub`] (who is connected), the `worker_jobs` entity |
//! | `client` | the `stump-worker` binary | [`client`] (the protocol client), [`transcode`] (its `ffmpeg` runner), and the optional worker-local [`storyteller`] adapter |
//! | `browser` | the same binary, opted in | [`challenge`]: `challenge_solve` in a real, visible Chrome. Default off — a CDP stack is a lot to compile for an operator who only wants Opus |
//! | — | both | [`protocol`] (the frames), [`kind`] (job inputs and capability matching), and [`alignment`] (tier-2 align/SyncMap contracts) |
//!
//! The server half deliberately links no HTTP stack. The hub is fed frames and
//! hands back an outbound channel, so the axum `ws` glue lives in
//! `apps/server/src/routers/api/v2/workers.rs`, where the auth middleware has
//! already resolved the device — and `stump_core` links this crate without
//! linking axum.
//!
//! Decisions, layout, and verification commands: `crates/worker/README.md`.
//! Protocol reference: `docs/content/docs/developer/workers.mdx`.

pub mod alignment;
pub mod kind;
pub mod protocol;

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

#[cfg(feature = "browser")]
pub mod challenge;
#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub mod storyteller;
#[cfg(feature = "client")]
pub mod transcode;

#[cfg(all(test, feature = "server", feature = "client"))]
mod tests;

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

#[cfg(feature = "client")]
pub use client::{Assignment, ClientConfig, JobRunner, Progress};
#[cfg(feature = "client")]
pub use storyteller::{StorytellerConfig, StorytellerRunner};
#[cfg(feature = "client")]
pub use transcode::{FfmpegCapabilities, TranscodeRunner};

#[cfg(feature = "browser")]
pub use challenge::{ChallengeRunner, SOLVE_BUDGET};
