//! async-graphql schema for the web/desktop/mobile clients (server feature
//! `graphql`; absent from `minimal`). Feature `web` (default on, forwarded by
//! the server's `webui`) carries the resolvers only the upstream React SPA and
//! the Expo app reach; a `headless` server drops them. `schema.graphql` is the
//! *full* schema, generated with every feature on by the `graphql-gen` binary
//! (`cargo dump-schema`) and checked in CI.
//! See `crates/graphql/README.md`.

pub mod data;
pub(crate) mod error;
pub mod error_message;
pub mod filter;
pub mod guard;
pub mod input;
pub mod loader;
pub mod mutation;
pub mod object;
pub mod order;
pub mod pagination;
pub mod query;
pub mod schema;
pub(crate) mod social;
pub mod subscription;
pub(crate) mod utils;

#[cfg(test)]
mod tests;

// TODO: Look into https://async-graphql.github.io/async-graphql/en/extensions_available.html#tracing
// TODO: Look into https://async-graphql.github.io/async-graphql/en/extensions_available.html#apollo-persisted-queries
// TODO: Look into data loaders: https://async-graphql.github.io/async-graphql/en/dataloader.html
