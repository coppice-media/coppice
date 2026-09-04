use std::sync::Arc;

use super::{
	coordinator::IngestCoordinator, providers::ProviderRegistry,
	quality::QualityRegistry, store::IngestStore,
};
use crate::config::StumpConfig;
use sea_orm::DatabaseConnection;

/// Lazily constructed services backing the staged ingest workflow.
pub struct IngestServices {
	pub store: IngestStore,
	pub coordinator: IngestCoordinator,
	pub quality: Arc<QualityRegistry>,
	pub providers: Arc<ProviderRegistry>,
}

impl IngestServices {
	pub fn new(config: Arc<StumpConfig>, conn: Arc<DatabaseConnection>) -> Self {
		let store = IngestStore::new(config.clone(), conn.clone());
		let quality = Arc::new(QualityRegistry::builtin(conn.clone()));
		let providers = Arc::new(ProviderRegistry::new(config, conn));
		let coordinator =
			IngestCoordinator::new(store.clone(), quality.clone(), providers.clone());
		Self {
			store,
			coordinator,
			quality,
			providers,
		}
	}
}
