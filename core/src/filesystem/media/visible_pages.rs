//! Serve-time page filtering for librarian-marked duplicate pages.
//!
//! A library can mark a page hash as `SKIP` (see
//! `models::entity::known_duplicate_page`). Every page-serving route asks
//! [`visible_pages`] for the ordered list of physical 1-based pages that are
//! still visible and renumbers from it; the underlying files are never
//! modified. Results are cached per media on [`VisiblePagesCache`] and
//! invalidated when a mark changes or when analysis writes new hashes.

use std::{
	collections::HashMap,
	sync::{Arc, RwLock},
};

use models::{
	entity::{known_duplicate_page, media, page_hash, series},
	shared::enums::DuplicatePageAction,
};
use sea_orm::{
	ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};
use stump_media::hamming;

use crate::CoreResult;

/// Maximum Hamming distance between two dHashes for pages to count as the
/// same page. Shared by the quality check, the candidate aggregation, and the
/// serve-time skip.
pub const DUPLICATE_PAGE_TOLERANCE: u32 = 4;

/// Ordered physical page numbers (1-based) that remain visible for a media.
pub type VisiblePages = Arc<[i32]>;

/// Per-media cache of [`VisiblePages`].
#[derive(Default)]
pub struct VisiblePagesCache {
	entries: RwLock<HashMap<String, VisiblePages>>,
}

impl VisiblePagesCache {
	fn get(&self, media_id: &str) -> Option<VisiblePages> {
		self.entries
			.read()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
			.get(media_id)
			.cloned()
	}

	fn insert(&self, media_id: &str, pages: VisiblePages) {
		self.entries
			.write()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
			.insert(media_id.to_string(), pages);
	}

	/// Drop the entry for one media (new hashes were written).
	pub fn invalidate_media(&self, media_id: &str) {
		self.entries
			.write()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
			.remove(media_id);
	}

	/// Drop every entry (a library mark changed; the affected media set is
	/// not known cheaply and the cache refills lazily).
	pub fn invalidate_all(&self) {
		self.entries
			.write()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
			.clear();
	}
}

/// Whether `hash` is within [`DUPLICATE_PAGE_TOLERANCE`] of any hash in `set`.
pub fn matches_any(hash: i64, set: &[i64]) -> bool {
	set.iter()
		.any(|known| hamming(hash as u64, *known as u64) <= DUPLICATE_PAGE_TOLERANCE)
}

/// Load the `SKIP` hashes of the library that owns `media_id`.
async fn skip_hashes_for_media(
	conn: &DatabaseConnection,
	media_id: &str,
) -> CoreResult<Vec<i64>> {
	let Some(Some(library_id)) = media::Entity::find_by_id(media_id)
		.select_only()
		.column(series::Column::LibraryId)
		.inner_join(series::Entity)
		.into_tuple::<Option<String>>()
		.one(conn)
		.await?
	else {
		return Ok(Vec::new());
	};
	Ok(known_duplicate_page::Entity::find()
		.select_only()
		.column(known_duplicate_page::Column::Dhash)
		.filter(known_duplicate_page::Column::LibraryId.eq(library_id))
		.filter(known_duplicate_page::Column::Action.eq(DuplicatePageAction::Skip))
		.into_tuple::<i64>()
		.all(conn)
		.await?)
}

/// Compute the visible page list for a media with `pages` physical pages,
/// without consulting the cache. Pages without a stored hash stay visible.
pub async fn compute_visible_pages(
	conn: &DatabaseConnection,
	media_id: &str,
	pages: i32,
) -> CoreResult<VisiblePages> {
	let pages = pages.max(0);
	let skip = skip_hashes_for_media(conn, media_id).await?;
	if skip.is_empty() {
		return Ok((1..=pages).collect());
	}
	let hashes = page_hash::Entity::find()
		.select_only()
		.column(page_hash::Column::Page)
		.column(page_hash::Column::Dhash)
		.filter(page_hash::Column::MediaId.eq(media_id))
		.order_by_asc(page_hash::Column::Page)
		.into_tuple::<(i32, i64)>()
		.all(conn)
		.await?;
	let mut hashes = hashes.into_iter().peekable();
	let visible = (1..=pages)
		.filter(|page| {
			let hash = hashes.next_if(|(hashed, _)| hashed == page).map(|(_, h)| h);
			hash.is_none_or(|hash| !matches_any(hash, &skip))
		})
		.collect::<Vec<_>>();
	Ok(visible.into())
}

/// The ordered physical pages (1-based) still visible for `media_id`, served
/// from `cache` when present.
pub async fn visible_pages(
	conn: &DatabaseConnection,
	cache: &VisiblePagesCache,
	media_id: &str,
	pages: i32,
) -> CoreResult<VisiblePages> {
	if let Some(cached) = cache.get(media_id) {
		return Ok(cached);
	}
	let computed = compute_visible_pages(conn, media_id, pages).await?;
	cache.insert(media_id, computed.clone());
	Ok(computed)
}

/// The visible page count for one media — the number a client should see in
/// place lists and PSE counts — served from `cache` when present. Equals the
/// physical count when nothing is skipped.
pub async fn visible_page_count(
	conn: &DatabaseConnection,
	cache: &VisiblePagesCache,
	media_id: &str,
	pages: i32,
) -> CoreResult<i32> {
	Ok(visible_pages(conn, cache, media_id, pages).await?.len() as i32)
}

/// Visible page counts for a batch of `(media_id, physical_pages)` pairs,
/// computed through the shared cache. Fails fast on the first database error.
pub async fn visible_page_counts(
	conn: &DatabaseConnection,
	cache: &VisiblePagesCache,
	media: impl IntoIterator<Item = (String, i32)>,
) -> CoreResult<HashMap<String, i32>> {
	let mut counts = HashMap::new();
	for (media_id, pages) in media {
		let count = visible_page_count(conn, cache, &media_id, pages).await?;
		counts.insert(media_id, count);
	}
	Ok(counts)
}

/// Map a visible 1-based page number to its physical page, or `None` when it
/// is past the end of the visible list.
pub fn physical_page(visible: &[i32], page: i32) -> Option<i32> {
	usize::try_from(page.checked_sub(1)?)
		.ok()
		.and_then(|index| visible.get(index).copied())
}

#[cfg(test)]
mod tests {
	use super::*;
	use chrono::Utc;
	use models::shared::enums::FileStatus;
	use sea_orm::{ActiveModelTrait, Set};

	async fn seed(conn: &DatabaseConnection) {
		::tests::fake_data::Library {
			id: Some("lib".to_string()),
			..Default::default()
		}
		.insert(conn)
		.await;
		series::ActiveModel {
			id: Set("series".to_string()),
			name: Set("Series".to_string()),
			path: Set("/lib/series".to_string()),
			status: Set(FileStatus::Ready),
			library_id: Set(Some("lib".to_string())),
			created_at: Set(Utc::now().into()),
			..Default::default()
		}
		.insert(conn)
		.await
		.unwrap();
		media::ActiveModel {
			id: Set("book".to_string()),
			name: Set("Book".to_string()),
			path: Set("/lib/series/book.cbz".to_string()),
			extension: Set("cbz".to_string()),
			series_id: Set(Some("series".to_string())),
			pages: Set(5),
			size: Set(1),
			status: Set(FileStatus::Ready),
			created_at: Set(Utc::now().into()),
			..Default::default()
		}
		.insert(conn)
		.await
		.unwrap();
		// Pages 1, 2, 3, 5 carry hashes; page 4 has none. Page 1 equals the
		// SKIP mark exactly, page 3 differs by one bit.
		for (page, dhash) in [(1, 0x00ffi64), (2, 0x0f0f), (3, 0x00fe), (5, 0xf0f0)] {
			page_hash::ActiveModel {
				media_id: Set("book".to_string()),
				page: Set(page),
				dhash: Set(dhash),
				created_at: Set(Utc::now().into()),
			}
			.insert(conn)
			.await
			.unwrap();
		}
	}

	#[tokio::test]
	async fn skips_only_marked_hashes_within_tolerance() {
		let conn = ::tests::db::test_database().await;
		seed(&conn).await;
		let cache = VisiblePagesCache::default();

		let all = visible_pages(&conn, &cache, "book", 5).await.unwrap();
		assert_eq!(all.as_ref(), &[1, 2, 3, 4, 5]);

		known_duplicate_page::ActiveModel {
			library_id: Set("lib".to_string()),
			dhash: Set(0x00ff),
			action: Set(DuplicatePageAction::Skip),
			created_by: Set(None),
			created_at: Set(Utc::now().into()),
		}
		.insert(&conn)
		.await
		.unwrap();
		// Cached: still the old list until invalidated.
		let cached = visible_pages(&conn, &cache, "book", 5).await.unwrap();
		assert_eq!(cached.as_ref(), &[1, 2, 3, 4, 5]);
		cache.invalidate_all();
		// Page 1 (exact) and page 3 (hamming 1) vanish; unhashed page 4 stays.
		let filtered = visible_pages(&conn, &cache, "book", 5).await.unwrap();
		assert_eq!(filtered.as_ref(), &[2, 4, 5]);
		assert_eq!(physical_page(&filtered, 1), Some(2));
		assert_eq!(physical_page(&filtered, 3), Some(5));
		assert_eq!(physical_page(&filtered, 4), None);
		assert_eq!(physical_page(&filtered, 0), None);

		// KEEP marks never hide anything.
		known_duplicate_page::ActiveModel {
			library_id: Set("lib".to_string()),
			dhash: Set(0xff00),
			action: Set(DuplicatePageAction::Keep),
			created_by: Set(None),
			created_at: Set(Utc::now().into()),
		}
		.insert(&conn)
		.await
		.unwrap();
		cache.invalidate_media("book");
		let kept = visible_pages(&conn, &cache, "book", 5).await.unwrap();
		assert_eq!(kept.as_ref(), &[2, 4, 5]);
	}
}
