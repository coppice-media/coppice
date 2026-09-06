//! Host-visible outcomes of a provider run (catalog refresh, source health
//! transitions, materialisation).
//!
//! The crate emits its own vocabulary rather than the host's event enum: that
//! enum is a GraphQL union and a serialized client contract, and pulling it in
//! here would point the dependency back at `stump_core`. `stump_core` maps
//! every variant onto a `CoreEvent` one-to-one in `core/src/providers.rs`.

use crate::health::HealthStatus;

/// Something the provider host did that a client may need to know about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
	/// A catalog source's health status changed between two probe runs. Only
	/// a transition is reported: the first observation of a source writes its
	/// `source_health` row silently, so one cold run over a full catalog
	/// cannot announce a thousand sources at once.
	SourceHealthChanged {
		/// `source_health.source_id`, the Keiyoushi catalog source id.
		source_id: String,
		name: String,
		status: HealthStatus,
	},
	/// A remote series finished materialising into a library.
	SeriesMaterialized {
		series_id: String,
		/// The `provider_sources` instance id the series came from.
		source: String,
		library_id: String,
	},
	/// The catalog index was re-fetched from its upstream URL.
	CatalogRefreshed {
		/// Catalog sources in the new snapshot.
		count: u64,
	},
}

/// Where [`ProviderEvent`]s go. Absent in tests, which assert on rows instead.
pub trait ProviderEventSink: Send + Sync + 'static {
	fn emit(&self, event: ProviderEvent);
}
