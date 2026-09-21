//! Optional worker-local Storyteller adapter.
//!
//! The server sends only an [`AlignInput`] and media ids. Storyteller's URL,
//! username, and password live in this worker process and are never serialized
//! into a job, archive, or result. The adapter uploads the server-prepared EPUB
//! and one M4B through TUS, creates one Storyteller book, polls a bounded
//! processing window, and converts the returned EPUB 3 overlays into the
//! server's `SyncMapV1` contract.

use quick_xml::{
	events::{BytesStart, Event},
	Reader,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
	io::{Cursor, Read},
	time::Duration,
};
use stump_media::read_aloud::{self, PreparedEpub};
use zip::ZipArchive;

use crate::{
	alignment::{
		AlignInput, AudioClip, SyncCue, SyncMapProvenance, SyncMapV1, TextFragment,
	},
	client::{Assignment, ClientConfig, JobRunner, Progress},
	kind::ALIGN,
};

/// Worker-local Storyteller connection settings. This type deliberately does
/// not implement `Serialize`, so credentials cannot cross the worker socket.
pub struct StorytellerConfig {
	pub url: String,
	pub username: String,
	pub password: String,
	pub poll_interval: Duration,
	pub max_polls: u32,
	pub version: String,
}

impl StorytellerConfig {
	#[must_use]
	pub fn new(
		url: impl Into<String>,
		username: impl Into<String>,
		password: impl Into<String>,
	) -> Self {
		Self {
			url: url.into(),
			username: username.into(),
			password: password.into(),
			poll_interval: Duration::from_secs(5),
			max_polls: 120,
			version: "storyteller-api-v1".to_owned(),
		}
	}
}

/// A single-threaded Storyteller-backed ALIGN runner.
pub struct StorytellerRunner {
	server: ClientConfig,
	storyteller: StorytellerConfig,
	http: reqwest::Client,
}

impl StorytellerRunner {
	pub fn new(
		server: ClientConfig,
		storyteller: StorytellerConfig,
	) -> Result<Self, String> {
		if storyteller.url.trim().is_empty()
			|| storyteller.username.trim().is_empty()
			|| storyteller.password.is_empty()
		{
			return Err("Storyteller URL, username, and password are all required".into());
		}
		Ok(Self {
			server,
			storyteller,
			http: reqwest::Client::new(),
		})
	}

	async fn align(
		&self,
		assignment: &Assignment,
		progress: &Progress,
	) -> Result<Value, String> {
		let input: AlignInput = serde_json::from_value(assignment.input.clone())
			.map_err(|error| format!("invalid align input: {error}"))?;
		progress.report(0.03, "downloading EPUB");
		let ebook = self.download_media(&input.text_media_id).await?;
		progress.report(0.08, "downloading M4B");
		let audio = self.download_media(&input.audio_media_id).await?;
		let ebook_file =
			tempfile::NamedTempFile::new().map_err(|error| error.to_string())?;
		tokio::fs::write(ebook_file.path(), &ebook)
			.await
			.map_err(|error| error.to_string())?;
		let prepared = tokio::task::spawn_blocking({
			let path = ebook_file.path().to_path_buf();
			move || read_aloud::prepare_epub(path)
		})
		.await
		.map_err(|error| format!("EPUB preparation panicked: {error}"))?
		.map_err(|error| format!("EPUB preparation failed: {error}"))?;
		let prepared_bytes = read_aloud::prepared_epub_bytes(&prepared)
			.map_err(|error| format!("prepared EPUB serialization failed: {error}"))?;

		let token = self.login().await?;
		progress.report(0.15, "uploading prepared EPUB through TUS");
		let epub_upload = self
			.tus_upload(&token, "book.epub", "application/epub+zip", prepared_bytes)
			.await?;
		progress.report(0.27, "uploading M4B through TUS");
		let audio_upload = self
			.tus_upload(&token, "book.m4b", "audio/mp4", audio)
			.await?;
		progress.report(0.35, "creating Storyteller book");
		let book_id = self
			.create_book(&token, &epub_upload, &audio_upload)
			.await?;
		self.process_book(&token, &book_id).await?;
		for attempt in 0..self.storyteller.max_polls {
			progress.report(
				0.4 + (attempt as f64 / self.storyteller.max_polls.max(1) as f64) * 0.45,
				"waiting for Storyteller alignment",
			);
			let status = self.book_status(&token, &book_id).await?;
			if status.finished {
				let output = self
					.download_synced(&token, &book_id, status.output_url)
					.await?;
				let map = parse_storyteller_output(
					&output,
					&prepared,
					&input,
					&self.storyteller.version,
				)?;
				let map_bytes =
					serde_json::to_vec(&map).map_err(|error| error.to_string())?;
				let digest = sha256_hex(&map_bytes);
				self.upload_result(&assignment.id, map_bytes.clone(), &digest)
					.await?;
				progress.report(1.0, "alignment complete");
				return Ok(
					json!({"bytes": map_bytes.len() as u64, "sha256": digest, "mime": "application/vnd.stump.sync-map+json"}),
				);
			}
			if status.failed {
				return Err(status
					.message
					.unwrap_or_else(|| "Storyteller alignment failed".into()));
			}
			tokio::time::sleep(self.storyteller.poll_interval).await;
		}
		Err("Storyteller alignment timed out".into())
	}

	async fn login(&self) -> Result<String, String> {
		#[derive(Deserialize)]
		struct Token {
			access_token: String,
		}
		let url = endpoint(&self.storyteller.url, "/token");
		let response = self
			.http
			.post(url)
			.form(&[
				("username", self.storyteller.username.as_str()),
				("password", self.storyteller.password.as_str()),
			])
			.send()
			.await
			.map_err(|error| format!("Storyteller login failed: {error}"))?;
		if !response.status().is_success() {
			return Err(format!(
				"Storyteller login failed: HTTP {}",
				response.status()
			));
		}
		response
			.json::<Token>()
			.await
			.map(|token| token.access_token)
			.map_err(|error| format!("Storyteller login response was invalid: {error}"))
	}

	async fn download_media(&self, media_id: &str) -> Result<Vec<u8>, String> {
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
		response
			.bytes()
			.await
			.map(|bytes| bytes.to_vec())
			.map_err(|error| format!("media download failed: {error}"))
	}

	async fn tus_upload(
		&self,
		token: &str,
		filename: &str,
		mime: &str,
		bytes: Vec<u8>,
	) -> Result<String, String> {
		use base64::Engine;
		let metadata = format!(
			"filename {},type {}",
			base64::engine::general_purpose::STANDARD.encode(filename),
			base64::engine::general_purpose::STANDARD.encode(mime)
		);
		let response = self
			.http
			.post(endpoint(&self.storyteller.url, "/tus/files"))
			.bearer_auth(token)
			.header("Tus-Resumable", "1.0.0")
			.header("Upload-Length", bytes.len().to_string())
			.header("Upload-Metadata", metadata)
			.send()
			.await
			.map_err(|error| format!("TUS create failed: {error}"))?;
		if !response.status().is_success() && response.status().as_u16() != 201 {
			return Err(format!("TUS create failed: HTTP {}", response.status()));
		}
		let location = response
			.headers()
			.get(reqwest::header::LOCATION)
			.and_then(|value| value.to_str().ok())
			.map(ToOwned::to_owned)
			.or_else(|| None)
			.ok_or_else(|| "TUS create did not return Location".to_owned())?;
		let response = self
			.http
			.patch(resolve_location(&self.storyteller.url, &location))
			.bearer_auth(token)
			.header("Tus-Resumable", "1.0.0")
			.header("Upload-Offset", "0")
			.header(
				reqwest::header::CONTENT_TYPE,
				"application/offset+octet-stream",
			)
			.body(bytes)
			.send()
			.await
			.map_err(|error| format!("TUS upload failed: {error}"))?;
		if !response.status().is_success() {
			return Err(format!("TUS upload failed: HTTP {}", response.status()));
		}
		Ok(location)
	}

	async fn create_book(
		&self,
		token: &str,
		epub_upload: &str,
		audio_upload: &str,
	) -> Result<String, String> {
		let response = self
			.http
			.post(endpoint(&self.storyteller.url, "/books"))
			.bearer_auth(token)
			.json(&json!({"epub_upload": epub_upload, "audio_upload": audio_upload}))
			.send()
			.await
			.map_err(|error| format!("Storyteller book creation failed: {error}"))?;
		if !response.status().is_success() {
			return Err(format!(
				"Storyteller book creation failed: HTTP {}",
				response.status()
			));
		}
		let body: Value = response.json().await.map_err(|error| error.to_string())?;
		value_string(&body, &["id", "book_id", "uuid"])
			.ok_or_else(|| "Storyteller book creation returned no id".into())
	}

	async fn process_book(&self, token: &str, book_id: &str) -> Result<(), String> {
		let response = self
			.http
			.post(endpoint(
				&self.storyteller.url,
				&format!("/books/{book_id}/process"),
			))
			.bearer_auth(token)
			.send()
			.await
			.map_err(|error| format!("Storyteller process request failed: {error}"))?;
		if !response.status().is_success() {
			return Err(format!(
				"Storyteller process request failed: HTTP {}",
				response.status()
			));
		}
		Ok(())
	}

	async fn book_status(
		&self,
		token: &str,
		book_id: &str,
	) -> Result<BookStatus, String> {
		let response = self
			.http
			.get(endpoint(
				&self.storyteller.url,
				&format!("/books/{book_id}"),
			))
			.bearer_auth(token)
			.send()
			.await
			.map_err(|error| format!("Storyteller status request failed: {error}"))?;
		if !response.status().is_success() {
			return Err(format!(
				"Storyteller status request failed: HTTP {}",
				response.status()
			));
		}
		let body: Value = response.json().await.map_err(|error| error.to_string())?;
		let status = body
			.get("status")
			.or_else(|| body.get("processing_status"))
			.and_then(Value::as_str)
			.unwrap_or_default()
			.to_ascii_lowercase();
		Ok(BookStatus {
			finished: matches!(
				status.as_str(),
				"complete" | "completed" | "done" | "finished" | "success"
			),
			failed: matches!(status.as_str(), "failed" | "error" | "cancelled"),
			output_url: value_string(
				&body,
				&["synced_url", "output_url", "download_url"],
			),
			message: value_string(&body, &["error", "message"]),
		})
	}

	async fn download_synced(
		&self,
		token: &str,
		book_id: &str,
		output_url: Option<String>,
	) -> Result<Vec<u8>, String> {
		let url = output_url.unwrap_or_else(|| {
			endpoint(&self.storyteller.url, &format!("/books/{book_id}/synced"))
		});
		let response = self
			.http
			.get(resolve_location(&self.storyteller.url, &url))
			.bearer_auth(token)
			.send()
			.await
			.map_err(|error| format!("Storyteller output download failed: {error}"))?;
		if !response.status().is_success() {
			return Err(format!(
				"Storyteller output download failed: HTTP {}",
				response.status()
			));
		}
		response
			.bytes()
			.await
			.map(|bytes| bytes.to_vec())
			.map_err(|error| error.to_string())
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

#[async_trait::async_trait]
impl JobRunner for StorytellerRunner {
	fn capabilities(&self) -> Value {
		// `align_requires` is intentionally profile-shaped. Storyteller is an
		// external implementation of the stable CPU/default profile, not a
		// new server execution provider.
		json!({
			ALIGN: {
				"device": "cpu",
				"models": ["default"],
				"algorithms": ["ctc"],
				"precisions": ["fp32"],
				"implementation": "storyteller"
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

struct BookStatus {
	finished: bool,
	failed: bool,
	output_url: Option<String>,
	message: Option<String>,
}

fn endpoint(base: &str, path: &str) -> String {
	format!(
		"{}/{}",
		base.trim_end_matches('/'),
		path.trim_start_matches('/')
	)
}

fn resolve_location(base: &str, location: &str) -> String {
	if location.starts_with("http://") || location.starts_with("https://") {
		location.to_owned()
	} else {
		endpoint(base, location)
	}
}

fn value_string(body: &Value, keys: &[&str]) -> Option<String> {
	keys.iter().find_map(|key| {
		body.get(*key)
			.and_then(|value| value.as_str())
			.map(ToOwned::to_owned)
	})
}

fn parse_storyteller_output(
	bytes: &[u8],
	prepared: &PreparedEpub,
	input: &AlignInput,
	version: &str,
) -> Result<SyncMapV1, String> {
	if let Ok(mut map) = serde_json::from_slice::<SyncMapV1>(bytes) {
		// The external service may include its own provenance envelope. The
		// queued request is authoritative for source/profile identity; cues
		// still go through the server's target and duration validator.
		map.schema = 1;
		map.text_media_id = input.text_media_id.clone();
		map.audio_media_id = input.audio_media_id.clone();
		map.provenance.text_digest = input.text_digest.clone();
		map.provenance.audio_manifest_digest = input.audio_manifest_digest.clone();
		map.provenance.algorithm = input.algorithm.clone();
		map.provenance.model = input.model.clone();
		map.provenance.model_revision = input.model_revision.clone();
		map.provenance.language = input.language.clone();
		map.provenance.granularity = input.granularity;
		map.provenance.execution_provider = input.execution_provider;
		map.provenance.precision = input.precision;
		map.provenance.options = input.options.clone();
		return Ok(map);
	}
	let mut archive = ZipArchive::new(Cursor::new(bytes))
		.map_err(|error| format!("Storyteller output is not an EPUB: {error}"))?;
	let mut cues = Vec::new();
	for index in 0..archive.len() {
		let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
		if entry.is_dir() || !entry.name().to_ascii_lowercase().ends_with(".smil") {
			continue;
		}
		let mut smil = Vec::new();
		entry
			.read_to_end(&mut smil)
			.map_err(|error| error.to_string())?;
		parse_smil(&smil, prepared, &mut cues)?;
	}
	cues.sort_by_key(|cue: &SyncCue| {
		(
			cue.text.spine_index,
			cue.text.ordinal,
			cue.audio.track_index,
			cue.audio.begin_ms,
		)
	});
	if cues.is_empty() {
		return Err("Storyteller output has no SMIL cues".into());
	}
	Ok(SyncMapV1 {
		schema: 1,
		text_media_id: input.text_media_id.clone(),
		audio_media_id: input.audio_media_id.clone(),
		provenance: SyncMapProvenance {
			text_digest: input.text_digest.clone(),
			audio_manifest_digest: input.audio_manifest_digest.clone(),
			implementation: "storyteller".into(),
			version: version.to_owned(),
			algorithm: input.algorithm.clone(),
			model: input.model.clone(),
			model_revision: input.model_revision.clone(),
			language: input.language.clone(),
			granularity: input.granularity,
			execution_provider: input.execution_provider,
			precision: input.precision,
			options: input.options.clone(),
		},
		cues,
	})
}

fn parse_smil(
	bytes: &[u8],
	prepared: &PreparedEpub,
	cues: &mut Vec<SyncCue>,
) -> Result<(), String> {
	let mut reader = Reader::from_reader(bytes);
	reader.config_mut().trim_text(true);
	let mut buffer = Vec::new();
	let mut current_text: Option<(String, i32)> = None;
	let mut current_audio: Option<AudioClip> = None;
	loop {
		match reader.read_event_into(&mut buffer) {
			Ok(Event::Start(event)) | Ok(Event::Empty(event)) => {
				match local_name(event.name().as_ref()) {
					"text" => {
						if let Some(src) = attr(&event, "src") {
							let (path, id) = src
								.split_once('#')
								.map(|(path, id)| (path, id))
								.unwrap_or((src.as_str(), ""));
							let normalized = normalize(path);
							if let Some(spine) = prepared.spines.iter().find(|spine| {
								normalize(&spine.package_path).ends_with(&normalized)
							}) {
								let target = spine
									.targets
									.iter()
									.find(|target| target.element_id == id)
									.ok_or_else(|| {
										format!("Storyteller returned an unprepared target {id}")
									})?;
								current_text =
									Some((target.element_id.clone(), spine.spine_index));
							}
						}
					},
					"audio" => {
						let begin_ms = attr(&event, "clipBegin")
							.and_then(|value| parse_time_ms(&value))
							.ok_or_else(|| {
								"SMIL audio clipBegin is missing".to_owned()
							})?;
						let end_ms = attr(&event, "clipEnd")
							.and_then(|value| parse_time_ms(&value))
							.ok_or_else(|| "SMIL audio clipEnd is missing".to_owned())?;
						current_audio = Some(AudioClip {
							track_index: 0,
							begin_ms,
							end_ms,
						});
					},
					_ => {},
				}
			},
			Ok(Event::End(event)) if local_name(event.name().as_ref()) == "par" => {
				if let (Some((element_id, spine_index)), Some(audio)) =
					(current_text.take(), current_audio.take())
				{
					let ordinal = prepared
						.spines
						.iter()
						.find(|spine| spine.spine_index == spine_index)
						.and_then(|spine| {
							spine
								.targets
								.iter()
								.find(|target| target.element_id == element_id)
						})
						.map(|target| target.ordinal)
						.ok_or_else(|| "prepared target disappeared".to_owned())?;
					cues.push(SyncCue {
						text: TextFragment {
							spine_index,
							ordinal,
							element_id,
						},
						audio,
						confidence: None,
					});
				}
			},
			Ok(Event::Eof) => break,
			Err(error) => return Err(format!("invalid Storyteller SMIL: {error}")),
			_ => {},
		}
		buffer.clear();
	}
	Ok(())
}

fn attr(event: &BytesStart<'_>, wanted: &str) -> Option<String> {
	event
		.attributes()
		.with_checks(false)
		.flatten()
		.find(|attr| local_name(attr.key.as_ref()) == wanted)
		.map(|attr| String::from_utf8_lossy(&attr.value).into_owned())
}

fn local_name(name: &[u8]) -> &str {
	std::str::from_utf8(name)
		.unwrap_or_default()
		.rsplit(':')
		.next()
		.unwrap_or_default()
}

fn normalize(path: &str) -> String {
	path.trim_start_matches('/')
		.split('/')
		.filter(|part| !part.is_empty() && *part != ".")
		.fold(Vec::new(), |mut output, part| {
			if part == ".." {
				let _ = output.pop();
			} else {
				output.push(part);
			}
			output
		})
		.join("/")
}

fn parse_time_ms(value: &str) -> Option<i64> {
	let value = value.trim();
	if let Some(value) = value.strip_suffix("ms") {
		return value.trim().parse().ok();
	}
	if let Some(value) = value.strip_suffix('s') {
		return value
			.trim()
			.parse::<f64>()
			.ok()
			.map(|seconds| (seconds * 1000.0).round() as i64);
	}
	let parts = value.split(':').collect::<Vec<_>>();
	if parts.len() == 3 {
		let hours = parts[0].parse::<i64>().ok()?;
		let minutes = parts[1].parse::<i64>().ok()?;
		let seconds = parts[2].parse::<f64>().ok()?;
		return Some(
			hours * 3_600_000 + minutes * 60_000 + (seconds * 1000.0).round() as i64,
		);
	}
	value.parse::<i64>().ok()
}

fn sha256_hex(bytes: &[u8]) -> String {
	use sha2::{Digest, Sha256};
	let mut digest = Sha256::new();
	digest.update(bytes);
	digest
		.finalize()
		.iter()
		.map(|byte| format!("{byte:02x}"))
		.collect()
}
