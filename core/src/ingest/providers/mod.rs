//! Metadata provider façade and field-level application for staged ingest.
//!
//! The provider registry keeps the existing metadata-integrations clients as the
//! source of network behaviour while exposing the source-digest-bound contract
//! used by the staged ingest workflow.

pub mod apply;
mod builtin_embedded;
mod facade;
mod llm;
mod registry;
pub use apply::{
	apply_to_media, apply_to_media_txn, apply_to_media_txn_with_context, resolve_picks,
	resolve_picks_for_context, validate_picks, ApplyError, CandidateModel,
	ResolvedFields, STORABLE_FIELDS,
};
pub use builtin_embedded::EmbeddedProvider;
pub use facade::IntegrationProvider;
pub use registry::{ProviderDescriptor, ProviderRegistry};
