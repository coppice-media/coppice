#![warn(clippy::dbg_macro)]
// The merged GraphQL schema nests deeply enough that layout computation
// overflows rustc's default query depth.
#![recursion_limit = "256"]

pub mod config;
pub mod errors;
pub mod http_server;
pub mod middleware;
pub mod routers;
pub mod utils;

pub use http_server::{bootstrap_http_server_config, run_http_server};
