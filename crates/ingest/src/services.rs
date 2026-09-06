//! The host-facing facade: everything a server needs to run staged ingest,
//! plus the host collaborators the pipeline refuses to implement itself.

use std::sync::Arc;

use sea_orm::DatabaseConnection;

use super::{
	coordinator::IngestCoordinator, providers::ProviderRegistry,
	quality::QualityRegistry, store::IngestStore,
};
use crate::{
	config::IngestSettings,
	contract::FieldPick,
	error::IngestResult,
	event::IngestEventSink,
	host::{ProviderClientFactory, RowFactory},
	store::DropItemModel,
};

/// Lazily constructed services backing the staged ingest workflow.
pub struct IngestServices {
	pub store: IngestStore,
	pub coordinator: IngestCoordinator,
	pub quality: Arc<QualityRegistry>,
	pub providers: Arc<ProviderRegistry>,
	/// Host-side library row construction, needed to commit an approved item.
	rows: Arc<dyn RowFactory>,
}

impl IngestServices {
	pub fn new(
		config: Arc<IngestSettings>,
		conn: Arc<DatabaseConnection>,
		rows: Arc<dyn RowFactory>,
		clients: Arc<dyn ProviderClientFactory>,
	) -> Self {
		let store = IngestStore::new(config, conn.clone());
		let quality = Arc::new(QualityRegistry::builtin(conn.clone()));
		let providers = Arc::new(ProviderRegistry::new(conn, clients));
		let coordinator =
			IngestCoordinator::new(store.clone(), quality.clone(), providers.clone());
		Self {
			store,
			coordinator,
			quality,
			providers,
			rows,
		}
	}

	/// Attach the host's event sink so analysis outcomes are announced to
	/// notification routing. Called by the host context at construction.
	pub fn with_event_sink(mut self, events: Arc<dyn IngestEventSink>) -> Self {
		self.coordinator = self.coordinator.with_event_sink(events);
		self
	}

	/// Commit a staged item into its library. Delegates to
	/// [`IngestStore::approve`] with the host's row factory, which is why
	/// approval goes through the facade rather than the store.
	pub async fn approve(
		&self,
		item_id: &str,
		picks: Vec<FieldPick>,
		expected_revision: i32,
	) -> IngestResult<(String, DropItemModel)> {
		self.store
			.approve(item_id, picks, expected_revision, self.rows.clone())
			.await
	}
}
