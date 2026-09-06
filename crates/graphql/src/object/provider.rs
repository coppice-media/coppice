//! GraphQL objects for the provider host (Mode A sources + Mode B virtual
//! libraries).

use async_graphql::Object;
use models::entity::{provider_source, source_health};
use stump_provider::RemoteSeries;

/// One source instance (`provider_sources` row).
pub struct ProviderSource(pub provider_source::Model);

#[Object]
impl ProviderSource {
	async fn id(&self) -> &str {
		&self.0.id
	}

	/// The compiled implementation backing this instance, e.g. `mangadex`.
	async fn implementation(&self) -> &str {
		&self.0.implementation
	}

	async fn catalog_id(&self) -> Option<&String> {
		self.0.catalog_id.as_ref()
	}

	async fn name(&self) -> &str {
		&self.0.name
	}

	async fn lang(&self) -> &str {
		&self.0.lang
	}

	async fn base_url(&self) -> &str {
		&self.0.base_url
	}

	async fn enabled(&self) -> bool {
		self.0.enabled
	}
}

/// A Keiyoushi catalog entry flattened to one source, annotated with
/// whether this server can actually run it.
#[derive(async_graphql::SimpleObject)]
pub struct ProviderCatalogEntry {
	/// The catalog source id to pass to `enableProviderSource`.
	pub id: String,
	pub name: String,
	pub lang: String,
	pub base_url: String,
	/// The Keiyoushi extension package backing the source.
	pub pkg: String,
	/// Whether `contentWarning` on the Keiyoushi extension is anything but
	/// `SAFE`, i.e. the flag a client badges an adult source with and the
	/// one the materialised series' age rating is derived from.
	pub nsfw: bool,
	/// Whether this server can actually run this entry — a compiled
	/// implementation *or* a source definition for its package; entries
	/// without either cannot be enabled.
	pub has_implementation: bool,
	/// The instance id that will be created on enable: `<implementation>-<lang>`
	/// for a compiled source, the definition id for a definition-backed one.
	pub instance_id: Option<String>,
	/// The latest health observation for this source, when it has been
	/// probed: the badge a client shows next to the entry. `dead` entries
	/// are only listed when `includeDead` was set.
	pub health: Option<ProviderSourceHealth>,
}

/// Latest health observation for one catalog source.
pub struct ProviderSourceHealth(pub source_health::Model);

#[Object]
impl ProviderSourceHealth {
	async fn source_id(&self) -> &str {
		&self.0.source_id
	}

	async fn name(&self) -> &str {
		&self.0.name
	}

	async fn base_url(&self) -> &str {
		&self.0.base_url
	}

	/// `UNKNOWN`, `OK`, `DEGRADED` or `DEAD`.
	async fn status(&self) -> &str {
		&self.0.status
	}

	async fn latency_ms(&self) -> Option<i32> {
		self.0.latency_ms
	}

	/// HTTP status of the last probe of the source's base URL.
	async fn http_status(&self) -> Option<i32> {
		self.0.http_status
	}

	/// Where the base URL redirected to, when it moved.
	async fn redirect_url(&self) -> Option<&String> {
		self.0.redirect_url.as_ref()
	}

	/// Whether the theme's latest-updates path answered.
	async fn latest_path_ok(&self) -> Option<bool> {
		self.0.latest_path_ok
	}

	async fn error(&self) -> Option<&String> {
		self.0.error.as_ref()
	}

	/// Failed runs in a row; reset by one reachable run.
	async fn consecutive_failures(&self) -> i32 {
		self.0.consecutive_failures
	}

	/// Whether the source reached `provider_health_dead_after` failures and
	/// is hidden from the catalog by default.
	async fn dead(&self) -> bool {
		stump_provider::HealthStatus::parse(&self.0.status)
			== stump_provider::HealthStatus::Dead
	}

	/// When the source was last probed.
	async fn last_checked_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
		self.0.checked_at.map(|ts| ts.into())
	}
}

/// One recorded cross-source duplicate: `series` is the same work as
/// `canonical`, which materialised first from another source.
#[derive(async_graphql::SimpleObject)]
pub struct ProviderSeriesDuplicate {
	pub series_id: String,
	pub series_name: String,
	/// The source instance `series` was materialised from.
	pub source_id: Option<String>,
	pub canonical_series_id: String,
	pub canonical_series_name: String,
	pub canonical_source_id: Option<String>,
	/// `EXTERNAL_KEY` when a cross-source id matched, `TITLE` when only the
	/// normalised titles did.
	pub reason: String,
	pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<stump_provider::SeriesDuplicate> for ProviderSeriesDuplicate {
	fn from(duplicate: stump_provider::SeriesDuplicate) -> Self {
		Self {
			series_id: duplicate.series.id,
			series_name: duplicate.series.name,
			source_id: duplicate.series.source_provider,
			canonical_series_id: duplicate.canonical.id,
			canonical_series_name: duplicate.canonical.name,
			canonical_source_id: duplicate.canonical.source_provider,
			reason: duplicate.link.reason,
			created_at: duplicate.link.created_at.into(),
		}
	}
}

/// A remote series surfaced by a live provider search, identified by its
/// deterministic Stump id.
#[derive(async_graphql::SimpleObject)]
pub struct ProviderSeriesSummary {
	/// The deterministic Stump series id (`uuid5(source, remote_id)`).
	pub stump_id: String,
	pub source_id: String,
	pub remote_id: String,
	pub title: String,
	pub cover_url: Option<String>,
	pub description: Option<String>,
	pub authors: Vec<String>,
	pub artists: Vec<String>,
	pub genres: Vec<String>,
	pub status: String,
	pub nsfw: bool,
}

/// One page of a live provider search.
#[derive(async_graphql::SimpleObject)]
pub struct ProviderSearchPage {
	pub items: Vec<ProviderSeriesSummary>,
	pub has_next: bool,
}

/// GraphQL-facing summary of a remote series for one source instance.
pub fn series_summary(source_id: &str, remote: &RemoteSeries) -> ProviderSeriesSummary {
	ProviderSeriesSummary {
		stump_id: stump_provider::virtual_path::series_id(source_id, &remote.remote_id),
		source_id: source_id.to_string(),
		remote_id: remote.remote_id.clone(),
		title: remote.title.clone(),
		cover_url: remote.thumbnail_url.clone(),
		description: remote.description.clone(),
		authors: remote.authors.clone(),
		artists: remote.artists.clone(),
		genres: remote.genres.clone(),
		status: remote.status.as_metadata_status().to_string(),
		nsfw: remote.nsfw,
	}
}
