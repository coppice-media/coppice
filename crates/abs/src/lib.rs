//! Audiobookshelf compatibility profile: the routes, DTOs and identity
//! mapping that current Audiobookshelf clients — Lissen `1.12.5-release`
//! (`fd5c0417`) first — exercise, pinned to Audiobookshelf 2.36.1.
//!
//! The crate owns request routing and DTO mapping; the server supplies the
//! [`routes::AbsBackend`] adapter over its context and authenticates requests
//! with the JWT/API-key/device rules documented in `abs-compat.mdx`. Every
//! captured wire shape here comes from the live `abs-ref` 2.36.0 container
//! (`../komga-compat/abs/`); v2.36.1 is the pinned compatibility target and
//! its client-used changes are recorded in `crates/abs/README.md`.
//!
//! An ABS "library item" is one Stump media row whose extension is an audio
//! container; an ABS "book" is that row's audio metadata. Podcast libraries
//! are deliberately not served. Design decisions and their evidence live in
//! `crates/abs/README.md`.

pub mod auth;
pub mod dto;
pub mod errors;
pub mod ids;
pub mod mapper;
pub mod model;
pub mod routes;
pub mod sessions;
pub mod socket;
#[cfg(test)]
mod test_support;

pub use auth::{mint_tokens, verify_token, AbsClaims, MintedTokens, TokenError};
pub use errors::{AbsError, AbsResult};
pub use ids::{AbsIds, IdKind, CREATE_ABS_IDS_SQL};
pub use model::{
	AbsAudio, AbsAudioChapter, AbsAudioTrack, AbsBookmark, AbsEbookFile, AbsImage,
	AbsPlaylist, AbsPositionUpdate, AbsProgress, ItemShape,
};
pub use routes::{
	authenticated_router, public_router, AbsBackend, AbsSession, PLAY_METHOD_DIRECT,
	PLAY_METHOD_LOCAL,
};
pub use sessions::{AbsSessions, CREATE_ABS_SESSIONS_SQL};
pub use socket::{router as socket_router, AbsEvent, AbsEvents};

/// The Audiobookshelf release reported by this compatibility profile. The
/// checked-in response captures are from 2.36.0; 2.36.1 is the current source
/// and image reference.
pub const ABS_VERSION: &str = "2.36.1";

/// `serverSettings.buildNumber` as reported by the 2.36.1 reference source.
pub const ABS_BUILD_NUMBER: i64 = 1;
