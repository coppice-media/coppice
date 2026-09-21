//! CrossPoint Reader protocol contracts for Coppice.
//!
//! This crate is transport-neutral: it defines the lossless rich-sync DTOs,
//! validation limits, durable-storage vocabulary, and the LAN transfer client
//! contract. The server owns authentication, user/media visibility, and
//! persistence; [`sync`] never accepts an unbound device identity. The
//! WebSocket implementation lives in [`transfer`] and is deliberately kept
//! separate from the authenticated sync API.
//!
//! Upstream evidence is pinned to CrossPoint firmware
//! `c33a8b0e883816c22cb1304e51464a4a04fd16dd`, crosspoint-sync
//! `d4cd96649dfcdded47ef40e939f994ebb923dbf6`, and the Calibre plugin
//! `460c53088860a118c4f408cb4bc37d3e3dea74d6`. Those projects are protocol
//! references only; this crate is a clean-room Rust implementation.
//!
//! See `crates/crosspoint/README.md` for exact routes, limits, security
//! boundaries, and the pinned-firmware custom-URL limitation.

pub mod profile;
pub mod storage;
pub mod sync;
pub mod transfer;

pub use profile::{
	CrosspointProfileError, CrosspointTargetModel, CrosspointTransferProfile,
	CrosspointTransferProfileInput,
};
pub use storage::{
	DeliverySnapshot, DeliveryStatus, DiscoveryMethod, TargetFingerprint, TargetSnapshot,
};
pub use sync::{
	Bookmark, Clipping, DocumentMetadata, GlobalStats, Position, Progress, RichSyncError,
	StatsBook, MAX_BATCH_ITEMS, MAX_CLIPPING_NOTE_BYTES, MAX_CLIPPING_TEXT_BYTES,
	MAX_DOCUMENTS_PAGE, MAX_DOCUMENT_BYTES, MAX_POSITION_ANCHOR_BYTES,
	MAX_POSITION_XPATH_BYTES, MAX_PROGRESS_BYTES, MAX_STATS_BOOK_BATCH,
	MAX_SUMMARY_BYTES,
};
