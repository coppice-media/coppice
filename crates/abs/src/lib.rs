//! Audiobookshelf compatibility profile: the routes, DTOs and identity
//! mapping that Audiobookshelf clients — Lissen `1.11.22-release`
//! (`f30bf9be`) first — exercise, pinned to Audiobookshelf 2.36.0.
//!
//! The crate owns request routing and DTO mapping; the server supplies the
//! [`routes::AbsBackend`] adapter over its context and authenticates requests
//! with the JWT/API-key/device rules documented in `abs-compat.mdx`. Every
//! wire shape here was captured from a live `abs-ref` 2.36.0 container
//! (`../komga-compat/abs/`), never copied from its GPL-3.0 source.
//!
//! An ABS "library item" is one Stump media row whose extension is an audio
//! container; an ABS "book" is that row's audio metadata. Podcast libraries
//! are deliberately not served. Design decisions and their evidence live in
//! `crates/abs/README.md`.

pub mod auth;
pub mod errors;
pub mod ids;

pub use auth::{mint_tokens, verify_token, AbsClaims, MintedTokens, TokenError};
pub use errors::{AbsError, AbsResult};
pub use ids::{AbsIds, IdKind, CREATE_ABS_IDS_SQL};

/// The Audiobookshelf release whose reference responses this profile
/// reproduces.
pub const ABS_VERSION: &str = "2.36.0";

/// `serverSettings.buildNumber` as reported by 2.36.0.
pub const ABS_BUILD_NUMBER: i64 = 1;
