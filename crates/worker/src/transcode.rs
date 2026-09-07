//! The worker's `transcode` runner: download, encode, upload.
//!
//! It shells out through `stump_tools::ffmpeg`, the *same* adapter the
//! server's local fallback uses, so the argv is built in one place. That is
//! the property that makes the fallback honest: a VPS with `ffmpeg` and a GPU
//! box with `ffmpeg` produce the same Opus stream from the same track, and a
//! cache entry does not depend on who filled it.
//!
//! Capability probing is a one-time `ffmpeg -hwaccels` and `-encoders` read at
//! startup, not per job: the answer cannot change while the process runs, and
//! a probe per job would put two extra child processes in front of every
//! encode.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use stump_tools::external::Install;
use stump_tools::ffmpeg::{self, Codec, Encode};

use crate::client::{Assignment, ClientConfig, JobRunner, Progress};
use crate::kind::{TranscodeInput, TranscodeOutput, TranscodeResult, TRANSCODE};

/// What one host's `ffmpeg` install can do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegCapabilities {
	pub version: String,
	pub hwaccel: Vec<String>,
}

/// A worker that can transcode audio and nothing else.
pub struct TranscodeRunner {
	config: ClientConfig,
	install: Install,
	capabilities: FfmpegCapabilities,
	http: reqwest::Client,
}

impl TranscodeRunner {
	/// Locate `ffmpeg` and probe what it supports.
	///
	/// A missing or too-old `ffmpeg` is a startup failure, not a per-job
	/// failure: a worker that advertises `transcode` and then fails every job
	/// is worse than a worker that refuses to start, because the server would
	/// keep routing to it instead of falling back.
	pub fn probe(config: ClientConfig, bin_dir: Option<&Path>) -> Result<Self, String> {
		let install = ffmpeg::locate(bin_dir).map_err(|error| error.to_string())?;
		let hwaccel = probe_hwaccels(&install);
		let (major, minor, patch) = install.version;
		let capabilities = FfmpegCapabilities {
			version: format!("{major}.{minor}.{patch}"),
			hwaccel,
		};
		Ok(Self {
			config,
			install,
			capabilities,
			http: reqwest::Client::new(),
		})
	}

	#[must_use]
	pub fn capabilities_report(&self) -> &FfmpegCapabilities {
		&self.capabilities
	}

	async fn transcode(
		&self,
		assignment: &Assignment,
		progress: &Progress,
	) -> Result<Value, String> {
		let input: TranscodeInput = serde_json::from_value(assignment.input.clone())
			.map_err(|error| format!("invalid transcode input: {error}"))?;

		let staging = tempfile::Builder::new()
			.prefix("stump-worker-")
			.tempdir()
			.map_err(|error| error.to_string())?;
		// `ffmpeg` reads the muxer off the output path, so the source keeps a
		// neutral name and the output carries the real extension.
		let source = staging.path().join("source");
		let encoded = staging
			.path()
			.join(format!("output.{}", input.output.extension()));

		progress.report(0.05, "downloading the source track");
		let bytes = self.download_track(&input).await?;
		tokio::fs::write(&source, &bytes)
			.await
			.map_err(|error| error.to_string())?;

		progress.report(0.25, format!("encoding {}", input.output.mime()));
		let install = self.install.clone();
		let output = input.output.clone();
		let duration_ms = input.duration_ms;
		let (source, encoded) = tokio::task::spawn_blocking(move || {
			run_ffmpeg(&install, &source, &encoded, &output, duration_ms)
				.map(|()| (source, encoded))
		})
		.await
		.map_err(|error| format!("transcode task panicked: {error}"))??;
		drop(source);

		progress.report(0.85, "uploading the result");
		let produced = tokio::fs::read(&encoded)
			.await
			.map_err(|error| error.to_string())?;
		let digest = hex_sha256(&produced);
		let bytes = produced.len() as u64;
		self.upload(&assignment.id, produced, &digest).await?;

		Ok(serde_json::to_value(TranscodeResult {
			bytes,
			sha256: digest,
			mime: input.output.mime().to_string(),
		})
		.unwrap_or(Value::Null))
	}

	/// Fetch the source through the ordinary, authenticated track route.
	///
	/// The worker's own device carries no transform profile, so the route
	/// answers with the stored bytes; the server refuses to negotiate a
	/// transcode for a `Worker` device at all, which is what keeps a worker
	/// from asking the server for the very thing it was asked to produce.
	async fn download_track(&self, input: &TranscodeInput) -> Result<Vec<u8>, String> {
		let url = format!(
			"{}/api/v2/media/{}/audio/track/{}",
			self.config.server.trim_end_matches('/'),
			input.media_id,
			input.track_index
		);
		let response = self
			.http
			.get(&url)
			.bearer_auth(&self.config.api_key)
			.send()
			.await
			.map_err(|error| format!("download failed: {error}"))?;
		if !response.status().is_success() {
			return Err(format!("download failed: HTTP {}", response.status()));
		}
		Ok(response
			.bytes()
			.await
			.map_err(|error| format!("download failed: {error}"))?
			.to_vec())
	}

	async fn upload(
		&self,
		job_id: &str,
		body: Vec<u8>,
		digest: &str,
	) -> Result<(), String> {
		let response = self
			.http
			.put(self.config.upload_url(job_id))
			.bearer_auth(&self.config.api_key)
			.header("x-stump-sha256", digest)
			.header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
			.body(body)
			.send()
			.await
			.map_err(|error| format!("upload failed: {error}"))?;
		if !response.status().is_success() {
			let status = response.status();
			let detail = response.text().await.unwrap_or_default();
			return Err(format!("upload failed: HTTP {status} {}", detail.trim()));
		}
		Ok(())
	}
}

#[async_trait::async_trait]
impl JobRunner for TranscodeRunner {
	fn capabilities(&self) -> Value {
		json!({
			TRANSCODE: {
				"ffmpeg": self.capabilities.version,
				"hwaccel": self.capabilities.hwaccel,
			}
		})
	}

	async fn run(
		&self,
		assignment: Assignment,
		progress: Progress,
	) -> Result<Value, String> {
		if assignment.kind != TRANSCODE {
			return Err(format!("unsupported job kind {}", assignment.kind));
		}
		self.transcode(&assignment, &progress).await
	}
}

fn run_ffmpeg(
	install: &Install,
	source: &Path,
	output: &Path,
	target: &TranscodeOutput,
	duration_ms: i64,
) -> Result<(), String> {
	let inputs = [PathBuf::from(source)];
	let codec = match target {
		TranscodeOutput::Opus { .. } => Codec::Opus,
		TranscodeOutput::Aac { .. } => Codec::Aac,
	};
	let result = ffmpeg::encode(
		install,
		&Encode {
			inputs: &inputs,
			output,
			codec,
			bitrate: Some(target.bitrate()),
			metadata: None,
			cover: None,
			// MP4-only; an Ogg stream has no `moov` atom to move.
			faststart: matches!(target, TranscodeOutput::Aac { .. }),
			// Scaled from the track's own length, which the server put in the
			// job input precisely because the worker has no manifest to read
			// it from — an unabridged chapter is hours of audio.
			timeout: ffmpeg::timeout_for(duration_ms),
		},
	)
	.map_err(|error| error.to_string())?;

	if !result.succeeded() {
		return Err(format!(
			"ffmpeg {}: {}",
			result.describe_status(),
			result.stderr.trim()
		));
	}
	Ok(())
}

/// The hardware encoders this install lists. Best effort: a build that does not
/// answer `-hwaccels` advertises none, which only costs the worker some GPU
/// jobs it would have been offered.
fn probe_hwaccels(install: &Install) -> Vec<String> {
	let Ok(output) = std::process::Command::new(install.dir.join("ffmpeg"))
		.args(["-hide_banner", "-hwaccels"])
		.output()
	else {
		return Vec::new();
	};
	String::from_utf8_lossy(&output.stdout)
		.lines()
		.skip(1)
		.map(str::trim)
		.filter(|line| !line.is_empty())
		.map(ToString::to_string)
		.collect()
}

fn hex_sha256(bytes: &[u8]) -> String {
	let mut hasher = Sha256::new();
	hasher.update(bytes);
	hasher
		.finalize()
		.iter()
		.fold(String::with_capacity(64), |mut acc, byte| {
			use std::fmt::Write;
			let _ = write!(acc, "{byte:02x}");
			acc
		})
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The digest the upload route re-computes and compares. A different
	/// encoding of the same bytes turns every upload into a 400, so the
	/// alphabet and the width are pinned against a published vector.
	#[test]
	fn sha256_is_lowercase_hex() {
		assert_eq!(
			hex_sha256(b""),
			"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
		);
		assert_eq!(
			hex_sha256(b"abc"),
			"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
		);
	}
}
