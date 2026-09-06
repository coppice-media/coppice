use std::path::{Path, PathBuf};

use tokio::fs;

use crate::{error::IngestError, IngestResult};

/// A regular file discovered under a library's ingest drop folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropFile {
	pub path: PathBuf,
	pub relative_path: String,
	pub filename: String,
	pub byte_size: u64,
}

/// Traverse a drop folder without following symlinks. Hidden files and
/// macOS metadata directories are ignored consistently with the scanner.
pub async fn list_files(root: &Path) -> IngestResult<Vec<DropFile>> {
	if !fs::try_exists(root).await? {
		return Ok(Vec::new());
	}
	let metadata = fs::symlink_metadata(root).await?;
	if !metadata.is_dir() {
		return Err(IngestError::BadRequest(format!(
			"drop folder is not a directory: {}",
			root.display()
		)));
	}
	let mut files = Vec::new();
	let mut pending = vec![root.to_path_buf()];
	while let Some(directory) = pending.pop() {
		let mut entries = fs::read_dir(&directory).await?;
		while let Some(entry) = entries.next_entry().await? {
			let file_name = entry.file_name();
			let file_name = file_name.to_string_lossy();
			if file_name.starts_with('.') || file_name == "__MACOSX" {
				continue;
			}
			let path = entry.path();
			let metadata = fs::symlink_metadata(&path).await?;
			if metadata.is_dir() {
				pending.push(path);
				continue;
			}
			if !metadata.is_file() {
				continue;
			}
			let relative = path.strip_prefix(root).map_err(|error| {
				IngestError::InternalError(format!(
					"failed to derive drop-folder relative path: {error}"
				))
			})?;
			let relative_path = relative.to_string_lossy().replace('\\', "/");
			let filename = relative
				.file_name()
				.ok_or_else(|| {
					IngestError::BadRequest("drop file has no filename".to_string())
				})?
				.to_string_lossy()
				.into_owned();
			files.push(DropFile {
				path,
				relative_path,
				filename,
				byte_size: metadata.len(),
			});
		}
	}
	files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
	Ok(files)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropFolderStats {
	pub pending_files: u32,
	pub total_bytes: u64,
}

pub async fn stats(root: &Path) -> IngestResult<DropFolderStats> {
	let files = list_files(root).await?;
	let pending_files = u32::try_from(files.len()).unwrap_or(u32::MAX);
	let total_bytes = files.iter().try_fold(0_u64, |total, file| {
		total.checked_add(file.byte_size).ok_or_else(|| {
			IngestError::BadRequest("drop-folder size exceeds u64".to_string())
		})
	})?;
	Ok(DropFolderStats {
		pending_files,
		total_bytes,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use tokio::io::AsyncWriteExt;

	#[tokio::test]
	async fn lists_nested_files_and_skips_hidden_entries() {
		let temporary = tempfile::tempdir().unwrap();
		fs::create_dir_all(temporary.path().join("series"))
			.await
			.unwrap();
		let mut file = fs::File::create(temporary.path().join("series/book.cbz"))
			.await
			.unwrap();
		file.write_all(b"book").await.unwrap();
		fs::write(temporary.path().join(".partial"), b"ignore")
			.await
			.unwrap();
		let files = list_files(temporary.path()).await.unwrap();
		assert_eq!(files.len(), 1);
		assert_eq!(files[0].relative_path, "series/book.cbz");
		let stats = stats(temporary.path()).await.unwrap();
		assert_eq!(stats.pending_files, 1);
		assert_eq!(stats.total_bytes, 4);
	}
}
