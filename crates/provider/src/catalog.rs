//! Keiyoushi extension catalog.
//!
//! The catalog is pulled at runtime from the Keiyoushi `repo` branch and cached
//! on disk under `<cache_dir>/providers/keiyoushi-index.json`. Two index shapes
//! exist upstream and both are accepted:
//!
//! * legacy `index.min.json`: a flat array of
//!   `{name, pkg, apk, lang, version, nsfw, sources[{name, lang, id, baseUrl}]}`
//!   (the URL now serves a two-entry "update your app" stub);
//! * current `index.json`: `{extensionList: {extensions: [{name, packageName,
//!   resources{apkUrl, iconUrl}, versionName, contentWarning,
//!   sources[{id, name, language, homeUrl}]}]}}`.
//!
//! Nothing here is compiled in as source data; entries only describe what
//! exists upstream. Whether Stump can drive a source is decided by the host's
//! registered [`crate::host::SourceFactory`] set.

use std::{
	path::{Path, PathBuf},
	sync::{Arc, RwLock},
	time::Duration,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const DEFAULT_INDEX_URL: &str =
	"https://raw.githubusercontent.com/keiyoushi/extensions/repo/index.json";
pub const INDEX_FILE_NAME: &str = "keiyoushi-index.json";
/// Refresh cadence used by the maintenance loop.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Multi-source theme families from `keiyoushi/extensions-source/lib-multisrc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SourceTheme {
	Madara,
	MangaThemesia,
	Mmrcms,
}

impl SourceTheme {
	/// The theme's "latest updates" path, probed by the health job.
	pub fn latest_path(self) -> &'static str {
		match self {
			SourceTheme::Madara => "/manga/?m_orderby=latest",
			SourceTheme::MangaThemesia => "/manga/?order=update",
			SourceTheme::Mmrcms => "/latest-release",
		}
	}

	pub fn as_str(self) -> &'static str {
		match self {
			SourceTheme::Madara => "MADARA",
			SourceTheme::MangaThemesia => "MANGA_THEMESIA",
			SourceTheme::Mmrcms => "MMRCMS",
		}
	}

	pub fn parse(value: &str) -> Option<Self> {
		match value {
			"MADARA" => Some(SourceTheme::Madara),
			"MANGA_THEMESIA" => Some(SourceTheme::MangaThemesia),
			"MMRCMS" => Some(SourceTheme::Mmrcms),
			_ => None,
		}
	}

	/// Best-effort detection from a site's landing page markup.
	pub fn detect(html: &str) -> Option<Self> {
		let lower = html.to_ascii_lowercase();
		if lower.contains("wp-content/themes/madara")
			|| lower.contains("madara-core")
			|| lower.contains("wp-manga")
		{
			return Some(SourceTheme::Madara);
		}
		if lower.contains("ts_reader")
			|| lower.contains("wp-content/themes/mangastream")
			|| lower.contains("wp-content/themes/mangareader")
			|| lower.contains("mangathemesia")
		{
			return Some(SourceTheme::MangaThemesia);
		}
		if lower.contains("/latest-release")
			&& (lower.contains("mangas-list") || lower.contains("cms-"))
		{
			return Some(SourceTheme::Mmrcms);
		}
		None
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogSource {
	/// Keiyoushi source id (a decimal string).
	pub id: String,
	pub name: String,
	pub lang: String,
	pub base_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
	pub name: String,
	pub pkg: String,
	pub apk_url: Option<String>,
	pub icon_url: Option<String>,
	pub lang: String,
	pub version: String,
	pub nsfw: bool,
	pub sources: Vec<CatalogSource>,
}

#[derive(Debug)]
pub struct CatalogSnapshot {
	pub fetched_at: DateTime<Utc>,
	pub entries: Vec<CatalogEntry>,
}

impl CatalogSnapshot {
	pub fn find_source(
		&self,
		catalog_id: &str,
	) -> Option<(&CatalogEntry, &CatalogSource)> {
		self.entries.iter().find_map(|entry| {
			entry
				.sources
				.iter()
				.find(|source| source.id == catalog_id)
				.map(|source| (entry, source))
		})
	}

	pub fn is_stale(&self, max_age: Duration) -> bool {
		let age = Utc::now() - self.fetched_at;
		age.to_std().map(|age| age > max_age).unwrap_or(true)
	}
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
	#[error("Failed to download the source catalog: {0}")]
	Http(#[from] reqwest::Error),
	#[error("Catalog download returned HTTP {0}")]
	Status(u16),
	#[error("Unrecognised catalog index format: {0}")]
	Parse(String),
	#[error("Catalog cache I/O failed: {0}")]
	Io(#[from] std::io::Error),
}

pub struct SourceCatalog {
	client: reqwest::Client,
	url: String,
	path: PathBuf,
	snapshot: RwLock<Option<Arc<CatalogSnapshot>>>,
}

impl std::fmt::Debug for SourceCatalog {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("SourceCatalog")
			.field("url", &self.url)
			.field("path", &self.path)
			.finish()
	}
}

impl SourceCatalog {
	/// `cache_dir` is the provider cache root; the index is stored beneath it.
	pub fn new(client: reqwest::Client, cache_dir: &Path, url: Option<String>) -> Self {
		Self {
			client,
			url: url.unwrap_or_else(|| DEFAULT_INDEX_URL.to_string()),
			path: cache_dir.join(INDEX_FILE_NAME),
			snapshot: RwLock::new(None),
		}
	}

	pub fn url(&self) -> &str {
		&self.url
	}

	pub fn path(&self) -> &Path {
		&self.path
	}

	/// The in-memory snapshot, if one was loaded or fetched.
	pub fn current(&self) -> Option<Arc<CatalogSnapshot>> {
		self.snapshot.read().expect("catalog lock poisoned").clone()
	}

	/// Load the on-disk copy without touching the network.
	pub async fn load_cached(
		&self,
	) -> Result<Option<Arc<CatalogSnapshot>>, CatalogError> {
		let bytes = match tokio::fs::read(&self.path).await {
			Ok(bytes) => bytes,
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
				return Ok(None)
			},
			Err(error) => return Err(error.into()),
		};
		let fetched_at = tokio::fs::metadata(&self.path)
			.await
			.ok()
			.and_then(|metadata| metadata.modified().ok())
			.map(DateTime::<Utc>::from)
			.unwrap_or_else(Utc::now);
		let snapshot = Arc::new(CatalogSnapshot {
			fetched_at,
			entries: parse_index(&bytes)?,
		});
		self.store(snapshot.clone());
		Ok(Some(snapshot))
	}

	/// Download the index, persist it, and replace the snapshot.
	pub async fn refresh(&self) -> Result<Arc<CatalogSnapshot>, CatalogError> {
		let response = self.client.get(&self.url).send().await?;
		if !response.status().is_success() {
			return Err(CatalogError::Status(response.status().as_u16()));
		}
		let bytes = response.bytes().await?;
		let entries = parse_index(&bytes)?;
		if let Some(parent) = self.path.parent() {
			tokio::fs::create_dir_all(parent).await?;
		}
		let temporary = self
			.path
			.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
		tokio::fs::write(&temporary, &bytes).await?;
		tokio::fs::rename(&temporary, &self.path).await?;
		let snapshot = Arc::new(CatalogSnapshot {
			fetched_at: Utc::now(),
			entries,
		});
		self.store(snapshot.clone());
		Ok(snapshot)
	}

	/// Memory, then disk, then network.
	pub async fn snapshot(&self) -> Result<Arc<CatalogSnapshot>, CatalogError> {
		if let Some(snapshot) = self.current() {
			return Ok(snapshot);
		}
		if let Some(snapshot) = self.load_cached().await? {
			return Ok(snapshot);
		}
		self.refresh().await
	}

	/// Refresh when the on-disk copy is older than [`REFRESH_INTERVAL`].
	pub async fn refresh_if_stale(&self) -> Result<Arc<CatalogSnapshot>, CatalogError> {
		match self.snapshot().await {
			Ok(snapshot) if !snapshot.is_stale(REFRESH_INTERVAL) => Ok(snapshot),
			Ok(_) => self.refresh().await,
			Err(error) => Err(error),
		}
	}

	fn store(&self, snapshot: Arc<CatalogSnapshot>) {
		*self.snapshot.write().expect("catalog lock poisoned") = Some(snapshot);
	}
}

/// Parse either index shape into catalog entries.
pub fn parse_index(bytes: &[u8]) -> Result<Vec<CatalogEntry>, CatalogError> {
	let value: serde_json::Value = serde_json::from_slice(bytes)
		.map_err(|error| CatalogError::Parse(error.to_string()))?;
	match value {
		serde_json::Value::Array(_) => {
			let entries: Vec<LegacyEntry> = serde_json::from_value(value)
				.map_err(|error| CatalogError::Parse(error.to_string()))?;
			Ok(entries.into_iter().map(CatalogEntry::from).collect())
		},
		serde_json::Value::Object(_) => {
			let index: RepoIndex = serde_json::from_value(value)
				.map_err(|error| CatalogError::Parse(error.to_string()))?;
			Ok(index
				.extension_list
				.extensions
				.into_iter()
				.map(CatalogEntry::from)
				.collect())
		},
		_ => Err(CatalogError::Parse(
			"expected a JSON array or object".to_string(),
		)),
	}
}

/// `eu.kanade.tachiyomi.extension.<lang>.<name>` -> `<lang>`.
fn lang_from_pkg(pkg: &str) -> Option<&str> {
	pkg.strip_prefix("eu.kanade.tachiyomi.extension.")
		.and_then(|rest| rest.split('.').next())
		.filter(|lang| !lang.is_empty())
}

#[derive(Deserialize)]
struct LegacyEntry {
	name: String,
	pkg: String,
	#[serde(default)]
	apk: Option<String>,
	#[serde(default)]
	lang: Option<String>,
	#[serde(default)]
	version: Option<String>,
	#[serde(default)]
	nsfw: u8,
	#[serde(default)]
	sources: Vec<LegacySource>,
}

#[derive(Deserialize)]
struct LegacySource {
	id: String,
	name: String,
	lang: String,
	#[serde(rename = "baseUrl")]
	base_url: String,
}

impl From<LegacyEntry> for CatalogEntry {
	fn from(entry: LegacyEntry) -> Self {
		let lang = entry
			.lang
			.or_else(|| lang_from_pkg(&entry.pkg).map(str::to_string))
			.unwrap_or_else(|| "all".to_string());
		CatalogEntry {
			name: entry.name,
			apk_url: entry.apk.map(|apk| {
				format!("https://raw.githubusercontent.com/keiyoushi/extensions/repo/apk/{apk}")
			}),
			icon_url: None,
			lang,
			version: entry.version.unwrap_or_default(),
			nsfw: entry.nsfw != 0,
			sources: entry
				.sources
				.into_iter()
				.map(|source| CatalogSource {
					id: source.id,
					name: source.name,
					lang: source.lang,
					base_url: source.base_url,
				})
				.collect(),
			pkg: entry.pkg,
		}
	}
}

#[derive(Deserialize)]
struct RepoIndex {
	#[serde(rename = "extensionList")]
	extension_list: RepoExtensionList,
}

#[derive(Deserialize)]
struct RepoExtensionList {
	#[serde(default)]
	extensions: Vec<RepoExtension>,
}

#[derive(Deserialize)]
struct RepoExtension {
	name: String,
	#[serde(rename = "packageName")]
	package_name: String,
	#[serde(default)]
	resources: RepoResources,
	#[serde(rename = "versionName", default)]
	version_name: String,
	#[serde(rename = "contentWarning", default)]
	content_warning: Option<String>,
	#[serde(default)]
	sources: Vec<RepoSource>,
}

#[derive(Deserialize, Default)]
struct RepoResources {
	#[serde(rename = "apkUrl", default)]
	apk_url: Option<String>,
	#[serde(rename = "iconUrl", default)]
	icon_url: Option<String>,
}

#[derive(Deserialize)]
struct RepoSource {
	id: String,
	name: String,
	#[serde(default)]
	language: String,
	#[serde(rename = "homeUrl", default)]
	home_url: String,
}

impl From<RepoExtension> for CatalogEntry {
	fn from(extension: RepoExtension) -> Self {
		let lang = lang_from_pkg(&extension.package_name)
			.map(str::to_string)
			.unwrap_or_else(|| "all".to_string());
		CatalogEntry {
			name: extension.name,
			apk_url: extension.resources.apk_url,
			icon_url: extension.resources.icon_url,
			lang,
			version: extension.version_name,
			nsfw: extension.content_warning.as_deref().is_some_and(|warning| {
				warning.eq_ignore_ascii_case("CONTENT_WARNING_NSFW")
			}),
			sources: extension
				.sources
				.into_iter()
				.map(|source| CatalogSource {
					id: source.id,
					name: source.name,
					lang: source.language,
					base_url: source.home_url,
				})
				.collect(),
			pkg: extension.package_name,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const REPO_FIXTURE: &[u8] = include_bytes!("../tests/fixtures/keiyoushi-index.json");
	const LEGACY_FIXTURE: &[u8] =
		include_bytes!("../tests/fixtures/keiyoushi-index.legacy.json");

	#[test]
	fn parses_repo_index_fixture() {
		let entries = parse_index(REPO_FIXTURE).unwrap();
		assert_eq!(entries.len(), 4);

		let mangadex = entries
			.iter()
			.find(|entry| entry.pkg == "eu.kanade.tachiyomi.extension.all.mangadex")
			.expect("mangadex entry");
		assert_eq!(mangadex.name, "MangaDex");
		assert_eq!(mangadex.lang, "all");
		assert_eq!(mangadex.version, "1.4.212");
		assert!(!mangadex.nsfw);
		assert!(mangadex
			.apk_url
			.as_deref()
			.is_some_and(|url| url.ends_with("tachiyomi-all.mangadex-v1.4.212.apk")));
		assert_eq!(mangadex.sources.len(), 3);
		let english = mangadex
			.sources
			.iter()
			.find(|source| source.lang == "en")
			.expect("english mangadex source");
		assert_eq!(english.id, "2499283573021220255");
		assert_eq!(english.base_url, "https://mangadex.org");

		let nsfw = entries
			.iter()
			.find(|entry| entry.pkg == "eu.kanade.tachiyomi.extension.all.beauty3600000")
			.expect("nsfw entry");
		assert!(nsfw.nsfw);
		assert_eq!(nsfw.sources[0].base_url, "https://3600000.xyz");

		let madara = entries
			.iter()
			.find(|entry| entry.pkg == "eu.kanade.tachiyomi.extension.en.mangareadorg")
			.expect("madara entry");
		assert_eq!(madara.lang, "en");
		assert_eq!(madara.sources[0].lang, "en");
	}

	#[test]
	fn parses_legacy_index_fixture() {
		let entries = parse_index(LEGACY_FIXTURE).unwrap();
		assert_eq!(entries.len(), 2);
		let comick = &entries[1];
		assert_eq!(comick.pkg, "eu.kanade.tachiyomi.extension.all.comick");
		assert_eq!(comick.lang, "all");
		assert!(comick.nsfw);
		assert_eq!(comick.version, "1.4.55");
		assert_eq!(
			comick.apk_url.as_deref(),
			Some("https://raw.githubusercontent.com/keiyoushi/extensions/repo/apk/tachiyomi-all.comick-v1.4.55.apk")
		);
		assert_eq!(comick.sources[0].id, "5720883462268529433");
		assert_eq!(comick.sources[0].base_url, "https://comick.io");
	}

	#[test]
	fn rejects_unknown_shapes() {
		assert!(parse_index(b"42").is_err());
		assert!(parse_index(b"{\"nope\": []}").is_err());
	}

	#[test]
	fn snapshot_lookup_and_staleness() {
		let snapshot = CatalogSnapshot {
			fetched_at: Utc::now() - chrono::Duration::hours(48),
			entries: parse_index(REPO_FIXTURE).unwrap(),
		};
		let (entry, source) = snapshot.find_source("2499283573021220255").unwrap();
		assert_eq!(entry.name, "MangaDex");
		assert_eq!(source.lang, "en");
		assert!(snapshot.find_source("missing").is_none());
		assert!(snapshot.is_stale(REFRESH_INTERVAL));
	}

	#[test]
	fn detects_themes_from_markup() {
		assert_eq!(
			SourceTheme::detect("<link href='/wp-content/themes/madara/style.css'>"),
			Some(SourceTheme::Madara)
		);
		assert_eq!(
			SourceTheme::detect("<script>ts_reader.run({})</script>"),
			Some(SourceTheme::MangaThemesia)
		);
		assert_eq!(
			SourceTheme::detect(
				"<a href='/latest-release'>x</a><div class='mangas-list'>"
			),
			Some(SourceTheme::Mmrcms)
		);
		assert_eq!(SourceTheme::detect("<html></html>"), None);
		assert_eq!(SourceTheme::parse("MADARA"), Some(SourceTheme::Madara));
		assert_eq!(
			SourceTheme::Madara.latest_path(),
			"/manga/?m_orderby=latest"
		);
	}

	#[tokio::test]
	async fn load_cached_reads_index_from_disk() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join(INDEX_FILE_NAME), REPO_FIXTURE).unwrap();
		let catalog = SourceCatalog::new(
			reqwest::Client::new(),
			dir.path(),
			Some("http://127.0.0.1:9/".into()),
		);
		assert!(catalog.current().is_none());
		let snapshot = catalog
			.load_cached()
			.await
			.unwrap()
			.expect("cached snapshot");
		assert_eq!(snapshot.entries.len(), 4);
		assert!(catalog.current().is_some());

		let empty = tempfile::tempdir().unwrap();
		let missing = SourceCatalog::new(
			reqwest::Client::new(),
			empty.path(),
			Some("http://127.0.0.1:9/".into()),
		);
		assert!(missing.load_cached().await.unwrap().is_none());
	}
}
