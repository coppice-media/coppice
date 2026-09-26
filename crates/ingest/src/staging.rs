use std::path::{Path, PathBuf};

use ring::digest::{Context, SHA256};
use tokio::{
	fs::{self, File, OpenOptions},
	io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
};
use uuid::Uuid;

use crate::{error::IngestError, IngestResult};

/// The immutable staged file produced by admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedFile {
	pub path: PathBuf,
	pub source_sha256: String,
	pub byte_size: u64,
}

/// Normalise a user supplied relative path to slash-separated components.
/// Absolute paths and parent traversal are rejected before any filesystem access.
pub fn normalize_relative_path(value: Option<&str>) -> IngestResult<Option<String>> {
	let Some(value) = value else {
		return Ok(None);
	};
	if value.is_empty() {
		return Ok(None);
	}
	if value.contains('\0') {
		return Err(IngestError::BadRequest(
			"relative path contains NUL".to_string(),
		));
	}
	let value = value.replace('\\', "/");
	if value.starts_with('/')
		|| value.starts_with("//")
		|| has_windows_drive_prefix(&value)
	{
		return Err(IngestError::BadRequest(
			"relative path must not be absolute".to_string(),
		));
	}
	let mut components = Vec::new();
	for component in value.split('/') {
		match component {
			"" | "." => {},
			".." => {
				return Err(IngestError::BadRequest(
					"relative path must not contain '..'".to_string(),
				));
			},
			component => components.push(component),
		}
	}
	if components.is_empty() {
		Ok(None)
	} else {
		Ok(Some(components.join("/")))
	}
}

/// Validate that a filename is a single path component and return a stable,
/// filesystem-safe representation for the staged filename.
pub fn sanitize_filename(filename: &str) -> IngestResult<String> {
	if filename.is_empty()
		|| filename == "."
		|| filename == ".."
		|| filename.contains('/')
		|| filename.contains('\\')
		|| filename.contains('\0')
		|| has_windows_drive_prefix(filename)
	{
		return Err(IngestError::BadRequest(
			"filename must be a non-empty relative file name".to_string(),
		));
	}
	let mut sanitized = filename
		.chars()
		.map(|c| {
			if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
				c
			} else {
				'_'
			}
		})
		.collect::<String>();
	while sanitized.starts_with('.') {
		sanitized.remove(0);
	}
	if sanitized.is_empty() {
		sanitized.push_str("file");
	}
	// Keep the generated path portable and below common NAME_MAX limits.  The
	// digest prefix still makes truncation collision-safe for different bytes.
	if sanitized.len() > 200 {
		sanitized.truncate(200);
	}
	Ok(sanitized)
}

pub fn staging_path(
	staging_root: &Path,
	library_id: &str,
	sha256: &str,
	filename: &str,
) -> IngestResult<PathBuf> {
	validate_segment(library_id, "library id")?;
	if sha256.is_empty() || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
		return Err(IngestError::BadRequest("invalid source digest".to_string()));
	}
	let filename = sanitize_filename(filename)?;
	Ok(staging_root
		.join(library_id)
		.join(format!("{sha256}-{filename}")))
}

/// Stream an upload into an immutable staging file while calculating its full
/// SHA-256 digest.  The temporary file is atomically renamed only after the
/// stream and flush complete.
pub async fn stage_reader<R>(
	staging_root: &Path,
	library_id: &str,
	filename: &str,
	reader: R,
) -> IngestResult<StagedFile>
where
	R: AsyncRead + Unpin + Send,
{
	validate_segment(library_id, "library id")?;
	let filename = sanitize_filename(filename)?;
	let library_dir = staging_root.join(library_id);
	fs::create_dir_all(&library_dir).await?;

	let temporary_path = library_dir.join(format!(".tmp-{}", Uuid::new_v4()));
	let mut output = OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(&temporary_path)
		.await?;
	let mut reader = reader;
	let mut context = Context::new(&SHA256);
	let mut byte_size = 0_u64;
	let mut buffer = vec![0_u8; 128 * 1024];
	let result = async {
		loop {
			let read = reader.read(&mut buffer).await?;
			if read == 0 {
				break;
			}
			context.update(&buffer[..read]);
			output.write_all(&buffer[..read]).await?;
			byte_size = byte_size.checked_add(read as u64).ok_or_else(|| {
				IngestError::BadRequest("uploaded file is too large".to_string())
			})?;
		}
		output.flush().await?;
		output.sync_all().await?;
		Ok::<(), IngestError>(())
	}
	.await;
	if let Err(error) = result {
		drop(output);
		let _ = fs::remove_file(&temporary_path).await;
		return Err(error);
	}
	drop(output);

	let source_sha256 = digest_hex(context.finish());
	let final_path = staging_root
		.join(library_id)
		.join(format!("{source_sha256}-{filename}"));
	match fs::rename(&temporary_path, &final_path).await {
		Ok(()) => {},
		Err(_error) if fs::metadata(&final_path).await.is_ok() => {
			let _ = fs::remove_file(&temporary_path).await;
		},
		Err(error) => {
			let _ = fs::remove_file(&temporary_path).await;
			return Err(error.into());
		},
	}

	Ok(StagedFile {
		path: final_path,
		source_sha256,
		byte_size,
	})
}

/// Calculate the full digest and size of an existing file without changing it.
pub async fn hash_file(path: &Path) -> IngestResult<(String, u64)> {
	let mut input = File::open(path).await?;
	let mut context = Context::new(&SHA256);
	let mut byte_size = 0_u64;
	let mut buffer = vec![0_u8; 128 * 1024];
	loop {
		let read = input.read(&mut buffer).await?;
		if read == 0 {
			break;
		}
		context.update(&buffer[..read]);
		byte_size = byte_size
			.checked_add(read as u64)
			.ok_or_else(|| IngestError::BadRequest("file is too large".to_string()))?;
	}
	Ok((digest_hex(context.finish()), byte_size))
}

/// Digest and total size of a *directory* of files, for the one publication
/// whose target is a folder: an audiobook of parts.
///
/// The digest is over the sorted `<relative path>\0<file digest>\n` lines
/// rather than over the concatenated bytes, so it identifies the set of files
/// the folder holds independently of traversal order, and two folders differ
/// when a part was renamed even if every byte is the same. That is what makes
/// a re-dropped archive deduplicate against the item it already produced.
pub async fn hash_dir(path: &Path) -> IngestResult<(String, u64)> {
	let mut entries = Vec::new();
	collect_files(path, &mut entries).await?;
	entries.sort();

	let mut context = Context::new(&SHA256);
	let mut byte_size = 0_u64;
	for (relative, absolute) in entries {
		let (digest, size) = hash_file(&absolute).await?;
		context.update(relative.as_bytes());
		context.update(&[0]);
		context.update(digest.as_bytes());
		context.update(b"\n");
		byte_size = byte_size.checked_add(size).ok_or_else(|| {
			IngestError::BadRequest("directory is too large".to_string())
		})?;
	}
	Ok((digest_hex(context.finish()), byte_size))
}

/// Absolute paths of every regular file directly or indirectly under `root`,
/// sorted. What an assembled audiobook's kept parts become.
pub async fn files_in(root: &Path) -> IngestResult<Vec<String>> {
	let mut entries = Vec::new();
	collect_files(root, &mut entries).await?;
	let mut paths = entries
		.into_iter()
		.map(|(_, absolute)| absolute.to_string_lossy().into_owned())
		.collect::<Vec<_>>();
	paths.sort();
	Ok(paths)
}

/// The files of a staged folder audiobook that describe it without being it:
/// cover art, an `.nfo`, a chapter sheet.
///
/// The audio parts *are* the publication, so they are never sidecars while
/// the folder is the item's target; they only become sidecars once an
/// assembled M4B has taken their place.
pub async fn sidecars_in(root: &Path) -> IngestResult<Vec<String>> {
	let mut entries = Vec::new();
	collect_files(root, &mut entries).await?;
	let mut paths = entries
		.into_iter()
		.filter(|(relative, _)| !is_audio_name(relative))
		.map(|(_, absolute)| absolute.to_string_lossy().into_owned())
		.collect::<Vec<_>>();
	paths.sort();
	Ok(paths)
}

fn is_audio_name(name: &str) -> bool {
	Path::new(name)
		.extension()
		.and_then(|extension| extension.to_str())
		.map(|extension| {
			models::entity::media::AUDIO_EXTENSIONS
				.contains(&extension.to_ascii_lowercase().as_str())
		})
		.unwrap_or(false)
}

/// Move a directory to `destination`, copying across devices.
pub async fn move_dir(source: &Path, destination: &Path) -> IngestResult<()> {
	if let Some(parent) = destination.parent() {
		fs::create_dir_all(parent).await?;
	}
	if fs::rename(source, destination).await.is_ok() {
		return Ok(());
	}
	copy_dir(source, destination).await?;
	fs::remove_dir_all(source).await?;
	Ok(())
}

/// Every regular file under `root`, as `(slash-separated relative path,
/// absolute path)`. Hidden entries are skipped, consistently with the drop
/// folder walker and the scanner.
async fn collect_files(
	root: &Path,
	out: &mut Vec<(String, PathBuf)>,
) -> IngestResult<()> {
	let mut pending = vec![root.to_path_buf()];
	while let Some(current) = pending.pop() {
		let mut entries = fs::read_dir(&current).await?;
		while let Some(entry) = entries.next_entry().await? {
			let name = entry.file_name();
			let name = name.to_string_lossy();
			if name.starts_with('.') || name == "__MACOSX" {
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
			let relative = path
				.strip_prefix(root)
				.map_err(|error| {
					IngestError::InternalError(format!(
						"failed to derive staged relative path: {error}"
					))
				})?
				.to_string_lossy()
				.replace('\\', "/");
			out.push((relative, path));
		}
	}
	Ok(())
}

/// Copy a publication directory into immutable staging without consuming or
/// changing its source. Used by acquisition handoff so a seeding client keeps
/// owning the original files.
pub async fn copy_dir_into_staging(
	source: &Path,
	staging_root: &Path,
	library_id: &str,
	filename: &str,
	sha256: &str,
) -> IngestResult<PathBuf> {
	let destination = staging_path(staging_root, library_id, sha256, filename)?;
	if fs::try_exists(&destination).await? {
		return Ok(destination);
	}

	match copy_dir(source, &destination).await {
		Ok(()) => Ok(destination),
		Err(error) => {
			if fs::try_exists(&destination).await? {
				Ok(destination)
			} else {
				let _ = fs::remove_dir_all(&destination).await;
				Err(error)
			}
		},
	}
}

/// Move a directory into the immutable staging location, for a folder
/// audiobook.
///
/// Same deterministic `<digest>-<name>` layout as a staged file, so one
/// listing of a library's staging directory shows every staged publication
/// whatever shape it has.
pub async fn move_dir_into_staging(
	source: &Path,
	staging_root: &Path,
	library_id: &str,
	dirname: &str,
	sha256: &str,
) -> IngestResult<PathBuf> {
	let destination = staging_path(staging_root, library_id, sha256, dirname)?;
	if let Some(parent) = destination.parent() {
		fs::create_dir_all(parent).await?;
	}
	if fs::try_exists(&destination).await? {
		// The same folder book staged earlier; the caller deduplicates on the
		// digest, so the freshly extracted copy is the redundant one.
		fs::remove_dir_all(source).await?;
		return Ok(destination);
	}
	match fs::rename(source, &destination).await {
		Ok(()) => Ok(destination),
		Err(rename_error) => {
			// Cross-device: copy the tree, then remove the source. A partial
			// copy is removed so staging never holds half a publication.
			match copy_dir(source, &destination).await {
				Ok(()) => {
					fs::remove_dir_all(source).await?;
					Ok(destination)
				},
				Err(copy_error) => {
					let _ = fs::remove_dir_all(&destination).await;
					Err(IngestError::InternalError(format!(
						"rename failed ({rename_error}); copy fallback failed: {copy_error}"
					)))
				},
			}
		},
	}
}

async fn copy_dir(source: &Path, destination: &Path) -> IngestResult<()> {
	fs::create_dir_all(destination).await?;
	let mut files = Vec::new();
	collect_files(source, &mut files).await?;
	for (relative, absolute) in files {
		let target = destination.join(&relative);
		if let Some(parent) = target.parent() {
			fs::create_dir_all(parent).await?;
		}
		fs::copy(&absolute, &target).await?;
	}
	Ok(())
}

/// Remove a staged target whatever shape it has.
pub async fn remove_staged(path: &Path) -> IngestResult<()> {
	match fs::symlink_metadata(path).await {
		Ok(metadata) if metadata.is_dir() => {
			fs::remove_dir_all(path).await?;
			Ok(())
		},
		Ok(_) => remove_if_exists(path).await,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
		Err(error) => Err(error.into()),
	}
}

/// Move a file into the immutable staging location.  Rename is attempted
/// first so same-filesystem scans are move-only; copy/delete is the explicit
/// fallback for cross-device drops.
pub async fn move_into_staging(
	source: &Path,
	staging_root: &Path,
	library_id: &str,
	filename: &str,
	sha256: &str,
) -> IngestResult<PathBuf> {
	let destination = staging_path(staging_root, library_id, sha256, filename)?;
	if let Some(parent) = destination.parent() {
		fs::create_dir_all(parent).await?;
	}
	match fs::rename(source, &destination).await {
		Ok(()) => Ok(destination),
		Err(rename_error) => {
			if fs::metadata(&destination).await.is_ok() {
				fs::remove_file(source).await?;
				return Ok(destination);
			}
			match fs::copy(source, &destination).await {
				Ok(_) => {
					if let Err(error) = fs::remove_file(source).await {
						let _ = fs::remove_file(&destination).await;
						return Err(error.into());
					}
					Ok(destination)
				},
				Err(copy_error) => Err(IngestError::IoError(std::io::Error::new(
					copy_error.kind(),
					format!("rename failed ({rename_error}); copy fallback failed: {copy_error}"),
				))),
			}
		},
	}
}

pub async fn remove_if_exists(path: &Path) -> IngestResult<()> {
	match fs::remove_file(path).await {
		Ok(()) => Ok(()),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
		Err(error) => Err(error.into()),
	}
}

fn validate_segment(value: &str, label: &str) -> IngestResult<()> {
	if value.is_empty()
		|| value == "."
		|| value == ".."
		|| value.contains('/')
		|| value.contains('\\')
		|| value.contains('\0')
		|| has_windows_drive_prefix(value)
	{
		return Err(IngestError::BadRequest(format!(
			"{label} is not a safe path segment"
		)));
	}
	Ok(())
}

fn has_windows_drive_prefix(value: &str) -> bool {
	let bytes = value.as_bytes();
	bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn digest_hex(digest: ring::digest::Digest) -> String {
	const HEX: &[u8; 16] = b"0123456789abcdef";
	let mut output = String::with_capacity(digest.as_ref().len() * 2);
	for byte in digest.as_ref() {
		output.push(HEX[(byte >> 4) as usize] as char);
		output.push(HEX[(byte & 0x0f) as usize] as char);
	}
	output
}

#[cfg(test)]
mod tests {
	use super::*;
	use tokio::io::AsyncWriteExt;

	#[test]
	fn rejects_parent_and_absolute_paths() {
		assert!(normalize_relative_path(Some("../book.cbz")).is_err());
		assert!(normalize_relative_path(Some("/tmp/book.cbz")).is_err());
		assert!(normalize_relative_path(Some("C:\\book.cbz")).is_err());
		assert_eq!(
			normalize_relative_path(Some("series\\book.cbz")).unwrap(),
			Some("series/book.cbz".to_string())
		);
	}

	#[tokio::test]
	async fn stages_stream_and_reuses_digest_path() {
		let temporary = tempfile::tempdir().unwrap();
		let staged = stage_reader(
			temporary.path(),
			"library",
			"Book#1.cbz",
			&b"book bytes"[..],
		)
		.await
		.unwrap();
		assert!(staged.path.exists());
		let (digest, size) = hash_file(&staged.path).await.unwrap();
		assert_eq!(digest, staged.source_sha256);
		assert_eq!(size, staged.byte_size);
		assert!(staged
			.path
			.file_name()
			.unwrap()
			.to_string_lossy()
			.contains("Book_1.cbz"));

		let (mut writer, reader) = tokio::io::duplex(16);
		writer.write_all(b"book bytes").await.unwrap();
		drop(writer);
		let second = stage_reader(temporary.path(), "library", "Book#1.cbz", reader)
			.await
			.unwrap();
		assert_eq!(second.source_sha256, staged.source_sha256);
	}
	#[tokio::test]
	async fn copies_directory_into_staging_without_consuming_the_source() {
		let temporary = tempfile::tempdir().unwrap();
		let source = temporary.path().join("release");
		tokio::fs::create_dir_all(source.join("disc 1"))
			.await
			.unwrap();
		tokio::fs::write(source.join("disc 1/01.mp3"), b"part one")
			.await
			.unwrap();
		tokio::fs::write(source.join("cover.jpg"), b"cover")
			.await
			.unwrap();
		let (digest, size) = hash_dir(&source).await.unwrap();

		let staged = copy_dir_into_staging(
			&source,
			&temporary.path().join("staging"),
			"library",
			"release",
			&digest,
		)
		.await
		.unwrap();
		assert_eq!(size, 13);
		assert_eq!(
			tokio::fs::read(staged.join("disc 1/01.mp3")).await.unwrap(),
			b"part one"
		);
		assert_eq!(
			tokio::fs::read(source.join("disc 1/01.mp3")).await.unwrap(),
			b"part one"
		);
		let duplicate = copy_dir_into_staging(
			&source,
			&temporary.path().join("staging"),
			"library",
			"release",
			&digest,
		)
		.await
		.unwrap();
		assert_eq!(duplicate, staged);
	}
}
