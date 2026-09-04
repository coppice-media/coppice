use std::{
	collections::HashMap,
	path::{Path, PathBuf},
	sync::Arc,
	time::UNIX_EPOCH,
};

use chrono::{DateTime, Utc};
use globset::GlobSet;
use stump_media::PathUtils;
use tokio::task::spawn_blocking;
use tracing::{debug, error, trace, warn};
use walkdir::{DirEntry, WalkDir};

use crate::{BookVisitOperation, ScanError, ScanOptions, ScanResult, ScanSource};

/// Inputs shared by library and series walks.
///
/// `dir_mtimes` may contain a scan-wide snapshot loaded by the caller. When it is empty, the
/// walker obtains the snapshot through [`ScanSource::stored_dir_mtimes`]. This lets a library job
/// reuse one database read across all of its series while preserving a self-contained series walk.
pub struct WalkerCtx {
	/// The globset of ignore rules to apply during the walk.
	pub ignore_rules: GlobSet,
	/// The maximum depth to traverse, if any.
	pub max_depth: Option<usize>,
	/// The scan options to apply during the walk.
	pub options: ScanOptions,
	/// Stored directory mtimes used to short-circuit unchanged subtrees.
	pub dir_mtimes: Arc<HashMap<String, u64>>,
	/// The library ID owning the walk.
	pub library_id: String,
	/// The series ID for this walk, if scoped to a specific series.
	pub series_id: Option<String>,
}

/// The output of walking a library.
#[derive(Debug, Default)]
pub struct WalkedLibrary {
	/// The total number of directories seen during the walk.
	pub seen_directories: u64,
	/// The number of directories ignored via ignore rules or common scan rules.
	pub ignored_directories: u64,
	/// The paths for series that need to be created.
	pub series_to_create: Vec<PathBuf>,
	/// IDs for series previously marked missing that were found on disk again.
	pub recovered_series: Vec<String>,
	/// Existing series paths that need to be visited.
	pub series_to_visit: Vec<PathBuf>,
	/// Paths for series missing from the filesystem.
	pub missing_series: Vec<PathBuf>,
	/// Whether the library itself is missing from the filesystem.
	pub library_is_missing: bool,
}

impl WalkedLibrary {
	fn missing() -> Self {
		Self {
			library_is_missing: true,
			..Default::default()
		}
	}
}

/// Walks a library root and reconciles discovered series against the read-only source.
pub async fn walk_library<S: ScanSource + ?Sized>(
	path: &str,
	source: &S,
	WalkerCtx {
		ignore_rules,
		max_depth,
		library_id,
		..
	}: WalkerCtx,
) -> ScanResult<WalkedLibrary> {
	let library_path = PathBuf::from(path);
	if !library_path.exists() {
		error!("Failed to walk: {} is missing or inaccessible", path);
		return Ok(WalkedLibrary::missing());
	}

	let walk_start = std::time::Instant::now();
	let is_collection_based = max_depth.is_some_and(|depth| depth == 1);
	debug!(
		?path,
		max_depth,
		is_collection_based,
		?ignore_rules,
		"Walking library"
	);

	let path_owned = path.to_string();
	let (valid_entries, ignored_entries) = spawn_blocking(move || {
		let mut walkdir = WalkDir::new(&path_owned);
		if let Some(depth) = max_depth {
			walkdir = walkdir.max_depth(depth);
		}

		let mut valid = Vec::new();
		let mut ignored = Vec::new();
		for entry in walkdir
			.min_depth(0)
			.into_iter()
			.filter_entry(|entry| entry.path().is_dir())
		{
			let Ok(entry) = entry else {
				continue;
			};
			let entry_path = entry.path();
			let entry_path_str = entry_path.to_string_lossy();
			let check_deep = is_collection_based && entry_path_str != path_owned;
			let should_ignore = ignore_rules.is_match(entry_path);

			// A top-level collection scan checks for media deeply below each child folder. A
			// regular library scan checks only the direct contents of each candidate series.
			let is_valid = !should_ignore
				&& ((check_deep && entry_path.dir_has_media_deep(&ignore_rules))
					|| (!check_deep && entry_path.dir_has_media(&ignore_rules)));

			trace!(is_valid, path = %entry_path.display());
			if is_valid {
				valid.push(entry);
			} else {
				ignored.push(entry);
			}
		}
		(valid, ignored)
	})
	.await
	.map_err(|error| ScanError::Internal(format!("Failed to walk library: {error}")))?;

	let ignored_directories = ignored_entries.len() as u64;
	let seen_directories = valid_entries.len() as u64 + ignored_directories;
	debug!(
		seen_directories,
		ignored_directories,
		"Walk finished in {}ms",
		walk_start.elapsed().as_millis()
	);

	let computation_start = std::time::Instant::now();
	let existing_records = source.existing_series(&library_id).await?;
	let (series_to_create, missing_series, recovered_series, series_to_visit) =
		if existing_records.is_empty() {
			debug!("No existing series found in the source, all series are new");
			(
				valid_entries
					.into_iter()
					.map(|entry| entry.into_path())
					.collect(),
				vec![],
				vec![],
				vec![],
			)
		} else {
			let existing_series_map = existing_records
				.iter()
				.map(|series| (series.path.clone(), series.clone()))
				.collect::<HashMap<_, _>>();

			let missing_series = existing_series_map
				.iter()
				.filter(|(series_path, _)| !Path::new(series_path).exists())
				.map(|(series_path, _)| PathBuf::from(series_path))
				.collect::<Vec<_>>();

			let recovered_series = existing_records
				.iter()
				.filter(|series| {
					series.status.is_recovered_if_present()
						&& Path::new(&series.path).exists()
				})
				.map(|series| series.id.clone())
				.collect::<Vec<_>>();

			let existing_empty_series = ignored_entries
				.iter()
				.filter_map(|entry| {
					let path = entry.path().to_string_lossy().into_owned();
					existing_series_map
						.contains_key(&path)
						.then(|| entry.path().to_owned())
				})
				.collect::<Vec<_>>();

			let mut series_to_create = Vec::new();
			let mut series_to_visit = Vec::new();
			for path in valid_entries
				.into_iter()
				.map(|entry| entry.into_path())
				.filter(|path| !missing_series.contains(path))
				.chain(existing_empty_series)
			{
				if existing_series_map.contains_key(path.to_string_lossy().as_ref()) {
					series_to_visit.push(path);
				} else {
					series_to_create.push(path);
				}
			}

			(
				series_to_create,
				missing_series,
				recovered_series,
				series_to_visit,
			)
		};

	trace!(count = series_to_create.len(), "Found series to create");
	trace!(?missing_series, "Found series to mark as missing");
	debug!(
		"Finished computation steps in {}ms",
		computation_start.elapsed().as_millis()
	);

	Ok(WalkedLibrary {
		seen_directories,
		ignored_directories,
		series_to_create,
		recovered_series,
		series_to_visit,
		missing_series,
		library_is_missing: false,
	})
}

/// The output of walking a series.
#[derive(Debug, Default)]
pub struct WalkedSeries {
	/// The total number of files seen during the walk.
	pub seen_files: u64,
	/// The number of files ignored by rules or common scan rules.
	pub ignored_files: u64,
	/// The number of existing files skipped because neither mtime nor options require a visit.
	pub skipped_files: u64,
	/// Paths for media that need to be created.
	pub media_to_create: Vec<PathBuf>,
	/// IDs for media previously marked missing that were found on disk again.
	pub recovered_media: Vec<String>,
	/// Paths and operations for existing media that need to be visited.
	pub media_to_visit: Vec<(PathBuf, BookVisitOperation)>,
	/// Paths for media missing from the filesystem.
	pub missing_media: Vec<PathBuf>,
	/// Whether the series is missing from the filesystem.
	pub series_is_missing: bool,
	/// Changed directory mtimes observed during the walk.
	pub observed_dir_mtimes: HashMap<String, u64>,
}

impl WalkedSeries {
	fn missing() -> Self {
		Self {
			series_is_missing: true,
			..Default::default()
		}
	}
}

/// Walks one series and reconciles media identities and directory mtimes.
pub async fn walk_series<S: ScanSource + ?Sized>(
	path: &Path,
	source: &S,
	WalkerCtx {
		ignore_rules,
		max_depth,
		options,
		dir_mtimes,
		library_id,
		series_id,
	}: WalkerCtx,
) -> ScanResult<WalkedSeries> {
	if tokio::fs::metadata(path).await.is_err() {
		error!(
			"Failed to walk: {} is missing or inaccessible",
			path.display()
		);
		return Ok(WalkedSeries::missing());
	}

	debug!("Walking series at {}", path.display());

	let stored_dir_mtimes = if dir_mtimes.is_empty() {
		Arc::new(source.stored_dir_mtimes(&library_id).await?)
	} else {
		dir_mtimes
	};

	let path_buf = path.to_path_buf();
	let walk_start = std::time::Instant::now();
	let (valid_entries, ignored_entries, observed_dir_mtimes) =
		spawn_blocking(move || {
			let mut walkdir = WalkDir::new(&path_buf);
			if let Some(depth) = max_depth {
				walkdir = walkdir.max_depth(depth);
			}

			let root_str = path_buf.to_string_lossy().into_owned();
			let mut iterator = walkdir.into_iter();
			let mut valid = Vec::new();
			let mut ignored = Vec::new();
			let mut observed = HashMap::new();

			while let Some(next) = iterator.next() {
				let Ok(entry) = next else {
					warn!("Error encountered during walk, skipping entry");
					continue;
				};
				let entry_path = entry.path();

				if entry_path.is_dir() {
					// Ignoring a directory must prune its subtree. Otherwise a glob that names a
					// directory still allows every child media file through the walk.
					if entry_path != path_buf
						&& (ignore_rules.is_match(entry_path)
							|| entry_path.is_hidden_file())
					{
						iterator.skip_current_dir();
						continue;
					}

					let path_str = entry_path.to_string_lossy().into_owned();
					let current_mtime = entry_path
						.metadata()
						.and_then(|metadata| metadata.modified())
						.map(|time| {
							time.duration_since(UNIX_EPOCH)
								.unwrap_or_default()
								.as_secs()
						})
						.unwrap_or(0);
					let did_change = stored_dir_mtimes
						.get(&path_str)
						.is_none_or(|previous| *previous != current_mtime);

					if did_change {
						trace!(mtime = current_mtime, path = %path_str, "Observed changed dir");
						observed.insert(path_str.clone(), current_mtime);
					}

					// The series root is always visited because its mtime does not reliably reflect
					// changes in nested directories.
					if path_str != root_str && !did_change {
						trace!(mtime = current_mtime, path = %path_str, "Skipping unchanged dir");
						iterator.skip_current_dir();
					}
					continue;
				}

				if ignore_rules.is_match(entry_path) || entry_path.is_default_ignored() {
					ignored.push(entry);
				} else {
					valid.push(entry);
				}
			}

			(valid, ignored, observed)
		})
		.await
		.map_err(|error| {
			ScanError::Internal(format!("Series walk task panicked: {error}"))
		})?;

	let valid_entries_len = valid_entries.len() as u64;
	let ignored_files = ignored_entries.len() as u64;
	let seen_files = valid_entries_len + ignored_files;
	debug!(
		seen_files,
		ignored_files,
		"Walk finished in {}ms",
		walk_start.elapsed().as_millis()
	);

	let existing_media = match series_id.as_deref() {
		Some(id) => source.existing_media(id).await?,
		None => Vec::new(),
	};
	trace!(count = existing_media.len(), "Fetched existing media");

	let existing_media_map = existing_media
		.into_iter()
		.map(|media| (media.path.clone(), media))
		.collect::<HashMap<_, _>>();

	let mut media_to_create = Vec::new();
	let mut media_to_visit = Vec::new();
	for entry in valid_entries {
		let entry_path = entry.path();
		let entry_path_str = entry_path.to_string_lossy();
		if let Some(media) = existing_media_map.get(entry_path_str.as_ref()) {
			let modified = media
				.modified_at
				.as_ref()
				.map(|modified_at| file_updated_since_scan(&entry, modified_at))
				.unwrap_or(false);
			if modified {
				media_to_visit.push((entry.into_path(), BookVisitOperation::Rebuild));
			} else if let Some(operation) = options.book_operation() {
				media_to_visit.push((entry.into_path(), operation));
			}
		} else {
			media_to_create.push(entry.into_path());
		}
	}

	let missing_media = existing_media_map
		.iter()
		.filter(|(media_path, _)| !Path::new(media_path).exists())
		.map(|(media_path, _)| PathBuf::from(media_path))
		.collect::<Vec<_>>();

	let recovered_media = existing_media_map
		.iter()
		.filter(|(media_path, media)| {
			media.status.is_recovered_if_present() && Path::new(media_path).exists()
		})
		.map(|(_, media)| media.id.clone())
		.collect::<Vec<_>>();

	let to_create = media_to_create.len();
	let to_visit = media_to_visit.len();
	let skipped_files = seen_files.saturating_sub((to_create + to_visit) as u64);
	trace!(
		to_create,
		to_visit,
		skipped_files,
		"Finished media reconciliation"
	);

	Ok(WalkedSeries {
		seen_files,
		ignored_files,
		skipped_files,
		media_to_create,
		recovered_media,
		media_to_visit,
		missing_media,
		series_is_missing: false,
		observed_dir_mtimes,
	})
}

fn file_updated_since_scan(
	entry: &DirEntry,
	last_modified_at: &DateTime<chrono::FixedOffset>,
) -> bool {
	if let Ok(Ok(system_time)) = entry.metadata().map(|metadata| metadata.modified()) {
		let system_time_converted: DateTime<Utc> = system_time.into();
		let media_modified_at = last_modified_at.with_timezone(&Utc);
		trace!(?system_time_converted, ?media_modified_at);
		system_time_converted > media_modified_at
	} else {
		error!(path = ?entry.path(), "Error occurred trying to read modified date for media");
		true
	}
}

#[cfg(test)]
mod tests {
	use std::{collections::HashMap, sync::Arc};

	use parking_lot::Mutex;

	use async_trait::async_trait;
	use chrono::{DateTime, FixedOffset, Utc};
	use globset::{Glob, GlobSet, GlobSetBuilder};
	use tempfile::TempDir;

	use super::*;
	use crate::{MediaIdentity, ScanResult, ScanStatus, SeriesIdentity};
	#[derive(Clone, Default)]
	struct FakeSource {
		series: Arc<Mutex<Vec<SeriesIdentity>>>,
		media: Arc<Mutex<Vec<MediaIdentity>>>,
		dir_mtimes: Arc<Mutex<HashMap<String, u64>>>,
	}

	#[async_trait]
	impl ScanSource for FakeSource {
		async fn existing_series(
			&self,
			_library_id: &str,
		) -> ScanResult<Vec<SeriesIdentity>> {
			Ok(self.series.lock().clone())
		}

		async fn existing_media(
			&self,
			_series_id: &str,
		) -> ScanResult<Vec<MediaIdentity>> {
			Ok(self.media.lock().clone())
		}

		async fn stored_dir_mtimes(
			&self,
			_library_id: &str,
		) -> ScanResult<HashMap<String, u64>> {
			Ok(self.dir_mtimes.lock().clone())
		}
	}

	fn rules(patterns: &[&str]) -> GlobSet {
		let mut builder = GlobSetBuilder::new();
		for pattern in patterns {
			builder.add(Glob::new(pattern).unwrap());
		}
		builder.build().unwrap()
	}

	fn mtime(path: &Path) -> u64 {
		path.metadata()
			.unwrap()
			.modified()
			.unwrap()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs()
	}

	fn modified_at(path: &Path) -> DateTime<FixedOffset> {
		let modified: DateTime<Utc> = path.metadata().unwrap().modified().unwrap().into();
		modified.fixed_offset()
	}

	fn ctx(
		root: &Path,
		rules: GlobSet,
		max_depth: Option<usize>,
		series_id: &str,
	) -> WalkerCtx {
		WalkerCtx {
			ignore_rules: rules,
			max_depth,
			options: ScanOptions::default(),
			dir_mtimes: Arc::new(HashMap::new()),
			library_id: root.to_string_lossy().into_owned(),
			series_id: Some(series_id.to_string()),
		}
	}

	fn write_media(path: &Path) {
		std::fs::write(path, b"test media").unwrap();
	}

	#[tokio::test]
	async fn unchanged_directories_are_skipped_and_changed_directories_are_walked() {
		let temp = TempDir::new().unwrap();
		let nested = temp.path().join("nested");
		std::fs::create_dir(&nested).unwrap();
		let book = nested.join("book.cbz");
		write_media(&book);

		let source = FakeSource::default();
		let root_mtime = mtime(temp.path());
		let nested_mtime = mtime(&nested);
		source.dir_mtimes.lock().extend([
			(temp.path().to_string_lossy().into_owned(), root_mtime),
			(nested.to_string_lossy().into_owned(), nested_mtime),
		]);

		let skipped = walk_series(
			temp.path(),
			&source,
			ctx(temp.path(), rules(&[]), None, "series"),
		)
		.await
		.unwrap();
		assert!(skipped.media_to_create.is_empty());
		assert_eq!(skipped.seen_files, 0);

		source.dir_mtimes.lock().insert(
			nested.to_string_lossy().into_owned(),
			nested_mtime.saturating_sub(1),
		);
		let walked = walk_series(
			temp.path(),
			&source,
			ctx(temp.path(), rules(&[]), None, "series"),
		)
		.await
		.unwrap();
		assert_eq!(walked.media_to_create, vec![book]);
		assert!(walked
			.observed_dir_mtimes
			.contains_key(&nested.to_string_lossy().to_string()));
	}

	#[tokio::test]
	async fn removed_series_is_missing_and_restored_series_reappears() {
		let temp = TempDir::new().unwrap();
		let series_path = temp.path().join("series");
		std::fs::create_dir(&series_path).unwrap();
		let source = FakeSource::default();
		source.series.lock().push(SeriesIdentity::new(
			"series-id",
			series_path.to_string_lossy(),
			ScanStatus::Missing,
		));
		std::fs::remove_dir(&series_path).unwrap();

		let missing = walk_library(
			temp.path().to_str().unwrap(),
			&source,
			WalkerCtx {
				ignore_rules: rules(&[]),
				max_depth: None,
				options: ScanOptions::default(),
				dir_mtimes: Arc::new(HashMap::new()),
				library_id: "library-id".to_string(),
				series_id: None,
			},
		)
		.await
		.unwrap();
		assert_eq!(missing.missing_series, vec![series_path.clone()]);

		std::fs::create_dir(&series_path).unwrap();
		write_media(&series_path.join("book.cbz"));
		let restored = walk_library(
			temp.path().to_str().unwrap(),
			&source,
			WalkerCtx {
				ignore_rules: rules(&[]),
				max_depth: None,
				options: ScanOptions::default(),
				dir_mtimes: Arc::new(HashMap::new()),
				library_id: "library-id".to_string(),
				series_id: None,
			},
		)
		.await
		.unwrap();
		assert_eq!(restored.series_to_visit, vec![series_path]);
		assert_eq!(restored.recovered_series, vec!["series-id"]);
	}

	#[tokio::test]
	async fn unchanged_media_is_skipped_and_changed_media_is_revisited() {
		let temp = TempDir::new().unwrap();
		let book = temp.path().join("book.cbz");
		write_media(&book);
		let source = FakeSource::default();
		source.media.lock().push(MediaIdentity::new(
			"media-id",
			book.to_string_lossy(),
			Some(modified_at(&book)),
			ScanStatus::Ready,
		));

		let unchanged = walk_series(
			temp.path(),
			&source,
			ctx(temp.path(), rules(&[]), None, "series"),
		)
		.await
		.unwrap();
		assert!(unchanged.media_to_visit.is_empty());
		assert_eq!(unchanged.skipped_files, 1);

		source.media.lock()[0].modified_at =
			Some(modified_at(&book) - chrono::Duration::seconds(1));
		let changed = walk_series(
			temp.path(),
			&source,
			ctx(temp.path(), rules(&[]), None, "series"),
		)
		.await
		.unwrap();
		assert_eq!(changed.media_to_visit.len(), 1);
		assert_eq!(changed.media_to_visit[0].1, BookVisitOperation::Rebuild);
	}

	#[tokio::test]
	async fn ignore_rules_exclude_files_and_directories() {
		let temp = TempDir::new().unwrap();
		let ignored_dir = temp.path().join("ignored-dir");
		std::fs::create_dir(&ignored_dir).unwrap();
		let ignored_file = temp.path().join("ignored.cbz");
		let included_file = temp.path().join("included.cbz");
		write_media(&ignored_dir.join("nested.cbz"));
		write_media(&ignored_file);
		write_media(&included_file);
		let source = FakeSource::default();

		let patterns = [
			ignored_dir.to_string_lossy().into_owned(),
			ignored_file.to_string_lossy().into_owned(),
		];
		let pattern_refs = patterns.iter().map(String::as_str).collect::<Vec<_>>();
		let walked = walk_series(
			temp.path(),
			&source,
			ctx(temp.path(), rules(&pattern_refs), None, "series"),
		)
		.await
		.unwrap();

		assert_eq!(walked.media_to_create, vec![included_file]);
		assert_eq!(walked.ignored_files, 1);
		assert!(!walked
			.media_to_create
			.iter()
			.any(|path| path.starts_with(&ignored_dir)));
	}

	#[tokio::test]
	async fn max_depth_limits_library_collection_discovery() {
		let temp = TempDir::new().unwrap();
		let collection = temp.path().join("collection");
		let volume = collection.join("volume");
		std::fs::create_dir(&collection).unwrap();
		std::fs::create_dir(&volume).unwrap();
		let book = volume.join("book.cbz");
		write_media(&book);
		let source = FakeSource::default();

		let walked = walk_library(
			temp.path().to_str().unwrap(),
			&source,
			WalkerCtx {
				ignore_rules: rules(&[]),
				max_depth: Some(1),
				options: ScanOptions::default(),
				dir_mtimes: Arc::new(HashMap::new()),
				library_id: "library-id".to_string(),
				series_id: None,
			},
		)
		.await
		.unwrap();

		assert_eq!(walked.series_to_create, vec![collection]);
		assert!(!walked.series_to_create.contains(&book));
	}

	#[tokio::test]
	async fn max_depth_limits_collection_walk() {
		let temp = TempDir::new().unwrap();
		let first = temp.path().join("first");
		let second = first.join("second");
		std::fs::create_dir(&first).unwrap();
		std::fs::create_dir(&second).unwrap();
		let book = second.join("book.cbz");
		write_media(&book);
		let source = FakeSource::default();

		let shallow = walk_series(
			temp.path(),
			&source,
			ctx(temp.path(), rules(&[]), Some(1), "series"),
		)
		.await
		.unwrap();
		assert!(shallow.media_to_create.is_empty());

		let deep = walk_series(
			temp.path(),
			&source,
			ctx(temp.path(), rules(&[]), None, "series"),
		)
		.await
		.unwrap();
		assert_eq!(deep.media_to_create, vec![book]);
	}

	#[test]
	fn mtime_helper_uses_real_file_times() {
		let temp = TempDir::new().unwrap();
		let file = temp.path().join("book.cbz");
		write_media(&file);
		let entry = WalkDir::new(temp.path())
			.into_iter()
			.filter_map(Result::ok)
			.find(|entry| entry.path() == file)
			.unwrap();
		let now = modified_at(&file);
		assert!(!file_updated_since_scan(&entry, &now));
		assert!(file_updated_since_scan(
			&entry,
			&(now - chrono::Duration::seconds(1))
		));
	}
}
