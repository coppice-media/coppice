//! Durable, privacy-minimized catalog state for source workers.
//!
//! The catalog is the only place where a source worker keeps absolute paths.
//! Control frames contain opaque item IDs and sanitized display paths only.
//! Catalog writes use a temporary file followed by an atomic rename so a
//! process or machine failure cannot leave a half-written inventory.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use uuid::Uuid;

use crate::source_protocol::{
	SourceManifestItem, SourceReadGrant, SourceReadMode, SourceRootHello, SourceTransport,
};

pub const CATALOG_SCHEMA_VERSION: u32 = 1;
pub const CATALOG_FILE_NAME: &str = "catalog.json";
pub const QUICK_FINGERPRINT_BYTES: usize = 64 * 1024;
pub const DEFAULT_MAX_ENTRIES: usize = 1_000_000;
pub const PRIVACY_CATALOG: &str = "catalog";
pub const PRIVACY_CANDIDATE_ONLY: &str = "candidate_only";

/// A configured, explicit local root. This type is never deserialized from a
/// network frame and its path is never sent to the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRootConfig {
	pub root_id: String,
	pub label: String,
	pub kind: String,
	pub privacy_mode: String,
	pub path: PathBuf,
	pub transport: SourceTransport,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub direct_base_url: Option<String>,
}

impl SourceRootConfig {
	pub fn validate(&self) -> Result<(), CatalogError> {
		if self.root_id.is_empty()
			|| self.root_id.len() > 128
			|| self.root_id.contains('/')
			|| self.root_id.contains('\\')
			|| self.root_id == "."
			|| self.root_id == ".."
		{
			return Err(CatalogError::InvalidRoot(
				"root_id must be a simple opaque identifier".into(),
			));
		}
		if self.label.trim().is_empty() || self.kind.trim().is_empty() {
			return Err(CatalogError::InvalidRoot(
				"root label and kind are required".into(),
			));
		}
		if self.privacy_mode != PRIVACY_CATALOG {
			return Err(CatalogError::CandidateOnlyUnsupported);
		}
		if !self.path.is_absolute() {
			return Err(CatalogError::InvalidRoot(
				"source root must be absolute".into(),
			));
		}
		if self.transport == SourceTransport::Direct {
			let Some(base) = self.direct_base_url.as_deref() else {
				return Err(CatalogError::InvalidRoot(
					"direct roots require direct_base_url".into(),
				));
			};
			let parsed = reqwest::Url::parse(base).map_err(|_| {
				CatalogError::InvalidRoot(
					"direct_base_url must be an absolute HTTP URL".into(),
				)
			})?;
			if !matches!(parsed.scheme(), "http" | "https")
				|| parsed.host_str().is_none()
				|| !parsed.username().is_empty()
				|| parsed.password().is_some()
				|| parsed.query().is_some()
				|| parsed.fragment().is_some()
				|| parsed.path() != "/"
			{
				return Err(CatalogError::InvalidRoot(
					"direct_base_url must contain only an HTTP(S) origin".into(),
				));
			}
		} else if self.direct_base_url.is_some() {
			return Err(CatalogError::InvalidRoot(
				"tunnel roots cannot set direct_base_url".into(),
			));
		}
		Ok(())
	}

	/// Render only the public root advertisement. Absolute paths never cross
	/// this boundary.
	pub fn hello(&self) -> Result<SourceRootHello, CatalogError> {
		self.validate()?;
		Ok(SourceRootHello {
			root_id: self.root_id.clone(),
			label: self.label.clone(),
			kind: self.kind.clone(),
			privacy_mode: self.privacy_mode.clone(),
			transport: self.transport,
			direct_base_url: self.direct_base_url.clone(),
		})
	}
}

/// Parse one CLI root specification.
///
/// The syntax is `root_id=absolute/path` with optional semicolon-separated
/// fields: `root_id=...;label=...;kind=...;privacy=catalog`. A path is always
/// local CLI input; it is not accepted by any source protocol frame.
pub fn parse_source_root_spec(spec: &str) -> Result<(String, PathBuf), String> {
	let mut parts = spec.split(';');
	let first = parts
		.next()
		.ok_or_else(|| "source root is empty; use root_id=/absolute/path".to_owned())?;
	let (root_id, path) = first
		.split_once('=')
		.ok_or_else(|| "source root must use root_id=/absolute/path".to_owned())?;
	if root_id.trim().is_empty() || path.trim().is_empty() {
		return Err("source root requires a non-empty root_id and path".into());
	}
	let root_id = root_id.trim().to_owned();
	let path = PathBuf::from(path.trim());
	if !path.is_absolute() {
		return Err("source root path must be absolute".into());
	}
	validate_root_id(&root_id).map_err(|error| error.to_string())?;
	// Validate optional fields here even though this function returns only the
	// stable id/path pair. This catches typos rather than silently ignoring a
	// caller's attempted privacy or path mutation setting.
	for field in parts {
		let (key, value) = field
			.split_once('=')
			.ok_or_else(|| format!("source root field `{field}` must use key=value"))?;
		match key.trim() {
			"label" | "kind" | "privacy" | "privacy_mode" => {
				if value.trim().is_empty() {
					return Err(format!("source root field `{key}` is empty"));
				}
				if matches!(key.trim(), "privacy" | "privacy_mode")
					&& value.trim() != PRIVACY_CATALOG
				{
					return Err("candidate_only source roots are not supported".into());
				}
			},
			other => return Err(format!("unknown source root field `{other}`")),
		}
	}
	Ok((root_id, path))
}
/// Parse the complete local root configuration with the process-wide
/// transport choice. This is the preferred CLI helper; it still never accepts
/// a path from a network frame.
pub fn parse_source_root_config(
	spec: &str,
	transport: SourceTransport,
	direct_base_url: Option<String>,
) -> Result<SourceRootConfig, String> {
	let mut parts = spec.split(';');
	let first = parts
		.next()
		.ok_or_else(|| "source root is empty; use root_id=/absolute/path".to_owned())?;
	let (root_id, path) = first
		.split_once('=')
		.ok_or_else(|| "source root must use root_id=/absolute/path".to_owned())?;
	let root_id = root_id.trim().to_owned();
	let path = PathBuf::from(path.trim());
	validate_root_id(&root_id).map_err(|error| error.to_string())?;
	if !path.is_absolute() {
		return Err("source root path must be absolute".into());
	}
	let mut label = root_id.clone();
	let mut kind = String::from("library");
	let mut privacy_mode = String::from(PRIVACY_CATALOG);
	for field in parts {
		let (key, value) = field
			.split_once('=')
			.ok_or_else(|| format!("source root field `{field}` must use key=value"))?;
		let value = value.trim();
		if value.is_empty() {
			return Err(format!("source root field `{key}` is empty"));
		}
		match key.trim() {
			"label" => label = value.to_owned(),
			"kind" => kind = value.to_owned(),
			"privacy" | "privacy_mode" => privacy_mode = value.to_owned(),
			other => return Err(format!("unknown source root field `{other}`")),
		}
	}
	let config = SourceRootConfig {
		root_id,
		label,
		kind,
		privacy_mode,
		path,
		transport,
		direct_base_url,
	};
	config.validate().map_err(|error| error.to_string())?;
	Ok(config)
}

fn validate_root_id(root_id: &str) -> Result<(), CatalogError> {
	if root_id.is_empty()
		|| root_id.len() > 128
		|| root_id == "."
		|| root_id == ".."
		|| root_id.contains('/')
		|| root_id.contains('\\')
	{
		Err(CatalogError::InvalidRoot(
			"root_id must be a simple opaque identifier".into(),
		))
	} else {
		Ok(())
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
	pub worker_item_id: String,
	pub worker_content_version: String,
	pub root_id: String,
	pub absolute_path: PathBuf,
	pub relative_path: String,
	pub size: u64,
	pub modified_at_ms: Option<i64>,
	pub quick_fingerprint: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub media_type: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub metadata: Option<serde_json::Value>,
}

/// The scanner's observation before identity continuity is assigned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogObservation {
	pub absolute_path: PathBuf,
	pub relative_path: String,
	pub size: u64,
	pub modified_at_ms: Option<i64>,
	pub quick_fingerprint: String,
	pub media_type: Option<String>,
	pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSnapshot {
	pub root: SourceRootConfig,
	pub revision: u64,
	pub items: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSourceItem {
	pub root: SourceRootConfig,
	pub item: CatalogEntry,
	pub path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
	#[error("catalog I/O error: {0}")]
	Io(#[from] io::Error),
	#[error("catalog JSON error: {0}")]
	Json(#[from] serde_json::Error),
	#[error("invalid source root: {0}")]
	InvalidRoot(String),
	#[error(
		"candidate_only roots are unavailable until an interest-set protocol exists"
	)]
	CandidateOnlyUnsupported,
	#[error("source root `{0}` is not configured")]
	RootNotFound(String),
	#[error("source item `{item_id}` is not present in root `{root_id}`")]
	ItemNotFound { root_id: String, item_id: String },
	#[error("source item content version does not match the catalog")]
	VersionMismatch,
	#[error("source item does not match the server-verified SHA-256")]
	DigestMismatch,
	#[error("source grant is expired")]
	Expired,
	#[error("source grant range is invalid")]
	InvalidRange,
	#[error("source path is not a regular file below its configured root")]
	UnsafePath,
	#[error("catalog entry identity is ambiguous; refusing to guess")]
	AmbiguousIdentity,
	#[error("catalog revision is stale")]
	StaleRevision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedCatalog {
	schema_version: u32,
	roots: BTreeMap<String, PersistedRoot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedRoot {
	config: SourceRootConfig,
	revision: u64,
	entries: Vec<CatalogEntry>,
}

#[derive(Debug)]
struct CatalogState {
	roots: BTreeMap<String, PersistedRoot>,
}

/// The identity used to bind a cached verification to one local object.
///
/// The catalog lineage (`worker_item_id`/`worker_content_version`) is not
/// sufficient: a file can be replaced without a scan changing either value.
/// Unix change time is included in addition to size and mtime so an in-place
/// rewrite that restores mtime still invalidates the entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LocalFileIdentity {
	path: PathBuf,
	len: u64,
	modified: Option<(u64, u32)>,
	#[cfg(unix)]
	device: u64,
	#[cfg(unix)]
	inode: u64,
	#[cfg(unix)]
	changed: (i64, i64),
}

fn local_file_identity(path: &Path, metadata: &fs::Metadata) -> LocalFileIdentity {
	let modified = metadata.modified().ok().and_then(|value| {
		value
			.duration_since(UNIX_EPOCH)
			.ok()
			.map(|duration| (duration.as_secs(), duration.subsec_nanos()))
	});
	#[cfg(unix)]
	{
		use std::os::unix::fs::MetadataExt;
		LocalFileIdentity {
			path: path.to_path_buf(),
			len: metadata.len(),
			modified,
			device: metadata.dev(),
			inode: metadata.ino(),
			changed: (metadata.ctime(), metadata.ctime_nsec()),
		}
	}
	#[cfg(not(unix))]
	{
		LocalFileIdentity {
			path: path.to_path_buf(),
			len: metadata.len(),
			modified,
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct VerificationKey {
	identity: LocalFileIdentity,
	expected_sha256: String,
}

const VERIFIED_DIGEST_CACHE_CAPACITY: usize = 128;

#[derive(Debug, Default)]
struct VerificationCache {
	entries: HashMap<VerificationKey, ()>,
	order: VecDeque<VerificationKey>,
}

impl VerificationCache {
	fn invalidate_for_identity(&mut self, identity: &LocalFileIdentity) {
		self.entries.retain(|key, _| {
			&key.identity.path != &identity.path || &key.identity == identity
		});
		self.order.retain(|key| {
			&key.identity.path != &identity.path || &key.identity == identity
		});
	}

	fn contains(&mut self, identity: &LocalFileIdentity, expected_sha256: &str) -> bool {
		self.invalidate_for_identity(identity);
		let key = VerificationKey {
			identity: identity.clone(),
			expected_sha256: expected_sha256.to_owned(),
		};
		if !self.entries.contains_key(&key) {
			return false;
		}
		self.order.retain(|entry| entry != &key);
		self.order.push_back(key);
		true
	}

	fn insert(&mut self, identity: LocalFileIdentity, expected_sha256: &str) {
		self.invalidate_for_identity(&identity);
		let key = VerificationKey {
			identity,
			expected_sha256: expected_sha256.to_owned(),
		};
		self.entries.insert(key.clone(), ());
		self.order.retain(|entry| entry != &key);
		self.order.push_back(key);
		while self.order.len() > VERIFIED_DIGEST_CACHE_CAPACITY {
			if let Some(evicted) = self.order.pop_front() {
				self.entries.remove(&evicted);
			}
		}
	}
}

/// Durable local catalog. The mutex is intentionally synchronous: scanner
/// commits are short metadata transactions and atomic JSON writes do not hold
/// an async lock across `.await`.
#[derive(Debug, Clone)]
pub struct SourceCatalog {
	state_dir: Arc<PathBuf>,
	state: Arc<Mutex<CatalogState>>,
	verification_cache: Arc<Mutex<VerificationCache>>,
}

impl SourceCatalog {
	pub fn open(
		state_dir: impl AsRef<Path>,
		roots: Vec<SourceRootConfig>,
	) -> Result<Self, CatalogError> {
		let state_dir = state_dir.as_ref().to_path_buf();
		fs::create_dir_all(&state_dir)?;
		set_private_mode(&state_dir, true)?;
		let mut configured = BTreeMap::new();
		for root in roots {
			root.validate()?;
			if configured.insert(root.root_id.clone(), root).is_some() {
				return Err(CatalogError::InvalidRoot("duplicate root_id".into()));
			}
		}
		let path = state_dir.join(CATALOG_FILE_NAME);
		let mut persisted = if path.exists() {
			let bytes = fs::read(&path)?;
			serde_json::from_slice::<PersistedCatalog>(&bytes)?
		} else {
			PersistedCatalog {
				schema_version: CATALOG_SCHEMA_VERSION,
				roots: BTreeMap::new(),
			}
		};
		if persisted.schema_version != CATALOG_SCHEMA_VERSION {
			return Err(CatalogError::InvalidRoot(format!(
				"unsupported catalog schema {}",
				persisted.schema_version
			)));
		}
		// Configured roots are authoritative local input. Keep only those
		// roots, but retain their prior revision/entries for ID continuity.
		let mut roots_out = BTreeMap::new();
		for (root_id, config) in configured {
			if let Some(mut old) = persisted.roots.remove(&root_id) {
				old.config = config;
				roots_out.insert(root_id, old);
			} else {
				roots_out.insert(
					root_id,
					PersistedRoot {
						config,
						revision: 0,
						entries: Vec::new(),
					},
				);
			}
		}
		let catalog = Self {
			state_dir: Arc::new(state_dir),
			state: Arc::new(Mutex::new(CatalogState { roots: roots_out })),
			verification_cache: Arc::new(Mutex::new(VerificationCache::default())),
		};
		catalog.persist_locked()?;
		Ok(catalog)
	}

	#[must_use]
	pub fn state_path(&self) -> PathBuf {
		self.state_dir.join(CATALOG_FILE_NAME)
	}

	pub fn roots(&self) -> Vec<SourceRootConfig> {
		self.state
			.lock()
			.expect("source catalog mutex poisoned")
			.roots
			.values()
			.map(|root| root.config.clone())
			.collect()
	}

	pub fn root(&self, root_id: &str) -> Result<SourceRootConfig, CatalogError> {
		self.state
			.lock()
			.expect("source catalog mutex poisoned")
			.roots
			.get(root_id)
			.map(|root| root.config.clone())
			.ok_or_else(|| CatalogError::RootNotFound(root_id.to_owned()))
	}

	pub fn snapshot(&self, root_id: &str) -> Result<CatalogSnapshot, CatalogError> {
		let state = self.state.lock().expect("source catalog mutex poisoned");
		let root = state
			.roots
			.get(root_id)
			.ok_or_else(|| CatalogError::RootNotFound(root_id.to_owned()))?;
		Ok(CatalogSnapshot {
			root: root.config.clone(),
			revision: root.revision,
			items: root.entries.clone(),
		})
	}

	/// Replace one root's scan result and atomically persist its new revision.
	/// Identity continuity is retained only for an exact same-path record or a
	/// unique old size/mtime/fingerprint match. Ambiguous rename matches never
	/// guess an ID.
	pub fn commit_scan(
		&self,
		root_id: &str,
		mut observations: Vec<CatalogObservation>,
	) -> Result<CatalogSnapshot, CatalogError> {
		let mut state = self.state.lock().expect("source catalog mutex poisoned");
		let root = state
			.roots
			.get_mut(root_id)
			.ok_or_else(|| CatalogError::RootNotFound(root_id.to_owned()))?;
		observations.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
		let previous = std::mem::take(&mut root.entries);
		let mut by_path = HashMap::new();
		let mut by_signature: HashMap<(u64, Option<i64>, String), Vec<usize>> =
			HashMap::new();
		for (index, old) in previous.iter().enumerate() {
			by_path.insert(old.relative_path.clone(), index);
			by_signature
				.entry((old.size, old.modified_at_ms, old.quick_fingerprint.clone()))
				.or_default()
				.push(index);
		}
		let mut used_old = HashSet::new();
		let mut used_ids = HashSet::new();
		let mut entries = Vec::with_capacity(observations.len());
		for observation in observations {
			let signature = (
				observation.size,
				observation.modified_at_ms,
				observation.quick_fingerprint.clone(),
			);
			let old_index = by_path
				.get(&observation.relative_path)
				.copied()
				.filter(|index| !used_old.contains(index))
				.or_else(|| {
					let matches = by_signature.get(&signature)?;
					if matches.len() == 1 && !used_old.contains(&matches[0]) {
						Some(matches[0])
					} else {
						None
					}
				});
			let worker_item_id = old_index
				.and_then(|index| {
					used_old.insert(index);
					let id = previous[index].worker_item_id.clone();
					(!used_ids.contains(&id)).then_some(id)
				})
				.unwrap_or_else(new_item_id);
			used_ids.insert(worker_item_id.clone());
			let worker_content_version = content_version(
				observation.size,
				observation.modified_at_ms,
				&observation.quick_fingerprint,
			);
			entries.push(CatalogEntry {
				worker_item_id,
				worker_content_version,
				root_id: root_id.to_owned(),
				absolute_path: observation.absolute_path,
				relative_path: observation.relative_path,
				size: observation.size,
				modified_at_ms: observation.modified_at_ms,
				quick_fingerprint: observation.quick_fingerprint,
				media_type: observation.media_type,
				metadata: observation.metadata,
			});
		}
		root.entries = entries;
		root.revision = root.revision.saturating_add(1);
		let snapshot = CatalogSnapshot {
			root: root.config.clone(),
			revision: root.revision,
			items: root.entries.clone(),
		};
		self.persist_locked_state(&state)?;
		Ok(snapshot)
	}

	pub fn resolve(
		&self,
		root_id: &str,
		worker_item_id: &str,
	) -> Result<ResolvedSourceItem, CatalogError> {
		let state = self.state.lock().expect("source catalog mutex poisoned");
		let root = state
			.roots
			.get(root_id)
			.ok_or_else(|| CatalogError::RootNotFound(root_id.to_owned()))?;
		let item = root
			.entries
			.iter()
			.find(|entry| entry.worker_item_id == worker_item_id)
			.cloned()
			.ok_or_else(|| CatalogError::ItemNotFound {
				root_id: root_id.to_owned(),
				item_id: worker_item_id.to_owned(),
			})?;
		let path = checked_entry_path(&root.config, &item)?;
		Ok(ResolvedSourceItem {
			root: root.config.clone(),
			item,
			path,
		})
	}

	pub fn resolve_grant(
		&self,
		grant: &SourceReadGrant,
	) -> Result<ResolvedSourceItem, CatalogError> {
		grant.validate().map_err(|_| CatalogError::InvalidRange)?;
		if grant.is_expired() {
			return Err(CatalogError::Expired);
		}
		let resolved = self.resolve(&grant.root_id, &grant.worker_item_id)?;
		if resolved.item.worker_content_version != grant.worker_content_version {
			return Err(CatalogError::VersionMismatch);
		}
		validate_range(
			&resolved.item,
			grant.mode,
			grant.offset,
			grant.length,
			grant.max_bytes,
		)?;
		Ok(resolved)
	}

	/// Open a regular file only after validating the catalog lineage and path
	/// containment. The resulting file starts at the requested offset.
	pub fn open_grant(
		&self,
		grant: &SourceReadGrant,
	) -> Result<(ResolvedSourceItem, File), CatalogError> {
		let resolved = self.resolve_grant(grant)?;
		let mut file = open_safe_file(&resolved.root, &resolved.item, &resolved.path)?;
		if let Some(expected) = grant.expected_sha256.as_deref() {
			let identity = local_file_identity(&resolved.path, &file.metadata()?);
			let cached = self
				.verification_cache
				.lock()
				.expect("source verification cache mutex poisoned")
				.contains(&identity, expected);
			if !cached {
				let mut digest = Sha256::new();
				let mut buffer = [0_u8; 128 * 1024];
				loop {
					let read = file.read(&mut buffer)?;
					if read == 0 {
						break;
					}
					digest.update(&buffer[..read]);
				}
				let final_metadata = file.metadata()?;
				if final_metadata.len() != resolved.item.size
					|| local_file_identity(&resolved.path, &final_metadata) != identity
				{
					self.verification_cache
						.lock()
						.expect("source verification cache mutex poisoned")
						.invalidate_for_identity(&identity);
					return Err(CatalogError::VersionMismatch);
				}
				let actual = format!("{:x}", digest.finalize());
				if actual != expected {
					self.verification_cache
						.lock()
						.expect("source verification cache mutex poisoned")
						.invalidate_for_identity(&identity);
					return Err(CatalogError::DigestMismatch);
				}
				self.verification_cache
					.lock()
					.expect("source verification cache mutex poisoned")
					.insert(identity, expected);
			}
		}
		file.seek(SeekFrom::Start(grant.offset))?;
		Ok((resolved, file))
	}
	/// Read a bounded exact slice for focused tests and small control-plane
	/// probes. Production direct/tunnel paths stream through fixed-size
	/// buffers instead of allocating the whole object.
	pub fn read_exact_slice(
		&self,
		grant: &SourceReadGrant,
	) -> Result<Vec<u8>, CatalogError> {
		const MAX_HELPER_BYTES: u64 = 256 * 1024 * 1024;
		if grant.length > MAX_HELPER_BYTES || grant.length > usize::MAX as u64 {
			return Err(CatalogError::InvalidRange);
		}
		let (_resolved, mut file) = self.open_grant(grant)?;
		let mut bytes = Vec::with_capacity(grant.length as usize);
		let mut limited = (&mut file).take(grant.length);
		limited.read_to_end(&mut bytes)?;
		if bytes.len() as u64 != grant.length {
			return Err(CatalogError::VersionMismatch);
		}
		Ok(bytes)
	}

	/// Alias with the terminology used by byte-serving tests.
	pub fn read_slice(&self, grant: &SourceReadGrant) -> Result<Vec<u8>, CatalogError> {
		self.read_exact_slice(grant)
	}

	/// Compute a full SHA-256 after checking that the object still has the
	/// catalog's quick identity. It is deliberately opt-in and never included
	/// in ordinary inventory frames.
	pub async fn full_sha256(
		&self,
		root_id: &str,
		worker_item_id: &str,
		worker_content_version: &str,
	) -> Result<String, CatalogError> {
		let resolved = self.resolve(root_id, worker_item_id)?;
		if resolved.item.worker_content_version != worker_content_version {
			return Err(CatalogError::VersionMismatch);
		}
		let root = resolved.root.clone();
		let item = resolved.item.clone();
		let path = resolved.path.clone();
		tokio::task::spawn_blocking(move || {
			let mut file = open_safe_file(&root, &item, &path)?;
			let mut digest = Sha256::new();
			let mut buf = [0_u8; 128 * 1024];
			loop {
				let read = file.read(&mut buf)?;
				if read == 0 {
					break;
				}
				digest.update(&buf[..read]);
			}
			let actual = fs::metadata(&path)?;
			if !actual.is_file() || actual.len() != item.size {
				return Err(CatalogError::VersionMismatch);
			}
			Ok(format!("{:x}", digest.finalize()))
		})
		.await
		.map_err(|error| {
			CatalogError::Io(io::Error::other(format!("hash task failed: {error}")))
		})?
	}

	pub fn manifest_items(snapshot: &CatalogSnapshot) -> Vec<SourceManifestItem> {
		snapshot
			.items
			.iter()
			.map(|item| SourceManifestItem {
				worker_item_id: item.worker_item_id.clone(),
				worker_content_version: item.worker_content_version.clone(),
				relative_path: Some(item.relative_path.clone()),
				size: item.size,
				modified_at: item.modified_at_ms.map(Value::from),
				media_type: item.media_type.clone(),
				quick_fingerprint: Some(item.quick_fingerprint.clone()),
				metadata: item.metadata.clone(),
				sha256: None,
				retention: None,
			})
			.collect()
	}

	fn persist_locked(&self) -> Result<(), CatalogError> {
		let state = self.state.lock().expect("source catalog mutex poisoned");
		self.persist_locked_state(&state)
	}

	fn persist_locked_state(&self, state: &CatalogState) -> Result<(), CatalogError> {
		let persisted = PersistedCatalog {
			schema_version: CATALOG_SCHEMA_VERSION,
			roots: state.roots.clone(),
		};
		let bytes = serde_json::to_vec_pretty(&persisted)?;
		let mut temp = NamedTempFile::new_in(self.state_dir.as_path())?;
		temp.as_file_mut().write_all(&bytes)?;
		temp.as_file_mut().sync_all()?;
		temp.persist(self.state_path())
			.map_err(|error| CatalogError::Io(error.error))?;
		// Best effort directory sync is not available on every platform, but
		// the file itself is durable before the rename becomes visible.
		if let Ok(dir) = File::open(self.state_dir.as_path()) {
			let _ = dir.sync_all();
		}
		set_private_mode(&self.state_path(), false)?;
		Ok(())
	}
}

fn new_item_id() -> String {
	format!("item_{}", Uuid::new_v4().simple())
}

fn content_version(size: u64, modified_at_ms: Option<i64>, quick: &str) -> String {
	let mut digest = Sha256::new();
	digest.update(size.to_le_bytes());
	digest.update(modified_at_ms.unwrap_or_default().to_le_bytes());
	digest.update(quick.as_bytes());
	format!("v1-{:x}", digest.finalize())
}

fn checked_entry_path(
	root: &SourceRootConfig,
	entry: &CatalogEntry,
) -> Result<PathBuf, CatalogError> {
	if entry.root_id != root.root_id || !entry.absolute_path.is_absolute() {
		return Err(CatalogError::UnsafePath);
	}
	let relative =
		sanitize_relative_path(&entry.relative_path).ok_or(CatalogError::UnsafePath)?;
	let expected = root.path.join(&relative);
	// A stale catalog entry may not silently follow a root reconfiguration.
	if entry.absolute_path != expected {
		return Err(CatalogError::UnsafePath);
	}
	Ok(expected)
}

fn open_safe_file(
	root: &SourceRootConfig,
	item: &CatalogEntry,
	path: &Path,
) -> Result<File, CatalogError> {
	reject_symlink_ancestors(&root.path).map_err(|_| CatalogError::UnsafePath)?;
	let root_meta = fs::symlink_metadata(&root.path)?;
	if root_meta.file_type().is_symlink() || !root_meta.is_dir() {
		return Err(CatalogError::UnsafePath);
	}
	let relative = path
		.strip_prefix(&root.path)
		.map_err(|_| CatalogError::UnsafePath)?;
	let mut cursor = root.path.to_path_buf();
	for component in relative.components() {
		let Component::Normal(name) = component else {
			return Err(CatalogError::UnsafePath);
		};
		cursor.push(name);
		let metadata = fs::symlink_metadata(&cursor)?;
		if metadata.file_type().is_symlink() {
			return Err(CatalogError::UnsafePath);
		}
		if cursor.as_path() != path && !metadata.is_dir() {
			return Err(CatalogError::UnsafePath);
		}
	}
	let metadata = fs::symlink_metadata(path)?;
	if metadata.file_type().is_symlink()
		|| !metadata.is_file()
		|| metadata.len() != item.size
	{
		return Err(CatalogError::VersionMismatch);
	}
	let file = OpenOptions::new().read(true).open(path)?;
	Ok(file)
}

pub(crate) fn reject_symlink_ancestors(path: &Path) -> io::Result<()> {
	let mut cursor = PathBuf::new();
	for component in path.components() {
		match component {
			Component::Prefix(prefix) => cursor.push(prefix.as_os_str()),
			Component::RootDir => cursor.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
			Component::Normal(name) => cursor.push(name),
			Component::CurDir => {},
			Component::ParentDir => cursor.push(".."),
		}
		if cursor.as_os_str().is_empty()
			|| cursor == Path::new(std::path::MAIN_SEPARATOR_STR)
		{
			continue;
		}
		let metadata = fs::symlink_metadata(&cursor)?;
		if metadata.file_type().is_symlink() {
			return Err(io::Error::new(
				io::ErrorKind::PermissionDenied,
				"source root contains a symlink",
			));
		}
	}
	Ok(())
}

fn validate_range(
	item: &CatalogEntry,
	mode: SourceReadMode,
	offset: u64,
	length: u64,
	max_bytes: u64,
) -> Result<(), CatalogError> {
	if length > max_bytes {
		return Err(CatalogError::InvalidRange);
	}
	if mode == SourceReadMode::Full && (offset != 0 || length != item.size) {
		return Err(CatalogError::InvalidRange);
	}
	if offset > item.size || length > item.size.saturating_sub(offset) {
		return Err(CatalogError::InvalidRange);
	}
	Ok(())
}

/// Sanitize a relative display path. Empty paths and every `..`, root, prefix,
/// or control-containing component are rejected rather than normalized.
pub fn sanitize_relative_path(path: &str) -> Option<String> {
	if path.is_empty() || path.contains('\0') || path.contains('\\') {
		return None;
	}
	let mut components = Vec::new();
	for component in Path::new(path).components() {
		let Component::Normal(value) = component else {
			return None;
		};
		let value = value.to_str()?;
		if value.is_empty()
			|| value == "."
			|| value == ".."
			|| value.chars().any(char::is_control)
		{
			return None;
		}
		components.push(value);
	}
	(!components.is_empty()).then(|| components.join("/"))
}

pub fn quick_fingerprint(
	path: &Path,
	size: u64,
	modified_at_ms: Option<i64>,
) -> io::Result<String> {
	let mut file = File::open(path)?;
	let mut digest = Sha256::new();
	digest.update(size.to_le_bytes());
	digest.update(modified_at_ms.unwrap_or_default().to_le_bytes());
	let mut first = vec![0_u8; QUICK_FINGERPRINT_BYTES.min(size as usize)];
	file.read_exact(&mut first)?;
	digest.update(&first);
	if size > QUICK_FINGERPRINT_BYTES as u64 {
		file.seek(SeekFrom::Start(size - QUICK_FINGERPRINT_BYTES as u64))?;
		let mut last = vec![0_u8; QUICK_FINGERPRINT_BYTES];
		file.read_exact(&mut last)?;
		digest.update(&last);
	}
	Ok(format!("q1-{:x}", digest.finalize()))
}

pub fn modified_at_ms(metadata: &fs::Metadata) -> Option<i64> {
	metadata
		.modified()
		.ok()
		.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
		.and_then(|duration| i64::try_from(duration.as_millis()).ok())
}

fn set_private_mode(path: &Path, directory: bool) -> io::Result<()> {
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		let mode = if directory { 0o700 } else { 0o600 };
		fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use tempfile::tempdir;

	fn root(path: &Path) -> SourceRootConfig {
		SourceRootConfig {
			root_id: "root".into(),
			label: "Root".into(),
			kind: "library".into(),
			privacy_mode: PRIVACY_CATALOG.into(),
			path: path.to_path_buf(),
			transport: SourceTransport::Tunnel,
			direct_base_url: None,
		}
	}

	#[test]
	fn rejects_traversal_and_candidate_only() {
		assert!(sanitize_relative_path("../secret").is_none());
		assert!(sanitize_relative_path("/secret").is_none());
		assert_eq!(
			sanitize_relative_path("book/chapter.epub"),
			Some("book/chapter.epub".into())
		);
		let dir = tempdir().unwrap();
		let mut configured = root(dir.path());
		configured.privacy_mode = PRIVACY_CANDIDATE_ONLY.into();
		assert!(matches!(
			configured.validate(),
			Err(CatalogError::CandidateOnlyUnsupported)
		));
	}

	#[test]
	fn rename_keeps_id_only_for_unique_exact_identity() {
		let dir = tempdir().unwrap();
		let root_path = dir.path().join("root");
		fs::create_dir(&root_path).unwrap();
		let catalog =
			SourceCatalog::open(dir.path().join("state"), vec![root(&root_path)])
				.unwrap();
		let first = CatalogObservation {
			absolute_path: root_path.join("one.txt"),
			relative_path: "one.txt".into(),
			size: 3,
			modified_at_ms: Some(1),
			quick_fingerprint: "q".into(),
			media_type: Some("text/plain".into()),
			metadata: None,
		};
		let initial = catalog.commit_scan("root", vec![first.clone()]).unwrap();
		let id = initial.items[0].worker_item_id.clone();
		let renamed = CatalogObservation {
			absolute_path: root_path.join("two.txt"),
			relative_path: "two.txt".into(),
			..first
		};
		let next = catalog.commit_scan("root", vec![renamed]).unwrap();
		assert_eq!(next.items[0].worker_item_id, id);
	}

	#[test]
	fn verified_digest_binds_a_grant_to_the_exact_object() {
		let dir = tempdir().unwrap();
		let root_path = dir.path().join("root");
		fs::create_dir(&root_path).unwrap();
		let path = root_path.join("book.epub");
		fs::write(&path, b"abcdef").unwrap();
		let catalog =
			SourceCatalog::open(dir.path().join("state"), vec![root(&root_path)])
				.unwrap();
		let snapshot = catalog
			.commit_scan(
				"root",
				vec![CatalogObservation {
					absolute_path: path.clone(),
					relative_path: "book.epub".into(),
					size: 6,
					modified_at_ms: None,
					quick_fingerprint: "q".into(),
					metadata: None,
					media_type: Some("application/epub+zip".into()),
				}],
			)
			.unwrap();
		let item = &snapshot.items[0];
		let grant = SourceReadGrant {
			grant_id: "grant".into(),
			root_id: "root".into(),
			worker_item_id: item.worker_item_id.clone(),
			worker_content_version: item.worker_content_version.clone(),
			expected_sha256: Some(format!("{:x}", Sha256::digest(b"abcdef"))),
			mode: SourceReadMode::Range,
			offset: 2,
			length: 3,
			transport: SourceTransport::Tunnel,
			expires_at: i64::MAX,
			max_bytes: 3,
		};
		assert_eq!(catalog.read_exact_slice(&grant).unwrap(), b"cde");

		fs::write(path, b"abcdeg").unwrap();
		assert!(matches!(
			catalog.open_grant(&grant),
			Err(CatalogError::DigestMismatch)
		));
	}
}
