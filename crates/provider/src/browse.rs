//! Virtual-library live browse (provider host "Mode B").
//!
//! A virtual library is a `libraries` row with a `source_provider` and no
//! scanned files. Browsing it must never write rows: [`ProviderHost::browse`]
//! pages a [`Source`](crate::Source) live and caches each
//! `(source, kind, page)` result in a [`VirtualBrowseCache`] for
//! `virtual_series_ttl` (five minutes by default). Every remote series is
//! surfaced under its deterministic id —
//! [`virtual_path::series_id`]`(source, remote_id)` — so the same remote
//! series keeps one identity across browse calls, providers, and
//! materialisation.
//!
//! Deterministic ids are UUID v5 and therefore cannot be reversed, so the
//! cache also keeps a small reverse index from stump id to
//! `(source, remote_id)` for entries it has actually served. Index entries
//! expire with the browse entry they came from; materialised series are
//! always resolvable from the database regardless of this cache.

use std::{
	collections::HashMap,
	sync::Mutex,
	time::{Duration, Instant},
};

use crate::{
	source::{RemoteSeries, SearchFilter, SourcePage},
	virtual_path,
};

/// Separator for the browse cache key. `\x1f` cannot appear in source ids,
/// query text is hashed in, and the page is numeric.
const KEY_SEP: char = '\x1f';

/// The maximum number of cached entries kept before expired ones are pruned.
const PRUNE_THRESHOLD: usize = 1024;

/// Which Mihon-style browse a virtual library request maps to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseKind {
	/// Mihon "popular": engagement-ordered. Mapped from Komga sorts like
	/// `readCount`/`booksCount`; the neutral default when a feed has no sort.
	Popular,
	/// Mihon "latest": recency-ordered. The default for Komga requests
	/// sorted by `metadata.created`/`lastModified` or unsorted library feeds.
	Latest,
	/// Mihon "search": a query with optional source filters. Mapped from the
	/// Komga condition DSL `fullTextSearch`.
	Search {
		query: String,
		filters: Vec<SearchFilter>,
	},
}

impl BrowseKind {
	/// Stable, loggable cache-key component for this kind.
	pub fn cache_key(&self) -> String {
		match self {
			BrowseKind::Popular => "popular".to_string(),
			BrowseKind::Latest => "latest".to_string(),
			BrowseKind::Search { query, filters } => {
				let mut normalized = query.trim().to_ascii_lowercase();
				let mut parts: Vec<String> =
					filters.iter().map(|f| format!("{}={}", f.key, f.value)).collect();
				parts.sort();
				normalized.push_str(&parts.join(","));
				// Hash instead of storing raw queries to bound key size.
				format!("search:{:x}", md5::compute(normalized.as_bytes()))
			},
		}
	}
}

/// Where a deterministic series id came from, kept while its browse entry is
/// fresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteOrigin {
	pub source_id: String,
	pub remote_id: String,
	/// The virtual library the browse served; used when materialising.
	pub library_id: String,
}

struct BrowseEntry {
	resolved_at: Instant,
	page: Arc<SourcePage<RemoteSeries>>,
}

struct ReverseEntry {
	origin: RemoteOrigin,
	resolved_at: Instant,
}

/// TTL cache for virtual-library browse pages plus the deterministic-id
/// reverse index.
///
/// Both maps live behind their own mutexes; lookups clone cheap `Arc`s and
/// never block a source fetch.
#[derive(Debug)]
pub struct VirtualBrowseCache {
	entries: Mutex<HashMap<String, BrowseEntry>>,
	reverse: Mutex<HashMap<String, ReverseEntry>>,
	ttl: Duration,
}

impl VirtualBrowseCache {
	pub fn new(ttl: Duration) -> Self {
		Self {
			entries: Mutex::new(HashMap::new()),
			reverse: Mutex::new(HashMap::new()),
			ttl,
		}
	}

	pub fn ttl(&self) -> Duration {
		self.ttl
	}

	/// The full cache key for one browse page.
	pub fn key(source_id: &str, kind: &BrowseKind, page: u32) -> String {
		format!(
			"{source_id}{KEY_SEP}{}{KEY_SEP}{page}",
			kind.cache_key()
		)
	}

	/// A fresh cached page for `key`, if one exists.
	pub fn get(&self, key: &str) -> Option<Arc<SourcePage<RemoteSeries>>> {
		let entries = self.entries.lock().expect("browse cache poisoned");
		entries.get(key).filter(|entry| !self.expired(entry.resolved_at)).map(
			|entry| entry.page.clone(),
		)
	}

	/// Store a browse page and index every result's deterministic id.
	pub fn insert(
		&self,
		source_id: &str,
		library_id: &str,
		kind: &BrowseKind,
		page: u32,
		result: Arc<SourcePage<RemoteSeries>>,
	) {
		let now = Instant::now();
		let key = Self::key(source_id, kind, page);
		let mut entries = self.entries.lock().expect("browse cache poisoned");
		entries.insert(key, BrowseEntry { resolved_at: now, page: result });
		if entries.len() > PRUNE_THRESHOLD {
			entries.retain(|_, entry| !self.expired(entry.resolved_at));
		}
		drop(entries);

		let mut reverse = self.reverse.lock().expect("browse reverse poisoned");
		for item in &result.items {
			let stump_id = virtual_path::series_id(source_id, &item.remote_id);
			reverse.insert(
				stump_id,
				ReverseEntry {
					origin: RemoteOrigin {
						source_id: source_id.to_string(),
						remote_id: item.remote_id.clone(),
						library_id: library_id.to_string(),
					},
					resolved_at: now,
				},
			);
		}
		if reverse.len() > PRUNE_THRESHOLD {
			reverse.retain(|_, entry| !self.expired(entry.resolved_at));
		}
	}

	/// Reverse lookup of a deterministic series id served by a fresh browse.
	pub fn origin(&self, stump_series_id: &str) -> Option<RemoteOrigin> {
		let reverse = self.reverse.lock().expect("browse reverse poisoned");
		reverse
			.get(stump_series_id)
			.filter(|entry| !self.expired(entry.resolved_at))
			.map(|entry| entry.origin.clone())
	}

	/// Number of live browse entries (used by tests and health reporting).
	pub fn len(&self) -> usize {
		self.entries.lock().expect("browse cache poisoned").len()
	}

	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}

	fn expired(&self, resolved_at: Instant) -> bool {
		resolved_at.elapsed() >= self.ttl
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn series(remote_id: &str) -> RemoteSeries {
		RemoteSeries {
			remote_id: remote_id.to_string(),
			title: format!("Series {remote_id}"),
			..Default::default()
		}
	}

	fn page(ids: &[&str]) -> Arc<SourcePage<RemoteSeries>> {
		Arc::new(SourcePage {
			items: ids.iter().map(|id| series(id)).collect(),
			has_next: false,
		})
	}

	#[test]
	fn cache_hit_within_ttl_and_miss_after() {
		let cache = VirtualBrowseCache::new(Duration::from_secs(300));
		let kind = BrowseKind::Latest;
		cache.insert("mock-en", "lib-1", &kind, 0, page(&["alpha"]));

		let key = VirtualBrowseCache::key("mock-en", &kind, 0);
		let hit = cache.get(&key).expect("fresh entry must hit");
		assert_eq!(hit.items.len(), 1);
		assert_eq!(hit.items[0].remote_id, "alpha");

		// A zero TTL cache never serves fresh entries.
		let expired = VirtualBrowseCache::new(Duration::ZERO);
		expired.insert("mock-en", "lib-1", &kind, 0, page(&["alpha"]));
		assert!(expired.get(&key).is_none(), "expired entry must miss");
	}

	#[test]
	fn keys_distinguish_source_kind_and_page() {
		let search = BrowseKind::Search {
			query: "berserk".to_string(),
			filters: vec![SearchFilter {
				key: "lang".to_string(),
				value: "en".to_string(),
			}],
		};
		let search_again = BrowseKind::Search {
			query: "berserk".to_string(),
			filters: vec![SearchFilter {
				key: "lang".to_string(),
				value: "en".to_string(),
			}],
		};
		assert_eq!(
			VirtualBrowseCache::key("mock-en", &search, 0),
			VirtualBrowseCache::key("mock-en", &search_again, 0),
			"equal kinds must share a key"
		);
		assert_ne!(
			VirtualBrowseCache::key("mock-en", &search, 0),
			VirtualBrowseCache::key("mock-en", &search, 1)
		);
		assert_ne!(
			VirtualBrowseCache::key("mock-en", &search, 0),
			VirtualBrowseCache::key("mock-en", &BrowseKind::Popular, 0)
		);
		assert_ne!(
			VirtualBrowseCache::key("mock-en", &BrowseKind::Popular, 0),
			VirtualBrowseCache::key("mangadex-en", &BrowseKind::Popular, 0)
		);
	}

	#[test]
	fn reverse_index_resolves_deterministic_ids_until_expiry() {
		let cache = VirtualBrowseCache::new(Duration::from_secs(300));
		cache.insert("mock-en", "lib-1", &BrowseKind::Popular, 0, page(&["alpha"]));

		let stump_id = virtual_path::series_id("mock-en", "alpha");
		let origin = cache.origin(&stump_id).expect("browsed id must resolve");
		assert_eq!(origin.source_id, "mock-en");
		assert_eq!(origin.remote_id, "alpha");
		assert_eq!(origin.library_id, "lib-1");

		// Deterministic ids are stable: browsing again maps to the same key.
		cache.insert("mock-en", "lib-1", &BrowseKind::Popular, 0, page(&["alpha"]));
		assert!(cache.origin(&stump_id).is_some());

		let expired = VirtualBrowseCache::new(Duration::ZERO);
		expired.insert("mock-en", "lib-1", &BrowseKind::Popular, 0, page(&["alpha"]));
		assert!(
			expired.origin(&stump_id).is_none(),
			"expired reverse entries must not resolve"
		);
		assert!(cache.origin("not-a-real-id").is_none());
	}
}
