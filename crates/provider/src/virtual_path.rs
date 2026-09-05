//! Provider-backed rows keep `path` non-null and store a `provider://` URI
//! instead of a filesystem location. The URI is the only thing the media
//! layer needs to resolve pages, so it doubles as the identity handed to
//! [`stump_media::virtual_media::VirtualMediaResolver`].
//!
//! Layout: `provider://<source_id>/<remote_id>[/<remote_chapter_id>]` with
//! every component percent-encoded.

use std::fmt;

pub const SCHEME: &str = "provider://";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VirtualPath {
	pub source_id: String,
	pub remote_id: String,
	pub remote_chapter_id: Option<String>,
}

impl VirtualPath {
	pub fn series(source_id: impl Into<String>, remote_id: impl Into<String>) -> Self {
		Self {
			source_id: source_id.into(),
			remote_id: remote_id.into(),
			remote_chapter_id: None,
		}
	}

	pub fn chapter(
		source_id: impl Into<String>,
		remote_id: impl Into<String>,
		remote_chapter_id: impl Into<String>,
	) -> Self {
		Self {
			source_id: source_id.into(),
			remote_id: remote_id.into(),
			remote_chapter_id: Some(remote_chapter_id.into()),
		}
	}

	/// Whether a stored path is a provider URI.
	pub fn is_virtual(path: &str) -> bool {
		path.starts_with(SCHEME)
	}

	pub fn parse(path: &str) -> Option<Self> {
		let rest = path.strip_prefix(SCHEME)?;
		let mut parts = rest.split('/');
		let source_id = decode(parts.next()?)?;
		let remote_id = decode(parts.next()?)?;
		let remote_chapter_id = match parts.next() {
			Some(chapter) => Some(decode(chapter)?),
			None => None,
		};
		if parts.next().is_some() || source_id.is_empty() || remote_id.is_empty() {
			return None;
		}
		Some(Self {
			source_id,
			remote_id,
			remote_chapter_id,
		})
	}
}

impl fmt::Display for VirtualPath {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"{SCHEME}{}/{}",
			urlencoding::encode(&self.source_id),
			urlencoding::encode(&self.remote_id)
		)?;
		if let Some(chapter) = &self.remote_chapter_id {
			write!(f, "/{}", urlencoding::encode(chapter))?;
		}
		Ok(())
	}
}

fn decode(component: &str) -> Option<String> {
	if component.is_empty() {
		return None;
	}
	urlencoding::decode(component)
		.ok()
		.map(|value| value.into_owned())
}

/// Namespace for provider-derived row ids (UUID v5), fixed so the same remote
/// series/chapter always maps to the same Stump id regardless of how (or on
/// which instance) it was materialised.
const ID_NAMESPACE: uuid::Uuid = uuid::Uuid::from_bytes([
	0x9f, 0x2c, 0x5a, 0x1e, 0x8b, 0x47, 0x4d, 0x3a, 0xa6, 0x0f, 0x1d, 0x2e, 0x7c, 0x4b, 0x9a,
	0x31,
]);

/// Deterministic `series.id` for `(source, remote_id)`.
pub fn series_id(source_id: &str, remote_id: &str) -> String {
	uuid::Uuid::new_v5(&ID_NAMESPACE, format!("series:{source_id}:{remote_id}").as_bytes())
		.to_string()
}

/// Deterministic `media.id` for `(source, remote_chapter_id)`.
pub fn media_id(source_id: &str, remote_chapter_id: &str) -> String {
	uuid::Uuid::new_v5(
		&ID_NAMESPACE,
		format!("media:{source_id}:{remote_chapter_id}").as_bytes(),
	)
	.to_string()
}

/// Deterministic `library.id` for a virtual library over `source_id`.
pub fn library_id(source_id: &str) -> String {
	uuid::Uuid::new_v5(&ID_NAMESPACE, format!("library:{source_id}").as_bytes()).to_string()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn deterministic_ids_are_stable_and_distinct() {
		assert_eq!(series_id("mock-en", "alpha"), series_id("mock-en", "alpha"));
		assert_ne!(series_id("mock-en", "alpha"), series_id("mock-ja", "alpha"));
		assert_ne!(series_id("mock-en", "alpha"), media_id("mock-en", "alpha"));
		assert_ne!(library_id("mock-en"), series_id("mock-en", "mock-en"));
		assert_eq!(series_id("mock-en", "alpha").len(), 36);
	}

	#[test]
	fn round_trips_series_and_chapter_paths() {
		let series = VirtualPath::series("mangadex-en", "abc/def");
		let rendered = series.to_string();
		assert_eq!(rendered, "provider://mangadex-en/abc%2Fdef");
		assert_eq!(VirtualPath::parse(&rendered), Some(series));

		let chapter = VirtualPath::chapter("mangadex-en", "abc", "ch 1");
		let rendered = chapter.to_string();
		assert_eq!(rendered, "provider://mangadex-en/abc/ch%201");
		assert_eq!(VirtualPath::parse(&rendered), Some(chapter));
	}

	#[test]
	fn rejects_filesystem_paths_and_malformed_uris() {
		assert!(!VirtualPath::is_virtual("/library/series/book.cbz"));
		assert_eq!(VirtualPath::parse("/library/book.cbz"), None);
		assert_eq!(VirtualPath::parse("provider://only-source"), None);
		assert_eq!(VirtualPath::parse("provider://a/b/c/d"), None);
		assert_eq!(VirtualPath::parse("provider:///b"), None);
	}
}
