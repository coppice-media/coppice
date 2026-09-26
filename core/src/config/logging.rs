use tracing_subscriber::{
	filter::LevelFilter, prelude::__tracing_subscriber_SubscriberExt,
	util::SubscriberInitExt, EnvFilter,
};

use super::StumpConfig;

pub const STUMP_SHADOW_TEXT: &str = include_str!("stump_shadow_text.txt");

/// Once `Stump.log` exceeds this size at startup it is moved to `Stump.log.1`
/// (replacing any previous one) so a long-lived verbose install cannot fill the
/// disk. The live file keeps its fixed name because the log-file GraphQL
/// query, clear mutation, and tail subscription address it directly.
const MAX_LOG_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Initializes the logging system, which uses the [tracing] crate. Logs are written to
/// both the console and a file in the config directory. The file is called `Stump.log`
/// by default.
pub fn init_tracing(config: &StumpConfig) {
	let log_dir = config.get_log_dir();
	roll_oversized_log(&log_dir.join("Stump.log"), MAX_LOG_FILE_BYTES);
	let file_appender = tracing_appender::rolling::never(log_dir, "Stump.log");

	let max_level = match config.server.verbosity {
		0 => LevelFilter::OFF,
		1 => LevelFilter::INFO,
		2 => LevelFilter::DEBUG,
		_3_or_more => LevelFilter::TRACE,
	};

	let mut env_filter = EnvFilter::from_default_env()
		.add_directive(
			"stump_core=trace"
				.parse()
				.expect("Error invalid tracing directive for stump_core!"),
		)
		.add_directive(
			"stump_server=trace"
				.parse()
				.expect("Error invalid tracing directive for stump_server!"),
		)
		.add_directive(
			"stump_provider=trace"
				.parse()
				.expect("Error invalid tracing directive for stump_provider!"),
		)
		.add_directive(
			"metadata_integrations=trace"
				.parse()
				.expect("Error invalid tracing directive for metadata_integrations!"),
		)
		.add_directive(
			"graphql=trace"
				.parse()
				.expect("Error invalid tracing directive for graphql!"),
		)
		.add_directive(
			"email=trace"
				.parse()
				.expect("Error invalid tracing directive for email!"),
		)
		.add_directive(
			"tower_http=debug"
				.parse()
				.expect("Error invalid tracing directive for tower_http!"),
		);

	if config.server.verbosity > 2 {
		env_filter = env_filter.add_directive(
			"sqlx::query=debug"
				.parse()
				.expect("Failed to parse tracing directive for sqlx!"),
		);
	}

	if cfg!(debug_assertions) && config.server.verbosity > 2 {
		env_filter = env_filter.add_directive(
			"sea_orm::driver::sqlx_sqlite=debug"
				.parse()
				.expect("Failed to parse tracing directive for sea_orm!"),
		)
	}

	let base_layer = tracing_subscriber::registry()
		.with(max_level)
		.with(env_filter);

	// TODO: This is likely unnecessary duplication(?), and should be revisited
	if config.server.pretty_logs {
		base_layer
			.with(
				tracing_subscriber::fmt::layer()
					.pretty()
					.with_ansi(true)
					.with_writer(std::io::stdout),
			)
			.with(
				tracing_subscriber::fmt::layer()
					.pretty()
					.with_ansi(config.server.colorful_logs)
					.with_writer(file_appender),
			)
			.init();
	} else {
		base_layer
			.with(
				tracing_subscriber::fmt::layer()
					.with_ansi(true)
					.with_writer(std::io::stdout),
			)
			.with(
				tracing_subscriber::fmt::layer()
					.with_ansi(config.server.colorful_logs)
					.with_writer(file_appender),
			)
			.init();
	};

	tracing::info!(verbosity = ?max_level, verbosity_num = config.server.verbosity, "Tracing initialized");
}

/// Moves `path` to `<path>.1` when it is larger than `max_bytes`. Runs before the
/// subscriber exists, so failures go to stderr instead of the log.
fn roll_oversized_log(path: &std::path::Path, max_bytes: u64) {
	let Ok(metadata) = std::fs::metadata(path) else {
		return;
	};
	if metadata.len() <= max_bytes {
		return;
	}
	let mut rolled = path.as_os_str().to_owned();
	rolled.push(".1");
	if let Err(error) = std::fs::rename(path, &rolled) {
		eprintln!("Failed to roll oversized log {}: {error}", path.display());
	}
}

#[cfg(test)]
mod tests {
	use super::roll_oversized_log;

	#[test]
	fn oversized_log_is_rolled_once_and_small_log_is_kept() {
		let dir = tempfile::tempdir().unwrap();
		let log = dir.path().join("Stump.log");
		let rolled = dir.path().join("Stump.log.1");

		std::fs::write(&log, b"old").unwrap();
		std::fs::write(&rolled, b"older").unwrap();
		roll_oversized_log(&log, 3);
		assert_eq!(
			std::fs::read(&log).unwrap(),
			b"old",
			"at the limit stays live"
		);

		std::fs::write(&log, b"over").unwrap();
		roll_oversized_log(&log, 3);
		assert!(!log.exists());
		assert_eq!(
			std::fs::read(&rolled).unwrap(),
			b"over",
			"replaces previous roll"
		);

		roll_oversized_log(&log, 3);
		assert_eq!(
			std::fs::read(&rolled).unwrap(),
			b"over",
			"missing log is a no-op"
		);
	}
}
