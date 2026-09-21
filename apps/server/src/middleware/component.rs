use axum::{
	body::Body,
	http::{Request, StatusCode},
	middleware::{self, Next},
	response::IntoResponse,
	Router,
};
use serde_json::json;

use crate::config::state::AppState;

/// Applies an effective component gate to every route in `router`.
///
/// Routes are still compiled and mounted when their startup flag is false so a
/// HOT transition can make them available later. The guard is therefore the
/// single request-time source of truth and rejects a disabled integration with
/// a stable 503 rather than allowing a partially configured handler to run.
pub(crate) fn gate(
	router: Router<AppState>,
	state: AppState,
	component_key: &'static str,
) -> Router<AppState> {
	router.layer(middleware::from_fn(
		move |request: Request<Body>, next: Next| {
			let state = state.clone();
			async move {
				if state.component_enabled(component_key) {
					state.component_runtime().record_activity(component_key);
					next.run(request).await
				} else {
					(
						StatusCode::SERVICE_UNAVAILABLE,
						axum::Json(json!({
							"error": "component_disabled",
							"component": component_key,
						})),
					)
						.into_response()
				}
			}
		},
	))
}
