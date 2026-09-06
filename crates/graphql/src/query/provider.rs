//! GraphQL queries for the provider host: source instances, the Keiyoushi
//! catalog, source health, and live provider search.

use crate::{
	data::CoreContext,
	guard::PermissionGuard,
	object::provider::{
		series_summary, ProviderCatalogEntry, ProviderSearchPage, ProviderSource,
		ProviderSourceHealth,
	},
};
use async_graphql::{Context, Object, Result};
use models::{
	entity::{provider_source, source_health},
	shared::enums::UserPermission,
};
use sea_orm::EntityTrait;
use stump_provider::Source;

#[derive(Default)]
pub struct ProviderQuery;

#[Object]
impl ProviderQuery {
	/// All registered provider source instances.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn provider_sources(&self, ctx: &Context<'_>) -> Result<Vec<ProviderSource>> {
		let core = ctx.data::<CoreContext>()?;
		let rows = provider_source::Entity::find()
			.all(core.conn.as_ref())
			.await?;
		Ok(rows.into_iter().map(ProviderSource).collect())
	}

	/// The Keiyoushi extension catalog, filtered by language and name.
	/// `has_implementation` marks entries this server can actually run.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn provider_catalog(
		&self,
		ctx: &Context<'_>,
		lang: Option<String>,
		query: Option<String>,
	) -> Result<Vec<ProviderCatalogEntry>> {
		let core = ctx.data::<CoreContext>()?;
		let host = core.provider_host().ok_or("Provider host is not enabled")?;
		let snapshot = host
			.catalog_snapshot()
			.await
			.map_err(|error| error.to_string())?;

		let query = query
			.as_deref()
			.map(str::trim)
			.filter(|query| !query.is_empty())
			.map(str::to_ascii_lowercase);

		let mut entries = Vec::new();
		for entry in &snapshot.entries {
			for source in &entry.sources {
				if let Some(lang) = lang.as_deref() {
					if !lang.is_empty() && !source.lang.eq_ignore_ascii_case(lang) {
						continue;
					}
				}
				if let Some(query) = query.as_deref() {
					if !source.name.to_ascii_lowercase().contains(query) {
						continue;
					}
				}
				let factory = host.factory_for_pkg(&entry.pkg);
				entries.push(ProviderCatalogEntry {
					id: source.id.clone(),
					name: source.name.clone(),
					lang: source.lang.clone(),
					base_url: source.base_url.clone(),
					pkg: entry.pkg.clone(),
					has_implementation: factory.is_some(),
					instance_id: factory.map(|factory| factory.instance_id(&source.lang)),
				});
			}
		}
		Ok(entries)
	}

	/// The latest health observations for catalog sources.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn provider_source_health(
		&self,
		ctx: &Context<'_>,
	) -> Result<Vec<ProviderSourceHealth>> {
		let core = ctx.data::<CoreContext>()?;
		let rows = source_health::Entity::find()
			.all(core.conn.as_ref())
			.await?;
		Ok(rows.into_iter().map(ProviderSourceHealth).collect())
	}

	/// Live search on one provider source, without materialising anything.
	/// Results carry deterministic Stump ids and can be added to a virtual
	/// library with `addProviderSeries`.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn provider_search(
		&self,
		ctx: &Context<'_>,
		source_id: String,
		query: String,
		page: Option<i32>,
	) -> Result<ProviderSearchPage> {
		let core = ctx.data::<CoreContext>()?;
		let host = core.provider_host().ok_or("Provider host is not enabled")?;
		let source = host.source(&source_id).map_err(|error| error.to_string())?;
		// Search goes straight to the source: results are not cached under
		// a virtual library (there is none to attribute them to).
		let result = source
			.search(&query, &[], page.unwrap_or(0).max(0) as u32)
			.await
			.map_err(|error| error.to_string())?;
		Ok(ProviderSearchPage {
			items: result
				.items
				.iter()
				.map(|remote| series_summary(&source_id, remote))
				.collect(),
			has_next: result.has_next,
		})
	}
}
