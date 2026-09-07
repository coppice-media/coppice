//! `stump-worker`: the remote worker process.
//!
//! ```sh
//! stump-worker --server https://stump.example --api-key stump_ab_cd
//! stump-worker --server https://stump.example --chrome google-chrome-stable
//! ```
//!
//! It pairs like any other device: create a device of kind `Worker` in the
//! console, take the API key it mints, and hand it here. The worker dials out,
//! advertises what this machine can do — an `ffmpeg` for `transcode`, a Chrome
//! for `challenge_solve` — and serves those jobs until it is stopped. Nothing
//! listens on this machine; the server never connects in.
//!
//! `--chrome` needs a build with the `browser` feature; without it the flag is
//! not compiled and the binary is `ffmpeg`-only.
//!
//! Protocol and pairing: `docs/content/docs/developer/workers.mdx`.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use serde_json::Value;
use stump_worker::client::{self, Assignment, ClientConfig, JobRunner, Progress};
use stump_worker::transcode::TranscodeRunner;

#[derive(Debug, Parser)]
#[command(
	name = "stump-worker",
	about = "Run Stump jobs (transcode, challenge_solve) on this machine for a remote Stump server"
)]
struct Args {
	/// The Stump server's base URL, e.g. `https://stump.example`.
	#[arg(long)]
	server: String,
	/// The API key of a paired device of kind `Worker`.
	#[arg(long, env = "STUMP_WORKER_API_KEY")]
	api_key: String,
	/// The `ffmpeg` binary to use. Defaults to searching `PATH`.
	#[arg(long)]
	ffmpeg: Option<PathBuf>,
	/// The Chrome (or Chromium) binary to solve Cloudflare challenges with.
	/// Omitted, this worker does not advertise `browser`.
	#[cfg(feature = "browser")]
	#[arg(long)]
	chrome: Option<PathBuf>,
	/// Where the browser's persistent profile lives. Defaults to
	/// `$XDG_DATA_HOME/stump-worker/chrome` (`~/.local/share/…`).
	///
	/// Never point this at a Chrome profile you use: Chrome refuses a
	/// `--user-data-dir` another process holds, and a source has no business
	/// near your cookies.
	#[cfg(feature = "browser")]
	#[arg(long)]
	chrome_profile: Option<PathBuf>,
	/// How long a solve may take, in seconds, before the job fails. The
	/// default leaves room for a human to notice the window and tick a
	/// Turnstile checkbox; an unattended solve never comes near it.
	#[cfg(feature = "browser")]
	#[arg(long, default_value_t = stump_worker::SOLVE_BUDGET.as_secs())]
	solve_budget: u64,
	/// The name reported to the server's Workers page. Defaults to the
	/// hostname, which is what an operator recognises the box by.
	#[arg(long)]
	name: Option<String>,
}

/// Every runner this worker was started with, keyed by the job kind it serves.
///
/// The client holds one `JobRunner`; a worker that can do two things is
/// therefore a runner that dispatches on `assignment.kind` and advertises the
/// union of what it wraps. Keeping the composite here rather than in the
/// protocol crate is deliberate: which capabilities a process has is a
/// question about its command line, not about the protocol.
struct Runners(Vec<(&'static str, Arc<dyn JobRunner>)>);

#[async_trait::async_trait]
impl JobRunner for Runners {
	fn capabilities(&self) -> Value {
		let mut merged = serde_json::Map::new();
		for (_, runner) in &self.0 {
			if let Value::Object(map) = runner.capabilities() {
				merged.extend(map);
			}
		}
		Value::Object(merged)
	}

	async fn run(
		&self,
		assignment: Assignment,
		progress: Progress,
	) -> Result<Value, String> {
		let runner = self
			.0
			.iter()
			.find(|(kind, _)| *kind == assignment.kind)
			.map(|(_, runner)| runner.clone())
			.ok_or_else(|| {
				format!("This worker does not run `{}` jobs", assignment.kind)
			})?;
		runner.run(assignment, progress).await
	}
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
	tracing_subscriber::fmt()
		.with_env_filter(
			tracing_subscriber::EnvFilter::try_from_env("STUMP_WORKER_LOG")
				.unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
		)
		.init();

	let args = Args::parse();
	let config = ClientConfig {
		server: args.server.clone(),
		api_key: args.api_key,
		name: args.name.or_else(hostname),
	};

	let mut runners: Vec<(&'static str, Arc<dyn JobRunner>)> = Vec::new();

	// `ffmpeg::locate` searches a *directory*; `--ffmpeg` names the binary,
	// which is what an operator has a path to.
	let bin_dir = args
		.ffmpeg
		.as_deref()
		.and_then(std::path::Path::parent)
		.map(std::path::Path::to_path_buf);
	match TranscodeRunner::probe(config.clone(), bin_dir.as_deref()) {
		Ok(runner) => {
			let capabilities = runner.capabilities_report().clone();
			tracing::info!(
				ffmpeg = %capabilities.version,
				hwaccel = ?capabilities.hwaccel,
				"Advertising transcode"
			);
			runners.push((stump_worker::TRANSCODE, Arc::new(runner)));
		},
		Err(error) => {
			// Not fatal any more: a browser worker is a legitimate deployment
			// with no `ffmpeg` on it at all. Advertising `transcode` anyway
			// would be, though — the server would route to it instead of using
			// its own fallback.
			tracing::warn!(%error, "No usable ffmpeg; not advertising transcode");
		},
	}

	#[cfg(feature = "browser")]
	if let Some(chrome) = args.chrome.as_deref() {
		let profile = args.chrome_profile.clone().unwrap_or_else(default_profile);
		match stump_worker::ChallengeRunner::probe(chrome, &profile) {
			Ok(runner) => {
				let runner = runner.with_budget(std::time::Duration::from_secs(
					args.solve_budget.max(5),
				));
				tracing::info!(
					chrome = %runner.version(),
					profile = %runner.profile().display(),
					budget_secs = args.solve_budget,
					"Advertising browser (headed; a challenge may need one click)"
				);
				runners.push((stump_worker::CHALLENGE_SOLVE, Arc::new(runner)));
			},
			Err(error) => {
				tracing::error!(%error, "The browser named by --chrome is unusable");
				return std::process::ExitCode::FAILURE;
			},
		}
	}

	if runners.is_empty() {
		tracing::error!(
			"This worker can do nothing: no usable ffmpeg, and no --chrome. Refusing to connect"
		);
		return std::process::ExitCode::FAILURE;
	}

	let runner = Runners(runners);
	tracing::info!(
		server = %args.server,
		capabilities = %runner.capabilities(),
		"stump-worker starting"
	);
	client::run(config, Arc::new(runner)).await
}

/// The worker's own data directory, following the XDG layout the rest of a
/// Linux desktop uses.
#[cfg(feature = "browser")]
fn default_profile() -> PathBuf {
	std::env::var_os("XDG_DATA_HOME")
		.map(PathBuf::from)
		.or_else(|| {
			std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
		})
		.unwrap_or_else(std::env::temp_dir)
		.join("stump-worker")
		.join("chrome")
}

fn hostname() -> Option<String> {
	std::env::var("HOSTNAME").ok().or_else(|| {
		std::fs::read_to_string("/etc/hostname")
			.ok()
			.map(|name| name.trim().to_string())
			.filter(|name| !name.is_empty())
	})
}
