//! Persistence contract: SeaORM entities, stored value types, `EntityError`,
//! and the shared DB services (reading progress, list membership cleanup).
//! GraphQL derives are opt-in via the `graphql` feature. Schema changes are
//! made in `crates/migrations`, never here alone.
//! See `crates/models/README.md`.

#[cfg(test)]
mod tests {
	pub mod common;
}

pub mod domain;
pub mod entity;
pub mod error;
pub mod prefixer;
pub mod services;
pub mod shared;
