//! Kavita compatibility profile: the routes, DTOs and identity mapping that
//! Kavita clients (Kavita Tachiyomi extension, Mihon tracker, Kover, Turnleaf,
//! Kamigura, Kamare, Inkita) exercise, pinned to Kavita 0.9.1.4.
//!
//! The crate owns request routing and DTO mapping; the server supplies the
//! [`routes::KavitaBackend`] adapter over its context and authenticates
//! requests with the API-key/JWT rules documented in `kavita-compat.mdx`.

#[macro_use]
mod macros;

pub mod auth;
pub mod dto;
pub mod errors;
pub mod filter;
pub mod ids;
pub mod mapper;
pub mod progress;
pub mod routes;

pub use auth::{mint_token, verify_token, KavitaClaims, TokenError};
pub use dto::*;
pub use filter::{
	decode_series_filter, encode_series_filter, FilterCombination, FilterComparison,
	FilterEntityType, SeriesFilterField, SeriesFilterStatementDto, SeriesFilterV2Dto,
	SeriesSortField, SeriesSortOptionDto,
};
pub use ids::{IdKind, KavitaIds};
pub use routes::{is_kavita_path, router, KavitaBackend, KavitaImage};

/// The Kavita release whose OpenAPI document and reference responses this
/// profile reproduces.
pub const KAVITA_VERSION: &str = "0.9.1.4";
