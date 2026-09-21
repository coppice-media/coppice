use async_graphql::{Context, Object, Result};

use crate::{
	data::CoreContext, guard::ServerOwnerGuard,
	object::runtime_component::RuntimeComponent,
};

#[derive(Default)]
pub struct RuntimeComponentMutation;

#[Object]
impl RuntimeComponentMutation {
	/// Requests a component transition. HOT components change their effective
	/// route state immediately; RESTART components persist the desired value and
	/// report `restartRequired` until the next process start.
	#[graphql(guard = "ServerOwnerGuard")]
	async fn set_runtime_component_enabled(
		&self,
		ctx: &Context<'_>,
		key: String,
		enabled: bool,
	) -> Result<RuntimeComponent> {
		let core = ctx.data::<CoreContext>()?;
		Ok(core
			.component_runtime()
			.set_enabled(&key, enabled)
			.await?
			.into())
	}
}
