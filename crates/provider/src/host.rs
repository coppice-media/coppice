//! The process-wide provider host: the registry of runnable source instances,
//! the page cache, the catalog, and the resolver that lets `stump_media`
//! serve `provider://` paths.

use std::{
	collections::HashMap,
	io::{Cursor, Write},
	path::PathBuf,
	sync::{Arc, Mutex, OnceLock, RwLock},
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
	browse::{BrowseKind, RemoteOrigin, VirtualBrowseCache},
	cache::{CacheKey, PageCache},
	catalog::{CatalogError, CatalogSnapshot, SourceCatalog},
	definition::{DefinitionEngine, DefinitionError, DefinitionLoader, SourceDefinition},
	event::{ProviderEvent, ProviderEventSink},
	health::{self, HealthChecker, HealthRunSummary},
	http::RequestHeaders,
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
	#[error("No compiled source or theme engine implements catalog entry `{0}`")]
	NotImplemented(String),
	#[error(transparent)]
	Definition(#[from] DefinitionError),
	#[error("Catalog source `{0}` was not found in the Keiyoushi index")]
	CatalogSourceNotFound(String),
	#[error("`{0}` is not a provider-backed media path")]
	NotVirtual(String),
	#[error("Page {page} does not exist; the chapter has {available} pages")]
	PageOutOfRange { page: i32, available: usize },
	#[error("HTTP client error: {0}")]
	Http(#[from] reqwest::Error),
	#[error(transparent)]
	Source(SourceError),
	/// The source's host answered with a Cloudflare managed challenge. The
	/// site is up: it refuses every client that cannot run the challenge
	/// script until the source carries a browser's `cf_clearance` cookie
	/// (`setProviderSourceHeaders`).
	#[error("`{host}` is behind a Cloudflare challenge; configure a `cf_clearance` cookie and matching User-Agent for the source")]
	Challenged { host: String },
	/// A `challenge_solve` was asked for on a server whose remote-worker
	/// queue was never installed. Not an outage: nothing is going to make
	/// this server able to queue one.
	#[error("The remote-worker queue is not available on this server")]
	NoWorkerQueue,
	#[error(transparent)]
	Catalog(#[from] CatalogError),
	#[error("Database error: {0}")]
	Db(#[from] sea_orm::DbErr),
	#[error("I/O error: {0}")]
	Io(#[from] std::io::Error),
	#[error("Failed to build archive: {0}")]
	Archive(String),
	/// The source has the chapter listed but cannot serve its pages: the page
	/// manifest lookup answered 404 (a licensed, taken-down, or externally
	/// hosted chapter). Distinct from a missing file, because there is
	/// nothing on disk to be missing.
	#[error("Chapter is not available from {source_id}")]
	Unavailable { source_id: String },
	#[error("{0}")]
	Other(String),
}

impl ProviderError {
	/// Whether the failure is an outage rather than a verdict about the
	/// source: retrying it later may well succeed, so nothing durable should
	/// be decided from it.
	///
	/// Network, filesystem, database and catalog failures are outages; a
	/// selector no engine implements, an unknown theme, a schema from the
	/// future or a definition that is simply not in the index are verdicts.
	pub fn is_transient(&self) -> bool {
		match self {
			ProviderError::Http(_)
			| ProviderError::Io(_)
			| ProviderError::Db(_)
			| ProviderError::Catalog(_)
			| ProviderError::Challenged { .. } => true,
			ProviderError::Source(error) => error.is_transient(),
			ProviderError::Definition(error) => error.is_transient(),
			_ => false,
		}
	}
}

/// A challenge is lifted out of `Source` so the host, the resolver, and the
/// API all see one error for it instead of a status buried in a source error.
impl From<SourceError> for ProviderError {
	fn from(error: SourceError) -> Self {
		match error {
			SourceError::Challenged { host } => ProviderError::Challenged { host },
			other => ProviderError::Source(other),
		}
	}
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
			ProviderError::Unavailable { .. } => {
				FileError::Unavailable(error.to_string())
			},
			// The page exists and the host is up; it is gated. `Unavailable`
			// is the lane-wide 404 with a reason, which is what every reader
			// (native, Komga, OPDS, Kobo) can show a user.
			ProviderError::Challenged { .. } => FileError::Unavailable(error.to_string()),
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
	/// Definition repository index URL, `file://` URL, or local directory
	/// (`STUMP_SOURCE_DEFINITIONS_URL`). `None` uses
	/// [`crate::definition::DEFAULT_DEFINITIONS_URL`].
	pub definitions_url: Option<String>,
	/// How long virtual-library browse pages stay cached before the source is
	/// hit again (`virtual_series_ttl`, five minutes by default).
	pub virtual_series_ttl: Duration,
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
	conn: Arc<DatabaseConnection>,
	factories: Vec<SourceFactory>,
	engines: Vec<DefinitionEngine>,
	sources: RwLock<HashMap<String, Arc<dyn Source>>>,
	cache: PageCache,
	catalog: SourceCatalog,
	definitions: DefinitionLoader,
	checker: HealthChecker,
	manifests: Mutex<HashMap<String, Manifest>>,
	browse: VirtualBrowseCache,
	/// Where host activity is announced. Installed once, after `open`, by
	/// whoever owns an event channel (`core/src/providers.rs`); absent in
	/// tests, which assert on rows instead.
	events: OnceLock<Arc<dyn ProviderEventSink>>,
	/// The remote-worker queue, for the `challenge_solve` jobs a gated source
	/// needs. Installed once by `core/src/providers.rs` after the queue's own
	/// registry is in place; absent in tests and on a build with no worker
	/// support, where asking for a solve is [`ProviderError::NoWorkerQueue`].
	worker_jobs: OnceLock<Arc<stump_worker::WorkerJobs>>,
	/// Instances a `challenge_solve` has been decided for but whose row may
	/// not exist yet: the enqueue happens on a spawned task, so this is what
	/// keeps a double-clicked "Solve now" from queueing twice
	/// (`crate::challenge`).
	pub(crate) solving: Mutex<std::collections::HashSet<String>>,
}

impl std::fmt::Debug for ProviderHost {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ProviderHost")
			.field("factories", &self.factories)
			.field("cache", &self.cache)
			.field("catalog", &self.catalog)
			.field("definitions", &self.definitions)
			.field("browse", &self.browse)
			.finish()
	}
}

impl ProviderHost {
	/// Open the page cache, prepare the catalog, and instantiate every enabled
	/// `provider_sources` row that has a compiled implementation.
	///
	/// A row that cannot be built is flipped back to `enabled = false`, so the
	/// enabled set the API reports is the set that actually runs.
	pub async fn open(
		conn: Arc<DatabaseConnection>,
		factories: Vec<SourceFactory>,
		engines: Vec<DefinitionEngine>,
		config: ProviderHostConfig,
	) -> Result<Arc<Self>, ProviderError> {
		let browse = VirtualBrowseCache::new(config.virtual_series_ttl);
		let cache =
			PageCache::open(config.cache_dir.join("pages"), config.cache_max_bytes)
				.await?;
		let client = crate::http::build_client(None, crate::http::DEFAULT_TIMEOUT)?;
		let catalog =
			SourceCatalog::new(client.clone(), &config.cache_dir, config.catalog_url);
		let definitions =
			DefinitionLoader::new(client, &config.cache_dir, config.definitions_url);
		let checker = HealthChecker::new()?;
		let host = Arc::new(Self {
			conn,
			factories,
			engines,
			sources: RwLock::new(HashMap::new()),
			cache,
			catalog,
			definitions,
			checker,
			manifests: Mutex::new(HashMap::new()),
			browse,
			events: OnceLock::new(),
			worker_jobs: OnceLock::new(),
			solving: Mutex::new(std::collections::HashSet::new()),
		});
		let unbuildable = host.reload_sources().await?;
		host.disable_unbuildable(&unbuildable).await?;
		Ok(host)
	}

	/// Make this host the resolver for `provider://` media paths.
	pub fn install(self: &Arc<Self>) {
		let resolver: Arc<dyn VirtualMediaResolver> = self.clone();
		virtual_media::register(resolver);
	}

	/// Install the sink host activity is announced on. Called once, right
	/// after [`ProviderHost::open`]; a second call is ignored.
	pub fn set_event_sink(&self, events: Arc<dyn ProviderEventSink>) {
		if self.events.set(events).is_err() {
			tracing::debug!("Provider event sink already installed");
		}
	}

	/// Install the remote-worker queue. Called once by `core/src/providers.rs`
	/// during startup; a second call is ignored.
	pub fn set_worker_jobs(&self, jobs: Arc<stump_worker::WorkerJobs>) {
		if self.worker_jobs.set(jobs).is_err() {
			tracing::debug!("Worker queue already installed on the provider host");
		}
	}

	/// The remote-worker queue, or the verdict that this server cannot queue
	/// remote work at all.
	pub fn worker_jobs(&self) -> Result<Arc<stump_worker::WorkerJobs>, ProviderError> {
		self.worker_jobs
			.get()
			.cloned()
			.ok_or(ProviderError::NoWorkerQueue)
	}

	/// Announce host activity, if a sink was installed.
	pub(crate) fn emit(&self, event: ProviderEvent) {
		if let Some(sink) = self.events.get() {
			sink.emit(event);
		}
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

	/// The runtime definition repository: `index.json` plus the definitions
	/// already loaded from it.
	pub fn definitions(&self) -> &DefinitionLoader {
		&self.definitions
	}

	pub fn engines(&self) -> &[DefinitionEngine] {
		&self.engines
	}

	pub fn engine_for_theme(&self, theme: &str) -> Option<&DefinitionEngine> {
		self.engines.iter().find(|engine| engine.theme == theme)
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
	///
	/// A row whose `implementation` is not a compiled factory is looked up in
	/// the definition index instead, so definition-backed sources survive a
	/// restart without any per-site code.
	///
	/// Answers the rows that could not be built, so the caller can decide
	/// what a skipped row means; a reload during operation only logs them,
	/// while [`ProviderHost::open`] repairs them.
	pub async fn reload_sources(
		&self,
	) -> Result<Vec<(String, ProviderError)>, ProviderError> {
		let rows = provider_source::Entity::find()
			.filter(provider_source::Column::Enabled.eq(true))
			.all(self.conn.as_ref())
			.await?;
		let mut built: Vec<(String, Arc<dyn Source>)> = Vec::with_capacity(rows.len());
		let mut failed: Vec<(String, ProviderError)> = Vec::new();
		for row in rows {
			match self.build_row(&row).await {
				Ok(source) => built.push((row.id.clone(), source)),
				Err(error) => {
					tracing::error!(
						?error,
						source = row.id,
						implementation = row.implementation,
						"Failed to build enabled provider source; skipping"
					);
					failed.push((row.id, error));
				},
			}
		}
		let mut registry = self.sources.write().expect("source registry poisoned");
		for (id, source) in built {
			registry.insert(id, source);
		}
		Ok(failed)
	}

	/// Flip rows this build cannot construct back to `enabled = false`.
	///
	/// Rows that claim to be enabled but cannot be built are invisible work:
	/// the API lists them as enabled, nothing resolves from them, and every
	/// boot logs the same failure. They exist because enabling used to
	/// persist the row before it built the instance, so a catalog-wide sweep
	/// left one behind for every source this build has no engine or no
	/// working selector for.
	///
	/// Only a verdict about the source disables it: a transient failure
	/// ([`ProviderError::is_transient`]) is an outage — an unreachable
	/// definition index would otherwise disable every definition-backed
	/// source on the first boot without network.
	async fn disable_unbuildable(
		&self,
		failures: &[(String, ProviderError)],
	) -> Result<Vec<String>, ProviderError> {
		let ids: Vec<&str> = failures
			.iter()
			.filter(|(_, error)| !error.is_transient())
			.map(|(id, _)| id.as_str())
			.collect();
		if ids.is_empty() {
			return Ok(Vec::new());
		}
		provider_source::Entity::update_many()
			.col_expr(provider_source::Column::Enabled, Expr::value(false))
			.col_expr(
				provider_source::Column::UpdatedAt,
				Expr::value(chrono::Utc::now().fixed_offset()),
			)
			.filter(provider_source::Column::Id.is_in(ids.iter().copied()))
			.exec(self.conn.as_ref())
			.await?;
		tracing::warn!(
			sources = ?ids,
			"Disabled provider sources this build cannot construct; the failure of each is logged above"
		);
		Ok(ids.into_iter().map(str::to_string).collect())
	}

	/// Instantiate one `provider_sources` row: a compiled factory when the
	/// implementation is one, otherwise the theme engine named by the
	/// definition whose id it is.
	async fn build_row(
		&self,
		row: &provider_source::Model,
	) -> Result<Arc<dyn Source>, ProviderError> {
		if let Some(factory) = self.factory_for_implementation(&row.implementation) {
			return (factory.build)(row);
		}
		let definition = self.definitions.definition(&row.implementation).await?;
		self.build_definition_source(&definition, row)
	}

	/// Hand a definition to its theme engine.
	pub fn build_definition_source(
		&self,
		definition: &SourceDefinition,
		row: &provider_source::Model,
	) -> Result<Arc<dyn Source>, ProviderError> {
		definition.validate()?;
		let engine = self.engine_for_theme(&definition.theme).ok_or_else(|| {
			DefinitionError::UnknownTheme {
				id: definition.id.clone(),
				theme: definition.theme.clone(),
			}
		})?;
		(engine.build)(definition, row)
	}

	/// Enable a catalog source. The entry must be backed either by a compiled
	/// implementation or by a definition for the same package; the instance id
	/// is `<implementation>-<lang>` for the former and the definition id for
	/// the latter.
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
		if let Some(factory) = self.factory_for_pkg(&entry.pkg) {
			return self
				.enable_instance(
					&factory.instance_id(&catalog_source.lang),
					factory.implementation,
					&catalog_source.lang,
					Some(catalog_id),
					&catalog_source.name,
					&catalog_source.base_url,
					created_by,
					&|row| (factory.build)(row),
				)
				.await;
		}
		let pkg = entry.pkg.clone();
		let lang = catalog_source.lang.clone();
		drop(snapshot);
		let definition = self
			.definitions
			.definition_for_pkg(&pkg, Some(&lang))
			.await?
			.ok_or(ProviderError::NotImplemented(pkg))?;
		self.enable_definition(&definition, &lang, Some(catalog_id), created_by)
			.await
	}

	/// Enable an implementation for a language without a catalog entry. A
	/// definition id is accepted here too, so a source can be brought up from
	/// `/tmp/stump-sources` before the Keiyoushi catalog knows about it.
	pub async fn enable_implementation(
		&self,
		implementation: &str,
		lang: &str,
		created_by: Option<&str>,
	) -> Result<provider_source::Model, ProviderError> {
		if let Some(factory) = self.factory_for_implementation(implementation) {
			return self
				.enable_instance(
					&factory.instance_id(lang),
					factory.implementation,
					lang,
					None,
					factory.name,
					factory.base_url,
					created_by,
					&|row| (factory.build)(row),
				)
				.await;
		}
		let definition = self.definitions.definition(implementation).await?;
		let lang = if lang.trim().is_empty() {
			definition.lang().to_string()
		} else {
			lang.to_string()
		};
		self.enable_definition(&definition, &lang, None, created_by)
			.await
	}

	/// Enable a definition-backed source. The instance id is the definition id,
	/// which already carries the language (`en.somesite`), so re-enabling is
	/// idempotent and survives a catalog id change.
	pub async fn enable_definition(
		&self,
		definition: &SourceDefinition,
		lang: &str,
		catalog_id: Option<&str>,
		created_by: Option<&str>,
	) -> Result<provider_source::Model, ProviderError> {
		definition.validate()?;
		let lang = if lang.trim().is_empty() {
			definition.lang()
		} else {
			lang
		};
		self.enable_instance(
			&definition.id,
			&definition.id,
			lang,
			catalog_id,
			&definition.name,
			definition.base_url(),
			created_by,
			&|row| self.build_definition_source(definition, row),
		)
		.await
	}

	/// Persist the row and register the instance it describes, or neither.
	///
	/// The build is the only thing that can tell whether this server can run
	/// the source at all — a selector the engine does not implement, a theme
	/// no engine claims, a schema from the future — and it needs the row to
	/// build from (the base URL override and the operator's request headers
	/// live there). So the row is written inside a transaction the failing
	/// build rolls back: a source that cannot be built leaves no enabled row
	/// (new) and does not touch the previous one (re-enable), instead of
	/// leaving behind an enabled row the host skips on every boot.
	#[allow(clippy::too_many_arguments)]
	async fn enable_instance(
		&self,
		id: &str,
		implementation: &str,
		lang: &str,
		catalog_id: Option<&str>,
		name: &str,
		base_url: &str,
		created_by: Option<&str>,
		build: &(dyn Fn(&provider_source::Model) -> Result<Arc<dyn Source>, ProviderError>
		      + Send
		      + Sync),
	) -> Result<provider_source::Model, ProviderError> {
		let txn = models::txn::begin_write(self.conn.as_ref()).await?;
		let existing = provider_source::Entity::find_by_id(id).one(&txn).await?;
		let row = match existing {
			Some(existing) => {
				let mut active: provider_source::ActiveModel = existing.into();
				active.enabled = Set(true);
				active.catalog_id = Set(catalog_id.map(str::to_string));
				active.name = Set(name.to_string());
				active.base_url = Set(base_url.to_string());
				active.update(&txn).await?
			},
			None => {
				provider_source::ActiveModel {
					id: Set(id.to_string()),
					implementation: Set(implementation.to_string()),
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
				.insert(&txn)
				.await?
			},
		};
		let source = match build(&row) {
			Ok(source) => source,
			Err(error) => {
				txn.rollback().await?;
				return Err(error);
			},
		};
		txn.commit().await?;
		self.register_source(source);
		Ok(row)
	}

	/// Disable an instance; materialised rows stay but pages stop resolving.
	pub async fn disable_source(&self, id: &str) -> Result<bool, ProviderError> {
		let Some(existing) = provider_source::Entity::find_by_id(id)
			.one(self.conn.as_ref())
			.await?
		else {
			return Ok(false);
		};
		let mut active: provider_source::ActiveModel = existing.into();
		active.enabled = Set(false);
		active.update(self.conn.as_ref()).await?;
		self.sources
			.write()
			.expect("source registry poisoned")
			.remove(id);
		Ok(true)
	}

	/// Replace the operator-configured request headers of one instance and
	/// rebuild it so the next request carries them.
	///
	/// The row is the only store: the built source holds a copy, so a source
	/// that is not rebuilt keeps sending the old cookie. A disabled instance
	/// has nothing in the registry — the headers are persisted and applied
	/// when it is enabled again.
	pub async fn set_source_headers(
		&self,
		id: &str,
		headers: RequestHeaders,
	) -> Result<(provider_source::Model, RequestHeaders), ProviderError> {
		let existing = provider_source::Entity::find_by_id(id)
			.one(self.conn.as_ref())
			.await?
			.ok_or_else(|| ProviderError::UnknownSource(id.to_string()))?;
		let mut active: provider_source::ActiveModel = existing.into();
		active.request_headers = Set(headers.to_json());
		let row = active.update(self.conn.as_ref()).await?;
		if row.enabled {
			self.register_source(self.build_row(&row).await?);
		}
		Ok((row, headers))
	}

	/// The headers configured for one instance, read back from the row.
	pub async fn source_headers(
		&self,
		id: &str,
	) -> Result<RequestHeaders, ProviderError> {
		let row = provider_source::Entity::find_by_id(id)
			.one(self.conn.as_ref())
			.await?
			.ok_or_else(|| ProviderError::UnknownSource(id.to_string()))?;
		Ok(RequestHeaders::parse(row.request_headers.as_deref()))
	}

	pub async fn catalog_snapshot(&self) -> Result<Arc<CatalogSnapshot>, ProviderError> {
		Ok(self.catalog.snapshot().await?)
	}

	pub async fn refresh_catalog(&self) -> Result<Arc<CatalogSnapshot>, ProviderError> {
		let snapshot = self.catalog.refresh().await?;
		self.emit(ProviderEvent::CatalogRefreshed {
			count: snapshot
				.entries
				.iter()
				.map(|entry| entry.sources.len() as u64)
				.sum(),
		});
		Ok(snapshot)
	}

	/// Probe every catalog source, enabled instances first. `dead_after` is
	/// the consecutive-failure count that marks a source dead
	/// (`provider_health_dead_after`).
	pub async fn run_health_checks(
		&self,
		concurrency: usize,
		dead_after: i32,
	) -> Result<HealthRunSummary, ProviderError> {
		let snapshot = self.catalog.snapshot().await?;
		let priority: Vec<String> = self
			.sources()
			.iter()
			.map(|source| source.info().base_url.clone())
			.collect();
		let summary = health::check_catalog(
			self.conn.as_ref(),
			&self.checker,
			&snapshot,
			&priority,
			concurrency,
			dead_after,
		)
		.await?;
		for change in &summary.changed {
			self.emit(change.as_event());
		}
		Ok(summary)
	}

	/// The health checker this host probes with, shared by the core health
	/// job so both paths use the same client and timeout.
	pub fn health_checker(&self) -> &HealthChecker {
		&self.checker
	}

	/// The page list for a chapter, re-resolved after [`MANIFEST_TTL`].
	pub async fn pages(
		&self,
		source_id: &str,
		chapter_id: &str,
	) -> Result<Arc<Vec<RemotePage>>, ProviderError> {
		if let Some(pages) = self.cached_manifest(&format!("{source_id}/{chapter_id}")) {
			return Ok(pages);
		}
		self.resolve_manifest(source_id, chapter_id).await
	}

	/// Ask the source whether it can serve `chapter_id` *now*, ignoring the
	/// cached manifest.
	///
	/// A feed can list a chapter its own source refuses: MangaDex reports
	/// `pages: 14, isUnavailable: false` for One Piece ch. 1191 while
	/// `/at-home/server/391c1555-…` answers `404`. The manifest is the only
	/// authority, and a cached one only proves what was true up to
	/// [`MANIFEST_TTL`] ago, so a reconciliation pass
	/// ([`crate::materialize`]) must re-ask.
	pub async fn verify_chapter(
		&self,
		source_id: &str,
		chapter_id: &str,
	) -> Result<(), ProviderError> {
		self.resolve_manifest(source_id, chapter_id).await.map(drop)
	}

	/// Fetch the page manifest from the source and cache it.
	async fn resolve_manifest(
		&self,
		source_id: &str,
		chapter_id: &str,
	) -> Result<Arc<Vec<RemotePage>>, ProviderError> {
		let source = self.source(source_id)?;
		// A 404 from the page-manifest lookup is the source saying it cannot
		// serve this chapter (MangaDex answers `/at-home/server/{id}` with
		// 404 for licensed or taken-down chapters). That is a permanent
		// per-chapter verdict, not a missing file.
		let pages = Arc::new(source.pages(chapter_id).await.map_err(
			|error| match error {
				SourceError::NotFound(_) => ProviderError::Unavailable {
					source_id: source_id.to_string(),
				},
				other => ProviderError::from(other),
			},
		)?);
		self.manifests.lock().expect("manifests poisoned").insert(
			format!("{source_id}/{chapter_id}"),
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
			.exec(self.conn.as_ref())
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
	///
	/// A row can lack `comic_image` when the browse response the series was
	/// materialised from carried no cover relationship, so the fallback is
	/// what makes `series/{id}/thumbnail` work at all for those. The fetched
	/// URL is written back so the next thumbnail request is one query again.
	async fn cover_url(
		&self,
		source_id: &str,
		remote_id: &str,
	) -> Result<Option<String>, ProviderError> {
		let stored = series_metadata::Entity::find()
			.inner_join(models::entity::series::Entity)
			.filter(models::entity::series::Column::SourceProvider.eq(source_id))
			.filter(models::entity::series::Column::RemoteId.eq(remote_id))
			.one(self.conn.as_ref())
			.await?
			.and_then(|metadata| metadata.comic_image);
		if stored.is_some() {
			return Ok(stored);
		}
		let details: RemoteSeries = self.source(source_id)?.details(remote_id).await?;
		if let Some(url) = details.thumbnail_url.as_deref() {
			self.store_cover_url(source_id, remote_id, url).await;
		}
		Ok(details.thumbnail_url)
	}

	/// Write a freshly discovered cover URL onto the materialised series'
	/// metadata row, if there is one. Live-only series have nothing to write.
	async fn store_cover_url(&self, source_id: &str, remote_id: &str, url: &str) {
		let series_id = crate::virtual_path::series_id(source_id, remote_id);
		let result = series_metadata::Entity::update_many()
			.col_expr(series_metadata::Column::ComicImage, Expr::value(url))
			.filter(series_metadata::Column::SeriesId.eq(series_id))
			.filter(series_metadata::Column::ComicImage.is_null())
			.exec(self.conn.as_ref())
			.await;
		if let Err(error) = result {
			tracing::warn!(?error, source_id, remote_id, "Failed to store cover URL");
		}
	}

	/// Whether the catalog marks the source instance's extension as adult.
	///
	/// Stump has no library-level age rating (`age_restrictions` is per user
	/// and compares against `series_metadata.age_rating`), so an adult
	/// source is applied per materialised series instead; see
	/// [`crate::materialize::add_series`].
	pub async fn source_is_adult(&self, source_id: &str) -> bool {
		let Ok(Some(row)) = provider_source::Entity::find_by_id(source_id)
			.one(self.conn.as_ref())
			.await
		else {
			return false;
		};
		let Some(catalog_id) = row.catalog_id else {
			return false;
		};
		self.catalog
			.current()
			.and_then(|snapshot| {
				snapshot
					.find_source(&catalog_id)
					.map(|(entry, _)| entry.nsfw)
			})
			.unwrap_or(false)
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

	/// Live browse for a virtual library: page the source, caching the
	/// result for `virtual_series_ttl` and indexing every returned series
	/// under its deterministic id. No rows are written.
	pub async fn browse(
		&self,
		source_id: &str,
		library_id: &str,
		kind: &BrowseKind,
		page: u32,
	) -> Result<Arc<crate::source::SourcePage<RemoteSeries>>, ProviderError> {
		let key = VirtualBrowseCache::key(source_id, kind, page);
		if let Some(cached) = self.browse.get(&key) {
			return Ok(cached);
		}
		let source = self.source(source_id)?;
		let result = match kind {
			BrowseKind::Popular => source.popular(page).await?,
			BrowseKind::Latest => source.latest(page).await?,
			BrowseKind::Search { query, filters } => {
				source.search(query, filters, page).await?
			},
		};
		let result = Arc::new(result);
		self.browse
			.insert(source_id, library_id, kind, page, result.clone());
		Ok(result)
	}

	/// Details for a remote series, fetched live (details are cheap enough
	/// not to cache; chapters and pages have their own flows).
	pub async fn remote_series_details(
		&self,
		source_id: &str,
		remote_id: &str,
	) -> Result<RemoteSeries, ProviderError> {
		Ok(self.source(source_id)?.details(remote_id).await?)
	}

	/// Where a deterministic series id served by a recent browse came from.
	pub fn virtual_series_origin(&self, stump_series_id: &str) -> Option<RemoteOrigin> {
		self.browse.origin(stump_series_id)
	}

	/// The configured virtual-series TTL.
	pub fn virtual_series_ttl(&self) -> std::time::Duration {
		self.browse.ttl()
	}

	/// Materialise (or extend) a remote series into `library_id`. The first
	/// call for a remote series creates the `series`/`media` rows under the
	/// same deterministic ids the browse surface reported.
	pub async fn materialise_series(
		&self,
		library_id: &str,
		source_id: &str,
		remote_id: &str,
	) -> Result<crate::materialize::Materialized, ProviderError> {
		crate::materialize::add_series(self, library_id, source_id, remote_id).await
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
	async fn get_archive(
		&self,
		path: &str,
		file_stem: &str,
	) -> Result<stump_media::virtual_media::VirtualArchive, FileError> {
		let virtual_path = Self::parse_path(path)?;
		let chapter = virtual_path.remote_chapter_id.ok_or_else(|| {
			FileError::UnsupportedFileType(
				"Provider-backed series covers do not have a downloadable archive"
					.to_owned(),
			)
		})?;
		let archive = self
			.build_archive(&virtual_path.source_id, &chapter, file_stem)
			.await?;
		Ok(stump_media::virtual_media::VirtualArchive {
			file_name: archive.file_name,
			content_type: archive.content_type,
			bytes: archive.bytes,
		})
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

	/// A host that reaches nothing: port 9 refuses every connection, so the
	/// catalog and the definition index are unavailable the way they are on a
	/// server without network.
	fn offline_config(dir: &tempfile::TempDir) -> ProviderHostConfig {
		ProviderHostConfig {
			cache_dir: dir.path().to_path_buf(),
			cache_max_bytes: u64::MAX,
			catalog_url: Some("http://127.0.0.1:9/".to_string()),
			definitions_url: Some("http://127.0.0.1:9/".to_string()),
			virtual_series_ttl: Duration::from_secs(300),
		}
	}

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

	/// The row is the only store, and the built instance holds a copy of it:
	/// setting headers must rebuild the source, or the next request still
	/// carries the cookie the operator just replaced.
	#[tokio::test]
	async fn setting_headers_persists_them_and_rebuilds_the_instance() {
		use std::sync::Mutex as StdMutex;

		/// What the factory saw the last time it built the instance.
		static BUILT: StdMutex<Vec<Option<String>>> = StdMutex::new(Vec::new());

		let conn = ::tests::db::test_database().await;
		let dir = tempfile::tempdir().unwrap();
		let factory = SourceFactory {
			implementation: "mock",
			name: "Mock",
			catalog_pkg: "eu.kanade.tachiyomi.extension.all.mock",
			base_url: "https://mock.test",
			build: |row| {
				BUILT
					.lock()
					.expect("built")
					.push(row.request_headers.clone());
				Ok(crate::mock::MockSource::with_id(&row.id))
			},
		};
		let host = ProviderHost::open(
			Arc::new(conn),
			vec![factory],
			Vec::new(),
			offline_config(&dir),
		)
		.await
		.unwrap();
		host.enable_implementation("mock", "en", None)
			.await
			.unwrap();
		assert_eq!(BUILT.lock().expect("built").last(), Some(&None));

		let headers = RequestHeaders::from_pairs([(
			"Cookie".to_string(),
			"cf_clearance=abcdefghijklmnop".to_string(),
		)])
		.expect("valid headers");
		let (row, _) = host
			.set_source_headers("mock-en", headers.clone())
			.await
			.unwrap();
		assert_eq!(
			row.request_headers.as_deref(),
			Some(r#"{"cookie":"cf_clearance=abcdefghijklmnop"}"#)
		);
		// Rebuilt, and rebuilt *with* the new headers.
		assert_eq!(
			BUILT.lock().expect("built").last(),
			Some(&row.request_headers)
		);
		assert_eq!(host.source_headers("mock-en").await.unwrap(), headers);

		// Clearing puts the column back to NULL rather than storing `{}`.
		let (cleared, _) = host
			.set_source_headers("mock-en", RequestHeaders::default())
			.await
			.unwrap();
		assert_eq!(cleared.request_headers, None);
		assert!(host.source_headers("mock-en").await.unwrap().is_empty());

		assert!(matches!(
			host.set_source_headers("nope", RequestHeaders::default())
				.await,
			Err(ProviderError::UnknownSource(id)) if id == "nope"
		));
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

	/// Enabling a source this build cannot construct must persist nothing: an
	/// enabled row the host cannot build resolves no page, yet the API lists
	/// it as enabled and every boot logs the same failure.
	#[tokio::test]
	async fn a_failed_build_persists_nothing() {
		use std::sync::atomic::{AtomicBool, Ordering};

		/// Whether the factory refuses to build, standing in for a selector
		/// no engine implements.
		static FAILING: AtomicBool = AtomicBool::new(false);

		let conn = Arc::new(::tests::db::test_database().await);
		let dir = tempfile::tempdir().unwrap();
		let factory = SourceFactory {
			implementation: "mock",
			name: "Mock",
			catalog_pkg: "eu.kanade.tachiyomi.extension.all.mock",
			base_url: "https://mock.test",
			build: |row| {
				if FAILING.load(Ordering::SeqCst) {
					return Err(ProviderError::Other("unsupported selector".into()));
				}
				Ok(crate::mock::MockSource::with_id(&row.id))
			},
		};
		let host = ProviderHost::open(
			conn.clone(),
			vec![factory],
			Vec::new(),
			offline_config(&dir),
		)
		.await
		.unwrap();

		// A source that never built leaves no row behind at all.
		FAILING.store(true, Ordering::SeqCst);
		let error = host
			.enable_implementation("mock", "en", None)
			.await
			.expect_err("a source that cannot be built cannot be enabled");
		assert!(matches!(error, ProviderError::Other(_)), "{error}");
		assert!(provider_source::Entity::find_by_id("mock-en")
			.one(conn.as_ref())
			.await
			.unwrap()
			.is_none());
		assert!(host.source("mock-en").is_err());

		// A re-enable that cannot build leaves the previous row untouched.
		FAILING.store(false, Ordering::SeqCst);
		host.enable_implementation("mock", "en", None)
			.await
			.unwrap();
		assert!(host.disable_source("mock-en").await.unwrap());
		FAILING.store(true, Ordering::SeqCst);
		host.enable_implementation("mock", "en", None)
			.await
			.expect_err("the build still fails");
		let row = provider_source::Entity::find_by_id("mock-en")
			.one(conn.as_ref())
			.await
			.unwrap()
			.expect("the row a previous enable created survives");
		assert!(!row.enabled, "a failed re-enable must not enable the row");
		assert!(host.source("mock-en").is_err());
	}

	/// The rows an earlier build left enabled-but-unbuildable heal on the
	/// next boot — but only when the failure is a verdict about the source.
	#[tokio::test]
	async fn boot_disables_unbuildable_rows_but_never_on_an_outage() {
		let conn = Arc::new(::tests::db::test_database().await);
		let dir = tempfile::tempdir().unwrap();
		for id in ["broken-en", "en.nosuch"] {
			provider_source::ActiveModel {
				id: Set(id.to_string()),
				implementation: Set(id.trim_end_matches("-en").to_string()),
				name: Set(id.to_string()),
				lang: Set("en".to_string()),
				base_url: Set("https://mock.test".to_string()),
				enabled: Set(true),
				..Default::default()
			}
			.insert(conn.as_ref())
			.await
			.unwrap();
		}
		let broken = SourceFactory {
			implementation: "broken",
			name: "Broken",
			catalog_pkg: "eu.kanade.tachiyomi.extension.all.broken",
			base_url: "https://mock.test",
			build: |_| Err(ProviderError::Other("unsupported selector".into())),
		};
		let host = ProviderHost::open(
			conn.clone(),
			vec![broken],
			Vec::new(),
			offline_config(&dir),
		)
		.await
		.unwrap();
		assert!(host.sources().is_empty());

		let broken_row = provider_source::Entity::find_by_id("broken-en")
			.one(conn.as_ref())
			.await
			.unwrap()
			.expect("row survives");
		assert!(!broken_row.enabled, "a row no engine can build is disabled");
		// The definition index is unreachable, so nothing is known about the
		// definition-backed row: disabling it would take out every such row
		// on the first boot without network.
		let definition_row = provider_source::Entity::find_by_id("en.nosuch")
			.one(conn.as_ref())
			.await
			.unwrap()
			.expect("row survives");
		assert!(
			definition_row.enabled,
			"an unreachable definition index must not disable anything"
		);
	}
}
