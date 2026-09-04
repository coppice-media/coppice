use std::sync::Arc;

use axum::{middleware, Router};
use stump_komga::routes::KomgaBackend;

use crate::{config::state::AppState, middleware::auth::auth_middleware};

mod identity;
mod settings;

/// Mount the extracted Komga compatibility router together with the server-local
/// identity and settings endpoints.
///
/// Two authenticated trees share one backend and event bus:
/// - the root tree serves Komga clients (Komelia, Mihon) with Stump's Komga
///   identity routes;
/// - `/komga` serves Liseur's Grimmory profile, where Grimmory's own listing and
///   identity shapes replace the two routes they overlap with.
pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	let backend: Arc<dyn KomgaBackend> = Arc::new(
		super::komga_backend::KomgaBackendAdapter::new(app_state.clone()),
	);
	compose(app_state, backend)
}

fn compose(app_state: AppState, backend: Arc<dyn KomgaBackend>) -> Router<AppState> {
	let provider = stump_komga::routes::router::<AppState>(backend.clone());
	let legacy_books =
		stump_komga::routes::legacy_books_router::<AppState>(backend.clone());

	let root = provider
		.clone()
		.merge(legacy_books)
		.merge(identity::routes())
		.merge(settings::routes());

	// Grimmory owns `GET /api/v2/users/me` under the alias; keep only the
	// identity routes that do not overlap with its shim.
	let grimmory = provider
		.merge(stump_komga::routes::grimmory_routes::<AppState>(backend))
		.merge(identity::routes_without_current_user())
		.merge(settings::routes());

	let auth = |router: Router<AppState>| {
		router.layer(middleware::from_fn_with_state(
			app_state.clone(),
			auth_middleware,
		))
	};

	identity::public_routes()
		.merge(auth(root))
		.nest("/komga", auth(grimmory))
}

#[cfg(test)]
mod tests {
	use super::*;
	use sea_orm::{DatabaseBackend, MockDatabase};

	/// Axum panics on overlapping routes when the router is *built*, which
	/// would otherwise surface only at server start (as it once did when the
	/// Grimmory identity route collided with Stump's at the root).
	#[tokio::test]
	async fn komga_router_composes_without_route_collisions() {
		let ctx = Arc::new(stump_core::Ctx::mock_sea(MockDatabase::new(
			DatabaseBackend::Sqlite,
		)));
		let backend: Arc<dyn KomgaBackend> = Arc::new(
			super::super::komga_backend::KomgaBackendAdapter::new(ctx.clone()),
		);
		let _router: Router<()> = compose(ctx.clone(), backend).with_state(ctx);
	}
}
