//! Filesystem scan planning primitives.
//!
//! `stump_scanner` deliberately contains no application context, job runtime, event channel, or
//! database connection. Persistence adapters implement [`ScanSource`] in their owning crate.
//!
//! Upstream provenance, the reconciliation rules (mtime short-circuit, ignored-directory
//! pruning, recovered/missing status), and verification commands are in
//! `crates/scanner/README.md`.

mod options;
pub mod sequence;
mod source;
mod tag_cache;
mod walk;

pub use options::{
	BookVisitOperation, CustomVisit, CustomVisitResult, ScanConfig, ScanOptions,
};
pub use sequence::{
	analyze_sequence, clean_name, parse_identifier, parse_identifier_of, to_ranges,
	ParsedIdentifier, SequenceAnalysis, SequenceEntry, SequenceKind, SequenceNumber,
	SequenceRange, SequenceSource,
};
pub use source::{
	MediaIdentity, ScanError, ScanResult, ScanSource, ScanStatus, SeriesIdentity,
};
pub use tag_cache::TagCache;
pub use walk::{walk_library, walk_series, WalkedLibrary, WalkedSeries, WalkerCtx};
