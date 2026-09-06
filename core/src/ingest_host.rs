//! Core's side of the `stump_ingest` boundary.
//!
//! The dependency points one way — `stump_core` -> `stump_ingest` — so the
//! pipeline can neither read [`StumpConfig`] nor build library rows nor
//! decrypt a provider API token. This module is the only place that knows
//! both vocabularies: it resolves the settings, implements the two host
//! traits, and maps ingest events onto [`CoreEvent`].

use std::{path::Path, sync::Arc};

use models::entity::{
	library_config, media, media_metadata, metadata_provider_config, series,
	series_metadata, server_config,
};
use sea_orm::{DatabaseConnection, EntityTrait, QuerySelect, SelectColumns};
use stump_ingest::{
	config::IngestSettings,
	error::{IngestError, IngestResult},
	event::{IngestEvent, IngestEventSink},
	host::{ProviderClientFactory, RowFactory},
};
use tokio::sync::{broadcast::Sender, OnceCell};

use crate::{
	config::StumpConfig,
	error::CoreError,
	event::{
		AnalysisJobFailed, CoreEvent, IngestAwaitingReview, IngestItemChanged,
		ProviderMatchDone, QualityFailed,
	},
	filesystem::{
		media::MediaBuilder, metadata::ProviderClientCache, series::SeriesBuilder,
	},
};

impl From<IngestError> for CoreError {
	fn from(error: IngestError) -> Self {
		match error {
			IngestError::NotFound(message) => Self::NotFound(message),
			IngestError::BadRequest(message) => Self::BadRequest(message),
			IngestError::FileNotFound(message) => Self::FileNotFound(message),
			IngestError::IoError(error) => Self::IoError(error),
			IngestError::InitializationError(message) => {
				Self::InitializationError(message)
			},
			IngestError::InternalError(message) => Self::InternalError(message),
			IngestError::DBError(error) => Self::DBError(error),
			IngestError::SerdeFailure(error) => Self::SerdeFailure(error),
			IngestError::Unknown(message) => Self::Unknown(message),
		}
	}
}

/// The pipeline's settings, with the directory defaults resolved against the
/// application configuration directory.
pub(crate) fn ingest_settings(config: &StumpConfig) -> IngestSettings {
	IngestSettings {
		drop_dir: config.get_ingest_drop_dir(),
		staging_dir: config.get_ingest_staging_dir(),
		progress_retention: config.ingest.ingest_progress_retention,
		preprocess_command: config.ingest.ingest_preprocess_command.clone(),
		preprocess_timeout_secs: config.ingest.ingest_preprocess_timeout_secs,
		media: config.media.clone(),
	}
}

/// Fail startup on an ingest configuration the pipeline cannot use — today a
/// preprocess command that does not resolve to an executable. A typo must
/// stop the server, not silently fail every dropped file for the rest of its
/// uptime.
pub(crate) fn validate_ingest_config(config: &StumpConfig) -> crate::CoreResult<()> {
	stump_ingest::preprocess::validate_config(&ingest_settings(config))?;
	Ok(())
}

/// Builds library rows exactly the way a library scan builds them, so a
/// committed staged file is indistinguishable from a scanned one.
pub(crate) struct CoreRowFactory {
	config: Arc<StumpConfig>,
}

impl CoreRowFactory {
	pub(crate) fn new(config: Arc<StumpConfig>) -> Self {
		Self { config }
	}
}

impl RowFactory for CoreRowFactory {
	fn series_rows(
		&self,
		path: &Path,
		library_id: &str,
	) -> IngestResult<(series::ActiveModel, Option<series_metadata::ActiveModel>)> {
		let built = SeriesBuilder::new(path, library_id)
			.build()
			.map_err(map_build_error)?;
		Ok((built.series, built.metadata))
	}

	fn media_rows(
		&self,
		path: &Path,
		series_id: &str,
		library_config: library_config::Model,
	) -> IngestResult<(media::ActiveModel, Option<media_metadata::ActiveModel>)> {
		let built = MediaBuilder::new(path, series_id, library_config, &self.config)
			.build()
			.map_err(map_build_error)?;
		Ok((built.media, built.metadata))
	}
}

/// The builders fail with core's error type; keep the kinds the pipeline acts
/// on and flatten the rest.
fn map_build_error(error: CoreError) -> IngestError {
	match error {
		CoreError::IoError(error) => IngestError::IoError(error),
		CoreError::FileNotFound(path) => IngestError::FileNotFound(path),
		CoreError::DBError(error) => IngestError::DBError(error),
		error => IngestError::InternalError(error.to_string()),
	}
}

/// Metadata provider clients, built from the encrypted per-provider tokens.
/// The server encryption key is read once, on first use.
pub(crate) struct CoreProviderClients {
	conn: Arc<DatabaseConnection>,
	cache: OnceCell<Arc<ProviderClientCache>>,
}

impl CoreProviderClients {
	pub(crate) fn new(conn: Arc<DatabaseConnection>) -> Self {
		Self {
			conn,
			cache: OnceCell::new(),
		}
	}
}

#[async_trait::async_trait]
impl ProviderClientFactory for CoreProviderClients {
	async fn client(
		&self,
		config: &metadata_provider_config::Model,
	) -> Result<Arc<dyn metadata_integrations::MetadataProvider + Send + Sync>, String> {
		let cache = self
			.cache
			.get_or_try_init(|| async {
				let encryption_key = server_config::Entity::find()
					.select_only()
					.select_column(server_config::Column::EncryptionKey)
					.into_model::<server_config::EncryptionKeySelect>()
					.one(self.conn.as_ref())
					.await
					.map_err(|error| error.to_string())?
					.and_then(|record| record.encryption_key)
					.ok_or_else(|| {
						"server encryption key is not configured".to_string()
					})?;
				Ok::<_, String>(Arc::new(ProviderClientCache::new(encryption_key)))
			})
			.await?;
		cache
			.get_or_create(config)
			.await
			.map_err(|error| error.to_string())
	}
}

/// Forwards analysis outcomes onto the core event channel, where notification
/// routing and GraphQL subscribers pick them up.
pub(crate) struct CoreEventSink {
	events: Sender<CoreEvent>,
}

impl CoreEventSink {
	pub(crate) fn new(events: Sender<CoreEvent>) -> Self {
		Self { events }
	}
}

impl IngestEventSink for CoreEventSink {
	fn emit(&self, event: IngestEvent) {
		let event = match event {
			IngestEvent::AnalysisJobFailed {
				analysis_job_id,
				error,
			} => CoreEvent::AnalysisJobFailed(AnalysisJobFailed {
				analysis_job_id,
				error,
			}),
			IngestEvent::QualityFailed {
				library_id,
				drop_item_id,
				media_id,
				created_by,
				score,
				failed_checks,
			} => CoreEvent::QualityFailed(QualityFailed {
				library_id,
				drop_item_id,
				media_id,
				created_by,
				score,
				failed_checks,
			}),
			IngestEvent::ProviderMatchDone {
				library_id,
				drop_item_id,
				created_by,
				candidate_count,
			} => CoreEvent::ProviderMatchDone(ProviderMatchDone {
				library_id,
				drop_item_id,
				created_by,
				candidate_count,
			}),
			IngestEvent::AwaitingReview {
				library_id,
				drop_item_id,
				source_filename,
				created_by,
			} => CoreEvent::IngestAwaitingReview(IngestAwaitingReview {
				library_id,
				drop_item_id,
				source_filename,
				created_by,
			}),
			IngestEvent::ItemChanged {
				library_id,
				item_id,
				status,
				revision,
			} => CoreEvent::IngestItemChanged(IngestItemChanged {
				library_id,
				item_id,
				status: status.as_str().to_string(),
				revision,
			}),
		};
		let _ = self.events.send(event);
	}
}
