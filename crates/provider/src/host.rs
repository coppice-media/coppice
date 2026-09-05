//! The process-wide provider host: the registry of runnable source instances,
//! the page cache, the catalog, and the resolver that lets `stump_media`
//! serve `provider://` paths.

use std::{
	collections::HashMap,
	io::{Cursor, Write},
	path::PathBuf,
	sync::{Arc, Mutex, RwLock},
	time::{Duration, Instant},
};

use async_trait::async_trait;
use models::entity::{media, provider_source, series_metadata};
use sea_orm::{
	sea_query::Expr, ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection,
	EntityTrait, QueryFilter,
};
use stump_media::{
	virtual_media::{self, VirtualMediaResolver},
	ContentType, FileError,
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

use crate::{
	cache::{CacheKey, PageCache},
	catalog::{CatalogError, CatalogSnapshot, SourceCatalog},
	health::{self, HealthChecker, HealthRunSummary},
	source::{RemotePage, RemoteSeries, Source, SourceError},
	virtual_path::VirtualPath,
};

/// How long a resolved page list stays valid before `pages()` is asked again
/// (MangaDex at-home URLs expire after roughly fifteen minutes).
pub const MANIFEST_TTL: Duration = Duration::from_secs(10 * 60);

/// Describes one compiled source implementation and how to instantiate it
/// for a `provider_sources` row.
#[derive(Clone)]
pub struct SourceFactory {
	/// Implementation id stored in `provider_sources.implementation`, e.g. `mangadex`.
	pub implementation: &'static str,
	pub name: &'static str,
	/// Keiyoushi extension package the implementation stands in for.
	pub catalog_pkg: &'static str,
	/// Base URL used when an instance is created without a catalog entry.
	pub base_url: &'static str,
	pub build: fn(&provider_source::Model) -> Result<Arc<dyn Source>, ProviderError>,
}

impl std::fmt::Debug for SourceFactory {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("SourceFactory")
			.field("implementation", &self.implementation)
			.field("catalog_pkg", &self.catalog_pkg)
			.finish()
	}
}

impl SourceFactory {
	/// Instance id for a language, e.g. `mangadex-en`.
	pub fn instance_id(&self, lang: &str) -> String {
		let lang = lang.trim().to_ascii_lowercase();
		format!(
			"{}-{}",
			self.implementation,
			if lang.is_empty() {
				"all"
			} else {
				lang.as_str()
			}
		)
	}
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
	#[error("Provider source `{0}` is not enabled")]
	UnknownSource(String),
	#[error("No compiled source implements catalog entry `{0}`")]
	NotImplemented(String),
	#[error("Catalog source `{0}` was not found in the Keiyoushi index")]
	CatalogSourceNotFound(String),
	#[error("`{0}` is not a provider-backed media path")]
	NotVirtual(String),
	#[error("Page {page} does not exist; the chapter has {available} pages")]
	PageOutOfRange { page: i32, available: usize },
	#[error("HTTP client error: {0}")]
	Http(#[from] reqwest::Error),
	#[error(transparent)]
	Source(#[from] SourceError),
	#[error(transparent)]
	Catalog(#[from] CatalogError),
	#[error("Database error: {0}")]
	Db(#[from] sea_orm::DbErr),
	#[error("I/O error: {0}")]
	Io(#[from] std::io::Error),
	#[error("Failed to build archive: {0}")]
	Archive(String),
	#[error("{0}")]
	Other(String),
}

impl From<ProviderError> for FileError {
	fn from(error: ProviderError) -> Self {
		match error {
			ProviderError::PageOutOfRange { page, available } => {
				FileError::PageNotFound {
					page: page.max(0) as usize,
					available,
				}
			},
			ProviderError::Source(SourceError::NotFound(_)) => FileError::NotFound,
			ProviderError::NotVirtual(path) => FileError::UnsupportedFileType(path),
			other => FileError::UnknownError(other.to_string()),
		}
	}
}

/// Settings the host needs from `StumpConfig`.
#[derive(Debug, Clone)]
pub struct ProviderHostConfig {
	/// Root for provider caches, normally `<config>/cache/providers`.
	pub cache_dir: PathBuf,
	pub cache_max_bytes: u64,
	pub catalog_url: Option<String>,
}

struct Manifest {
	resolved_at: Instant,
	pages: Arc<Vec<RemotePage>>,
}

/// A virtual CBZ built from cached pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualArchive {
	pub file_name: String,
	pub content_type: ContentType,
	pub bytes: Vec<u8>,
}

pub struct ProviderHost {
	conn: DatabaseConnection,
	factories: Vec<SourceFactory>,
	sources: RwLock<HashMap<String, Arc<dyn Source>>>,
	cache: PageCache,
	catalog: SourceCatalog,
	checker: HealthChecker,
	manifests: Mutex<HashMap<String, Manifest>>,
}

impl std::fmt::Debug for ProviderHost {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ProviderHost")
			.field("factories", &self.factories)
			.field("cache", &self.cache)
			.field("catalog", &self.catalog)
			.finish()
	}
}

impl ProviderHost {
	/// Open the page cache, prepare the catalog, and instantiate every enabled
	/// `provider_sources` row that has a compiled implementation.
	pub async fn open(
		conn: DatabaseConnection,
		factories: Vec<SourceFactory>,
		config: ProviderHostConfig,
	) -> Result<Arc<Self>, ProviderError> {
		let cache =
			PageCache::open(config.cache_dir.join("pages"), config.cache_max_bytes)
				.await?;
		let client = crate::http::build_client(None, crate::http::DEFAULT_TIMEOUT)?;
		let catalog = SourceCatalog::new(client, &config.cache_dir, config.catalog_url);
		let checker = HealthChecker::new()?;
		let host = Arc::new(Self {
			conn,
			factories,
			sources: RwLock::new(HashMap::new()),
			cache,
			catalog,
			checker,
			manifests: Mutex::new(HashMap::new()),
		});
		host.reload_sources().await?;
		Ok(host)
	}

	/// Make this host the resolver for `provider://` media paths.
	pub fn install(self: &Arc<Self>) {
		let resolver: Arc<dyn VirtualMediaResolver> = self.clone();
		virtual_media::register(resolver);
	}

	pub fn conn(&self) -> &DatabaseConnection {
		&self.conn
	}

	pub fn cache(&self) -> &PageCache {
		&self.cache
	}

	pub fn catalog(&self) -> &SourceCatalog {
		&self.catalog
	}

	pub fn factories(&self) -> &[SourceFactory] {
		&self.factories
	}

	pub fn factory_for_pkg(&self, pkg: &str) -> Option<&SourceFactory> {
		self.factories
			.iter()
			.find(|factory| factory.catalog_pkg == pkg)
	}

	pub fn factory_for_implementation(
		&self,
		implementation: &str,
	) -> Option<&SourceFactory> {
		self.factories
			.iter()
			.find(|factory| factory.implementation == implementation)
	}

	#[cfg(test)]
	pub(crate) fn clear_sources(&self) {
		self.sources
			.write()
			.expect("source registry poisoned")
			.clear();
	}

	/// Register a ready-made source instance (tests, embedded sources).
	pub fn register_source(&self, source: Arc<dyn Source>) {
		let id = source.info().id.clone();
		self.sources
			.write()
			.expect("source registry poisoned")
			.insert(id, source);
	}

	pub fn source(&self, id: &str) -> Result<Arc<dyn Source>, ProviderError> {
		self.sources
			.read()
			.expect("source registry poisoned")
			.get(id)
			.cloned()
			.ok_or_else(|| ProviderError::UnknownSource(id.to_string()))
	}

	pub fn sources(&self) -> Vec<Arc<dyn Source>> {
		let mut sources: Vec<_> = self
			.sources
			.read()
			.expect("source registry poisoned")
			.values()
			.cloned()
			.collect();
		sources.sort_by(|a, b| a.info().id.cmp(&b.info().id));
		sources
	}

	/// Rebuild the registry from enabled `provider_sources` rows, keeping
	/// sources registered by [`ProviderHost::register_source`] that have no row.
	pub async fn reload_sources(&self) -> Result<(), ProviderError> {
		let rows = provider_source::Entity::find()
			.filter(provider_source::Column::Enabled.eq(true))
			.all(&self.conn)
			.await?;
		let mut built: Vec<(String, Arc<dyn Source>)> = Vec::with_capacity(rows.len());
		for row in rows {
			let Some(factory) = self.factory_for_implementation(&row.implementation)
			else {
				tracing::warn!(
					source = row.id,
					implementation = row.implementation,
					"Enabled provider source has no compiled implementation; skipping"
				);
				continue;
			};
			match (factory.build)(&row) {
				Ok(source) => built.push((row.id.clone(), source)),
				Err(error) => {
					tracing::error!(?error, source = row.id, "Failed to build source")
				},
			}
		}
		let mut registry = self.sources.write().expect("source registry poisoned");
		for (id, source) in built {
			registry.insert(id, source);
		}
		Ok(())
	}

	/// Enable a catalog source. Only entries with a compiled implementation
	/// can be enabled; the instance id is `<implementation>-<lang>`.
	pub async fn enable_catalog_source(
		&self,
		catalog_id: &str,
		created_by: Option<&str>,
	) -> Result<provider_source::Model, ProviderError> {
		let snapshot = self.catalog.snapshot().await?;
		let (entry, catalog_source) =
			snapshot.find_source(catalog_id).ok_or_else(|| {
				ProviderError::CatalogSourceNotFound(catalog_id.to_string())
			})?;
		let factory = self
			.factory_for_pkg(&entry.pkg)
			.ok_or_else(|| ProviderError::NotImplemented(entry.pkg.clone()))?;
		let row = self
			.enable_instance(
				factory,
				&catalog_source.lang,
				Some(catalog_id),
				&catalog_source.name,
				&catalog_source.base_url,
				created_by,
			)
			.await?;
		Ok(row)
	}

	/// Enable an implementation for a language without a catalog entry.
	pub async fn enable_implementation(
		&self,
		implementation: &str,
		lang: &str,
		created_by: Option<&str>,
	) -> Result<provider_source::Model, ProviderError> {
		let factory = self
			.factory_for_implementation(implementation)
			.ok_or_else(|| ProviderError::NotImplemented(implementation.to_string()))?;
		self.enable_instance(
			factory,
			lang,
			None,
			factory.name,
			factory.base_url,
			created_by,
		)
		.await
	}

	async fn enable_instance(
		&self,
		factory: &SourceFactory,
		lang: &str,
		catalog_id: Option<&str>,
		name: &str,
		base_url: &str,
		created_by: Option<&str>,
	) -> Result<provider_source::Model, ProviderError> {
		let id = factory.instance_id(lang);
		let existing = provider_source::Entity::find_by_id(&id)
			.one(&self.conn)
			.await?;
		let row = match existing {
			Some(existing) => {
				let mut active: provider_source::ActiveModel = existing.into();
				active.enabled = Set(true);
				active.catalog_id = Set(catalog_id.map(str::to_string));
				active.name = Set(name.to_string());
				active.base_url = Set(base_url.to_string());
				active.update(&self.conn).await?
			},
			None => {
				provider_source::ActiveModel {
					id: Set(id.clone()),
					implementation: Set(factory.implementation.to_string()),
					catalog_id: Set(catalog_id.map(str::to_string)),
					name: Set(name.to_string()),
					lang: Set(if lang.trim().is_empty() {
						"all".to_string()
					} else {
						lang.trim().to_ascii_lowercase()
					}),
					base_url: Set(base_url.to_string()),
					enabled: Set(true),
					created_by: Set(created_by.map(str::to_string)),
					..Default::default()
				}
				.insert(&self.conn)
				.await?
			},
		};
		let source = (factory.build)(&row)?;
		self.register_source(source);
		Ok(row)
	}

	/// Disable an instance; materialised rows stay but pages stop resolving.
	pub async fn disable_source(&self, id: &str) -> Result<bool, ProviderError> {
		let Some(existing) = provider_source::Entity::find_by_id(id)
			.one(&self.conn)
			.await?
		else {
			return Ok(false);
		};
		let mut active: provider_source::ActiveModel = existing.into();
		active.enabled = Set(false);
		active.update(&self.conn).await?;
		self.sources
			.write()
			.expect("source registry poisoned")
			.remove(id);
		Ok(true)
	}

	pub async fn catalog_snapshot(&self) -> Result<Arc<CatalogSnapshot>, ProviderError> {
		Ok(self.catalog.snapshot().await?)
	}

	pub async fn refresh_catalog(&self) -> Result<Arc<CatalogSnapshot>, ProviderError> {
		Ok(self.catalog.refresh().await?)
	}

	/// Probe every catalog source, enabled instances first.
	pub async fn run_health_checks(
		&self,
		concurrency: usize,
	) -> Result<HealthRunSummary, ProviderError> {
		let snapshot = self.catalog.snapshot().await?;
		let priority: Vec<String> = self
			.sources()
			.iter()
			.map(|source| source.info().base_url.clone())
			.collect();
		Ok(health::check_catalog(
			&self.conn,
			&self.checker,
			&snapshot,
			&priority,
			concurrency,
		)
		.await?)
	}

	/// The page list for a chapter, re-resolved after [`MANIFEST_TTL`].
	pub async fn pages(
		&self,
		source_id: &str,
		chapter_id: &str,
	) -> Result<Arc<Vec<RemotePage>>, ProviderError> {
		let key = format!("{source_id}/{chapter_id}");
		if let Some(pages) = self.cached_manifest(&key) {
			return Ok(pages);
		}
		let source = self.source(source_id)?;
		let pages = Arc::new(source.pages(chapter_id).await?);
		self.manifests.lock().expect("manifests poisoned").insert(
			key,
			Manifest {
				resolved_at: Instant::now(),
				pages: pages.clone(),
			},
		);
		self.store_page_count(source_id, chapter_id, pages.len())
			.await;
		Ok(pages)
	}

	fn cached_manifest(&self, key: &str) -> Option<Arc<Vec<RemotePage>>> {
		let manifests = self.manifests.lock().expect("manifests poisoned");
		manifests
			.get(key)
			.filter(|manifest| manifest.resolved_at.elapsed() < MANIFEST_TTL)
			.map(|manifest| manifest.pages.clone())
	}

	/// Persist the page count on the media row the first time it is learned
	/// (or whenever the remote count changed).
	async fn store_page_count(&self, source_id: &str, chapter_id: &str, count: usize) {
		let count = i32::try_from(count).unwrap_or(i32::MAX);
		let result = media::Entity::update_many()
			.col_expr(media::Column::Pages, Expr::value(count))
			.filter(media::Column::SourceProvider.eq(source_id))
			.filter(media::Column::RemoteChapterId.eq(chapter_id))
			.filter(media::Column::Pages.ne(count))
			.exec(&self.conn)
			.await;
		if let Err(error) = result {
			tracing::warn!(
				?error,
				source_id,
				chapter_id,
				"Failed to store provider page count"
			);
		}
	}

	/// Bytes of a zero-based page, served from the cache when present.
	pub async fn page_bytes(
		&self,
		source_id: &str,
		chapter_id: &str,
		index: u32,
	) -> Result<(ContentType, Vec<u8>), ProviderError> {
		let key = CacheKey::page(source_id, chapter_id, index);
		if let Some(hit) = self.cache.get(key).await {
			return Ok(hit);
		}
		let pages = self.pages(source_id, chapter_id).await?;
		let page = pages.iter().find(|page| page.index == index).ok_or(
			ProviderError::PageOutOfRange {
				page: index as i32 + 1,
				available: pages.len(),
			},
		)?;
		let source = self.source(source_id)?;
		let fetch = async {
			let fetched = source.fetch_page(page).await?;
			let content_type = content_type_of(
				fetched.content_type.as_deref(),
				&page.url,
				&fetched.bytes,
			);
			Ok::<_, ProviderError>((content_type, fetched.bytes))
		};
		self.cache.get_or_fetch(key, fetch).await
	}

	/// Series cover, cached under `cover:<remote_id>`.
	pub async fn cover_bytes(
		&self,
		source_id: &str,
		remote_id: &str,
	) -> Result<(ContentType, Vec<u8>), ProviderError> {
		let item = format!("cover:{remote_id}");
		let key = CacheKey::page(source_id, &item, 0);
		if let Some(hit) = self.cache.get(key).await {
			return Ok(hit);
		}
		let source = self.source(source_id)?;
		let url = self.cover_url(source_id, remote_id).await?;
		let Some(url) = url else {
			return Err(ProviderError::Source(SourceError::NotFound(format!(
				"{remote_id} has no cover"
			))));
		};
		let fetch = async {
			let fetched = source.fetch_image(&url).await?;
			let content_type =
				content_type_of(fetched.content_type.as_deref(), &url, &fetched.bytes);
			Ok::<_, ProviderError>((content_type, fetched.bytes))
		};
		self.cache.get_or_fetch(key, fetch).await
	}

	/// The stored cover URL (`series_metadata.comic_image`), falling back to
	/// a details fetch.
	async fn cover_url(
		&self,
		source_id: &str,
		remote_id: &str,
	) -> Result<Option<String>, ProviderError> {
		let stored = series_metadata::Entity::find()
			.inner_join(models::entity::series::Entity)
			.filter(models::entity::series::Column::SourceProvider.eq(source_id))
			.filter(models::entity::series::Column::RemoteId.eq(remote_id))
			.one(&self.conn)
			.await?
			.and_then(|metadata| metadata.comic_image);
		if stored.is_some() {
			return Ok(stored);
		}
		let details: RemoteSeries = self.source(source_id)?.details(remote_id).await?;
		Ok(details.thumbnail_url)
	}

	/// Fetch every page (through the cache) and pack them into a Stored ZIP so
	/// clients that download books get a CBZ.
	pub async fn build_archive(
		&self,
		source_id: &str,
		chapter_id: &str,
		file_stem: &str,
	) -> Result<VirtualArchive, ProviderError> {
		let pages = self.pages(source_id, chapter_id).await?;
		let mut cursor = Cursor::new(Vec::new());
		{
			let mut writer = ZipWriter::new(&mut cursor);
			let options = SimpleFileOptions::default()
				.compression_method(CompressionMethod::Stored);
			for page in pages.iter() {
				let (content_type, bytes) =
					self.page_bytes(source_id, chapter_id, page.index).await?;
				let extension = match content_type.extension() {
					"" => "jpg",
					ext => ext,
				};
				writer
					.start_file(format!("{:04}.{extension}", page.index + 1), options)
					.map_err(|error| ProviderError::Archive(error.to_string()))?;
				writer
					.write_all(&bytes)
					.map_err(|error| ProviderError::Archive(error.to_string()))?;
			}
			writer
				.finish()
				.map_err(|error| ProviderError::Archive(error.to_string()))?;
		}
		Ok(VirtualArchive {
			file_name: format!("{}.cbz", sanitize_file_stem(file_stem)),
			content_type: ContentType::COMIC_ZIP,
			bytes: cursor.into_inner(),
		})
	}

	/// Resolve a virtual media path to its parts, rejecting other paths.
	pub fn parse_path(path: &str) -> Result<VirtualPath, ProviderError> {
		VirtualPath::parse(path)
			.ok_or_else(|| ProviderError::NotVirtual(path.to_string()))
	}
}

fn content_type_of(header: Option<&str>, url: &str, bytes: &[u8]) -> ContentType {
	let from_header = header
		.map(ContentType::from)
		.filter(|content_type| content_type.is_image());
	if let Some(content_type) = from_header {
		return content_type;
	}
	let extension = url
		.split(['?', '#'])
		.next()
		.and_then(|path| path.rsplit('.').next())
		.filter(|ext| ext.len() <= 5)
		.unwrap_or_default();
	let inferred = ContentType::from_bytes_with_fallback(bytes, extension);
	if inferred.is_image() {
		inferred
	} else {
		ContentType::JPEG
	}
}

fn sanitize_file_stem(stem: &str) -> String {
	let cleaned: String = stem
		.chars()
		.map(|c| match c {
			'/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
			c => c,
		})
		.collect();
	let trimmed = cleaned.trim();
	if trimmed.is_empty() {
		"chapter".to_string()
	} else {
		trimmed.to_string()
	}
}

#[async_trait]
impl VirtualMediaResolver for ProviderHost {
	fn owns(&self, path: &str) -> bool {
		VirtualPath::is_virtual(path)
	}

	async fn get_page(
		&self,
		path: &str,
		page: i32,
	) -> Result<(ContentType, Vec<u8>), FileError> {
		let virtual_path = Self::parse_path(path)?;
		match virtual_path.remote_chapter_id {
			Some(chapter) => {
				if page < 1 {
					return Err(FileError::PageNotFound {
						page: page.max(0) as usize,
						available: 0,
					});
				}
				Ok(self
					.page_bytes(&virtual_path.source_id, &chapter, (page - 1) as u32)
					.await?)
			},
			// A series path only has its cover.
			None => Ok(self
				.cover_bytes(&virtual_path.source_id, &virtual_path.remote_id)
				.await?),
		}
	}

	async fn get_page_count(&self, path: &str) -> Result<i32, FileError> {
		let virtual_path = Self::parse_path(path)?;
		match virtual_path.remote_chapter_id {
			Some(chapter) => {
				let pages = self.pages(&virtual_path.source_id, &chapter).await?;
				Ok(i32::try_from(pages.len()).unwrap_or(i32::MAX))
			},
			None => Ok(1),
		}
	}

	fn page_content_types(
		&self,
		path: &str,
		pages: &[i32],
	) -> Result<HashMap<i32, ContentType>, FileError> {
		let virtual_path = Self::parse_path(path)?;
		let Some(chapter) = virtual_path.remote_chapter_id else {
			return Ok(pages
				.iter()
				.map(|page| (*page, ContentType::JPEG))
				.collect());
		};
		let manifest =
			self.cached_manifest(&format!("{}/{chapter}", virtual_path.source_id));
		Ok(pages
			.iter()
			.map(|page| {
				let index = (*page - 1).max(0) as u32;
				let cached = self.cache.content_type(CacheKey::page(
					&virtual_path.source_id,
					&chapter,
					index,
				));
				let content_type = cached
					.or_else(|| {
						manifest.as_ref().and_then(|pages| {
							pages
								.iter()
								.find(|remote| remote.index == index)
								.map(|remote| content_type_of(None, &remote.url, &[]))
						})
					})
					.unwrap_or(ContentType::JPEG);
				(*page, content_type)
			})
			.collect())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn content_type_prefers_header_then_url_then_bytes() {
		assert_eq!(
			content_type_of(Some("image/png"), "https://x/1.jpg", &[]),
			ContentType::PNG
		);
		assert_eq!(
			content_type_of(
				Some("application/octet-stream"),
				"https://x/1.webp?x=1",
				&[]
			),
			ContentType::WEBP
		);
		let png_magic = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
		assert_eq!(
			content_type_of(None, "https://x/page", &png_magic),
			ContentType::PNG
		);
		assert_eq!(
			content_type_of(None, "https://x/page", &[]),
			ContentType::JPEG
		);
	}

	#[test]
	fn archive_names_are_filesystem_safe() {
		assert_eq!(
			sanitize_file_stem("Vol. 1 Ch. 2: Title?"),
			"Vol. 1 Ch. 2_ Title_"
		);
		assert_eq!(sanitize_file_stem("  "), "chapter");
	}

	#[test]
	fn instance_ids_are_lowercase_and_default_to_all() {
		let factory = SourceFactory {
			implementation: "mock",
			name: "Mock",
			catalog_pkg: "eu.kanade.tachiyomi.extension.all.mock",
			base_url: "http://mock",
			build: |_| Err(ProviderError::Other("unused".into())),
		};
		assert_eq!(factory.instance_id("EN"), "mock-en");
		assert_eq!(factory.instance_id(""), "mock-all");
	}
}
