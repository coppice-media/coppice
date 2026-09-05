use std::env;

use serde::{Deserialize, Serialize};
use stump_config_gen::StumpConfigGenerator;

use super::{defaults::*, env_keys::*};

/// Password hashing and session/token lifetimes. Flattened into [`super::StumpConfig`].
#[derive(StumpConfigGenerator, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "StumpAuthConfig"))]
pub struct AuthConfig {
	/// Password hash cost
	#[default_value(DEFAULT_PASSWORD_HASH_COST)]
	#[env_key(HASH_COST_KEY)]
	pub password_hash_cost: u32,

	/// The time in seconds that a login session will be valid for.
	#[default_value(DEFAULT_SESSION_TTL)]
	#[env_key(SESSION_TTL_KEY)]
	pub session_ttl: i64,

	#[default_value(DEFAULT_ACCESS_TOKEN_TTL)]
	#[env_key(ACCESS_TOKEN_TTL_KEY)]
	pub access_token_ttl: i64,

	#[default_value(DEFAULT_REFRESH_TOKEN_TTL)]
	#[env_key(REFRESH_TOKEN_TTL_KEY)]
	pub refresh_token_ttl: i64,

	/// The interval at which automatic deleted session cleanup is performed.
	#[default_value(DEFAULT_SESSION_EXPIRY_CLEANUP_INTERVAL)]
	#[env_key(SESSION_EXPIRY_INTERVAL_KEY)]
	pub expired_session_cleanup_interval: u64,
}
