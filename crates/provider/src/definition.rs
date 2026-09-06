//! Data-only source definitions (schema v1) and the loader that materialises
//! them at runtime.
//!
//! A definition describes *one* remote site in terms a compiled theme engine
//! can execute: which `lib-multisrc` theme it belongs to, its base URL, and the
//! `override val` knobs its Kotlin subclass declared. No site-specific code is
//! compiled into Stump; the definitions live in a separate repository generated
//! from `keiyoushi/extensions-source` and are fetched at runtime like the
//! Keiyoushi APK catalog next door in [`crate::catalog`].
//!
//! Layout of the definition repository (or local directory):
//!
//! ```text
//! index.json            [{ id, name, lang, theme, nsfw, version, file }, ...]
//! unsupported.json      [{ id, reason }, ...]        (informational)
//! en/en.somesite.json   one SourceDefinition per file
//! ```
//!
//! `index.json` is loaded eagerly (it is small and drives the "which catalog
//! entries can Stump actually drive" answer); individual definitions are
//! fetched on demand when a source is enabled or reloaded, so a repository with
//! a thousand entries costs one request at boot.

use std::{
	collections::{BTreeMap, HashMap},
	path::{Path, PathBuf},
	sync::{Arc, RwLock},
	time::Duration,
};

use chrono::{DateTime, Utc};
use models::entity::provider_source;
use serde::{Deserialize, Serialize};

use crate::{host::ProviderError, source::Source};

/// The only schema version this build understands.
pub const SCHEMA_VERSION: u32 = 1;

/// Default definition repository. Overridable with
/// `STUMP_SOURCE_DEFINITIONS_URL`, which also accepts a `file://` URL or a
/// bare local directory for offline/development use.
pub const DEFAULT_DEFINITIONS_URL: &str =
	"https://raw.githubusercontent.com/stumpapp/stump-sources/main/index.json";

/// Subdirectory of the provider cache root holding downloaded definitions.
pub const DEFINITIONS_DIR_NAME: &str = "source-definitions";

pub const INDEX_FILE_NAME: &str = "index.json";

/// Refresh cadence for the index, matching the catalog's.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

const PKG_PREFIX: &str = "eu.kanade.tachiyomi.extension.";

/// One knob value. Definitions carry data only: the generator emits strings,
/// booleans and integers, never code or structured objects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KnobValue {
	Bool(bool),
	Int(i64),
	Float(f64),
	Text(String),
}

impl KnobValue {
	pub fn as_str(&self) -> Option<&str> {
		match self {
			KnobValue::Text(value) => Some(value.as_str()),
			_ => None,
		}
	}

	/// Booleans, plus the `"true"`/`"false"` strings a generator may emit when
	/// the Kotlin literal was quoted.
	pub fn as_bool(&self) -> Option<bool> {
		match self {
			KnobValue::Bool(value) => Some(*value),
			KnobValue::Int(value) => Some(*value != 0),
			KnobValue::Text(value) => match value.trim().to_ascii_lowercase().as_str() {
				"true" | "yes" | "1" => Some(true),
				"false" | "no" | "0" => Some(false),
				_ => None,
			},
			KnobValue::Float(_) => None,
		}
	}

	pub fn as_int(&self) -> Option<i64> {
		match self {
			KnobValue::Int(value) => Some(*value),
			KnobValue::Bool(value) => Some(i64::from(*value)),
			KnobValue::Float(value) => Some(*value as i64),
			KnobValue::Text(value) => value.trim().parse().ok(),
		}
	}
}

/// Where a definition came from, so a misbehaving engine can be traced back to
/// the exact Kotlin it was derived from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionUpstream {
	pub repo: String,
	pub commit: String,
	pub path: String,
}

/// Schema v1. Frozen in `docs/content/docs/developer/source-definitions.mdx`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceDefinition {
	pub schema: u32,
	/// Extension package name, or the `<lang>.<dir>` suffix of one.
	pub id: String,
	pub name: String,
	pub lang: String,
	pub base_url: String,
	/// `lib-multisrc` theme directory name, lowercase (`madara`, ...).
	pub theme: String,
	#[serde(default)]
	pub version: u32,
	#[serde(default)]
	pub nsfw: bool,
	#[serde(default)]
	pub knobs: BTreeMap<String, KnobValue>,
	#[serde(default)]
	pub upstream: Option<DefinitionUpstream>,
}

impl SourceDefinition {
	/// The Keiyoushi package this definition stands in for, so a catalog entry
	/// can be matched to it.
	pub fn pkg(&self) -> String {
		pkg_of(&self.id)
	}

	/// Normalised base URL without a trailing slash.
	pub fn base_url(&self) -> &str {
		self.base_url.trim_end_matches('/')
	}

	pub fn lang(&self) -> &str {
		let lang = self.lang.trim();
		if lang.is_empty() {
			"all"
		} else {
			lang
		}
	}

	/// First knob present under any of `keys`, so an engine can accept both the
	/// current upstream property name and a legacy alias.
	pub fn knob(&self, keys: &[&str]) -> Option<&KnobValue> {
		keys.iter().find_map(|key| self.knobs.get(*key))
	}

	pub fn text_knob(&self, keys: &[&str]) -> Option<&str> {
		self.knob(keys)
			.and_then(KnobValue::as_str)
			.map(str::trim)
			.filter(|value| !value.is_empty())
	}

	pub fn bool_knob(&self, keys: &[&str]) -> Option<bool> {
		self.knob(keys).and_then(KnobValue::as_bool)
	}

	pub fn int_knob(&self, keys: &[&str]) -> Option<i64> {
		self.knob(keys).and_then(KnobValue::as_int)
	}

	/// Reject definitions this build cannot execute before an engine is built.
	pub fn validate(&self) -> Result<(), DefinitionError> {
		if self.schema != SCHEMA_VERSION {
			return Err(DefinitionError::UnsupportedSchema {
				id: self.id.clone(),
				schema: self.schema,
			});
		}
		if self.id.trim().is_empty() {
			return Err(DefinitionError::Parse("definition has an empty id".into()));
		}
		if !self.base_url.starts_with("http://") && !self.base_url.starts_with("https://")
		{
			return Err(DefinitionError::Parse(format!(
				"{} has a non-HTTP base_url `{}`",
				self.id, self.base_url
			)));
		}
		Ok(())
	}
}

/// One `index.json` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionIndexEntry {
	pub id: String,
	pub name: String,
	pub lang: String,
	pub theme: String,
	#[serde(default)]
	pub nsfw: bool,
	#[serde(default)]
	pub version: u32,
	/// POSIX path relative to the index, e.g. `en/en.somesite.json`.
	pub file: String,
}

impl DefinitionIndexEntry {
	pub fn pkg(&self) -> String {
		pkg_of(&self.id)
	}

	/// The package of the *extension* this definition is one source of, when
	/// the id carries a trailing `.<lang>` disambiguator.
	///
	/// A multi-language extension declares several `source {}` blocks that
	/// differ only in language, so the generator appends the language to keep
	/// the ids unique (`all.seraphicdeviltry.en`, `all.seraphicdeviltry.es`).
	/// The APK is still the one package `…extension.all.seraphicdeviltry`, so
	/// without this the catalog entry has no definition and the source shows
	/// as unimplemented.
	pub fn pkg_without_lang(&self) -> Option<String> {
		let stem = self
			.id
			.strip_suffix(self.lang.trim())
			.filter(|_| !self.lang.trim().is_empty())?
			.strip_suffix('.')
			.filter(|stem| !stem.is_empty())?;
		Some(pkg_of(stem))
	}
}

fn pkg_of(id: &str) -> String {
	if id.starts_with(PKG_PREFIX) {
		id.to_string()
	} else {
		format!("{PKG_PREFIX}{id}")
	}
}

#[derive(Debug)]
pub struct DefinitionIndex {
	pub fetched_at: DateTime<Utc>,
	pub entries: Vec<DefinitionIndexEntry>,
}

impl DefinitionIndex {
	pub fn find(&self, id: &str) -> Option<&DefinitionIndexEntry> {
		self.entries.iter().find(|entry| entry.id == id)
	}

	pub fn find_by_pkg(&self, pkg: &str) -> Option<&DefinitionIndexEntry> {
		self.find_by_pkg_lang(pkg, None)
	}

	/// The definition standing in for an extension package.
	///
	/// An id that *is* the package wins over one that only shares it after a
	/// trailing `.<lang>` is stripped, so a single-source extension is never
	/// shadowed by a multi-language sibling. `lang` picks the block of a
	/// multi-language extension the caller means; without it, or when no
	/// block carries it, the first match in index order is used.
	pub fn find_by_pkg_lang(
		&self,
		pkg: &str,
		lang: Option<&str>,
	) -> Option<&DefinitionIndexEntry> {
		let lang = lang.map(str::trim).filter(|lang| !lang.is_empty());
		let pick = |exact: bool| {
			let mut first = None;
			for entry in self.entries.iter().filter(|entry| {
				if exact {
					entry.pkg() == pkg
				} else {
					entry.pkg_without_lang().as_deref() == Some(pkg)
				}
			}) {
				if lang.is_some_and(|lang| entry.lang.eq_ignore_ascii_case(lang)) {
					return Some(entry);
				}
				first = first.or(Some(entry));
			}
			first
		};
		pick(true).or_else(|| pick(false))
	}

	pub fn is_stale(&self, max_age: Duration) -> bool {
		let age = Utc::now() - self.fetched_at;
		age.to_std().map(|age| age > max_age).unwrap_or(true)
	}
}

#[derive(Debug, thiserror::Error)]
pub enum DefinitionError {
	#[error("Failed to download a source definition: {0}")]
	Http(#[from] reqwest::Error),
	#[error("Definition download returned HTTP {status} for {url}")]
	Status { status: u16, url: String },
	#[error("Malformed source definition: {0}")]
	Parse(String),
	#[error("Definition cache I/O failed: {0}")]
	Io(#[from] std::io::Error),
	#[error("No source definition for `{0}`")]
	NotFound(String),
	#[error("Definition `{id}` uses schema {schema}; this build understands v1")]
	UnsupportedSchema { id: String, schema: u32 },
	#[error("No theme engine implements `{theme}` (source `{id}`)")]
	UnknownTheme { id: String, theme: String },
}

impl DefinitionError {
	/// Whether the failure says nothing about the definition itself: the
	/// index or the file could not be read (offline, no cache yet, a wiped
	/// local directory), so the answer may differ on the next attempt.
	///
	/// [`DefinitionError::NotFound`] is counted here on purpose: it is
	/// raised both for an id the index does not list *and* for a missing
	/// `index.json` or definition file, and the two are indistinguishable
	/// from the outside.
	pub fn is_transient(&self) -> bool {
		matches!(
			self,
			DefinitionError::Http(_)
				| DefinitionError::Status { .. }
				| DefinitionError::Io(_)
				| DefinitionError::NotFound(_)
		)
	}
}

/// Builds a [`Source`] from a definition. One engine per `lib-multisrc` theme;
/// registered with the host alongside the compiled [`crate::SourceFactory`] set.
#[derive(Clone, Copy)]
pub struct DefinitionEngine {
	/// Lowercase theme directory name, matching `SourceDefinition::theme`.
	pub theme: &'static str,
	pub build: fn(
		&SourceDefinition,
		&provider_source::Model,
	) -> Result<Arc<dyn Source>, ProviderError>,
}

impl std::fmt::Debug for DefinitionEngine {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("DefinitionEngine")
			.field("theme", &self.theme)
			.finish()
	}
}

/// Where the definitions come from: an HTTP(S) index or a local directory.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Origin {
	Remote { index: String, root: String },
	Local { root: PathBuf },
}

impl Origin {
	/// Accepts an index URL (`.../index.json`), a repository root URL, a
	/// `file://` URL, or a bare filesystem path.
	fn parse(url: &str) -> Self {
		let trimmed = url.trim();
		if let Some(path) = trimmed.strip_prefix("file://") {
			// `file:///abs/path` keeps the leading slash; `file://./rel` does not.
			let path = if path.is_empty() { "/" } else { path };
			return Origin::Local {
				root: local_root(Path::new(path)),
			};
		}
		if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
			let root = trimmed.trim_end_matches('/');
			let (index, root) = match root.rsplit_once('/') {
				Some((parent, last)) if last.ends_with(".json") => {
					(root.to_string(), parent.to_string())
				},
				_ => (format!("{root}/{INDEX_FILE_NAME}"), root.to_string()),
			};
			return Origin::Remote { index, root };
		}
		Origin::Local {
			root: local_root(Path::new(trimmed)),
		}
	}
}

/// Strip a trailing `index.json` so a directory and an index path both work.
fn local_root(path: &Path) -> PathBuf {
	if path.file_name().and_then(|name| name.to_str()) == Some(INDEX_FILE_NAME) {
		path.parent().unwrap_or(path).to_path_buf()
	} else {
		path.to_path_buf()
	}
}

/// Fetches and caches the definition index and individual definitions.
///
/// The index is cached under `<provider cache>/source-definitions/index.json`
/// and definitions under the same relative paths they have upstream, so the
/// cache is a mirror of the repository and survives restarts offline.
pub struct DefinitionLoader {
	client: reqwest::Client,
	origin: Origin,
	url: String,
	cache_dir: PathBuf,
	index: RwLock<Option<Arc<DefinitionIndex>>>,
	definitions: RwLock<HashMap<String, Arc<SourceDefinition>>>,
}

impl std::fmt::Debug for DefinitionLoader {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("DefinitionLoader")
			.field("url", &self.url)
			.field("cache_dir", &self.cache_dir)
			.finish()
	}
}

impl DefinitionLoader {
	/// `cache_dir` is the provider cache root; definitions land in
	/// [`DEFINITIONS_DIR_NAME`] beneath it.
	pub fn new(client: reqwest::Client, cache_dir: &Path, url: Option<String>) -> Self {
		let url = url
			.map(|url| url.trim().to_string())
			.filter(|url| !url.is_empty())
			.unwrap_or_else(|| DEFAULT_DEFINITIONS_URL.to_string());
		Self {
			client,
			origin: Origin::parse(&url),
			url,
			cache_dir: cache_dir.join(DEFINITIONS_DIR_NAME),
			index: RwLock::new(None),
			definitions: RwLock::new(HashMap::new()),
		}
	}

	pub fn url(&self) -> &str {
		&self.url
	}

	pub fn cache_dir(&self) -> &Path {
		&self.cache_dir
	}

	/// Whether definitions are read straight from the filesystem, in which case
	/// nothing is cached and edits are picked up by the next refresh.
	pub fn is_local(&self) -> bool {
		matches!(self.origin, Origin::Local { .. })
	}

	pub fn current(&self) -> Option<Arc<DefinitionIndex>> {
		self.index.read().expect("definition lock poisoned").clone()
	}

	/// Memory, then disk, then origin.
	pub async fn index(&self) -> Result<Arc<DefinitionIndex>, DefinitionError> {
		if let Some(index) = self.current() {
			return Ok(index);
		}
		if let Some(index) = self.load_cached_index().await? {
			return Ok(index);
		}
		self.refresh().await
	}

	/// Re-read the origin, replacing the in-memory index and dropping every
	/// cached definition so knob edits take effect.
	pub async fn refresh(&self) -> Result<Arc<DefinitionIndex>, DefinitionError> {
		let bytes = match &self.origin {
			Origin::Remote { index, .. } => self.get(index).await?,
			Origin::Local { root } => {
				let path = root.join(INDEX_FILE_NAME);
				tokio::fs::read(&path).await.map_err(|error| {
					if error.kind() == std::io::ErrorKind::NotFound {
						DefinitionError::NotFound(path.display().to_string())
					} else {
						DefinitionError::Io(error)
					}
				})?
			},
		};
		let entries = parse_index(&bytes)?;
		if let Origin::Remote { .. } = &self.origin {
			self.write_cached(Path::new(INDEX_FILE_NAME), &bytes)
				.await?;
		}
		let index = Arc::new(DefinitionIndex {
			fetched_at: Utc::now(),
			entries,
		});
		*self.index.write().expect("definition lock poisoned") = Some(index.clone());
		self.definitions
			.write()
			.expect("definition lock poisoned")
			.clear();
		Ok(index)
	}

	/// Refresh when the index is older than [`REFRESH_INTERVAL`].
	pub async fn refresh_if_stale(
		&self,
	) -> Result<Arc<DefinitionIndex>, DefinitionError> {
		match self.index().await {
			Ok(index) if !index.is_stale(REFRESH_INTERVAL) => Ok(index),
			Ok(_) => self.refresh().await,
			Err(error) => Err(error),
		}
	}

	/// The definition for one id, loaded and validated on first use.
	pub async fn definition(
		&self,
		id: &str,
	) -> Result<Arc<SourceDefinition>, DefinitionError> {
		if let Some(definition) = self
			.definitions
			.read()
			.expect("definition lock poisoned")
			.get(id)
			.cloned()
		{
			return Ok(definition);
		}
		let index = self.index().await?;
		let entry = index
			.find(id)
			.ok_or_else(|| DefinitionError::NotFound(id.to_string()))?;
		let file = sanitise_relative(&entry.file).ok_or_else(|| {
			DefinitionError::Parse(format!("unsafe file `{}`", entry.file))
		})?;
		let bytes = self.read_definition_bytes(&file).await?;
		let definition: SourceDefinition = serde_json::from_slice(&bytes)
			.map_err(|error| DefinitionError::Parse(error.to_string()))?;
		definition.validate()?;
		let definition = Arc::new(definition);
		self.definitions
			.write()
			.expect("definition lock poisoned")
			.insert(id.to_string(), definition.clone());
		Ok(definition)
	}

	/// The definition standing in for a Keiyoushi package, if any.
	///
	/// `lang` is the language of the catalog source being resolved: a
	/// multi-language extension has one definition per language, all sharing
	/// the package ([`DefinitionIndex::find_by_pkg_lang`]).
	///
	/// An unreachable or missing index answers `Ok(None)`: the caller asked
	/// "can anything drive this package", and "no definition repository" is the
	/// same answer as "no definition for it". Naming an id explicitly
	/// ([`DefinitionLoader::definition`]) still reports the failure.
	pub async fn definition_for_pkg(
		&self,
		pkg: &str,
		lang: Option<&str>,
	) -> Result<Option<Arc<SourceDefinition>>, DefinitionError> {
		let index = match self.index().await {
			Ok(index) => index,
			Err(error) => {
				tracing::debug!(
					?error,
					url = self.url,
					"No source-definition index available"
				);
				return Ok(None);
			},
		};
		let Some(entry) = index.find_by_pkg_lang(pkg, lang) else {
			return Ok(None);
		};
		let id = entry.id.clone();
		drop(index);
		self.definition(&id).await.map(Some)
	}

	async fn read_definition_bytes(
		&self,
		file: &Path,
	) -> Result<Vec<u8>, DefinitionError> {
		match &self.origin {
			Origin::Local { root } => {
				let path = root.join(file);
				tokio::fs::read(&path).await.map_err(|error| {
					if error.kind() == std::io::ErrorKind::NotFound {
						DefinitionError::NotFound(path.display().to_string())
					} else {
						DefinitionError::Io(error)
					}
				})
			},
			Origin::Remote { root, .. } => {
				let url = format!("{root}/{}", file.to_string_lossy());
				match self.get(&url).await {
					Ok(bytes) => {
						self.write_cached(file, &bytes).await?;
						Ok(bytes)
					},
					Err(error) => {
						// Fall back to the mirror so an offline server keeps
						// serving sources it has already downloaded.
						match tokio::fs::read(self.cache_dir.join(file)).await {
							Ok(bytes) => Ok(bytes),
							Err(_) => Err(error),
						}
					},
				}
			},
		}
	}

	async fn load_cached_index(
		&self,
	) -> Result<Option<Arc<DefinitionIndex>>, DefinitionError> {
		if self.is_local() {
			return Ok(None);
		}
		let path = self.cache_dir.join(INDEX_FILE_NAME);
		let bytes = match tokio::fs::read(&path).await {
			Ok(bytes) => bytes,
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
				return Ok(None)
			},
			Err(error) => return Err(error.into()),
		};
		let fetched_at = tokio::fs::metadata(&path)
			.await
			.ok()
			.and_then(|metadata| metadata.modified().ok())
			.map(DateTime::<Utc>::from)
			.unwrap_or_else(Utc::now);
		let index = Arc::new(DefinitionIndex {
			fetched_at,
			entries: parse_index(&bytes)?,
		});
		*self.index.write().expect("definition lock poisoned") = Some(index.clone());
		Ok(Some(index))
	}

	async fn get(&self, url: &str) -> Result<Vec<u8>, DefinitionError> {
		let response = self.client.get(url).send().await?;
		let status = response.status();
		if !status.is_success() {
			return Err(DefinitionError::Status {
				status: status.as_u16(),
				url: url.to_string(),
			});
		}
		Ok(response.bytes().await?.to_vec())
	}

	async fn write_cached(
		&self,
		file: &Path,
		bytes: &[u8],
	) -> Result<(), DefinitionError> {
		let path = self.cache_dir.join(file);
		if let Some(parent) = path.parent() {
			tokio::fs::create_dir_all(parent).await?;
		}
		let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
		tokio::fs::write(&temporary, bytes).await?;
		tokio::fs::rename(&temporary, &path).await?;
		Ok(())
	}
}

/// Accept both the index array and a `{ "sources": [...] }` wrapper so the
/// generator can add repository-level metadata without breaking older servers.
pub fn parse_index(bytes: &[u8]) -> Result<Vec<DefinitionIndexEntry>, DefinitionError> {
	let value: serde_json::Value = serde_json::from_slice(bytes)
		.map_err(|error| DefinitionError::Parse(error.to_string()))?;
	let array = match value {
		serde_json::Value::Array(_) => value,
		serde_json::Value::Object(mut object) => object
			.remove("sources")
			.ok_or_else(|| DefinitionError::Parse("index has no `sources`".into()))?,
		_ => {
			return Err(DefinitionError::Parse(
				"expected a JSON array or object".into(),
			))
		},
	};
	serde_json::from_value(array)
		.map_err(|error| DefinitionError::Parse(error.to_string()))
}

/// Reject absolute paths and `..` so a hostile index cannot write outside the
/// cache or read outside the definition root.
fn sanitise_relative(file: &str) -> Option<PathBuf> {
	let file = file.trim().trim_start_matches("./");
	if file.is_empty() || file.starts_with('/') || file.contains('\\') {
		return None;
	}
	let mut path = PathBuf::new();
	for segment in file.split('/') {
		if segment.is_empty() || segment == "." || segment == ".." {
			return None;
		}
		path.push(segment);
	}
	Some(path)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn definition(theme: &str) -> SourceDefinition {
		SourceDefinition {
			schema: 1,
			id: "en.example".into(),
			name: "Example".into(),
			lang: "en".into(),
			base_url: "https://example.com/".into(),
			theme: theme.into(),
			version: 7,
			nsfw: false,
			knobs: BTreeMap::new(),
			upstream: None,
		}
	}

	#[test]
	fn pkg_is_derived_from_a_suffix_or_kept_whole() {
		assert_eq!(
			definition("madara").pkg(),
			"eu.kanade.tachiyomi.extension.en.example"
		);
		let mut full = definition("madara");
		full.id = "eu.kanade.tachiyomi.extension.en.example".into();
		assert_eq!(full.pkg(), "eu.kanade.tachiyomi.extension.en.example");
	}

	fn entry(id: &str, lang: &str) -> DefinitionIndexEntry {
		DefinitionIndexEntry {
			id: id.into(),
			name: id.into(),
			lang: lang.into(),
			theme: "madara".into(),
			nsfw: false,
			version: 0,
			file: format!("{lang}/{id}.json"),
		}
	}

	/// A multi-language extension has one definition per language, all
	/// standing in for the one APK package the catalog knows.
	#[test]
	fn a_package_matches_its_language_disambiguated_definitions() {
		let index = DefinitionIndex {
			fetched_at: Utc::now(),
			entries: vec![
				entry("all.seraphicdeviltry.en", "en"),
				entry("all.seraphicdeviltry.es", "es"),
				entry("all.allporncomicsco", "all"),
			],
		};
		let pkg = "eu.kanade.tachiyomi.extension.all.seraphicdeviltry";

		assert_eq!(
			index
				.find_by_pkg_lang(pkg, Some("es"))
				.map(|e| e.id.as_str()),
			Some("all.seraphicdeviltry.es")
		);
		// No language asked, or a language no block carries: the first block.
		assert_eq!(
			index.find_by_pkg(pkg).map(|e| e.id.as_str()),
			Some("all.seraphicdeviltry.en")
		);
		assert_eq!(
			index
				.find_by_pkg_lang(pkg, Some("fr"))
				.map(|e| e.id.as_str()),
			Some("all.seraphicdeviltry.en")
		);
		// A single-source extension still matches exactly, and an id that is
		// not a package of this repository matches nothing.
		assert_eq!(
			index
				.find_by_pkg("eu.kanade.tachiyomi.extension.all.allporncomicsco")
				.map(|e| e.id.as_str()),
			Some("all.allporncomicsco")
		);
		assert!(index
			.find_by_pkg("eu.kanade.tachiyomi.extension.all.seraphic")
			.is_none());
	}

	/// An exact package match wins over a stripped one, so a single-source
	/// extension is never shadowed by a multi-language sibling.
	#[test]
	fn an_exact_package_beats_a_stripped_one() {
		let index = DefinitionIndex {
			fetched_at: Utc::now(),
			entries: vec![entry("all.site.en", "en"), entry("all.site", "all")],
		};
		assert_eq!(
			index
				.find_by_pkg_lang("eu.kanade.tachiyomi.extension.all.site", Some("en"))
				.map(|e| e.id.as_str()),
			Some("all.site")
		);
	}

	#[test]
	fn knob_lookup_falls_through_aliases_and_coerces() {
		let mut definition = definition("madara");
		definition
			.knobs
			.insert("manga_sub_string".into(), KnobValue::Text("series".into()));
		definition.knobs.insert(
			"use_new_chapter_endpoint".into(),
			KnobValue::Text("true".into()),
		);
		definition
			.knobs
			.insert("version_code".into(), KnobValue::Int(12));
		definition
			.knobs
			.insert("blank".into(), KnobValue::Text("   ".into()));

		assert_eq!(
			definition.text_knob(&["mangaSubString", "manga_sub_string"]),
			Some("series")
		);
		assert_eq!(
			definition.bool_knob(&["use_new_chapter_endpoint"]),
			Some(true)
		);
		assert_eq!(definition.int_knob(&["version_code"]), Some(12));
		assert_eq!(definition.text_knob(&["blank"]), None);
		assert_eq!(definition.text_knob(&["absent"]), None);
	}

	#[test]
	fn validation_rejects_other_schemas_and_bad_urls() {
		let mut wrong = definition("madara");
		wrong.schema = 2;
		assert!(matches!(
			wrong.validate(),
			Err(DefinitionError::UnsupportedSchema { schema: 2, .. })
		));

		let mut relative = definition("madara");
		relative.base_url = "example.com".into();
		assert!(matches!(
			relative.validate(),
			Err(DefinitionError::Parse(_))
		));

		assert!(definition("madara").validate().is_ok());
		assert_eq!(definition("madara").base_url(), "https://example.com");
	}

	#[test]
	fn origin_parsing_covers_index_urls_roots_and_local_dirs() {
		assert_eq!(
			Origin::parse("https://host/repo/main/index.json"),
			Origin::Remote {
				index: "https://host/repo/main/index.json".into(),
				root: "https://host/repo/main".into(),
			}
		);
		assert_eq!(
			Origin::parse("https://host/repo/main/"),
			Origin::Remote {
				index: "https://host/repo/main/index.json".into(),
				root: "https://host/repo/main".into(),
			}
		);
		assert_eq!(
			Origin::parse("file:///tmp/stump-sources"),
			Origin::Local {
				root: PathBuf::from("/tmp/stump-sources")
			}
		);
		assert_eq!(
			Origin::parse("file:///tmp/stump-sources/index.json"),
			Origin::Local {
				root: PathBuf::from("/tmp/stump-sources")
			}
		);
		assert_eq!(
			Origin::parse("/srv/definitions"),
			Origin::Local {
				root: PathBuf::from("/srv/definitions")
			}
		);
	}

	#[test]
	fn relative_paths_reject_traversal() {
		assert_eq!(
			sanitise_relative("en/en.example.json"),
			Some(PathBuf::from("en/en.example.json"))
		);
		assert_eq!(
			sanitise_relative("./en/x.json"),
			Some(PathBuf::from("en/x.json"))
		);
		assert_eq!(sanitise_relative("../secrets"), None);
		assert_eq!(sanitise_relative("/etc/passwd"), None);
		assert_eq!(sanitise_relative("en/../../x"), None);
		assert_eq!(sanitise_relative(""), None);
	}

	#[test]
	fn index_parses_bare_arrays_and_wrapped_objects() {
		let bare = br#"[{"id":"en.a","name":"A","lang":"en","theme":"madara","nsfw":false,"version":3,"file":"en/en.a.json"}]"#;
		let entries = parse_index(bare).unwrap();
		assert_eq!(entries.len(), 1);
		assert_eq!(entries[0].pkg(), "eu.kanade.tachiyomi.extension.en.a");

		let wrapped = br#"{"generated_at":"now","sources":[{"id":"en.a","name":"A","lang":"en","theme":"madara","file":"en/en.a.json"}]}"#;
		let entries = parse_index(wrapped).unwrap();
		assert_eq!(entries[0].version, 0);
		assert!(parse_index(b"{}").is_err());
	}

	#[tokio::test]
	async fn local_origin_reads_index_and_definitions_from_disk() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("sources");
		tokio::fs::create_dir_all(root.join("en")).await.unwrap();
		tokio::fs::write(
			root.join(INDEX_FILE_NAME),
			br#"[{"id":"en.example","name":"Example","lang":"en","theme":"madara","nsfw":false,"version":7,"file":"en/en.example.json"}]"#,
		)
		.await
		.unwrap();
		tokio::fs::write(
			root.join("en/en.example.json"),
			serde_json::to_vec(&definition("madara")).unwrap(),
		)
		.await
		.unwrap();

		let loader = DefinitionLoader::new(
			reqwest::Client::new(),
			dir.path(),
			Some(format!("file://{}", root.display())),
		);
		assert!(loader.is_local());
		let index = loader.index().await.unwrap();
		assert_eq!(index.entries.len(), 1);

		let definition = loader
			.definition_for_pkg("eu.kanade.tachiyomi.extension.en.example", Some("en"))
			.await
			.unwrap()
			.expect("definition for pkg");
		assert_eq!(definition.theme, "madara");
		assert_eq!(definition.base_url(), "https://example.com");

		assert!(loader
			.definition_for_pkg("eu.kanade.tachiyomi.extension.en.missing", None)
			.await
			.unwrap()
			.is_none());
	}
}
