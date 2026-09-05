//! Bounded on-disk page cache keyed by `(source, item, page)`.
//!
//! Layout: `<root>/<source>/<md5(item)>/<page>.<ext>`; the extension records the
//! content type so a hit never sniffs bytes. Entries are published atomically
//! (`.tmp` + rename) like the KEPUB cache, and the total size is bounded by
//! `max_bytes` with least-recently-used eviction. The in-memory index is
//! rebuilt from the directory on open, using file mtimes as the initial
//! recency order.

use std::{
	collections::HashMap,
	future::Future,
	io,
	path::{Path, PathBuf},
	sync::Mutex,
	time::UNIX_EPOCH,
};

use stump_media::ContentType;

/// Identity of one cached blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey<'a> {
	/// Source instance id, e.g. `mangadex-en`.
	pub source: &'a str,
	/// Chapter id, or a cover namespace such as `cover:<series>`.
	pub item: &'a str,
	/// Zero-based page index (covers use 0).
	pub page: u32,
}

impl<'a> CacheKey<'a> {
	pub fn page(source: &'a str, chapter: &'a str, page: u32) -> Self {
		Self {
			source,
			item: chapter,
			page,
		}
	}

	/// Relative directory + file stem (without extension), also the index key.
	fn index_key(&self) -> String {
		format!(
			"{}/{:x}/{:05}",
			sanitize_component(self.source),
			md5::compute(self.item.as_bytes()),
			self.page
		)
	}
}

fn sanitize_component(value: &str) -> String {
	value
		.chars()
		.map(|c| {
			if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
				c
			} else {
				'_'
			}
		})
		.collect()
}

#[derive(Debug, Clone)]
struct Entry {
	extension: String,
	size: u64,
	last_used: u64,
}

#[derive(Debug, Default)]
struct State {
	entries: HashMap<String, Entry>,
	total_bytes: u64,
	clock: u64,
}

impl State {
	fn touch(&mut self, key: &str) -> Option<&Entry> {
		self.clock += 1;
		let clock = self.clock;
		let entry = self.entries.get_mut(key)?;
		entry.last_used = clock;
		Some(entry)
	}

	fn insert(&mut self, key: String, extension: String, size: u64) -> Option<Entry> {
		self.clock += 1;
		let previous = self.entries.insert(
			key,
			Entry {
				extension,
				size,
				last_used: self.clock,
			},
		);
		if let Some(previous) = &previous {
			self.total_bytes -= previous.size;
		}
		self.total_bytes += size;
		previous
	}

	fn remove(&mut self, key: &str) -> Option<Entry> {
		let removed = self.entries.remove(key)?;
		self.total_bytes -= removed.size;
		Some(removed)
	}

	/// Pop least-recently-used entries until the total fits `max_bytes`.
	fn evict_to(&mut self, max_bytes: u64) -> Vec<(String, Entry)> {
		if self.total_bytes <= max_bytes {
			return Vec::new();
		}
		let mut by_age: Vec<(String, u64)> = self
			.entries
			.iter()
			.map(|(key, entry)| (key.clone(), entry.last_used))
			.collect();
		by_age.sort_unstable_by_key(|(_, used)| *used);
		let mut evicted = Vec::new();
		for (key, _) in by_age {
			if self.total_bytes <= max_bytes {
				break;
			}
			if let Some(entry) = self.remove(&key) {
				evicted.push((key, entry));
			}
		}
		evicted
	}
}

#[derive(Debug)]
pub struct PageCache {
	root: PathBuf,
	max_bytes: u64,
	state: Mutex<State>,
}

impl PageCache {
	/// Open (creating if needed) the cache rooted at `root` and index its
	/// existing files.
	pub async fn open(root: impl Into<PathBuf>, max_bytes: u64) -> io::Result<Self> {
		let root = root.into();
		tokio::fs::create_dir_all(&root).await?;
		let mut state = State::default();
		let mut files = scan(&root).await?;
		files.sort_by_key(|(_, _, _, mtime)| *mtime);
		for (key, extension, size, _) in files {
			state.insert(key, extension, size);
		}
		let cache = Self {
			root,
			max_bytes,
			state: Mutex::new(state),
		};
		cache.enforce_bound().await;
		Ok(cache)
	}

	pub fn root(&self) -> &Path {
		&self.root
	}

	pub fn max_bytes(&self) -> u64 {
		self.max_bytes
	}

	pub fn total_bytes(&self) -> u64 {
		self.state.lock().expect("page cache poisoned").total_bytes
	}

	pub fn len(&self) -> usize {
		self.state.lock().expect("page cache poisoned").entries.len()
	}

	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}

	/// Content type of a cached entry without reading it (no recency bump).
	pub fn content_type(&self, key: CacheKey<'_>) -> Option<ContentType> {
		let state = self.state.lock().expect("page cache poisoned");
		state
			.entries
			.get(&key.index_key())
			.map(|entry| ContentType::from_extension(&entry.extension))
	}

	pub fn contains(&self, key: CacheKey<'_>) -> bool {
		self.state
			.lock()
			.expect("page cache poisoned")
			.entries
			.contains_key(&key.index_key())
	}

	/// Read a cached entry, marking it recently used. A vanished file is
	/// treated as a miss and dropped from the index.
	pub async fn get(&self, key: CacheKey<'_>) -> Option<(ContentType, Vec<u8>)> {
		let index_key = key.index_key();
		let extension = {
			let mut state = self.state.lock().expect("page cache poisoned");
			state.touch(&index_key)?.extension.clone()
		};
		let path = self.file_path(&index_key, &extension);
		match tokio::fs::read(&path).await {
			Ok(bytes) => Some((ContentType::from_extension(&extension), bytes)),
			Err(error) => {
				tracing::debug!(?error, path = %path.display(), "Dropping unreadable cache entry");
				self.state
					.lock()
					.expect("page cache poisoned")
					.remove(&index_key);
				None
			},
		}
	}

	/// Publish bytes for `key`, replacing any previous entry, then enforce
	/// the byte bound.
	pub async fn put(
		&self,
		key: CacheKey<'_>,
		content_type: ContentType,
		bytes: &[u8],
	) -> io::Result<()> {
		let index_key = key.index_key();
		let extension = match content_type.extension() {
			"" => "bin".to_string(),
			ext => ext.to_string(),
		};
		let path = self.file_path(&index_key, &extension);
		let dir = path.parent().expect("cache file always has a parent");
		tokio::fs::create_dir_all(dir).await?;
		let temporary = dir.join(format!(
			"{}.{}.tmp",
			path.file_name()
				.and_then(|name| name.to_str())
				.unwrap_or("page"),
			uuid::Uuid::new_v4()
		));
		tokio::fs::write(&temporary, bytes).await?;
		if let Err(error) = tokio::fs::rename(&temporary, &path).await {
			let _ = tokio::fs::remove_file(&temporary).await;
			return Err(error);
		}

		let previous = self
			.state
			.lock()
			.expect("page cache poisoned")
			.insert(index_key.clone(), extension.clone(), bytes.len() as u64);
		if let Some(previous) = previous {
			if previous.extension != extension {
				let _ = tokio::fs::remove_file(self.file_path(&index_key, &previous.extension))
					.await;
			}
		}
		self.enforce_bound().await;
		Ok(())
	}

	/// Return the cached bytes or run `fetch`, caching a successful result.
	/// A cache write failure is logged and does not fail the fetch.
	pub async fn get_or_fetch<E, F>(
		&self,
		key: CacheKey<'_>,
		fetch: F,
	) -> Result<(ContentType, Vec<u8>), E>
	where
		F: Future<Output = Result<(ContentType, Vec<u8>), E>>,
	{
		if let Some(hit) = self.get(key).await {
			return Ok(hit);
		}
		let (content_type, bytes) = fetch.await?;
		if let Err(error) = self.put(key, content_type, &bytes).await {
			tracing::warn!(?error, "Failed to write provider page cache entry");
		}
		Ok((content_type, bytes))
	}

	/// Remove every entry belonging to `(source, item)`.
	pub async fn invalidate_item(&self, source: &str, item: &str) {
		let prefix = format!(
			"{}/{:x}/",
			sanitize_component(source),
			md5::compute(item.as_bytes())
		);
		let removed: Vec<(String, Entry)> = {
			let mut state = self.state.lock().expect("page cache poisoned");
			let keys: Vec<String> = state
				.entries
				.keys()
				.filter(|key| key.starts_with(&prefix))
				.cloned()
				.collect();
			keys.into_iter()
				.filter_map(|key| state.remove(&key).map(|entry| (key, entry)))
				.collect()
		};
		for (key, entry) in removed {
			let _ = tokio::fs::remove_file(self.file_path(&key, &entry.extension)).await;
		}
	}

	fn file_path(&self, index_key: &str, extension: &str) -> PathBuf {
		self.root.join(format!("{index_key}.{extension}"))
	}

	async fn enforce_bound(&self) {
		let evicted = self
			.state
			.lock()
			.expect("page cache poisoned")
			.evict_to(self.max_bytes);
		for (key, entry) in evicted {
			let path = self.file_path(&key, &entry.extension);
			if let Err(error) = tokio::fs::remove_file(&path).await {
				tracing::debug!(?error, path = %path.display(), "Failed to evict cache file");
			}
		}
	}
}

/// Walk `<root>/<source>/<hash>/<page>.<ext>` and return
/// `(index_key, extension, size, mtime_secs)` for every complete entry.
async fn scan(root: &Path) -> io::Result<Vec<(String, String, u64, u64)>> {
	let mut files = Vec::new();
	let mut sources = tokio::fs::read_dir(root).await?;
	while let Some(source) = sources.next_entry().await? {
		if !source.file_type().await?.is_dir() {
			continue;
		}
		let source_name = source.file_name().to_string_lossy().into_owned();
		let mut items = tokio::fs::read_dir(source.path()).await?;
		while let Some(item) = items.next_entry().await? {
			if !item.file_type().await?.is_dir() {
				continue;
			}
			let item_name = item.file_name().to_string_lossy().into_owned();
			let mut pages = tokio::fs::read_dir(item.path()).await?;
			while let Some(page) = pages.next_entry().await? {
				let file_name = page.file_name().to_string_lossy().into_owned();
				if file_name.ends_with(".tmp") {
					let _ = tokio::fs::remove_file(page.path()).await;
					continue;
				}
				let Some((stem, extension)) = file_name.rsplit_once('.') else {
					continue;
				};
				let metadata = page.metadata().await?;
				if !metadata.is_file() {
					continue;
				}
				let mtime = metadata
					.modified()
					.ok()
					.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
					.map(|duration| duration.as_secs())
					.unwrap_or_default();
				files.push((
					format!("{source_name}/{item_name}/{stem}"),
					extension.to_string(),
					metadata.len(),
					mtime,
				));
			}
		}
	}
	Ok(files)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn key<'a>(chapter: &'a str, page: u32) -> CacheKey<'a> {
		CacheKey::page("mock", chapter, page)
	}

	#[tokio::test]
	async fn put_get_round_trip_records_content_type() {
		let dir = tempfile::tempdir().unwrap();
		let cache = PageCache::open(dir.path(), 1024).await.unwrap();
		assert!(cache.get(key("c1", 0)).await.is_none());
		cache
			.put(key("c1", 0), ContentType::PNG, b"png-bytes")
			.await
			.unwrap();
		let (content_type, bytes) = cache.get(key("c1", 0)).await.unwrap();
		assert_eq!(content_type, ContentType::PNG);
		assert_eq!(bytes, b"png-bytes");
		assert_eq!(cache.content_type(key("c1", 0)), Some(ContentType::PNG));
		assert_eq!(cache.total_bytes(), 9);
	}

	#[tokio::test]
	async fn eviction_drops_least_recently_used_until_under_bound() {
		let dir = tempfile::tempdir().unwrap();
		let cache = PageCache::open(dir.path(), 25).await.unwrap();
		cache
			.put(key("c1", 0), ContentType::JPEG, &[0u8; 10])
			.await
			.unwrap();
		cache
			.put(key("c1", 1), ContentType::JPEG, &[0u8; 10])
			.await
			.unwrap();
		// Touch page 0 so page 1 becomes the eviction candidate.
		assert!(cache.get(key("c1", 0)).await.is_some());
		cache
			.put(key("c2", 0), ContentType::JPEG, &[0u8; 10])
			.await
			.unwrap();

		assert!(cache.total_bytes() <= 25);
		assert!(cache.contains(key("c1", 0)));
		assert!(!cache.contains(key("c1", 1)));
		assert!(cache.contains(key("c2", 0)));
		assert!(cache.get(key("c1", 1)).await.is_none());
		assert_eq!(cache.len(), 2);
	}

	#[tokio::test]
	async fn reopen_rebuilds_index_from_disk_and_reapplies_bound() {
		let dir = tempfile::tempdir().unwrap();
		{
			let cache = PageCache::open(dir.path(), 1024).await.unwrap();
			cache
				.put(key("c1", 0), ContentType::WEBP, &[1u8; 30])
				.await
				.unwrap();
			cache
				.put(key("c1", 1), ContentType::WEBP, &[2u8; 30])
				.await
				.unwrap();
		}
		let stale = dir.path().join("mock").join("junk.tmp");
		std::fs::write(&stale, b"x").unwrap();

		let reopened = PageCache::open(dir.path(), 1024).await.unwrap();
		assert_eq!(reopened.len(), 2);
		assert_eq!(reopened.total_bytes(), 60);
		let (content_type, bytes) = reopened.get(key("c1", 1)).await.unwrap();
		assert_eq!(content_type, ContentType::WEBP);
		assert_eq!(bytes, vec![2u8; 30]);

		let bounded = PageCache::open(dir.path(), 40).await.unwrap();
		assert_eq!(bounded.len(), 1);
		assert!(bounded.total_bytes() <= 40);
	}

	#[tokio::test]
	async fn invalidate_item_removes_all_pages_of_a_chapter() {
		let dir = tempfile::tempdir().unwrap();
		let cache = PageCache::open(dir.path(), 1024).await.unwrap();
		cache.put(key("c1", 0), ContentType::PNG, b"a").await.unwrap();
		cache.put(key("c1", 1), ContentType::PNG, b"b").await.unwrap();
		cache.put(key("c2", 0), ContentType::PNG, b"c").await.unwrap();
		cache.invalidate_item("mock", "c1").await;
		assert!(!cache.contains(key("c1", 0)));
		assert!(!cache.contains(key("c1", 1)));
		assert!(cache.contains(key("c2", 0)));
		assert_eq!(cache.total_bytes(), 1);
	}

	#[tokio::test]
	async fn get_or_fetch_only_fetches_on_miss() {
		let dir = tempfile::tempdir().unwrap();
		let cache = PageCache::open(dir.path(), 1024).await.unwrap();
		let fetched = cache
			.get_or_fetch::<(), _>(key("c9", 0), async { Ok((ContentType::GIF, b"gif".to_vec())) })
			.await
			.unwrap();
		assert_eq!(fetched.0, ContentType::GIF);
		let hit = cache
			.get_or_fetch::<(), _>(key("c9", 0), async { Err(()) })
			.await
			.unwrap();
		assert_eq!(hit.1, b"gif");
	}
}
