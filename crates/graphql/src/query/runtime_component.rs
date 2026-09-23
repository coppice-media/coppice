use async_graphql::{Context, Object, Result};
use models::shared::enums::{DeviceKind, DeviceProtocol};
use stump_core::component_runtime::ComponentRuntime;

use crate::{
	data::CoreContext,
	guard::ServerOwnerGuard,
	object::runtime_component::{DeviceCapability, RuntimeComponent, RuntimeMemory},
};

#[derive(Default)]
pub struct RuntimeComponentQuery;

#[Object]
impl RuntimeComponentQuery {
	/// Every code-inventoried component, including uncompiled integrations, with
	/// persisted desired/effective state and truthful restart information.
	#[graphql(guard = "ServerOwnerGuard")]
	async fn runtime_components(
		&self,
		ctx: &Context<'_>,
	) -> Result<Vec<RuntimeComponent>> {
		let core = ctx.data::<CoreContext>()?;
		Ok(core
			.component_runtime()
			.components()
			.await?
			.into_iter()
			.map(Into::into)
			.collect())
	}

	/// Process RSS and Linux PSS/private-dirty fields are independent,
	/// process-wide measurements. They are never attributed to a component.
	#[graphql(guard = "ServerOwnerGuard")]
	async fn runtime_memory(&self, ctx: &Context<'_>) -> Result<RuntimeMemory> {
		let core = ctx.data::<CoreContext>()?;
		Ok(core.process_memory().into())
	}

	/// Client/protocol registrations filtered from the same effective component
	/// state used by HTTP route guards. All rows are returned so an admin UI can
	/// explain an unavailable compiled integration rather than silently guessing.
	async fn device_capabilities(
		&self,
		ctx: &Context<'_>,
	) -> Result<Vec<DeviceCapability>> {
		let core = ctx.data::<CoreContext>()?;
		let runtime = core.component_runtime();
		Ok(capability_rows(&runtime))
	}
}

fn capability_rows(runtime: &ComponentRuntime) -> Vec<DeviceCapability> {
	[
		(DeviceKind::Kobo, DeviceProtocol::Kobo, "kobo"),
		(DeviceKind::Koreader, DeviceProtocol::Koreader, "koreader"),
		(DeviceKind::Coppice, DeviceProtocol::Koreader, "koreader"),
		(
			DeviceKind::Crosspoint,
			DeviceProtocol::Koreader,
			"crosspoint",
		),
		(DeviceKind::Mihon, DeviceProtocol::Komga, "komga"),
		(DeviceKind::Komelia, DeviceProtocol::Komga, "komga"),
		(DeviceKind::Liseur, DeviceProtocol::Liseur, "liseur-sync"),
		(DeviceKind::Opds, DeviceProtocol::Opds, "opds"),
		(DeviceKind::Abs, DeviceProtocol::Api, "abs"),
		(DeviceKind::Kavita, DeviceProtocol::Api, "kavita"),
		(DeviceKind::Api, DeviceProtocol::Api, "api"),
		(DeviceKind::Web, DeviceProtocol::Api, "webui"),
		(DeviceKind::Worker, DeviceProtocol::Api, "worker"),
		(DeviceKind::SourceWorker, DeviceProtocol::Api, "worker"),
	]
	.into_iter()
	.map(|(kind, protocol, key)| {
		let (compiled, enabled) = runtime
			.definition(key)
			.map(|definition| (definition.compiled, runtime.is_effective(key)))
			.unwrap_or((false, false));
		DeviceCapability::new(kind, protocol, key, compiled, enabled)
	})
	.collect()
}
