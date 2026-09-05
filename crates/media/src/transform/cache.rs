//! Byte-bounded LRU disk cache for transformed comics.
//!
//! Mirrors the KEPUB delivery cache's discipline: entries are published
//! atomically (UUID temp name → rename), hits refresh the entry's mtime so
//! LRU order tracks actual use, and a sweep removes the least-recently-used
//! files until the directory fits the configured byte budget.

use std::io;
use std::path::{Path, PathBuf};

use super::profile::TransformProfile;

/// Default in-memory view of one cache entry.
#[derive(Debug, Clone)]
pub struct CacheEntry {
	pub path: PathBuf,
	pub bytes: u64,
}

/// Statistics returned by a sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheSweep {
	/// Files removed by this sweep.
	pub removed: u64,
	/// Bytes reclaimed by this sweep.
	pub removed_bytes: u64,
	/// Total bytes remaining in the cache directory afterwards.
	pub remaining_bytes: u64,
}

/// A directory of transformed comics with a total-size budget.
#[derive(Debug, Clone)]
pub struct TransformCache {
	dir: PathBuf,
	max_bytes: u64,
}

impl TransformCache {
	/// Cache rooted at `dir`, evicting down to `max_bytes` total.
	pub fn new(dir: impl Into<PathBuf>, max_bytes: u64) -> Self {
		Self {
			dir: dir.into(),
			max_bytes,
		}
	}

	/// The cache directory.
	pub fn dir(&self) -> &Path {
		&self.dir
	}

	/// Deterministic cache file name for a media item, its source mtime, and
	/// the profile (including the profile digest).
	pub fn cache_file_name(
		media_id: &str,
		source_mtime_ns: u128,
		profile: &TransformProfile,
	) -> String {
		format!("{media_id}-{source_mtime_ns}-{}", profile.digest())
	}

	/// Full cache path for an entry; `extension` is e.g. `kepub.epub` or `cbz`.
	pub fn path_for(
		&self,
		media_id: &str,
		source_mtime_ns: u128,
		profile: &TransformProfile,
		extension: &str,
	) -> PathBuf {
		self.dir.join(format!(
			"{}.{extension}",
			Self::cache_file_name(media_id, source_mtime_ns, profile)
		))
	}

	/// Whether `path` is a complete cache hit; a hit refreshes the file's
	/// mtime so subsequent sweeps treat it as recently used.
	pub fn hit(&self, path: &Path) -> bool {
		if !path.is_file() {
			return false;
		}
		touch(path);
		true
	}

	/// Publish a completely written temp file by renaming it into place.
	pub fn publish(temp: &Path, final_path: &Path) -> io::Result<()> {
		std::fs::rename(temp, final_path)
	}

	/// Total bytes currently stored (regular files only).
	pub fn total_bytes(&self) -> io::Result<u64> {
		Ok(sweep_inputs(&self.dir)?
			.into_iter()
			.map(|(_, bytes, _)| bytes)
			.sum())
	}

	/// Evict least-recently-used entries until the directory fits
	/// `max_bytes`.
	pub fn sweep(&self) -> io::Result<CacheSweep> {
		let mut entries = sweep_inputs(&self.dir)?;
		entries.sort_by_key(|(_, _, mtime)| *mtime);

		let mut total: u64 = entries.iter().map(|(_, bytes, _)| bytes).sum();
		let mut removed = 0u64;
		let mut removed_bytes = 0u64;

		for (path, bytes, _) in entries {
			if total <= self.max_bytes {
				break;
			}
			if std::fs::remove_file(&path).is_ok() {
				total = total.saturating_sub(bytes);
				removed += 1;
				removed_bytes += bytes;
			}
		}

		Ok(CacheSweep {
			removed,
			removed_bytes,
			remaining_bytes: total,
		})
	}
}

fn touch(path: &Path) {
	let _ = filetime::set_file_mtime(path, filetime::FileTime::now());
}

/// Collect `(path, size, mtime)` for every regular, non-temp file in `dir`.
///
/// A missing directory yields an empty list; in-flight `*.tmp` files are
/// skipped so a sweep never removes a partially written entry.
fn sweep_inputs(dir: &Path) -> io::Result<Vec<(PathBuf, u64, i64)>> {
	let mut entries = Vec::new();
	let read = match std::fs::read_dir(dir) {
		Ok(read) => read,
		Err(error) if error.kind() == io::ErrorKind::NotFound => {
			return Ok(entries)
		},
		Err(error) => return Err(error),
	};

	for entry in read {
		let entry = entry?;
		let metadata = entry.metadata()?;
		if !metadata.is_file() {
			continue;
		}
		let path = entry.path();
		if path
			.file_name()
			.and_then(|name| name.to_str())
			.is_some_and(|name| name.ends_with(".tmp"))
		{
			continue;
		}
		let mtime = metadata
			.modified()
			.ok()
			.and_then(|mtime| mtime.duration_since(std::time::UNIX_EPOCH).ok())
			.map_or(0, |duration| duration.as_secs() as i64);
		entries.push((path, metadata.len(), mtime));
	}

	Ok(entries)
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::time::Duration;

	fn cache() -> (TransformCache, PathBuf) {
		let root = tempfile::tempdir().unwrap();
		let dir = root.path().join("transform");
		(TransformCache::new(&dir, 1000), dir)
	}

	fn write(dir: &Path, name: &str, bytes: usize) -> PathBuf {
		std::fs::create_dir_all(dir).unwrap();
		let path = dir.join(name);
		std::fs::write(&path, vec![0u8; bytes]).unwrap();
		path
	}

	fn mtime(path: &Path) -> i64 {
		path.metadata()
			.and_then(|meta| meta.modified())
			.ok()
			.and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
			.map_or(0, |duration| duration.as_secs() as i64)
	}

	fn age(path: &Path, seconds: i64) {
		let target = filetime::FileTime::from_unix_time(mtime(path) - seconds, 0);
		filetime::set_file_mtime(path, target).unwrap();
	}

	#[test]
	fn cache_file_name_is_deterministic_and_profile_sensitive() {
		let libra = TransformProfile::preset("libra").unwrap();
		let clara = TransformProfile::preset("clara").unwrap();
		assert_eq!(
			TransformCache::cache_file_name("media-1", 42, &libra),
			TransformCache::cache_file_name("media-1", 42, &libra)
		);
		assert_ne!(
			TransformCache::cache_file_name("media-1", 42, &libra),
			TransformCache::cache_file_name("media-1", 42, &clara)
		);
		assert_ne!(
			TransformCache::cache_file_name("media-1", 42, &libra),
			TransformCache::cache_file_name("media-2", 42, &libra)
		);
		assert_ne!(
			TransformCache::cache_file_name("media-1", 42, &libra),
			TransformCache::cache_file_name("media-1", 43, &libra)
		);
	}

	#[test]
	fn path_for_uses_extension() {
		let (cache, dir) = cache();
		let libra = TransformProfile::preset("libra").unwrap();
		let path = cache.path_for("m", 1, &libra, "kepub.epub");
		assert!(path.starts_with(&dir));
		assert!(path.to_string_lossy().ends_with(".kepub.epub"));
	}

	#[test]
	fn hit_only_matches_files_and_refreshes_mtime() {
		let (cache, dir) = cache();
		let path = write(&dir, "entry.kepub.epub", 10);
		age(&path, 120);
		let before = mtime(&path);

		std::thread::sleep(Duration::from_millis(1100));
		assert!(cache.hit(&path));
		assert!(mtime(&path) > before, "hit must refresh mtime for LRU");

		assert!(!cache.hit(&dir.join("missing.kepub.epub")));
		assert!(!cache.hit(&dir));
	}

	#[test]
	fn sweep_evicts_lru_until_within_budget() {
		let (cache, dir) = cache(); // budget: 1000 bytes
		let old = write(&dir, "a-old.kepub.epub", 400);
		let mid = write(&dir, "b-mid.kepub.epub", 400);
		let new = write(&dir, "c-new.kepub.epub", 400);
		age(&old, 300);
		age(&mid, 200);
		age(&new, 100);

		let sweep = cache.sweep().unwrap();
		assert_eq!(sweep.removed, 1);
		assert_eq!(sweep.removed_bytes, 400);
		assert_eq!(sweep.remaining_bytes, 800);
		assert!(!old.exists());
		assert!(mid.exists());
		assert!(new.exists());

		// Already within budget: another sweep removes nothing.
		let sweep = cache.sweep().unwrap();
		assert_eq!(sweep.removed, 0);
		assert_eq!(sweep.remaining_bytes, 800);
	}

	#[test]
	fn touched_survivor_beats_untouched_older_file() {
		let (cache, dir) = cache();
		let oldest = write(&dir, "a-old.kepub.epub", 400);
		let newer = write(&dir, "b-new.kepub.epub", 700);
		age(&oldest, 500);
		age(&newer, 100);

		// Refresh the newer file so it is now most recently used.
		std::thread::sleep(Duration::from_millis(1100));
		assert!(cache.hit(&newer));

		let sweep = cache.sweep().unwrap();
		assert_eq!(sweep.removed, 1);
		assert!(!oldest.exists(), "LRU victim must go");
		assert!(newer.exists(), "touched entry must survive");
	}

	#[test]
	fn sweep_skips_tmp_files_and_missing_dirs() {
		let (cache, dir) = cache();
		std::fs::create_dir_all(&dir).unwrap();
		write(&dir, "in-flight.abc123.tmp", 2000);

		let sweep = cache.sweep().unwrap();
		assert_eq!(sweep.removed, 0);
		assert!(dir.join("in-flight.abc123.tmp").exists());

		let empty = TransformCache::new(dir.join("nope"), 100);
		assert_eq!(empty.sweep().unwrap().remaining_bytes, 0);
		assert_eq!(empty.total_bytes().unwrap(), 0);
	}

	#[test]
	fn publish_renames_temp_into_place() {
		let (cache, dir) = cache();
		std::fs::create_dir_all(&dir).unwrap();
		let temp = write(&dir, "entry.abc.tmp", 5);
		let final_path = dir.join("entry.kepub.epub");

		TransformCache::publish(&temp, &final_path).unwrap();
		assert!(!temp.exists());
		assert!(final_path.exists());
		assert!(cache.hit(&final_path));
	}
}
