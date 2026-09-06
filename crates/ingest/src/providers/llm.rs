//! AI metadata provider backed by any OpenAI-compatible chat-completions API
//! (`{base_url}/v1/chat/completions` with `response_format: json_schema`).
//!
//! The provider is strictly evidence-driven: it parses a snapshot's filename
//! plus embedded metadata text into structured fields and can reformulate a
//! free-text search query, but it never mints external identifiers.  Its
//! `identify` identity is therefore the local source digest (like
//! [`super::builtin_embedded::EmbeddedProvider`]) and `search` always returns
//! zero hits — the LLM's reformulated query is only useful as input for other
//! providers (see the follow-up note on `rank_candidates` below).
//!
//! Settings (persisted through the regular ingest plugin-setting mechanism):
//! `enabled` (default `false`), `base_url` (API root *without* the `/v1`
//! suffix), `model`, and `api_key` (secret; may be empty for local servers).
//!
//! Follow-up (needs a façade hook that does not exist yet): a
//! `rank_candidates(candidates) -> ordered` hook so the LLM can re-score and
//! normalise candidates from other providers, and registry plumbing that feeds
//! the reformulated query from [`LlmProvider::search`] into the remote
//! providers' `search` implementations.

use std::{
	collections::BTreeMap,
	sync::{LazyLock, RwLock},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use stump_api_types::settings::{SettingDefinition, SettingKind, SettingValues};

use crate::contract::{
	BookSnapshot, IngestMediaKind, IngestMetadataProvider, MetadataCandidate,
	MetadataField, ProviderCapability, ProviderError, ProviderIdentity, SearchHit,
	SearchQuery,
};

pub const LLM_PROVIDER_ID: &str = "llm";
pub const LLM_PROVIDER_VERSION: &str = "llm-1";

pub const SETTING_ENABLED: &str = "enabled";
pub const SETTING_BASE_URL: &str = "base_url";
pub const SETTING_API_KEY: &str = "api_key";
pub const SETTING_MODEL: &str = "model";
pub const SETTING_EXTRA_INSTRUCTIONS: &str = "extra_instructions";

/// Confidence assigned to every LLM-derived field.  Model output is plausible
/// rather than authoritative, so it sits at the filename-parser level (0.6),
/// never at the embedded-metadata level (1.0).
const LLM_CONFIDENCE: f64 = 0.6;

const IDENTIFY_SYSTEM_PROMPT: &str = "\
You extract bibliographic metadata for a book, comic, or manga from a filename \
and an embedded metadata excerpt. Use only the evidence provided by the user \
message; never invent values, ISBNs, or external identifiers. If a field is \
not present in the evidence, set it to null. Respond only with JSON that \
matches the requested JSON schema.";

const REFORMULATE_SYSTEM_PROMPT: &str = "\
You rewrite a raw search string (often a media filename) into a concise \
bibliographic search query suitable for a metadata API. Use only the text \
provided; never add ISBNs, external ids, or details not present. Respond only \
with JSON that matches the requested JSON schema.";

static METADATA_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
	json!({
		"type": "object",
		"properties": {
			"title": {
				"type": ["string", "null"],
				"description": "Full title of the work, or null if unknown.",
			},
			"series": {
				"type": ["string", "null"],
				"description": "Series name, or null if unknown.",
			},
			"number": {
				"type": ["string", "null"],
				"description": "Issue/volume number exactly as printed, e.g. \"3\" or \"3.5\", or null.",
			},
			"year": {
				"type": ["integer", "null"],
				"description": "Publication year, or null if unknown.",
			},
			"authors": {
				"type": "array",
				"items": {"type": "string"},
				"description": "Author/writer names; empty when unknown.",
			},
			"language": {
				"type": ["string", "null"],
				"description": "Language, e.g. \"en\" or \"German\", or null if unknown.",
			},
		},
		"required": ["title", "series", "number", "year", "authors", "language"],
		"additionalProperties": false,
	})
});

static REFORMULATE_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
	json!({
		"type": "object",
		"properties": {
			"search_query": {
				"type": "string",
				"description": "The rewritten, concise search query.",
			},
		},
		"required": ["search_query"],
		"additionalProperties": false,
	})
});

static SETTINGS: LazyLock<Vec<SettingDefinition>> = LazyLock::new(|| {
	vec![
		SettingDefinition {
			key: SETTING_ENABLED,
			label: "Enabled",
			description: "Enable AI metadata extraction via an OpenAI-compatible chat API.",
			kind: SettingKind::Bool,
			default: Value::Bool(false),
			required: false,
			secret: false,
			help_url: None,
		},
		SettingDefinition {
			key: SETTING_BASE_URL,
			label: "Base URL",
			description: "OpenAI-compatible API root without the /v1 suffix, e.g. https://api.openai.com",
			kind: SettingKind::String,
			default: Value::String(String::new()),
			required: true,
			secret: false,
			help_url: None,
		},
		SettingDefinition {
			key: SETTING_MODEL,
			label: "Model",
			description: "Chat model name, e.g. gpt-4o-mini or a local model id.",
			kind: SettingKind::String,
			default: Value::String(String::new()),
			required: true,
			secret: false,
			help_url: None,
		},
		SettingDefinition {
			key: SETTING_API_KEY,
			label: "API key",
			description: "Bearer token; any OpenAI-compatible endpoint works — OpenAI (https://platform.openai.com/api-keys), DeepSeek (https://platform.deepseek.com), OpenRouter, or a local server like Ollama with an empty key.",
			kind: SettingKind::String,
			default: Value::String(String::new()),
			required: false,
			secret: true,
			help_url: Some("https://platform.openai.com/api-keys"),
		},
		SettingDefinition {
			key: SETTING_EXTRA_INSTRUCTIONS,
			label: "Extra instructions",
			description: "Free-form guidance appended to the system prompt, e.g. naming conventions like `Series - vNN`.",
			kind: SettingKind::String,
			default: Value::String(String::new()),
			required: false,
			secret: false,
			help_url: None,
		},
	]
});

/// Effective connection settings validated from [`SettingValues`].
struct LlmConfig {
	base_url: String,
	api_key: Option<String>,
	model: String,
	extra_instructions: Option<String>,
}

impl LlmConfig {
	fn from_settings(settings: &SettingValues) -> Result<Self, ProviderError> {
		let not_configured = |message: String| ProviderError::NotConfigured {
			provider_id: LLM_PROVIDER_ID.to_string(),
			message,
		};
		if !settings
			.get(SETTING_ENABLED)
			.and_then(Value::as_bool)
			.unwrap_or(false)
		{
			return Err(not_configured("provider is not enabled".to_string()));
		}
		let base_url = string_setting(settings, SETTING_BASE_URL)
			.ok_or_else(|| not_configured("base_url is not set".to_string()))?;
		let model = string_setting(settings, SETTING_MODEL)
			.ok_or_else(|| not_configured("model is not set".to_string()))?;
		Ok(Self {
			base_url,
			api_key: string_setting(settings, SETTING_API_KEY),
			model,
			extra_instructions: string_setting(settings, SETTING_EXTRA_INSTRUCTIONS),
		})
	}
}
/// Compose the final system prompt: the base instruction block plus, when
/// the library owner supplied `extra_instructions`, that text as a final
/// paragraph under a fixed attribution header.
fn compose_system_prompt(base: &str, extra_instructions: Option<&str>) -> String {
	match extra_instructions {
		Some(extra) => {
			format!("{base}\n\nAdditional instructions from the library owner:\n{extra}")
		},
		None => base.to_string(),
	}
}

fn string_setting(settings: &SettingValues, key: &str) -> Option<String> {
	settings
		.get(key)
		.and_then(Value::as_str)
		.map(str::trim)
		.filter(|value| !value.is_empty())
		.map(str::to_string)
}

/// Raw model output before normalisation.  Values are kept as JSON so a
/// non-strict server sending `"year": "2014"` or `"number": 3` still parses.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawBookMetadata {
	title: Option<Value>,
	series: Option<Value>,
	number: Option<Value>,
	year: Option<Value>,
	authors: Option<Value>,
	language: Option<Value>,
}

/// Normalised, evidence-bound metadata extracted by the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
struct BookMetadata {
	title: Option<String>,
	series: Option<String>,
	number: Option<f64>,
	year: Option<i32>,
	authors: Vec<String>,
	language: Option<String>,
}

impl BookMetadata {
	fn from_raw(raw: RawBookMetadata) -> Self {
		Self {
			title: clean_string(&raw.title),
			series: clean_string(&raw.series),
			number: clean_number(&raw.number),
			year: clean_year(&raw.year),
			authors: clean_strings(&raw.authors),
			language: clean_string(&raw.language),
		}
	}
}

fn clean_string(value: &Option<Value>) -> Option<String> {
	value
		.as_ref()
		.and_then(Value::as_str)
		.map(str::trim)
		.filter(|value| !value.is_empty())
		.map(str::to_string)
}

fn clean_number(value: &Option<Value>) -> Option<f64> {
	match value.as_ref() {
		Some(Value::Number(number)) => number.as_f64(),
		Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
		_ => None,
	}
	.filter(|value| value.is_finite())
}

fn clean_year(value: &Option<Value>) -> Option<i32> {
	match value.as_ref() {
		Some(Value::Number(number)) => {
			number.as_i64().and_then(|year| i32::try_from(year).ok())
		},
		Some(Value::String(text)) => text.trim().parse::<i32>().ok(),
		_ => None,
	}
}

fn clean_strings(value: &Option<Value>) -> Vec<String> {
	value
		.as_ref()
		.and_then(Value::as_array)
		.map(|values| {
			values
				.iter()
				.map(|value| match value {
					Value::String(text) => Some(text.clone()),
					other => other.as_str().map(str::to_string),
				})
				.collect::<Option<Vec<_>>>()
				.unwrap_or_default()
		})
		.unwrap_or_default()
		.into_iter()
		.map(|value| value.trim().to_string())
		.filter(|value| !value.is_empty())
		.collect()
}

#[derive(Debug, Deserialize)]
struct ChatCompletion {
	choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
	message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
	content: Option<String>,
}

/// AI provider over any OpenAI-compatible chat-completions endpoint.  The
/// `search` trait method carries no settings parameter, so the registry can
/// inject the effective settings for it via [`LlmProvider::set_search_settings`].
pub struct LlmProvider {
	client: reqwest::Client,
	search_settings: RwLock<SettingValues>,
}

impl LlmProvider {
	pub fn new() -> Self {
		Self {
			client: reqwest::Client::builder()
				.timeout(std::time::Duration::from_secs(60))
				.build()
				.expect("reqwest client with rustls must build"),
			search_settings: RwLock::new(SettingValues::new()),
		}
	}

	// Unused until the registry's search plumbing lands (see module docs);
	// `search` is the only trait method without a settings parameter, so
	// this setter is how its configuration will be injected.
	#[allow(dead_code)]
	/// Store the effective settings used by [`LlmProvider::search`], whose
	/// trait signature has no settings parameter.  `identify`/`lookup` ignore
	/// this and use their per-call settings.
	pub fn set_search_settings(&self, settings: SettingValues) {
		*self
			.search_settings
			.write()
			.unwrap_or_else(std::sync::PoisonError::into_inner) = settings;
	}

	async fn identify_metadata(
		&self,
		config: &LlmConfig,
		book: &BookSnapshot,
	) -> Result<BookMetadata, ProviderError> {
		let user = metadata_user_prompt(book);
		let content = self
			.complete(
				config,
				&compose_system_prompt(
					IDENTIFY_SYSTEM_PROMPT,
					config.extra_instructions.as_deref(),
				),
				&user,
				"book_metadata",
				&METADATA_SCHEMA,
			)
			.await?;
		let raw: RawBookMetadata =
			serde_json::from_value(content).map_err(|error| ProviderError::Request {
				provider_id: LLM_PROVIDER_ID.to_string(),
				message: format!("model returned unexpected metadata shape: {error}"),
			})?;
		Ok(BookMetadata::from_raw(raw))
	}

	/// One strict-JSON chat completion; returns the parsed JSON content.
	/// Verify the configured endpoint answers a minimal structured completion.
	/// One cheap request; the model only has to echo `{"ok": true}`.
	pub async fn verify(&self, settings: &SettingValues) -> Result<(), ProviderError> {
		let config = LlmConfig::from_settings(settings)?;
		let schema = json!({
			"type": "object",
			"properties": { "ok": { "type": "boolean" } },
			"required": ["ok"],
			"additionalProperties": false
		});
		let value = self
			.complete(
				&config,
				"Respond only with JSON matching the requested schema.",
				"Reply with {\"ok\": true}.",
				"verify",
				&schema,
			)
			.await?;
		if value.get("ok").and_then(Value::as_bool) == Some(true) {
			Ok(())
		} else {
			Err(ProviderError::Request {
				provider_id: LLM_PROVIDER_ID.to_string(),
				message: "model did not return the expected verification payload"
					.to_string(),
			})
		}
	}

	async fn complete(
		&self,
		config: &LlmConfig,
		system: &str,
		user: &str,
		schema_name: &str,
		schema: &Value,
	) -> Result<Value, ProviderError> {
		let url = format!(
			"{}/v1/chat/completions",
			config.base_url.trim_end_matches('/')
		);
		let body = json!({
			"model": &config.model,
			"messages": [
				{"role": "system", "content": system},
				{"role": "user", "content": user},
			],
			"temperature": 0.0,
			"response_format": {
				"type": "json_schema",
				"json_schema": {
					"name": schema_name,
					"strict": true,
					"schema": schema,
				},
			},
		});
		let mut request = self.client.post(&url).json(&body);
		if let Some(api_key) = &config.api_key {
			request = request.bearer_auth(api_key);
		}
		let response = request
			.send()
			.await
			.map_err(|error| ProviderError::Request {
				provider_id: LLM_PROVIDER_ID.to_string(),
				message: error.to_string(),
			})?;
		let status = response.status();
		if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
			return Err(ProviderError::RateLimited {
				provider_id: LLM_PROVIDER_ID.to_string(),
			});
		}
		if !status.is_success() {
			return Err(ProviderError::Request {
				provider_id: LLM_PROVIDER_ID.to_string(),
				message: format!("chat completion failed with HTTP {status}"),
			});
		}
		let envelope: ChatCompletion =
			response
				.json()
				.await
				.map_err(|error| ProviderError::Request {
					provider_id: LLM_PROVIDER_ID.to_string(),
					message: format!("invalid JSON response body: {error}"),
				})?;
		let content = envelope
			.choices
			.into_iter()
			.next()
			.ok_or_else(|| ProviderError::Request {
				provider_id: LLM_PROVIDER_ID.to_string(),
				message: "chat completion contained no choices".to_string(),
			})?
			.message
			.content
			.filter(|content| !content.trim().is_empty())
			.ok_or_else(|| ProviderError::Request {
				provider_id: LLM_PROVIDER_ID.to_string(),
				message: "chat completion contained no content".to_string(),
			})?;
		parse_content_json(LLM_PROVIDER_ID, &content)
	}

	fn candidate(
		&self,
		config: &LlmConfig,
		book: &BookSnapshot,
		metadata: &BookMetadata,
	) -> MetadataCandidate {
		let mut fields = BTreeMap::new();
		let mut field_confidence = BTreeMap::new();
		let mut insert_string = |field: MetadataField, value: &Option<String>| {
			if let Some(value) = value {
				fields.insert(field, Value::String(value.clone()));
				field_confidence.insert(field, LLM_CONFIDENCE);
			}
		};
		insert_string(MetadataField::Title, &metadata.title);
		insert_string(MetadataField::Series, &metadata.series);
		insert_string(MetadataField::Language, &metadata.language);
		if let Some(number) = metadata.number {
			fields.insert(MetadataField::SeriesIndex, json!(number));
			field_confidence.insert(MetadataField::SeriesIndex, LLM_CONFIDENCE);
		}
		if let Some(year) = metadata.year {
			fields.insert(
				MetadataField::PublishedDate,
				Value::String(format!("{year:04}")),
			);
			field_confidence.insert(MetadataField::PublishedDate, LLM_CONFIDENCE);
		}
		if !metadata.authors.is_empty() {
			fields.insert(MetadataField::Authors, json!(metadata.authors));
			field_confidence.insert(MetadataField::Authors, LLM_CONFIDENCE);
		}
		MetadataCandidate {
			provider_id: LLM_PROVIDER_ID.to_string(),
			provider_version: LLM_PROVIDER_VERSION.to_string(),
			external_id: Some(book.source_sha256.clone()),
			source_sha256: book.source_sha256.clone(),
			confidence: LLM_CONFIDENCE,
			fields,
			field_confidence,
			provenance: json!({
				"source": "llm",
				"model": &config.model,
			}),
		}
	}
}

impl Default for LlmProvider {
	fn default() -> Self {
		Self::new()
	}
}

fn media_kind_label(kind: IngestMediaKind) -> String {
	match serde_json::to_value(kind) {
		Ok(Value::String(label)) => label,
		_ => format!("{kind:?}"),
	}
}

fn metadata_user_prompt(book: &BookSnapshot) -> String {
	let mut prompt = format!(
		"FILENAME: {}\nRELATIVE PATH: {}\nMEDIA KIND: {}\n",
		book.source_filename,
		book.relative_path,
		media_kind_label(book.media_kind),
	);
	match &book.embedded_metadata {
		Some(metadata) => match serde_json::to_string(metadata) {
			Ok(serialized) if !serialized.is_empty() => {
				prompt.push_str("EMBEDDED METADATA: ");
				prompt.push_str(&serialized);
			},
			_ => prompt.push_str("EMBEDDED METADATA: none"),
		},
		None => prompt.push_str("EMBEDDED METADATA: none"),
	}
	prompt
}

/// Parse the assistant message content as JSON, tolerating markdown code
/// fences some servers add despite `response_format`.
fn parse_content_json(provider_id: &str, content: &str) -> Result<Value, ProviderError> {
	let trimmed = content.trim();
	let stripped = strip_code_fence(trimmed);
	serde_json::from_str(stripped).map_err(|error| ProviderError::Request {
		provider_id: provider_id.to_string(),
		message: format!("model returned malformed JSON: {error}"),
	})
}

fn strip_code_fence(content: &str) -> &str {
	let Some(rest) = content.strip_prefix("```") else {
		return content;
	};
	let rest = rest.trim_start_matches(|c: char| c.is_ascii_alphanumeric());
	let rest = rest.trim_start_matches(['\r', '\n']);
	match rest.rfind("```") {
		Some(end) => rest[..end].trim(),
		None => rest.trim(),
	}
}

#[async_trait]
impl IngestMetadataProvider for LlmProvider {
	fn id(&self) -> &'static str {
		LLM_PROVIDER_ID
	}

	fn name(&self) -> &'static str {
		"AI (OpenAI-compatible)"
	}

	fn version(&self) -> &'static str {
		LLM_PROVIDER_VERSION
	}

	fn supported_media_kinds(&self) -> &[IngestMediaKind] {
		static KINDS: [IngestMediaKind; 5] = [
			IngestMediaKind::ComicArchive,
			IngestMediaKind::ComicRarArchive,
			IngestMediaKind::Epub,
			IngestMediaKind::Pdf,
			IngestMediaKind::Unknown,
		];
		&KINDS
	}

	fn capabilities(&self) -> &[ProviderCapability] {
		static CAPABILITIES: [ProviderCapability; 3] = [
			ProviderCapability::Identify,
			ProviderCapability::Lookup,
			ProviderCapability::Search,
		];
		&CAPABILITIES
	}

	fn settings(&self) -> &[SettingDefinition] {
		&SETTINGS
	}

	async fn identify(
		&self,
		book: &BookSnapshot,
		settings: &SettingValues,
	) -> Result<Vec<ProviderIdentity>, ProviderError> {
		let config = LlmConfig::from_settings(settings)?;
		let metadata = self.identify_metadata(&config, book).await?;
		let display = metadata
			.title
			.clone()
			.unwrap_or_else(|| book.source_filename.clone());
		// The parsed fields ride along on the identity so `lookup` needs no
		// second completion; they are provider-private detail in `factors`.
		Ok(vec![ProviderIdentity {
			provider_id: LLM_PROVIDER_ID.to_string(),
			external_id: book.source_sha256.clone(),
			display,
			confidence: LLM_CONFIDENCE,
			factors: json!({
				"source": "llm",
				"model": &config.model,
				"fields": metadata,
			}),
		}])
	}

	async fn lookup(
		&self,
		book: &BookSnapshot,
		identity: &ProviderIdentity,
		settings: &SettingValues,
	) -> Result<Vec<MetadataCandidate>, ProviderError> {
		// The LLM never mints external identities; it can only expand its own
		// source-digest identity from `identify`.
		if identity.provider_id != LLM_PROVIDER_ID {
			return Err(ProviderError::Unsupported {
				provider_id: LLM_PROVIDER_ID.to_string(),
			});
		}
		if identity.external_id != book.source_sha256 {
			return Err(ProviderError::Request {
				provider_id: LLM_PROVIDER_ID.to_string(),
				message: "llm identity does not match source digest".to_string(),
			});
		}
		let config = LlmConfig::from_settings(settings)?;
		let metadata = match identity.factors.get("fields").and_then(|fields| {
			serde_json::from_value::<BookMetadata>(fields.clone()).ok()
		}) {
			Some(fields) => fields,
			None => self.identify_metadata(&config, book).await?,
		};
		Ok(vec![self.candidate(&config, book, &metadata)])
	}

	async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>, ProviderError> {
		let settings = self
			.search_settings
			.read()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
			.clone();
		let config = LlmConfig::from_settings(&settings)?;
		let mut user = format!("SEARCH TEXT: {}", query.text);
		if let Some(media_kind) = query.media_kind {
			user.push_str(&format!("\nMEDIA KIND: {}", media_kind_label(media_kind)));
		}
		let content = self
			.complete(
				&config,
				&compose_system_prompt(
					REFORMULATE_SYSTEM_PROMPT,
					config.extra_instructions.as_deref(),
				),
				&user,
				"search_query",
				&REFORMULATE_SCHEMA,
			)
			.await?;
		// The LLM must not invent external ids, so it emits zero hits itself;
		// the reformulated query is only meaningful as input for other
		// providers (see the rank_candidates follow-up in the module docs).
		tracing::debug!(
			provider = LLM_PROVIDER_ID,
			original = %query.text,
			reformulated = %content,
			"LLM provider reformulated the query and returns no hits of its own"
		);
		Ok(Vec::new())
	}
}

#[cfg(test)]
mod tests {
	use std::{
		io::{BufRead, BufReader, Read, Write},
		net::TcpListener,
		sync::{Arc, Mutex},
	};

	use super::*;
	use serde_json::json;

	fn settings(base_url: &str) -> SettingValues {
		SettingValues::from([
			(SETTING_ENABLED.to_string(), json!(true)),
			(SETTING_BASE_URL.to_string(), json!(base_url)),
			(SETTING_MODEL.to_string(), json!("test-model")),
		])
	}

	fn snapshot(filename: &str) -> BookSnapshot {
		BookSnapshot {
			drop_item_id: "drop".to_string(),
			library_id: "library".to_string(),
			staged_path: std::path::PathBuf::from("unused"),
			source_sha256: "digest".to_string(),
			byte_size: 1,
			source_filename: filename.to_string(),
			relative_path: String::new(),
			media_kind: IngestMediaKind::Epub,
			embedded_metadata: None,
			pages: vec![],
			analysis: None,
		}
	}

	/// Minimal HTTP/1.1 stub serving one canned JSON response per connection
	/// and recording every request it receives.  The ComicVine integration
	/// tests use no HTTP mock, so this keeps new test dependencies out of the
	/// workspace.
	struct Stub {
		base_url: String,
		requests: Arc<Mutex<Vec<String>>>,
	}

	fn spawn_stub(status: &'static str, body: String) -> Stub {
		let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
		let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
		let requests = Arc::new(Mutex::new(Vec::new()));
		let captured = requests.clone();
		std::thread::spawn(move || {
			for stream in listener.incoming().flatten() {
				let captured = captured.clone();
				let body = body.clone();
				std::thread::spawn(move || {
					let mut reader = BufReader::new(stream);
					let mut request = String::new();
					loop {
						let mut line = String::new();
						match reader.read_line(&mut line) {
							Ok(0) | Err(_) => return,
							Ok(_) if line == "\r\n" => break,
							Ok(_) => request.push_str(&line),
						}
					}
					let content_length = request
						.lines()
						.find_map(|line| {
							let (name, value) = line.split_once(':')?;
							name.eq_ignore_ascii_case("content-length")
								.then(|| value.trim().parse::<usize>().ok())?
						})
						.unwrap_or(0);
					if content_length > 0 {
						let mut body = vec![0_u8; content_length];
						if reader.read_exact(&mut body).is_err() {
							return;
						}
						request.push_str(&String::from_utf8_lossy(&body));
					}
					captured.lock().expect("stub lock").push(request);
					let response = format!(
						"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
						body.len(),
					);
					let mut writer = reader.into_inner();
					let _ = writer.write_all(response.as_bytes());
					let _ = writer.flush();
				});
			}
		});
		Stub { base_url, requests }
	}

	fn completion_body(content: &str) -> String {
		json!({"choices": [{"message": {"role": "assistant", "content": content}}]})
			.to_string()
	}

	fn captured_requests(stub: &Stub) -> Vec<String> {
		stub.requests.lock().expect("stub lock").clone()
	}

	#[tokio::test]
	async fn unconfigured_or_disabled_provider_is_not_configured() {
		let provider = LlmProvider::new();
		let book = snapshot("book.epub");
		for disabled in [SettingValues::new(), {
			let mut values = SettingValues::new();
			values.insert(SETTING_ENABLED.to_string(), json!(false));
			values
		}] {
			let error = provider.identify(&book, &disabled).await.unwrap_err();
			assert!(matches!(error, ProviderError::NotConfigured { .. }));
		}
	}

	#[tokio::test]
	async fn identify_and_lookup_parse_completion_json() {
		let content = json!({
			"title": "The Long Way to a Small, Angry Planet",
			"series": "Wayfarers",
			"number": "1",
			"year": 2014,
			"authors": ["Becky Chambers"],
			"language": "en",
		});
		// Fenced output exercises the code-fence tolerance.
		let stub = spawn_stub(
			"200 OK",
			completion_body(&format!("```json\n{content}\n```")),
		);
		let provider = LlmProvider::new();
		let values = settings(&stub.base_url);
		let book = snapshot("Wayfarers - The Long Way 01 (2014).epub");

		let identity = provider
			.identify(&book, &values)
			.await
			.expect("identify succeeds")
			.remove(0);
		assert_eq!(identity.provider_id, LLM_PROVIDER_ID);
		// Never fabricates external identifiers: the identity is the digest.
		assert_eq!(identity.external_id, "digest");
		assert_eq!(identity.display, "The Long Way to a Small, Angry Planet");

		let candidate = provider
			.lookup(&book, &identity, &values)
			.await
			.expect("lookup succeeds")
			.remove(0);
		assert_eq!(candidate.external_id.as_deref(), Some("digest"));
		assert_eq!(
			candidate.fields[&MetadataField::Title],
			"The Long Way to a Small, Angry Planet"
		);
		assert_eq!(candidate.fields[&MetadataField::Series], "Wayfarers");
		assert_eq!(candidate.fields[&MetadataField::SeriesIndex], 1.0);
		assert_eq!(candidate.fields[&MetadataField::PublishedDate], "2014");
		assert_eq!(
			candidate.fields[&MetadataField::Authors],
			json!(["Becky Chambers"])
		);
		assert_eq!(candidate.fields[&MetadataField::Language], "en");
		assert!(!candidate.fields.contains_key(&MetadataField::Isbn));
		assert!(!candidate.fields.contains_key(&MetadataField::Identifiers));

		// `lookup` reuses the fields memoised on the identity: one completion.
		let requests = captured_requests(&stub);
		assert_eq!(requests.len(), 1);
		assert!(requests[0].contains("\"json_schema\""));
		assert!(requests[0].contains("test-model"));
		assert!(requests[0].contains("Wayfarers - The Long Way 01 (2014).epub"));
	}

	#[tokio::test]
	async fn malformed_model_content_is_a_clean_error() {
		let stub = spawn_stub("200 OK", completion_body("I cannot help with that."));
		let provider = LlmProvider::new();
		let values = settings(&stub.base_url);
		let error = provider
			.identify(&snapshot("book.epub"), &values)
			.await
			.unwrap_err();
		match error {
			ProviderError::Request {
				provider_id,
				message,
			} => {
				assert_eq!(provider_id, LLM_PROVIDER_ID);
				assert!(message.contains("malformed JSON"), "{message}");
			},
			other => panic!("expected Request error, got {other:?}"),
		}
	}

	#[tokio::test]
	async fn invalid_response_body_is_a_clean_error() {
		let stub = spawn_stub("200 OK", "<html>gateway error</html>".to_string());
		let provider = LlmProvider::new();
		let values = settings(&stub.base_url);
		let error = provider
			.identify(&snapshot("book.epub"), &values)
			.await
			.unwrap_err();
		match error {
			ProviderError::Request { message, .. } => {
				assert!(message.contains("invalid JSON response body"), "{message}");
			},
			other => panic!("expected Request error, got {other:?}"),
		}
	}

	#[tokio::test]
	async fn http_429_maps_to_rate_limited() {
		let stub = spawn_stub(
			"429 Too Many Requests",
			json!({"error": "slow down"}).to_string(),
		);
		let provider = LlmProvider::new();
		let values = settings(&stub.base_url);
		let error = provider
			.identify(&snapshot("book.epub"), &values)
			.await
			.unwrap_err();
		assert!(matches!(error, ProviderError::RateLimited { .. }));
	}

	#[tokio::test]
	async fn lookup_rejects_foreign_identity_as_unsupported() {
		let stub = spawn_stub("200 OK", completion_body("{}"));
		let provider = LlmProvider::new();
		let values = settings(&stub.base_url);
		let identity = ProviderIdentity {
			provider_id: "comic_vine".to_string(),
			external_id: "4050-1234".to_string(),
			display: "Some volume".to_string(),
			confidence: 0.9,
			factors: json!({"result_kind": "media"}),
		};
		let error = provider
			.lookup(&snapshot("book.epub"), &identity, &values)
			.await
			.unwrap_err();
		assert!(matches!(error, ProviderError::Unsupported { .. }));
		// The rejection happens before any network call.
		assert!(captured_requests(&stub).is_empty());
	}

	#[tokio::test]
	async fn search_reformulates_and_returns_zero_hits() {
		let stub = spawn_stub(
			"200 OK",
			completion_body("{\"search_query\": \"saga chapter one 2024\"}"),
		);
		let provider = LlmProvider::new();
		let mut values = settings(&stub.base_url);
		values.insert(SETTING_API_KEY.to_string(), json!("sk-test"));
		provider.set_search_settings(values);

		let hits = provider
			.search(&SearchQuery {
				text: "Saga - Chapter One 003 (2024).cbz".to_string(),
				media_kind: Some(IngestMediaKind::ComicArchive),
				limit: 5,
			})
			.await
			.expect("search succeeds");
		assert!(hits.is_empty(), "the LLM must not invent hits");

		let requests = captured_requests(&stub);
		assert_eq!(requests.len(), 1);
		assert!(requests[0].contains("SEARCH TEXT: Saga - Chapter One 003 (2024).cbz"));
		assert!(
			requests[0].contains("REFORMULATE") || requests[0].contains("bibliographic")
		);
		// The reformulate prompt forbids inventing identifiers.
		assert!(requests[0].contains("never"));
	}

	#[tokio::test]
	async fn search_without_settings_is_not_configured() {
		let provider = LlmProvider::new();
		let error = provider
			.search(&SearchQuery {
				text: "dune".to_string(),
				media_kind: None,
				limit: 10,
			})
			.await
			.unwrap_err();
		assert!(matches!(error, ProviderError::NotConfigured { .. }));
	}

	#[tokio::test]
	async fn extra_instructions_are_appended_to_system_prompt() {
		let stub = spawn_stub(
			"200 OK",
			completion_body("{\"search_query\": \"saga chapter one 2024\"}"),
		);
		let provider = LlmProvider::new();
		let mut values = settings(&stub.base_url);
		values.insert(
			SETTING_EXTRA_INSTRUCTIONS.to_string(),
			json!("Series - vNN"),
		);
		provider.set_search_settings(values);

		provider
			.search(&SearchQuery {
				text: "Saga 3.cbz".to_string(),
				media_kind: None,
				limit: 5,
			})
			.await
			.expect("search succeeds");

		let requests = captured_requests(&stub);
		assert_eq!(requests.len(), 1);
		let body = requests[0]
			.rsplit("\r\n")
			.next()
			.expect("request has a body");
		let payload: Value = serde_json::from_str(body).expect("body is JSON");
		assert_eq!(
			payload["messages"][0]["content"],
			json!(format!(
				"{REFORMULATE_SYSTEM_PROMPT}\n\nAdditional instructions from the library owner:\nSeries - vNN"
			))
		);
	}

	#[tokio::test]
	async fn empty_extra_instructions_leave_system_prompt_unchanged() {
		let stub = spawn_stub(
			"200 OK",
			completion_body("{\"search_query\": \"saga chapter one 2024\"}"),
		);
		let provider = LlmProvider::new();
		let mut values = settings(&stub.base_url);
		values.insert(SETTING_EXTRA_INSTRUCTIONS.to_string(), json!("   "));
		provider.set_search_settings(values);

		provider
			.search(&SearchQuery {
				text: "Saga 3.cbz".to_string(),
				media_kind: None,
				limit: 5,
			})
			.await
			.expect("search succeeds");

		let requests = captured_requests(&stub);
		assert_eq!(requests.len(), 1);
		let body = requests[0]
			.rsplit("\r\n")
			.next()
			.expect("request has a body");
		let payload: Value = serde_json::from_str(body).expect("body is JSON");
		assert_eq!(
			payload["messages"][0]["content"],
			json!(REFORMULATE_SYSTEM_PROMPT)
		);
	}
}
