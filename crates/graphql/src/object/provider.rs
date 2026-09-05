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
	/// Whether a compiled implementation exists for this entry on this
	/// server; entries without one cannot be enabled.
	pub has_implementation: bool,
	/// The instance id that will be created on enable
	/// (`<implementation>-<lang>`).
	pub instance_id: Option<String>,
}

/// Latest health observation for one catalog source.
pub struct ProviderSourceHealth(pub source_health::Model);

#[Object]
impl ProviderSourceHealth {
	async fn source_id(&self) -> &str {
		&self.0.source_id
	}

	async fn status(&self) -> &str {
		&self.0.status
	}

	async fn latency_ms(&self) -> Option<i32> {
		self.0.latency_ms
	}

	async fn checked_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
		self.0.checked_at.map(|ts| ts.into())
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

impl From<&RemoteSeries> for ProviderSeriesSummary {
	fn from(remote: &RemoteSeries) -> Self {
		// The source id is filled in by the caller (it is not part of the
		// remote description); it is patched in `from_parts` below.
		let stump_id = stump_provider::virtual_path::series_id("", &remote.remote_id);
		Self {
			stump_id,
			source_id: String::new(),
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
}

/// GraphQL-facing summary of a remote series for one source instance.
pub fn series_summary(source_id: &str, remote: &RemoteSeries) -> ProviderSeriesSummary {
	let mut summary = ProviderSeriesSummary::from(remote);
	summary.source_id = source_id.to_string();
	summary.stump_id =
		stump_provider::virtual_path::series_id(source_id, &remote.remote_id);
	summary
}
