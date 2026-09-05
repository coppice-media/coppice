use std::sync::Arc;

use super::{
	coordinator::IngestCoordinator, providers::ProviderRegistry,
	quality::QualityRegistry, store::IngestStore,
};
use crate::config::StumpConfig;
use crate::event::CoreEvent;
use sea_orm::DatabaseConnection;
use tokio::sync::broadcast;

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

	/// Attach the core event channel so analysis outcomes are announced to
	/// notification routing. Called by the host context at construction.
	pub fn with_event_tx(mut self, events: broadcast::Sender<CoreEvent>) -> Self {
		self.coordinator = self.coordinator.with_event_tx(events);
		self
	}
}
