use std::{
	env,
	num::{NonZeroU32, NonZeroUsize},
};

pub const RATELIMIT_ENABLED_KEY: &str = "STUMP_RATELIMIT_ENABLED";
pub const RATELIMIT_AUTH_PER_MIN_KEY: &str = "STUMP_RATELIMIT_AUTH_PER_MIN";
pub const RATELIMIT_WRITE_PER_MIN_KEY: &str = "STUMP_RATELIMIT_WRITE_PER_MIN";
pub const RATELIMIT_STREAM_CONCURRENCY_KEY: &str = "STUMP_RATELIMIT_STREAM_CONCURRENCY";

const DEFAULT_AUTH_PER_MIN: u32 = 10;
const DEFAULT_AUTH_BURST: u32 = 20;
const DEFAULT_WRITE_PER_MIN: u32 = 120;
const DEFAULT_WRITE_BURST: u32 = 60;
const DEFAULT_STREAM_CONCURRENCY: usize = 32;

/// Inbound rate limits, read from the environment at server start. These are
/// server-only knobs: the core config file never carries them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitConfig {
	pub enabled: bool,
	/// AUTH class: sustained credential-verifying requests per minute per client IP.
	pub auth_per_min: NonZeroU32,
	/// AUTH class: bucket size. The default bucket (20) absorbs a Komga/OPDS
	/// first-contact burst; raising `auth_per_min` above it raises the bucket too.
	pub auth_burst: NonZeroU32,
	/// WRITE class: sustained mutating requests per minute per user/API key/IP.
	pub write_per_min: NonZeroU32,
	/// WRITE class: bucket size, at least `write_per_min` / 2.
	pub write_burst: NonZeroU32,
	/// READ/STREAM class: in-flight requests (handler plus response body) per key.
	pub stream_concurrency: NonZeroUsize,
}

impl Default for RateLimitConfig {
	fn default() -> Self {
		Self::from_values(None, None, None, None)
			.expect("rate limit defaults are valid")
	}
}

impl RateLimitConfig {
	pub fn from_env() -> Result<Self, String> {
		Self::from_values(
			read_env(RATELIMIT_ENABLED_KEY)?,
			read_env(RATELIMIT_AUTH_PER_MIN_KEY)?,
			read_env(RATELIMIT_WRITE_PER_MIN_KEY)?,
			read_env(RATELIMIT_STREAM_CONCURRENCY_KEY)?,
		)
	}

	fn from_values(
		enabled: Option<String>,
		auth_per_min: Option<String>,
		write_per_min: Option<String>,
		stream_concurrency: Option<String>,
	) -> Result<Self, String> {
		let auth_per_min = parse_positive_u32(
			RATELIMIT_AUTH_PER_MIN_KEY,
			auth_per_min.as_deref(),
			DEFAULT_AUTH_PER_MIN,
		)?;
		let write_per_min = parse_positive_u32(
			RATELIMIT_WRITE_PER_MIN_KEY,
			write_per_min.as_deref(),
			DEFAULT_WRITE_PER_MIN,
		)?;
		let stream_concurrency = parse_positive_usize(
			RATELIMIT_STREAM_CONCURRENCY_KEY,
			stream_concurrency.as_deref(),
			DEFAULT_STREAM_CONCURRENCY,
		)?;

		Ok(Self {
			enabled: parse_bool(RATELIMIT_ENABLED_KEY, enabled.as_deref(), true)?,
			auth_per_min,
			auth_burst: non_zero_max(DEFAULT_AUTH_BURST, auth_per_min.get()),
			write_per_min,
			write_burst: non_zero_max(DEFAULT_WRITE_BURST, write_per_min.get() / 2),
			stream_concurrency,
		})
	}
}

fn non_zero_max(default: u32, derived: u32) -> NonZeroU32 {
	NonZeroU32::new(default.max(derived)).expect("defaults are non-zero")
}

fn read_env(key: &str) -> Result<Option<String>, String> {
	match env::var(key) {
		Ok(value) => Ok(Some(value)),
		Err(env::VarError::NotPresent) => Ok(None),
		Err(env::VarError::NotUnicode(_)) => {
			Err(format!("Invalid {key} value: it must be valid UTF-8"))
		},
	}
}

fn parse_bool(key: &str, value: Option<&str>, default: bool) -> Result<bool, String> {
	match value.map(str::trim) {
		None | Some("") => Ok(default),
		Some(value) => match value.to_ascii_lowercase().as_str() {
			"true" | "1" | "yes" | "on" => Ok(true),
			"false" | "0" | "no" | "off" => Ok(false),
			_ => Err(format!(
				"Invalid {key} value `{value}`: expected true or false"
			)),
		},
	}
}

fn parse_positive_u32(
	key: &str,
	value: Option<&str>,
	default: u32,
) -> Result<NonZeroU32, String> {
	let Some(value) = value else {
		return Ok(NonZeroU32::new(default).expect("defaults are non-zero"));
	};
	let parsed = value.trim().parse::<u32>().map_err(|error| {
		format!("Invalid {key} value `{value}`: expected a positive integer ({error})")
	})?;
	NonZeroU32::new(parsed)
		.ok_or_else(|| format!("Invalid {key} value `{value}`: it must be greater than zero"))
}

fn parse_positive_usize(
	key: &str,
	value: Option<&str>,
	default: usize,
) -> Result<NonZeroUsize, String> {
	let Some(value) = value else {
		return Ok(NonZeroUsize::new(default).expect("defaults are non-zero"));
	};
	let parsed = value.trim().parse::<usize>().map_err(|error| {
		format!("Invalid {key} value `{value}`: expected a positive integer ({error})")
	})?;
	NonZeroUsize::new(parsed)
		.ok_or_else(|| format!("Invalid {key} value `{value}`: it must be greater than zero"))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn defaults_match_documented_limits() {
		let config = RateLimitConfig::default();
		assert!(config.enabled);
		assert_eq!(config.auth_per_min.get(), 10);
		assert_eq!(config.auth_burst.get(), 20);
		assert_eq!(config.write_per_min.get(), 120);
		assert_eq!(config.write_burst.get(), 60);
		assert_eq!(config.stream_concurrency.get(), 32);
	}

	#[test]
	fn overrides_scale_bursts_but_never_shrink_them() {
		let config = RateLimitConfig::from_values(
			Some("false".into()),
			Some("60".into()),
			Some("600".into()),
			Some("4".into()),
		)
		.unwrap();
		assert!(!config.enabled);
		assert_eq!(config.auth_per_min.get(), 60);
		assert_eq!(config.auth_burst.get(), 60);
		assert_eq!(config.write_per_min.get(), 600);
		assert_eq!(config.write_burst.get(), 300);
		assert_eq!(config.stream_concurrency.get(), 4);

		let small = RateLimitConfig::from_values(
			None,
			Some("2".into()),
			Some("30".into()),
			None,
		)
		.unwrap();
		assert_eq!(small.auth_burst.get(), 20);
		assert_eq!(small.write_burst.get(), 60);
	}

	#[test]
	fn rejects_zero_and_garbage() {
		let error =
			RateLimitConfig::from_values(None, Some("0".into()), None, None)
				.expect_err("zero must be rejected");
		assert!(error.contains(RATELIMIT_AUTH_PER_MIN_KEY));
		assert!(error.contains("greater than zero"));

		let error =
			RateLimitConfig::from_values(None, None, None, Some("many".into()))
				.expect_err("garbage must be rejected");
		assert!(error.contains(RATELIMIT_STREAM_CONCURRENCY_KEY));
		assert!(error.contains("positive integer"));

		let error =
			RateLimitConfig::from_values(Some("maybe".into()), None, None, None)
				.expect_err("non-boolean must be rejected");
		assert!(error.contains(RATELIMIT_ENABLED_KEY));
	}
}
