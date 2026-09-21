//! Canonical annotation export and pluggable export sinks.
//!
//! The crate owns two things and deliberately nothing else:
//!
//! 1. [`model`] — the canonical, per-user export model. Every book a user has
//!    annotated (through Stump's native `media_annotations`, the liseur-sync
//!    CAS, native bookmarks, or any combination) is projected once into
//!    [`model::ExportBook`] with a deterministic field and row ordering, so a
//!    re-export of unchanged data is byte-identical. Explicit work/edition
//!    reviews are carried as one optional [`model::ExportReview`].
//! 2. [`sink`] — the [`sink::Sink`] contract plus the concrete sinks. The
//!    [`markdown`] sink renders one Obsidian-friendly file per book into a
//!    per-user directory; the [`git`] sink (feature `git`) commits and pushes
//!    that working tree with `git2`, rebasing once onto the remote when the
//!    push is rejected as non-fast-forward.
//!
//! Host wiring (config group, job, debounce, GraphQL) lives in `stump_core`
//! (`core/src/annotation_sync.rs`); this crate is host-agnostic and only ever
//! sees a `DatabaseConnection`. See `crates/annotation-sync/README.md` for
//! decisions, the markdown format, and verification commands.
//!
//! The liseur CAS rows are read through read-only SQL against the tables
//! created by `m20260904_000000_add_liseur_sync`; the write path stays
//! exclusively in the server's liseur-sync storage module.

pub mod error;
#[cfg(feature = "git")]
pub mod git;
pub mod markdown;
pub mod model;
pub mod registry;
pub mod sink;
#[cfg(test)]
mod test_support;

pub use error::AnnotationSyncError;
pub use model::{
	build_export_batch, BuildOptions, ExportBatch, ExportBook, ExportReview,
};
pub use sink::{SinkDescriptor, SinkPresetDescriptor, SinkState};
