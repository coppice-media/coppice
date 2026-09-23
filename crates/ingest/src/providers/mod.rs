//! Metadata provider façade and field-level application for staged ingest.
//!
//! The provider registry keeps the existing metadata-integrations clients as the
//! source of network behaviour while exposing the source-digest-bound contract
//! used by the staged ingest workflow.

pub mod apply;
pub mod builtin_embedded;
mod facade;
pub mod llm;
mod registry;
pub use apply::{
	apply_to_media, apply_to_media_txn, apply_to_media_txn_with_context,
	apply_to_media_txn_with_context_and_cover, apply_to_media_with_cover,
	prepare_cover_for_media, prepare_cover_for_new_media, resolve_picks,
	resolve_picks_for_context, validate_picks, ApplyError, CandidateModel,
	CoverApplyConfig, CoverWrite, ResolvedFields, STORABLE_FIELDS,
};
pub use builtin_embedded::EmbeddedProvider;
pub use facade::IntegrationProvider;
pub use registry::{ProviderDescriptor, ProviderRegistry};
