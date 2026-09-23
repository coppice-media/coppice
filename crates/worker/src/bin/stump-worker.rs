//! `stump-worker`: the remote worker process.
//!
//! ```sh
//! stump-worker --server https://stump.example --api-key stump_ab_cd
//! stump-worker --server https://stump.example --chrome google-chrome-stable
//! ```
//!
//! Pair a `Worker` device for compute jobs or a separate `SourceWorker` device
//! for remote-library inventory and reads. Compute and tunnel-source modes dial
//! out only. Direct-source mode explicitly binds `--source-listen` on a private
//! WireGuard, Tailscale, or LAN interface and advertises `--source-base-url`.
//! The credentials and protocols remain separate even when both roles run in
//! one process.
//!
//! `--chrome` needs a build with the `browser` feature; without it the flag is
//! not compiled and the binary is `ffmpeg`-only.
//!
//! Protocols: `docs/content/docs/developer/workers.mdx` and
//! `docs/content/docs/developer/remote-worker-libraries.mdx`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, ValueEnum};
use serde_json::Value;
use stump_worker::native_align::{NativeAlignConfig, NativeAlignRunner};
use stump_worker::source_catalog::parse_source_root_config;
use stump_worker::source_client::{run_source, SourceClientConfig};
use stump_worker::source_protocol::SourceTransport;
use stump_worker::storyteller::{StorytellerConfig, StorytellerRunner};
use stump_worker::{
	client, Assignment, ClientConfig, JobRunner, Progress, TranscodeRunner,
};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SourceTransportArg {
	Direct,
	Tunnel,
}

impl From<SourceTransportArg> for SourceTransport {
	fn from(value: SourceTransportArg) -> Self {
		match value {
			SourceTransportArg::Direct => Self::Direct,
			SourceTransportArg::Tunnel => Self::Tunnel,
		}
	}
}

#[derive(Debug, Parser)]
#[command(
	name = "stump-worker",
	about = "Run Coppice compute and/or source jobs on this machine for a remote Coppice server"
)]
struct Args {
	/// The Coppice server's base URL, e.g. `https://stump.example`.
	#[arg(long)]
	server: String,
	/// Compute-worker API key. Omit this for source-only mode.
	#[arg(long, env = "STUMP_WORKER_API_KEY")]
	api_key: Option<String>,
	/// Source-worker API key. It is a separate credential from `--api-key`.
	#[arg(long, env = "STUMP_SOURCE_API_KEY")]
	source_api_key: Option<String>,
	/// The `ffmpeg` binary to use. Defaults to searching `PATH`.
	#[arg(long)]
	ffmpeg: Option<PathBuf>,
	/// Repeatable source root in the form `root_id=/absolute/path`.
	/// Optional fields use `;label=...;kind=...;privacy=catalog`; use
	/// `kind=calibre` to read the root's `metadata.db` catalog read-only.
	/// candidate_only is intentionally rejected.
	#[arg(long = "source-root")]
	source_roots: Vec<String>,
	/// Durable source catalog directory. Defaults to the worker XDG data dir.
	#[arg(long)]
	source_state_dir: Option<PathBuf>,
	/// Seconds between source catalog scans.
	#[arg(long, default_value_t = 300)]
	source_scan_interval: u64,
	/// Source byte transport. Direct requires both `--source-listen` and
	/// `--source-base-url`; tunnel opens a separate outbound WebSocket.
	#[arg(long, value_enum, default_value_t = SourceTransportArg::Tunnel)]
	source_transport: SourceTransportArg,
	/// Private-network bind address for direct source reads.
	#[arg(long)]
	source_listen: Option<SocketAddr>,
	/// URL advertised for direct source reads, reachable by the server.
	#[arg(long)]
	source_base_url: Option<String>,
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
	/// Optional worker-local Storyteller URL. Credentials are read only by
	/// this process and are never included in the server job input/archive.
	#[arg(long, env = "STUMP_STORYTELLER_URL")]
	storyteller_url: Option<String>,
	#[arg(long, env = "STUMP_STORYTELLER_USERNAME", requires = "storyteller_url")]
	storyteller_username: Option<String>,
	#[arg(long, env = "STUMP_STORYTELLER_PASSWORD", requires = "storyteller_url")]
	storyteller_password: Option<String>,
	/// Optional worker-local native alignment executable. It receives
	/// `--epub`, `--audio`, `--model`, and `--output` paths and writes a
	/// typed `SyncMapV1` JSON artifact. The executable and model never enter
	/// the server job payload.
	#[arg(long, env = "STUMP_NATIVE_ALIGNER", requires = "native_align_model")]
	native_aligner: Option<PathBuf>,
	#[arg(long, env = "STUMP_NATIVE_ALIGN_MODEL", requires = "native_aligner")]
	native_align_model: Option<PathBuf>,
	#[arg(long, env = "STUMP_NATIVE_ALIGN_MODEL_ID", default_value = "default")]
	native_align_model_id: String,
	#[arg(
		long,
		env = "STUMP_NATIVE_ALIGN_MODEL_REVISION",
		default_value = "default"
	)]
	native_align_model_revision: String,
	#[arg(
		long,
		env = "STUMP_NATIVE_ALIGN_IMPLEMENTATION",
		default_value = "native-ctc"
	)]
	native_align_implementation: String,
	#[arg(
		long,
		env = "STUMP_NATIVE_ALIGN_VERSION",
		default_value = "coppice-native-align-v1"
	)]
	native_align_version: String,
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
	let compute_requested = args.api_key.is_some()
		|| args.ffmpeg.is_some()
		|| args.storyteller_url.is_some()
		|| args.native_aligner.is_some()
		|| args.native_align_model.is_some();
	#[cfg(feature = "browser")]
	let compute_requested = compute_requested || args.chrome.is_some();
	let source_requested = args.source_api_key.is_some()
		|| !args.source_roots.is_empty()
		|| args.source_state_dir.is_some()
		|| args.source_listen.is_some()
		|| args.source_base_url.is_some();
	if !compute_requested && !source_requested {
		tracing::error!("No compute or source capability was configured");
		return std::process::ExitCode::FAILURE;
	}

	let worker_name = args.name.or_else(hostname);
	let source_config = if source_requested {
		let Some(source_api_key) = args.source_api_key.clone() else {
			tracing::error!(
				"Source mode requires --source-api-key or STUMP_SOURCE_API_KEY"
			);
			return std::process::ExitCode::FAILURE;
		};
		if args.source_roots.is_empty() {
			tracing::error!("Source mode requires at least one --source-root");
			return std::process::ExitCode::FAILURE;
		}
		let transport: SourceTransport = args.source_transport.into();
		let mut roots = Vec::with_capacity(args.source_roots.len());
		for spec in &args.source_roots {
			let root = match parse_source_root_config(
				spec,
				transport,
				args.source_base_url.clone(),
			) {
				Ok(root) => root,
				Err(error) => {
					tracing::error!(%error, spec, "Invalid --source-root");
					return std::process::ExitCode::FAILURE;
				},
			};
			roots.push(root);
		}
		Some(SourceClientConfig {
			server: args.server.clone(),
			api_key: source_api_key,
			name: worker_name.clone(),
			roots,
			state_dir: args
				.source_state_dir
				.clone()
				.unwrap_or_else(default_source_state_dir),
			scan_interval: std::time::Duration::from_secs(
				args.source_scan_interval.max(1),
			),
			listen: args.source_listen,
		})
	} else {
		None
	};

	let compute_config = if compute_requested {
		let Some(api_key) = args.api_key.clone() else {
			tracing::error!("Compute mode requires --api-key or STUMP_WORKER_API_KEY");
			return std::process::ExitCode::FAILURE;
		};
		Some(ClientConfig {
			server: args.server.clone(),
			api_key,
			name: worker_name.clone(),
		})
	} else {
		None
	};
	let mut runners: Vec<(&'static str, Arc<dyn JobRunner>)> = Vec::new();
	if let Some(config) = compute_config.clone() {
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
				tracing::warn!(%error, "No usable ffmpeg; not advertising transcode");
			},
		}
		let native_configured = args.native_aligner.is_some();
		if let Some(executable) = args.native_aligner.clone() {
			let model_path = args
				.native_align_model
				.clone()
				.expect("clap requires --native-align-model with --native-aligner");
			match NativeAlignConfig::new(
				executable,
				model_path,
				args.native_align_model_id.clone(),
				args.native_align_model_revision.clone(),
			) {
				Ok(mut native_config) => {
					native_config.implementation =
						args.native_align_implementation.clone();
					native_config.version = args.native_align_version.clone();
					let runner = NativeAlignRunner::new(config.clone(), native_config);
					tracing::info!("Advertising native ALIGN");
					runners.push((stump_worker::ALIGN, Arc::new(runner)));
				},
				Err(error) => {
					tracing::error!(%error, "Native ALIGN configuration is unusable");
					return std::process::ExitCode::FAILURE;
				},
			}
		}
		if !native_configured {
			if let Some(url) = args.storyteller_url.clone() {
				let username = args.storyteller_username.clone().unwrap_or_default();
				let password = args.storyteller_password.clone().unwrap_or_default();
				match StorytellerRunner::new(
					config.clone(),
					StorytellerConfig::new(url, username, password),
				) {
					Ok(runner) => {
						tracing::info!("Advertising Storyteller ALIGN");
						runners.push((stump_worker::ALIGN, Arc::new(runner)));
					},
					Err(error) => {
						tracing::error!(%error, "Storyteller configuration is unusable");
						return std::process::ExitCode::FAILURE;
					},
				}
			}
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
				"This compute worker can do nothing: no usable ffmpeg, ALIGN runner, or --chrome"
			);
			return std::process::ExitCode::FAILURE;
		}
	}

	if let Some(source) = source_config {
		if compute_requested {
			tokio::spawn(async move {
				if let Err(error) = run_source(source).await {
					tracing::error!(%error, "Source worker stopped");
				}
			});
		} else {
			return match run_source(source).await {
				Ok(()) => std::process::ExitCode::SUCCESS,
				Err(error) => {
					tracing::error!(%error, "Source worker stopped");
					std::process::ExitCode::FAILURE
				},
			};
		}
	}

	let config = compute_config
		.expect("compute configuration exists when compute mode is enabled");
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

fn default_source_state_dir() -> PathBuf {
	std::env::var_os("XDG_DATA_HOME")
		.map(PathBuf::from)
		.or_else(|| {
			std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
		})
		.unwrap_or_else(std::env::temp_dir)
		.join("stump-worker")
		.join("source")
}

fn hostname() -> Option<String> {
	std::env::var("HOSTNAME").ok().or_else(|| {
		std::fs::read_to_string("/etc/hostname")
			.ok()
			.map(|name| name.trim().to_string())
			.filter(|name| !name.is_empty())
	})
}
