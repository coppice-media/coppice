//! Bounded filesystem scanning for explicit source roots.
//!
//! Scans run in `spawn_blocking` for ordinary filesystem roots; Calibre roots
//! use the asynchronous read-only SQLite adapter. Only regular files below a
//! non-symlink root are observed. The scanner emits sanitized display paths and
//! never sends absolute paths.

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::source_catalog::{
	modified_at_ms, quick_fingerprint, reject_symlink_ancestors, sanitize_relative_path,
	CatalogError, CatalogObservation, CatalogSnapshot, SourceCatalog, SourceRootConfig,
	DEFAULT_MAX_ENTRIES,
};

pub const DEFAULT_SCAN_CONCURRENCY: usize = 2;

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
	#[error(transparent)]
	Catalog(#[from] CatalogError),
	#[error("source root `{root_id}` scan failed: {error}")]
	Root { root_id: String, error: io::Error },
	#[error("Calibre source root `{root_id}` scan failed: {error}")]
	Calibre { root_id: String, error: String },
	#[error("source root `{root_id}` contains more than {max_entries} files")]
	EntryLimit { root_id: String, max_entries: usize },
	#[error("source root `{0}` is a symlink or not a directory")]
	UnsafeRoot(String),
	#[error("scan task failed: {0}")]
	Join(String),
}

fn calibre_error(
	root_id: String,
	error: crate::source_calibre::CalibreSourceError,
) -> ScanError {
	match error {
		crate::source_calibre::CalibreSourceError::EntryLimit { max_entries } => {
			ScanError::EntryLimit {
				root_id,
				max_entries,
			}
		},
		error => ScanError::Calibre {
			root_id,
			error: error.to_string(),
		},
	}
}

#[derive(Debug, Clone)]
pub struct SourceScanner {
	catalog: Arc<SourceCatalog>,
	max_entries: usize,
	concurrency: usize,
}

impl SourceScanner {
	#[must_use]
	pub fn new(catalog: Arc<SourceCatalog>) -> Self {
		Self {
			catalog,
			max_entries: DEFAULT_MAX_ENTRIES,
			concurrency: DEFAULT_SCAN_CONCURRENCY,
		}
	}

	#[must_use]
	pub fn with_limits(self, max_entries: usize, concurrency: usize) -> Self {
		Self {
			max_entries: max_entries.max(1),
			concurrency: concurrency.max(1),
			..self
		}
	}

	#[must_use]
	pub fn catalog(&self) -> &Arc<SourceCatalog> {
		&self.catalog
	}
	/// Scan and commit every configured root. Root tasks are bounded by a
	/// semaphore, and dropping this future drops the join set so no new
	/// catalog commit can occur after cancellation.
	pub async fn scan_once(&self) -> Result<Vec<CatalogSnapshot>, ScanError> {
		let roots = self.catalog.roots();
		let permits = Arc::new(Semaphore::new(self.concurrency));
		let mut tasks = tokio::task::JoinSet::new();
		for root in roots {
			let permit = permits
				.clone()
				.acquire_owned()
				.await
				.map_err(|_| ScanError::Join("scan semaphore closed".into()))?;
			let max_entries = self.max_entries;
			let root_id = root.root_id.clone();
			if root
				.kind
				.eq_ignore_ascii_case(crate::source_calibre::CALIBRE_ROOT_KIND)
			{
				tasks.spawn(async move {
					let _permit = permit;
					crate::source_calibre::discover(&root, max_entries)
						.await
						.map(|observations| (root, observations))
						.map_err(|error| calibre_error(root_id, error))
				});
			} else {
				tasks.spawn_blocking(move || {
					let _permit = permit;
					scan_root_blocking(&root, max_entries)
						.map(|observations| (root, observations))
						.map_err(|error| ScanError::Root { root_id, error })
				});
			}
		}
		let mut snapshots = Vec::new();
		while let Some(result) = tasks.join_next().await {
			let (root, observations) =
				result.map_err(|error| ScanError::Join(error.to_string()))??;
			let root_id = root.root_id.clone();
			let snapshot = self.catalog.commit_scan(&root_id, observations)?;
			snapshots.push(snapshot);
		}
		snapshots.sort_by(|left, right| left.root.root_id.cmp(&right.root.root_id));
		Ok(snapshots)
	}

	/// Scan one configured root in a bounded blocking task, or through the
	/// asynchronous Calibre SQLite adapter when `kind=calibre`.
	pub async fn scan_root(&self, root_id: &str) -> Result<CatalogSnapshot, ScanError> {
		let root = self.catalog.root(root_id)?;
		let max_entries = self.max_entries;
		let root_for_error = root.root_id.clone();
		let observations = if root
			.kind
			.eq_ignore_ascii_case(crate::source_calibre::CALIBRE_ROOT_KIND)
		{
			crate::source_calibre::discover(&root, max_entries)
				.await
				.map_err(|error| calibre_error(root_for_error, error))?
		} else {
			tokio::task::spawn_blocking(move || scan_root_blocking(&root, max_entries))
				.await
				.map_err(|error| ScanError::Join(error.to_string()))?
				.map_err(|error| ScanError::Root {
					root_id: root_for_error,
					error,
				})?
		};
		Ok(self.catalog.commit_scan(root_id, observations)?)
	}
}
fn scan_root_blocking(
	root: &SourceRootConfig,
	max_entries: usize,
) -> Result<Vec<CatalogObservation>, io::Error> {
	reject_symlink_ancestors(&root.path)?;
	let metadata = fs::symlink_metadata(&root.path)?;
	if metadata.file_type().is_symlink() || !metadata.is_dir() {
		return Err(io::Error::new(
			io::ErrorKind::PermissionDenied,
			"configured source root must be a non-symlink directory",
		));
	}
	let mut queue = VecDeque::from([(root.path.clone(), 0_u32)]);
	let mut observations = Vec::new();
	while let Some((directory, depth)) = queue.pop_front() {
		let entries = match fs::read_dir(&directory) {
			Ok(entries) => entries,
			Err(error) => {
				// A disappearing or unreadable child directory cannot grant
				// access to anything outside the root. Keep the rest of the
				// inventory useful rather than exposing a path in the error.
				if directory == root.path {
					return Err(error);
				}
				tracing::debug!(?error, "Skipping unreadable source directory");
				continue;
			},
		};
		for entry in entries {
			let entry = match entry {
				Ok(entry) => entry,
				Err(error) => {
					tracing::debug!(?error, "Skipping source directory entry");
					continue;
				},
			};
			let path = entry.path();
			let metadata = match fs::symlink_metadata(&path) {
				Ok(metadata) => metadata,
				Err(error) => {
					tracing::debug!(
						?error,
						"Skipping source entry with unavailable metadata"
					);
					continue;
				},
			};
			let file_type = metadata.file_type();
			if file_type.is_symlink() {
				// Do not follow even a symlink that appears to point back into
				// the root: this removes both escape and cycle ambiguity.
				continue;
			}
			if file_type.is_dir() {
				if depth < u32::MAX {
					queue.push_back((path, depth + 1));
				}
				continue;
			}
			if !file_type.is_file() {
				// Sockets, devices, fifos, and other special files never enter
				// the catalog and can never be served by a grant.
				continue;
			}
			if observations.len() >= max_entries {
				return Err(io::Error::new(
					io::ErrorKind::Other,
					format!("entry limit exceeded ({max_entries})"),
				));
			}
			let Some(relative_path) = path
				.strip_prefix(&root.path)
				.ok()
				.and_then(|relative| sanitize_relative_path(&relative.to_string_lossy()))
			else {
				continue;
			};
			let size = metadata.len();
			let modified = modified_at_ms(&metadata);
			let quick = match quick_fingerprint(&path, size, modified) {
				Ok(quick) => quick,
				Err(error) => {
					tracing::debug!(
						?error,
						"Skipping source file whose fingerprint failed"
					);
					continue;
				},
			};
			// Do not publish a fingerprint for a file that changed while it
			// was being sampled. It will be picked up consistently next scan.
			let after = match fs::symlink_metadata(&path) {
				Ok(after) => after,
				Err(_) => continue,
			};
			if after.file_type().is_symlink()
				|| !after.is_file()
				|| after.len() != size
				|| modified_at_ms(&after) != modified
			{
				continue;
			}
			observations.push(CatalogObservation {
				absolute_path: path,
				relative_path,
				size,
				modified_at_ms: modified,
				quick_fingerprint: quick,
				media_type: media_type_for_path(&entry.path()),
				metadata: None,
			});
		}
	}
	Ok(observations)
}

fn media_type_for_path(path: &Path) -> Option<String> {
	let extension = path.extension()?.to_str()?.to_ascii_lowercase();
	let value = match extension.as_str() {
		"epub" => "application/epub+zip",
		"pdf" => "application/pdf",
		"m4b" => "audio/mp4",
		"mp3" => "audio/mpeg",
		"m4a" => "audio/mp4",
		"flac" => "audio/flac",
		"ogg" | "oga" | "opus" => "audio/ogg",
		"mp4" => "video/mp4",
		"mkv" => "video/x-matroska",
		"jpg" | "jpeg" => "image/jpeg",
		"png" => "image/png",
		"webp" => "image/webp",
		"txt" => "text/plain",
		"json" => "application/json",
		_ => return None,
	};
	Some(value.into())
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::fs::File;
	use std::io::Write;
	use tempfile::tempdir;

	fn root(path: &Path) -> SourceRootConfig {
		SourceRootConfig {
			root_id: "root".into(),
			label: "root".into(),
			kind: "library".into(),
			privacy_mode: "catalog".into(),
			path: path.to_path_buf(),
			transport: crate::source_protocol::SourceTransport::Tunnel,
			direct_base_url: None,
		}
	}

	#[tokio::test]
	async fn scan_skips_symlinks_and_special_paths() {
		let dir = tempdir().unwrap();
		let root_dir = dir.path().join("root");
		fs::create_dir(&root_dir).unwrap();
		let mut file = File::create(root_dir.join("book.epub")).unwrap();
		file.write_all(b"book").unwrap();
		#[cfg(unix)]
		std::os::unix::fs::symlink(dir.path(), root_dir.join("escape")).unwrap();
		let catalog = Arc::new(
			SourceCatalog::open(dir.path().join("state"), vec![root(&root_dir)]).unwrap(),
		);
		let snapshots = SourceScanner::new(catalog).scan_once().await.unwrap();
		assert_eq!(snapshots[0].items.len(), 1);
		assert_eq!(snapshots[0].items[0].relative_path, "book.epub");
	}
}
