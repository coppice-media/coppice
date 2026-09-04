#[cfg(feature = "graphql")]
mod graphql;
pub(crate) mod v2;

use axum::Router;

use crate::config::state::AppState;

pub(crate) async fn mount(app_state: AppState) -> Router<AppState> {
	let mut api_router = Router::new();

	#[cfg(feature = "graphql")]
	{
		api_router = api_router.nest("/graphql", graphql::mount(app_state.clone()).await);
	}

	api_router = api_router.nest("/v2", v2::mount(app_state));

	Router::new().nest("/api", api_router)
}
