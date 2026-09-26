//! Media rows whose `path` is not a file on disk.
//!
//! A [`VirtualMediaResolver`] owns a URI scheme (the provider host owns
//! `provider://`) and answers both page-level questions and downloadable
//! archive requests for media paths. Registering one makes
//! [`crate::media::get_page_async`], [`crate::media::get_page_count_async`],
//! page content-type helpers, and file-serving routes transparent for those
//! rows. Synchronous file processors never see virtual paths: they fail with
//! [`FileError::UnsupportedFileType`] as for any non-archive.

use std::{
	collections::HashMap,
	sync::{Arc, RwLock},
};

use async_trait::async_trait;

use crate::{content_type::ContentType, error::FileError};

/// URI scheme of provider-backed (virtual) media and series paths.
pub const PROVIDER_SCHEME: &str = "provider://";

/// Whether `path` is a provider URI rather than a filesystem path. This is a
/// pure string check so the scanner can skip virtual rows even when no
/// resolver is registered (providers compiled out or disabled).
pub fn is_virtual_path(path: &str) -> bool {
	path.starts_with(PROVIDER_SCHEME)
}

/// A complete downloadable archive assembled from virtual media pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualArchive {
	pub file_name: String,
	pub content_type: ContentType,
	pub bytes: Vec<u8>,
}

#[async_trait]
pub trait VirtualMediaResolver: Send + Sync {
	/// Whether this resolver serves `path`.
	fn owns(&self, path: &str) -> bool;

	/// Bytes and content type of a 1-indexed page.
	async fn get_page(
		&self,
		path: &str,
		page: i32,
	) -> Result<(ContentType, Vec<u8>), FileError>;

	async fn get_page_count(&self, path: &str) -> Result<i32, FileError>;

	/// Build a downloadable archive from this virtual media's pages.
	async fn get_archive(
		&self,
		path: &str,
		file_stem: &str,
	) -> Result<VirtualArchive, FileError>;

	/// Content types for 1-indexed pages without fetching them; unknown pages
	/// may be reported as [`ContentType::UNKNOWN`].
	fn page_content_types(
		&self,
		path: &str,
		pages: &[i32],
	) -> Result<HashMap<i32, ContentType>, FileError>;
}

/// Return the resolver for a provider path, or a user-facing unavailability
/// error when the host is not registered.
pub fn resolver_for_virtual_path(
	path: &str,
) -> Result<Arc<dyn VirtualMediaResolver>, FileError> {
	resolver_for(path).ok_or_else(|| {
		FileError::Unavailable(
			"Provider host is disabled; provider-backed media is unavailable".to_owned(),
		)
	})
}

/// Build a downloadable archive for a virtual media path.
pub async fn get_archive(
	path: &str,
	file_stem: &str,
) -> Result<VirtualArchive, FileError> {
	resolver_for_virtual_path(path)?
		.get_archive(path, file_stem)
		.await
}

static RESOLVER: RwLock<Option<Arc<dyn VirtualMediaResolver>>> = RwLock::new(None);

/// Install the process-wide resolver, replacing any previous one.
pub fn register(resolver: Arc<dyn VirtualMediaResolver>) {
	*RESOLVER.write().expect("virtual media resolver poisoned") = Some(resolver);
}

/// Remove the process-wide resolver.
pub fn unregister() {
	*RESOLVER.write().expect("virtual media resolver poisoned") = None;
}

/// The registered resolver that owns `path`, if any.
pub fn resolver_for(path: &str) -> Option<Arc<dyn VirtualMediaResolver>> {
	let guard = RESOLVER.read().expect("virtual media resolver poisoned");
	guard
		.as_ref()
		.filter(|resolver| resolver.owns(path))
		.cloned()
}
