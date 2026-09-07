//! Library and series lifecycle as one shared path.
//!
//! Every surface that creates, updates, or deletes a library — the GraphQL
//! mutations, the Komga and Kavita compatibility routes — goes through
//! [`library`], and every regrouping of books between series goes through
//! [`series`], so validation, persistence, watcher and scheduled-scan wiring,
//! filesystem moves, and core-event emission can never drift between them.
//!
//! [`editions`] is the third shared path: which media rows are editions of the
//! same work (an audiobook and its ebook), and the chapter map that lets a
//! position in one be shown in the other.
//!
//! See `README.md` for the decisions behind the modules.

pub mod editions;
pub mod library;
pub mod series;
