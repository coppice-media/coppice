//! Staged ingest workflow: drop folder, analysis queue, quality reports,
//! metadata candidates, and field-level apply.  See `contract` for the
//! shared types every part of the pipeline builds against, `README.md` for
//! the pipeline's decisions, and `host` for the two things the crate refuses
//! to know (building library rows, decrypting provider tokens) and takes from
//! its host instead.
#![warn(clippy::dbg_macro)]

pub mod archive;
pub mod config;
pub mod contract;
pub mod coordinator;
pub mod drop_folder;
pub mod error;
pub mod event;
pub mod host;
pub mod pairing;
pub mod policy;
pub mod preprocess;
pub mod progress;
pub mod providers;
pub mod quality;
pub mod services;
pub mod staging;
pub mod store;

#[cfg(test)]
mod explode_tests;

pub use config::IngestSettings;
pub use error::{IngestError, IngestResult};
pub use event::{IngestEvent, IngestEventSink};
pub use host::{ProviderClientFactory, RowFactory};
pub use services::IngestServices;
