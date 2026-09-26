//! In-memory caches for interactive provider lookups. Type-ahead and the
//! `/search` page repeat the same term many times within seconds; caching the
//! provider's hit list keeps that to one outbound request per term while
//! callers still compute per-request state (library matches, request status)
//! on every call. [`TtlCache`] is the bounded, time-limited map underneath;
//! [`BriefSearchCache`] keys it for [`MetadataProvider::search_media_brief`].

use std::{
	collections::HashMap,
	hash::Hash,
	sync::{Arc, Mutex},
	time::{Duration, Instant},
};

use crate::{
	error::MetadataProviderError,
	provider::MetadataProvider,
	types::{SearchOutcome, SearchQuery},
};

/// Identifies one cached search: who may see it, what was asked, how many.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
	scope: String,
	query: String,
	limit: u32,
}

struct CacheEntry<V> {
	inserted_at: Instant,
	value: V,
}

/// Bounded, time-limited map. When full, expired entries go first, then the
/// oldest entry. Values are cloned out, so callers store an `Arc` for
/// anything larger than a handle.
pub struct TtlCache<K, V> {
	ttl: Duration,
	capacity: usize,
	entries: Mutex<HashMap<K, CacheEntry<V>>>,
}

impl<K: Hash + Eq + Clone, V: Clone> TtlCache<K, V> {
	pub fn new(ttl: Duration, capacity: usize) -> Self {
		Self {
			ttl,
			capacity: capacity.max(1),
			entries: Mutex::new(HashMap::new()),
		}
	}

	pub fn get(&self, key: &K) -> Option<V> {
		let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
		entries
			.get(key)
			.filter(|entry| entry.inserted_at.elapsed() < self.ttl)
			.map(|entry| entry.value.clone())
	}

	pub fn insert(&self, key: K, value: V) {
		let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
		if entries.len() >= self.capacity && !entries.contains_key(&key) {
			let ttl = self.ttl;
			entries.retain(|_, entry| entry.inserted_at.elapsed() < ttl);
			if entries.len() >= self.capacity {
				if let Some(oldest) = entries
					.iter()
					.min_by_key(|(_, entry)| entry.inserted_at)
					.map(|(key, _)| key.clone())
				{
					entries.remove(&oldest);
				}
			}
		}
		entries.insert(
			key,
			CacheEntry {
				inserted_at: Instant::now(),
				value,
			},
		);
	}

	/// Number of live (unexpired) entries; used by tests and diagnostics.
	pub fn len(&self) -> usize {
		let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
		entries
			.values()
			.filter(|entry| entry.inserted_at.elapsed() < self.ttl)
			.count()
	}

	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}
}

/// [`TtlCache`] in front of [`MetadataProvider::search_media_brief`].
///
/// Entries are keyed by a caller-supplied `scope` (the credential identity
/// the results were fetched with), the whitespace-collapsed, lowercased
/// query title, and the limit.
pub struct BriefSearchCache {
	entries: TtlCache<CacheKey, Arc<SearchOutcome>>,
}

impl BriefSearchCache {
	pub fn new(ttl: Duration, capacity: usize) -> Self {
		Self {
			entries: TtlCache::new(ttl, capacity),
		}
	}

	/// Run `query` through `provider` unless an unexpired result for the same
	/// scope/query/limit exists. Errors are never cached.
	pub async fn search_media_brief(
		&self,
		scope: &str,
		provider: &dyn MetadataProvider,
		query: &SearchQuery,
	) -> Result<Arc<SearchOutcome>, MetadataProviderError> {
		let key = CacheKey {
			scope: scope.to_owned(),
			query: normalize_query(&query.title),
			limit: query.limit.unwrap_or(10),
		};
		if let Some(hit) = self.entries.get(&key) {
			return Ok(hit);
		}

		let outcome = Arc::new(provider.search_media_brief(query).await?);
		self.entries.insert(key, Arc::clone(&outcome));
		Ok(outcome)
	}

	/// Number of live (unexpired) entries; used by tests and diagnostics.
	pub fn len(&self) -> usize {
		self.entries.len()
	}

	pub fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}
}

fn normalize_query(title: &str) -> String {
	title
		.split_whitespace()
		.map(str::to_lowercase)
		.collect::<Vec<_>>()
		.join(" ")
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{
		mock_http::{render_ok, MockServer},
		HardcoverClient,
	};

	fn search_body(id: u32, title: &str) -> String {
		serde_json::json!({
			"data": { "search": { "results": { "hits": [{
				"document": { "id": id, "title": title }
			}] } } }
		})
		.to_string()
	}

	fn query(title: &str) -> SearchQuery {
		SearchQuery {
			title: title.to_string(),
			limit: Some(5),
			..Default::default()
		}
	}

	#[tokio::test]
	async fn repeated_query_hits_cache_and_different_query_fetches() {
		let server = MockServer::spawn(vec![
			render_ok(&search_body(1, "Project Hail Mary")),
			render_ok(&search_body(2, "Artemis")),
		]);
		let client = HardcoverClient::new("test-token".to_string(), Some(u32::MAX))
			.with_api_url(format!("{}/v1/graphql", server.url));
		let cache = BriefSearchCache::new(Duration::from_secs(300), 256);

		let first = cache
			.search_media_brief("user:a", &client, &query("Project Hail Mary"))
			.await
			.unwrap();
		let second = cache
			.search_media_brief("user:a", &client, &query("  project   HAIL mary "))
			.await
			.unwrap();
		assert_eq!(
			server.requests().len(),
			1,
			"second call must be served from cache"
		);
		assert!(Arc::ptr_eq(&first, &second));
		assert_eq!(second.candidates[0].external_id, "1");

		let other = cache
			.search_media_brief("user:a", &client, &query("Artemis"))
			.await
			.unwrap();
		assert_eq!(server.requests().len(), 2, "a different query must fetch");
		assert_eq!(other.candidates[0].external_id, "2");
		assert_eq!(cache.len(), 2);
	}

	#[tokio::test]
	async fn scope_and_limit_separate_entries_and_ttl_expires() {
		let server = MockServer::spawn(vec![
			render_ok(&search_body(1, "Dune")),
			render_ok(&search_body(1, "Dune")),
			render_ok(&search_body(1, "Dune")),
			render_ok(&search_body(1, "Dune")),
		]);
		let client = HardcoverClient::new("test-token".to_string(), Some(u32::MAX))
			.with_api_url(format!("{}/v1/graphql", server.url));

		let cache = BriefSearchCache::new(Duration::from_secs(300), 256);
		cache
			.search_media_brief("user:a", &client, &query("Dune"))
			.await
			.unwrap();
		cache
			.search_media_brief("user:b", &client, &query("Dune"))
			.await
			.unwrap();
		let wider = SearchQuery {
			limit: Some(20),
			..query("Dune")
		};
		cache
			.search_media_brief("user:a", &client, &wider)
			.await
			.unwrap();
		assert_eq!(server.requests().len(), 3);

		let expiring = BriefSearchCache::new(Duration::ZERO, 256);
		expiring
			.search_media_brief("user:a", &client, &query("Dune"))
			.await
			.unwrap();
		assert_eq!(server.requests().len(), 4);
		assert!(expiring.is_empty(), "zero ttl never serves a cached entry");
	}

	#[tokio::test]
	async fn capacity_evicts_oldest_entry() {
		let server = MockServer::spawn(
			(1..=4).map(|id| render_ok(&search_body(id, "x"))).collect(),
		);
		let client = HardcoverClient::new("test-token".to_string(), Some(u32::MAX))
			.with_api_url(format!("{}/v1/graphql", server.url));
		let cache = BriefSearchCache::new(Duration::from_secs(300), 2);

		for title in ["one", "two", "three"] {
			cache
				.search_media_brief("s", &client, &query(title))
				.await
				.unwrap();
			// Instant resolution can tie on fast machines; keep insertions ordered.
			std::thread::sleep(Duration::from_millis(2));
		}
		assert_eq!(cache.len(), 2);

		// "one" was evicted (oldest); "three" is still cached.
		cache
			.search_media_brief("s", &client, &query("three"))
			.await
			.unwrap();
		assert_eq!(server.requests().len(), 3);
		cache
			.search_media_brief("s", &client, &query("one"))
			.await
			.unwrap();
		assert_eq!(server.requests().len(), 4);
	}
}
