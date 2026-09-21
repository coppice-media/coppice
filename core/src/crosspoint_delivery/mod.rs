//! Durable CrossPoint transfer queue and restart-safe delivery execution.
//!
//! The authenticated CrossPoint sync handlers and server route registration
//! live outside this module.  This module only owns local queue execution:
//! immutable source/profile snapshots, per-device serialization, bounded
//! retries, and retained attempt history.

mod service;

pub use service::{
	CrossPointDeliveryService, DeliveryConfig, DeliveryError, DeliveryOutcome,
	SourceRevision,
};
