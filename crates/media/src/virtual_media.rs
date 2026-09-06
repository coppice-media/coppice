//! Media rows whose `path` is not a file on disk.
//!
//! A [`VirtualMediaResolver`] owns a URI scheme (the provider host owns
//! `provider://`) and answers the page-level questions the rest of Stump asks
//! of a media path. Registering one makes [`crate::media::get_page_async`],
//! [`crate::media::get_page_count_async`], and the page content-type helpers
//! transparent for those rows, so every route that serves pages keeps calling
//! the same functions. Synchronous file processors never see virtual paths:
//! they fail with [`FileError::UnsupportedFileType`] as for any non-archive.

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

	/// Content types for 1-indexed pages without fetching them; unknown pages
	/// may be reported as [`ContentType::UNKNOWN`].
	fn page_content_types(
		&self,
		path: &str,
		pages: &[i32],
	) -> Result<HashMap<i32, ContentType>, FileError>;
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
