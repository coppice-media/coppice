//! What the pipeline needs from its host, and nothing more.
//!
//! Two things cannot live in this crate: building library rows from a file
//! (the file processors, thumbnailers and the library config live in
//! `stump_core`) and decrypting a stored provider API token (the server
//! encryption key and the encryption scheme live in `stump_core`). Both are
//! injected as traits so the dependency points from `stump_core` to
//! `stump_ingest` and never back — see `core/src/ingest_host.rs` for the only
//! implementations that ship.

use std::{path::Path, sync::Arc};

use async_trait::async_trait;
use metadata_integrations::MetadataProvider;
use models::entity::{
	library_config, media, media_metadata, metadata_provider_config, series,
	series_metadata,
};

use crate::error::IngestResult;

/// The library rows for a file that already sits at its final path.
///
/// Both methods block (they read and hash the file); the pipeline calls them
/// on a blocking thread, so implementations must not call back into async
/// code.
pub trait RowFactory: Send + Sync + 'static {
	/// Rows for the series directory a committed book lands in.
	fn series_rows(
		&self,
		path: &Path,
		library_id: &str,
	) -> IngestResult<(series::ActiveModel, Option<series_metadata::ActiveModel>)>;

	/// Rows for one book, built the same way a library scan builds them.
	fn media_rows(
		&self,
		path: &Path,
		series_id: &str,
		library_config: library_config::Model,
	) -> IngestResult<(media::ActiveModel, Option<media_metadata::ActiveModel>)>;
}

/// Metadata provider clients for configured integrations.
///
/// The stored API token is encrypted with the server key, so only the host can
/// turn a configuration row into a usable client. Errors are returned as
/// strings because they are surfaced verbatim as the provider's
/// `NotConfigured` message.
#[async_trait]
pub trait ProviderClientFactory: Send + Sync + 'static {
	async fn client(
		&self,
		config: &metadata_provider_config::Model,
	) -> Result<Arc<dyn MetadataProvider + Send + Sync>, String>;
}

/// A factory with no clients, for tests that assert on rows and never reach a
/// remote provider. Every call fails, loudly and by design.
#[cfg(test)]
pub(crate) struct NoProviderClients;

#[cfg(test)]
#[async_trait]
impl ProviderClientFactory for NoProviderClients {
	async fn client(
		&self,
		config: &metadata_provider_config::Model,
	) -> Result<Arc<dyn MetadataProvider + Send + Sync>, String> {
		Err(format!(
			"no provider client for {} is configured in tests",
			config.provider_type
		))
	}
}
