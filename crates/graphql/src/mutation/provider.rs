//! GraphQL mutations for the provider host: virtual libraries, source
//! enablement, and materialised series management.

use crate::{
	data::CoreContext, guard::PermissionGuard, object::provider::ProviderSource,
};
use async_graphql::{Context, Object, Result};
use models::{
	entity::{library, series},
	shared::enums::UserPermission,
};

#[derive(Default)]
pub struct ProviderMutation;

#[Object]
impl ProviderMutation {
	/// Create (or fetch) the virtual library backing a provider source.
	/// Browsing it goes to the source live; nothing is materialised until a
	/// series' books are opened.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn create_virtual_library(
		&self,
		ctx: &Context<'_>,
		source_id: String,
		name: Option<String>,
	) -> Result<library::Model> {
		let core = ctx.data::<CoreContext>()?;
		let host = core.provider_host().ok_or("Provider host is not enabled")?;
		// The source must exist and be enabled so the library cannot point
		// at a dead instance.
		let source = host.source(&source_id).map_err(|error| error.to_string())?;
		let name = name.or_else(|| Some(source.info().name.clone()));
		let row =
			stump_provider::create_virtual_library(core.conn.as_ref(), &source_id, name)
				.await
				.map_err(|error| error.to_string())?;
		Ok(row)
	}

	/// Enable a catalog source by Keiyoushi id (`providerCatalog` result),
	/// or directly by implementation + language for embedded sources.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn enable_provider_source(
		&self,
		ctx: &Context<'_>,
		catalog_id: Option<String>,
		implementation: Option<String>,
		lang: Option<String>,
	) -> Result<ProviderSource> {
		let core = ctx.data::<CoreContext>()?;
		let host = core.provider_host().ok_or("Provider host is not enabled")?;
		let user = ctx.data::<stump_auth::AuthContext>()?;
		let created_by = Some(user.id());
		let row = match (catalog_id, implementation) {
			(Some(catalog_id), _) => host
				.enable_catalog_source(&catalog_id, created_by.as_deref())
				.await
				.map_err(|error| error.to_string())?,
			(None, Some(implementation)) => host
				.enable_implementation(
					&implementation,
					lang.as_deref().unwrap_or("all"),
					created_by.as_deref(),
				)
				.await
				.map_err(|error| error.to_string())?,
			_ => {
				return Err("Provide either catalogId or implementation".into());
			},
		};
		Ok(ProviderSource(row))
	}

	/// Disable a source instance. Materialised rows remain, but pages stop
	/// resolving and live browse is rejected.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn disable_provider_source(
		&self,
		ctx: &Context<'_>,
		id: String,
	) -> Result<bool> {
		let core = ctx.data::<CoreContext>()?;
		let host = core.provider_host().ok_or("Provider host is not enabled")?;
		Ok(host
			.disable_source(&id)
			.await
			.map_err(|error| error.to_string())?)
	}

	/// Fetch (or refresh) the Keiyoushi catalog snapshot.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn refresh_provider_catalog(&self, ctx: &Context<'_>) -> Result<bool> {
		let core = ctx.data::<CoreContext>()?;
		let host = core.provider_host().ok_or("Provider host is not enabled")?;
		host.refresh_catalog()
			.await
			.map_err(|error| error.to_string())?;
		Ok(true)
	}

	/// Add (materialise) a remote series into a virtual library under its
	/// deterministic id.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn add_provider_series(
		&self,
		ctx: &Context<'_>,
		library_id: String,
		source_id: String,
		remote_id: String,
	) -> Result<series::Model> {
		let core = ctx.data::<CoreContext>()?;
		let host = core.provider_host().ok_or("Provider host is not enabled")?;
		let materialized = host
			.materialise_series(&library_id, &source_id, &remote_id)
			.await
			.map_err(|error| error.to_string())?;
		Ok(materialized.series)
	}

	/// Re-fetch details and chapters for a materialised provider series.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn refresh_provider_series(
		&self,
		ctx: &Context<'_>,
		series_id: String,
	) -> Result<series::Model> {
		let core = ctx.data::<CoreContext>()?;
		let host = core.provider_host().ok_or("Provider host is not enabled")?;
		let materialized = stump_provider::refresh_series(&host, &series_id)
			.await
			.map_err(|error| error.to_string())?;
		Ok(materialized.series)
	}
}
