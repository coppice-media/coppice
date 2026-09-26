use std::env;

use tokio::runtime::{Builder, Runtime};

pub const RUNTIME_WORKER_THREADS_KEY: &str = "STUMP_RUNTIME_WORKER_THREADS";
pub const RUNTIME_MAX_BLOCKING_THREADS_KEY: &str = "STUMP_RUNTIME_MAX_BLOCKING_THREADS";
/// Stack size for every runtime thread. async-graphql resolves nested objects
/// recursively on the polling thread's stack; the KOReader plugin's book-detail
/// query (media → series → media with its full field set) overflowed Tokio's
/// 2 MiB default in debug builds and aborted the whole server. Stacks are
/// reserved address space; pages are only committed as they are touched.
const RUNTIME_THREAD_STACK_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeConfig {
	worker_threads: Option<usize>,
	max_blocking_threads: Option<usize>,
}

impl RuntimeConfig {
	fn from_env() -> Result<Self, String> {
		Self::from_values(
			read_env(RUNTIME_WORKER_THREADS_KEY)?,
			read_env(RUNTIME_MAX_BLOCKING_THREADS_KEY)?,
		)
	}

	fn from_values(
		worker_threads: Option<String>,
		max_blocking_threads: Option<String>,
	) -> Result<Self, String> {
		Ok(Self {
			worker_threads: parse_positive(
				RUNTIME_WORKER_THREADS_KEY,
				worker_threads.as_deref(),
			)?,
			max_blocking_threads: parse_positive(
				RUNTIME_MAX_BLOCKING_THREADS_KEY,
				max_blocking_threads.as_deref(),
			)?,
		})
	}
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

fn parse_positive(key: &str, value: Option<&str>) -> Result<Option<usize>, String> {
	let Some(value) = value else {
		return Ok(None);
	};

	let parsed = value.parse::<usize>().map_err(|error| {
		format!("Invalid {key} value `{value}`: expected a positive integer ({error})")
	})?;
	if parsed == 0 {
		return Err(format!(
			"Invalid {key} value `{value}`: it must be greater than zero"
		));
	}

	Ok(Some(parsed))
}

pub fn build_runtime() -> Result<Runtime, String> {
	let config = RuntimeConfig::from_env()?;
	let mut builder = Builder::new_multi_thread();

	if let Some(worker_threads) = config.worker_threads {
		builder.worker_threads(worker_threads);
	}
	if let Some(max_blocking_threads) = config.max_blocking_threads {
		builder.max_blocking_threads(max_blocking_threads);
	}

	builder
		.thread_stack_size(RUNTIME_THREAD_STACK_BYTES)
		.enable_all()
		.build()
		.map_err(|error| format!("Failed to build Tokio runtime: {error}"))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn defaults_leave_tokio_settings_unset() {
		let config = RuntimeConfig::from_values(None, None).unwrap();
		assert_eq!(config.worker_threads, None);
		assert_eq!(config.max_blocking_threads, None);
	}

	#[test]
	fn parses_valid_overrides() {
		let config =
			RuntimeConfig::from_values(Some("2".to_string()), Some("8".to_string()))
				.unwrap();
		assert_eq!(config.worker_threads, Some(2));
		assert_eq!(config.max_blocking_threads, Some(8));
	}

	#[test]
	fn rejects_zero_values() {
		let worker_error = RuntimeConfig::from_values(Some("0".to_string()), None)
			.expect_err("zero worker threads must be rejected");
		assert!(worker_error.contains(RUNTIME_WORKER_THREADS_KEY));
		assert!(worker_error.contains("greater than zero"));

		let blocking_error = RuntimeConfig::from_values(None, Some("0".to_string()))
			.expect_err("zero blocking threads must be rejected");
		assert!(blocking_error.contains(RUNTIME_MAX_BLOCKING_THREADS_KEY));
		assert!(blocking_error.contains("greater than zero"));
	}

	#[test]
	fn rejects_invalid_values() {
		let error = RuntimeConfig::from_values(Some("not-a-number".to_string()), None)
			.expect_err("non-numeric worker threads must be rejected");
		assert!(error.contains(RUNTIME_WORKER_THREADS_KEY));
		assert!(error.contains("positive integer"));
	}
}
