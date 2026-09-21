use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
	fs::{self, File},
	io::{self, Read},
	path::{Path, PathBuf},
};

const MAX_PATH_BYTES: usize = 4096;

/// A completed payload that Coppice may admit through its configured staging
/// root. Every field is relative or content-derived; no absolute path leaves
/// the gateway.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffMetadata {
	pub relative_path: String,
	pub filename: String,
	pub sha256: String,
	pub byte_size: u64,
	pub media_kind: String,
}

pub enum HandoffInspection {
	Ready(HandoffMetadata),
	NotReady,
	Invalid(&'static str),
}

/// Inspect the qBittorrent-reported content path off the async executor. A
/// completed torrent can briefly report before its files are visible, so
/// missing paths are `NotReady` rather than a terminal failure.
pub async fn inspect(
	root: PathBuf,
	content_path: Option<String>,
	max_bytes: u64,
) -> HandoffInspection {
	tokio::task::spawn_blocking(move || {
		inspect_sync(&root, content_path.as_deref(), max_bytes)
	})
	.await
	.unwrap_or(HandoffInspection::Invalid("handoff_inspection_failed"))
}

fn inspect_sync(
	root: &Path,
	content_path: Option<&str>,
	max_bytes: u64,
) -> HandoffInspection {
	let Some(content_path) = content_path else {
		return HandoffInspection::NotReady;
	};
	if content_path.is_empty()
		|| content_path.len() > MAX_PATH_BYTES
		|| content_path
			.bytes()
			.any(|byte| byte == 0 || byte == b'\r' || byte == b'\n')
	{
		return HandoffInspection::Invalid("handoff_path_invalid");
	}

	let canonical_root = match fs::canonicalize(root) {
		Ok(path) => path,
		Err(error) if error.kind() == io::ErrorKind::NotFound => {
			return HandoffInspection::NotReady;
		},
		Err(_) => return HandoffInspection::Invalid("handoff_root_unavailable"),
	};
	let reported = PathBuf::from(content_path);
	let reported = if reported.is_absolute() {
		reported
	} else {
		canonical_root.join(reported)
	};
	if reported == canonical_root {
		return HandoffInspection::Invalid("handoff_path_invalid");
	}
	if !lexically_under_root(&reported, &canonical_root) {
		return HandoffInspection::Invalid("handoff_root_escape");
	}
	let Some(canonical_content) = canonicalize_checked(&reported, &canonical_root) else {
		return if reported.exists() {
			HandoffInspection::Invalid("handoff_root_escape")
		} else {
			HandoffInspection::NotReady
		};
	};
	if canonical_content == canonical_root {
		return HandoffInspection::Invalid("handoff_path_invalid");
	}

	let metadata = match fs::symlink_metadata(&reported) {
		Ok(metadata) => metadata,
		Err(error) if error.kind() == io::ErrorKind::NotFound => {
			return HandoffInspection::NotReady;
		},
		Err(_) => return HandoffInspection::Invalid("handoff_unreadable"),
	};
	if metadata.file_type().is_symlink() {
		return HandoffInspection::Invalid("handoff_symlink");
	}

	let mut payloads = Vec::new();
	if metadata.is_file() {
		payloads.push(canonical_content);
	} else if metadata.is_dir() {
		if let Err(outcome) =
			collect_files(&canonical_content, &canonical_root, &mut payloads)
		{
			return HandoffInspection::Invalid(outcome);
		}
	} else {
		return HandoffInspection::Invalid("unsupported_payload");
	}

	if payloads.is_empty() {
		return HandoffInspection::NotReady;
	}
	if payloads.len() > 1 {
		return HandoffInspection::Invalid("multiple_payloads");
	}
	let payload = payloads.pop().expect("payload count checked");
	let Some(media_kind) = supported_media_kind(&payload) else {
		return HandoffInspection::Invalid("unsupported_payload");
	};
	let Some(file_name) = payload.file_name().and_then(|name| name.to_str()) else {
		return HandoffInspection::Invalid("unsupported_payload");
	};
	if file_name.is_empty() || file_name == "." || file_name == ".." {
		return HandoffInspection::Invalid("unsupported_payload");
	}
	let relative_path = match payload.strip_prefix(&canonical_root) {
		Ok(relative) => relative_to_slash(relative),
		Err(_) => return HandoffInspection::Invalid("handoff_root_escape"),
	};
	let Some(relative_path) = relative_path else {
		return HandoffInspection::Invalid("handoff_root_escape");
	};
	match hash_file(&payload, max_bytes) {
		Ok((sha256, byte_size)) => HandoffInspection::Ready(HandoffMetadata {
			relative_path,
			filename: file_name.to_owned(),
			sha256,
			byte_size,
			media_kind: media_kind.to_owned(),
		}),
		Err(HashOutcome::NotReady) => HandoffInspection::NotReady,
		Err(HashOutcome::Invalid(code)) => HandoffInspection::Invalid(code),
	}
}

fn lexically_under_root(path: &Path, root: &Path) -> bool {
	let Ok(relative) = path.strip_prefix(root) else {
		return false;
	};
	!relative
		.components()
		.any(|component| matches!(component, std::path::Component::ParentDir))
}

fn canonicalize_checked(path: &Path, root: &Path) -> Option<PathBuf> {
	let canonical = fs::canonicalize(path).ok()?;
	canonical.strip_prefix(root).ok()?;
	Some(canonical)
}

fn collect_files(
	path: &Path,
	root: &Path,
	files: &mut Vec<PathBuf>,
) -> Result<(), &'static str> {
	let entries = fs::read_dir(path).map_err(|_| "handoff_unreadable")?;
	for entry in entries {
		let entry = entry.map_err(|_| "handoff_unreadable")?;
		let entry_path = entry.path();
		let metadata =
			fs::symlink_metadata(&entry_path).map_err(|_| "handoff_unreadable")?;
		if metadata.file_type().is_symlink() {
			return Err("handoff_symlink");
		}
		let canonical =
			canonicalize_checked(&entry_path, root).ok_or("handoff_root_escape")?;
		if metadata.is_dir() {
			collect_files(&canonical, root, files)?;
		} else if metadata.is_file() {
			files.push(canonical);
			if files.len() > 1 {
				// The caller only needs the terminal multiple-payload reason;
				// stop descending once it is unambiguous.
				return Ok(());
			}
		} else {
			return Err("unsupported_payload");
		}
	}
	Ok(())
}

fn relative_to_slash(path: &Path) -> Option<String> {
	let mut output = String::new();
	for component in path.components() {
		let part = component.as_os_str().to_str()?;
		if part.is_empty() || part == "." || part == ".." {
			return None;
		}
		if !output.is_empty() {
			output.push('/');
		}
		output.push_str(part);
	}
	(!output.is_empty() && output.len() <= MAX_PATH_BYTES).then_some(output)
}

fn supported_media_kind(path: &Path) -> Option<&'static str> {
	let extension = path.extension()?.to_str()?.to_ascii_lowercase();
	match extension.as_str() {
		"epub" => Some("EPUB"),
		"pdf" => Some("PDF"),
		"cbz" => Some("CBZ"),
		"cbr" => Some("CBR"),
		"cb7" => Some("CB7"),
		"mobi" => Some("MOBI"),
		"azw" => Some("AZW"),
		"azw3" => Some("AZW3"),
		"fb2" => Some("FB2"),
		"djvu" | "djv" => Some("DJVU"),
		_ => None,
	}
}

enum HashOutcome {
	NotReady,
	Invalid(&'static str),
}

fn hash_file(path: &Path, max_bytes: u64) -> Result<(String, u64), HashOutcome> {
	let before = fs::metadata(path).map_err(map_metadata_error)?;
	if !before.is_file() {
		return Err(HashOutcome::Invalid("unsupported_payload"));
	}
	if before.len() > max_bytes {
		return Err(HashOutcome::Invalid("handoff_too_large"));
	}
	let mut file = File::open(path).map_err(map_io_error)?;
	let mut hasher = Sha256::new();
	let mut buffer = [0u8; 64 * 1024];
	let mut total = 0u64;
	loop {
		let read = file.read(&mut buffer).map_err(map_io_error)?;
		if read == 0 {
			break;
		}
		total = total.saturating_add(read as u64);
		if total > max_bytes {
			return Err(HashOutcome::Invalid("handoff_too_large"));
		}
		hasher.update(&buffer[..read]);
	}
	let after = fs::metadata(path).map_err(map_metadata_error)?;
	if after.len() != total || after.len() != before.len() {
		return Err(HashOutcome::Invalid("handoff_changed"));
	}
	Ok((hex_lower(&hasher.finalize()), total))
}

fn map_metadata_error(error: io::Error) -> HashOutcome {
	if error.kind() == io::ErrorKind::NotFound {
		HashOutcome::NotReady
	} else {
		HashOutcome::Invalid("handoff_unreadable")
	}
}

fn map_io_error(error: io::Error) -> HashOutcome {
	if error.kind() == io::ErrorKind::NotFound {
		HashOutcome::NotReady
	} else {
		HashOutcome::Invalid("handoff_unreadable")
	}
}

fn hex_lower(bytes: &[u8]) -> String {
	const HEX: &[u8; 16] = b"0123456789abcdef";
	let mut output = String::with_capacity(bytes.len() * 2);
	for byte in bytes {
		output.push(HEX[(byte >> 4) as usize] as char);
		output.push(HEX[(byte & 0x0f) as usize] as char);
	}
	output
}

#[cfg(test)]
mod tests {
	use super::*;
	use tempfile::tempdir;

	#[tokio::test]
	async fn discovers_one_supported_regular_file_and_hashes_it() {
		let root = tempdir().expect("tempdir");
		let payload = root.path().join("book.epub");
		fs::write(&payload, b"fixture").expect("payload");
		let result = inspect(
			root.path().to_owned(),
			Some(payload.to_string_lossy().into_owned()),
			1024,
		)
		.await;
		match result {
			HandoffInspection::Ready(handoff) => {
				assert_eq!(handoff.relative_path, "book.epub");
				assert_eq!(handoff.filename, "book.epub");
				assert_eq!(handoff.byte_size, 7);
				assert_eq!(handoff.media_kind, "EPUB");
				assert_eq!(
					handoff.sha256,
					"f16d05ec6b29248d2c61adb1e9263f78e4f7bace1b955014a2d17872cfe4064d"
				);
			},
			HandoffInspection::NotReady => panic!("payload should be ready"),
			HandoffInspection::Invalid(code) => panic!("unexpected {code}"),
		}
	}

	#[cfg(unix)]
	#[tokio::test]
	async fn symlink_payload_is_rejected_even_when_it_points_inside_root() {
		use std::os::unix::fs::symlink;

		let root = tempdir().expect("tempdir");
		let payload = root.path().join("book.epub");
		fs::write(&payload, b"fixture").expect("payload");
		let link = root.path().join("link.epub");
		symlink(&payload, &link).expect("symlink");
		let result = inspect(
			root.path().to_owned(),
			Some(link.to_string_lossy().into_owned()),
			1024,
		)
		.await;
		assert!(matches!(
			result,
			HandoffInspection::Invalid("handoff_symlink")
		));
	}
}
