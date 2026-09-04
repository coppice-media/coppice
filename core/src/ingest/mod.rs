//! Staged ingest workflow: drop folder, analysis queue, quality reports,
//! metadata candidates, and field-level apply.  See `contract` for the
//! shared types every part of the pipeline builds against.

pub mod contract;
pub mod coordinator;
pub mod drop_folder;
pub mod progress;
pub mod providers;
pub mod quality;
pub mod services;
pub mod staging;
pub mod store;
