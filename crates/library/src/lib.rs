//! Library and series lifecycle as one shared path.
//!
//! Every surface that creates, updates, or deletes a library — the GraphQL
//! mutations, the Komga and Kavita compatibility routes — goes through
//! [`library`], and every regrouping of books between series goes through
//! [`series`], so validation, persistence, watcher and scheduled-scan wiring,
//! filesystem moves, and core-event emission can never drift between them.
//!
//! See `README.md` for the decisions behind the two modules.

pub mod library;
pub mod series;
