//! Worker-local native alignment adapter.
//!
//! The executable and model are operator-provided. The server sends only the
//! existing `AlignInput`; this runner downloads the authorized media, prepares
//! the EPUB, and invokes the external process with local temporary paths. The
//! external process must write a typed `SyncMapV1` JSON artifact to `--output`
//! (stdout is accepted when the file is left empty).

use std::{path::PathBuf, process::Command};

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use stump_media::read_aloud;
use tokio::io::AsyncWriteExt;

use crate::{
	client::{Assignment, ClientConfig, JobRunner, Progress},
	AlignExecutionProvider, AlignInput, AlignPrecision, SyncMapProvenance, SyncMapV1,
	ALIGN, SYNC_MAP_MIME, SYNC_MAP_SCHEMA_VERSION,
};
/// Configuration that remains local to one native worker process.
#[derive(Debug, Clone)]
pub struct NativeAlignConfig {
	pub executable: PathBuf,
	pub model_path: PathBuf,
	pub model: String,
	pub model_revision: String,
	pub implementation: String,
	pub version: String,
}

impl NativeAlignConfig {
	/// Validate the executable/model paths before advertising ALIGN capability.
	pub fn new(
		executable: impl Into<PathBuf>,
		model_path: impl Into<PathBuf>,
		model: impl Into<String>,
		model_revision: impl Into<String>,
	) -> Result<Self, String> {
		let executable = executable.into();
		let model_path = model_path.into();
		if !executable.is_file() {
			return Err(format!(
				"native aligner executable is not a regular file: {}",
				executable.display()
			));
		}
		if !model_path.exists() {
			return Err(format!(
				"native aligner model path does not exist: {}",
				model_path.display()
			));
		}
		Ok(Self {
			executable,
			model_path,
			model: model.into(),
			model_revision: model_revision.into(),
			implementation: "native-ctc".to_owned(),
			version: "coppice-native-align-v1".to_owned(),
		})
	}
}

/// External-process implementation of the stable CPU/fp32 ALIGN profile.
pub struct NativeAlignRunner {
	server: ClientConfig,
	config: NativeAlignConfig,
	http: reqwest::Client,
}

impl NativeAlignRunner {
	pub fn new(server: ClientConfig, config: NativeAlignConfig) -> Self {
		Self {
			server,
			config,
			http: reqwest::Client::new(),
		}
	}

	async fn align(
		&self,
		assignment: &Assignment,
		progress: &Progress,
	) -> Result<Value, String> {
		let input: AlignInput = serde_json::from_value(assignment.input.clone())
			.map_err(|error| format!("invalid align input: {error}"))?;
		if input.algorithm != "ctc"
			|| input.model != self.config.model
			|| input.model_revision != self.config.model_revision
			|| input.execution_provider != AlignExecutionProvider::Cpu
			|| input.precision != AlignPrecision::Fp32
		{
			return Err("native aligner does not support this ALIGN profile".into());
		}
		let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
		let source_epub = temp.path().join("source.epub");
		let prepared_epub = temp.path().join("prepared.epub");
		let audio_path = temp.path().join("audio.m4b");
		let output_path = temp.path().join("sync-map.json");
		progress.report(0.03, "downloading EPUB");
		self.download_media_to_path(&input.text_media_id, &source_epub)
			.await?;
		progress.report(0.08, "downloading M4B");
		self.download_media_to_path(&input.audio_media_id, &audio_path)
			.await?;
		let prepared = tokio::task::spawn_blocking({
			let source_epub = source_epub.clone();
			move || read_aloud::prepare_epub(source_epub)
		})
		.await
		.map_err(|error| format!("EPUB preparation panicked: {error}"))?
		.map_err(|error| format!("EPUB preparation failed: {error}"))?;
		let prepared_bytes = read_aloud::prepared_epub_bytes(&prepared)
			.map_err(|error| format!("prepared EPUB serialization failed: {error}"))?;
		tokio::fs::write(&prepared_epub, prepared_bytes)
			.await
			.map_err(|error| format!("write prepared EPUB: {error}"))?;
		progress.report(0.2, "running native aligner");
		let process = run_external(
			self.config.executable.clone(),
			self.config.model_path.clone(),
			self.config.model.clone(),
			self.config.model_revision.clone(),
			self.config.implementation.clone(),
			self.config.version.clone(),
			input.language.clone(),
			match input.granularity {
				crate::AlignGranularity::Sentence => "sentence".to_owned(),
				crate::AlignGranularity::Word => "word".to_owned(),
			},
			prepared_epub,
			audio_path,
			output_path.clone(),
		)
		.await?;
		let mut map: SyncMapV1 = serde_json::from_slice(&process).map_err(|error| {
			format!("native aligner returned invalid SyncMapV1: {error}")
		})?;
		if map.schema != SYNC_MAP_SCHEMA_VERSION || map.cues.is_empty() {
			return Err(
				"native aligner returned an empty or unsupported SyncMapV1".into()
			);
		}
		map.text_media_id = input.text_media_id.clone();
		map.audio_media_id = input.audio_media_id.clone();
		map.provenance = SyncMapProvenance {
			text_digest: input.text_digest.clone(),
			audio_manifest_digest: input.audio_manifest_digest.clone(),
			implementation: self.config.implementation.clone(),
			version: self.config.version.clone(),
			algorithm: input.algorithm,
			model: input.model,
			model_revision: self.config.model_revision.clone(),
			language: input.language,
			granularity: input.granularity,
			execution_provider: input.execution_provider,
			precision: input.precision,
			options: input.options,
		};
		let bytes = serde_json::to_vec(&map).map_err(|error| error.to_string())?;
		let digest = sha256_hex(&bytes);
		self.upload_result(&assignment.id, bytes.clone(), &digest)
			.await?;
		progress.report(1.0, "alignment complete");
		Ok(json!({
			"bytes": bytes.len() as u64,
			"sha256": digest,
			"mime": SYNC_MAP_MIME,
		}))
	}

	async fn download_media_to_path(
		&self,
		media_id: &str,
		destination: &std::path::Path,
	) -> Result<(), String> {
		let url = format!(
			"{}/api/v2/media/{media_id}/file",
			self.server.server.trim_end_matches('/')
		);
		let response = self
			.http
			.get(url)
			.bearer_auth(&self.server.api_key)
			.send()
			.await
			.map_err(|error| format!("media download failed: {error}"))?;
		if !response.status().is_success() {
			return Err(format!("media download failed: HTTP {}", response.status()));
		}
		let mut stream = response.bytes_stream();
		let mut file = tokio::fs::File::create(destination)
			.await
			.map_err(|error| format!("create media download path: {error}"))?;
		while let Some(chunk) = stream.next().await {
			let chunk =
				chunk.map_err(|error| format!("media download failed: {error}"))?;
			file.write_all(&chunk)
				.await
				.map_err(|error| format!("write media download: {error}"))?;
		}
		file.flush()
			.await
			.map_err(|error| format!("flush media download: {error}"))?;
		Ok(())
	}

	async fn upload_result(
		&self,
		job_id: &str,
		body: Vec<u8>,
		digest: &str,
	) -> Result<(), String> {
		let response = self
			.http
			.put(self.server.upload_url(job_id))
			.bearer_auth(&self.server.api_key)
			.header("x-stump-sha256", digest)
			.header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
			.body(body)
			.send()
			.await
			.map_err(|error| format!("alignment result upload failed: {error}"))?;
		if !response.status().is_success() {
			return Err(format!(
				"alignment result upload failed: HTTP {}",
				response.status()
			));
		}
		Ok(())
	}
}

#[async_trait]
impl JobRunner for NativeAlignRunner {
	fn capabilities(&self) -> Value {
		json!({
			ALIGN: {
				"device": "cpu",
				"models": [self.config.model],
				"algorithms": ["ctc"],
				"precisions": ["fp32"],
				"implementation": self.config.implementation,
				"model_revision": self.config.model_revision
			}
		})
	}

	async fn run(
		&self,
		assignment: Assignment,
		progress: Progress,
	) -> Result<Value, String> {
		if assignment.kind != ALIGN {
			return Err(format!("unsupported job kind {}", assignment.kind));
		}
		self.align(&assignment, &progress).await
	}
}

async fn run_external(
	executable: PathBuf,
	model_path: PathBuf,
	model: String,
	model_revision: String,
	implementation: String,
	version: String,
	language: String,
	granularity: String,
	epub: PathBuf,
	audio: PathBuf,
	output: PathBuf,
) -> Result<Vec<u8>, String> {
	tokio::task::spawn_blocking(move || {
		let process = Command::new(&executable)
			.arg("--epub")
			.arg(&epub)
			.arg("--audio")
			.arg(&audio)
			.arg("--model")
			.arg(&model_path)
			.arg("--model-id")
			.arg(&model)
			.arg("--model-revision")
			.arg(&model_revision)
			.arg("--implementation")
			.arg(&implementation)
			.arg("--version")
			.arg(&version)
			.arg("--language")
			.arg(&language)
			.arg("--granularity")
			.arg(&granularity)
			.arg("--output")
			.arg(&output)
			.output()
			.map_err(|error| format!("start native aligner: {error}"))?;
		if !process.status.success() {
			return Err(format!(
				"native aligner exited with {}: {}",
				process.status,
				String::from_utf8_lossy(&process.stderr)
			));
		}
		let bytes = std::fs::read(&output).unwrap_or_default();
		if bytes.is_empty() {
			if process.stdout.is_empty() {
				return Err("native aligner produced no SyncMapV1 output".into());
			}
			Ok(process.stdout)
		} else {
			Ok(bytes)
		}
	})
	.await
	.map_err(|error| format!("native aligner task failed: {error}"))?
}

fn sha256_hex(bytes: &[u8]) -> String {
	use sha2::{Digest, Sha256};
	Sha256::digest(bytes)
		.iter()
		.map(|byte| format!("{byte:02x}"))
		.collect()
}
