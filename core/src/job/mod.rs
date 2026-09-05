//! Core's concrete background jobs and their host adapter for `stump_jobs`.
//!
//! The lifecycle contracts (`JobLifecycle`, `JobContext`, `JobError`, ...) and the runtime
//! live in `stump_jobs`; this module names the jobs Stump runs ([`StumpJob`]), their output
//! union ([`CoreJobOutput`]), and the [`JobServices`] host that persists job records,
//! forwards events, and dispatches queued payloads.

mod error;
mod output;
mod services;
pub mod stump_job;

pub use output::CoreJobOutput;
pub use services::JobServices;
