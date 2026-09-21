#[cfg(feature = "crosspoint")]
mod crosspoint;
mod sync;

use axum::{
	extract::{Json, Path, State},
	middleware,
	response::{IntoResponse, Response},
	Extension, Router,
};
use stump_auth::AuthContext;
use stump_core::component_runtime::COMPONENT_CROSSPOINT;
use stump_koreader::{KoreaderBackend, PutProgressInput as ProviderPutProgressInput};

use crate::{
	config::state::AppState,
	middleware::{auth::api_key_middleware, component::gate},
};
#[derive(Clone)]
pub(crate) struct KoreaderBackendImpl(pub(crate) AppState);

pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	let router =
		stump_koreader::router::<AppState, _>(KoreaderBackendImpl(app_state.clone()));
	#[cfg(feature = "crosspoint")]
	let router = router.merge(gate(
		crosspoint::router(),
		app_state.clone(),
		COMPONENT_CROSSPOINT,
	));
	router.layer(middleware::from_fn(sync::authorize)).layer(
		middleware::from_fn_with_state(app_state, api_key_middleware),
	)
}
#[async_trait::async_trait]

impl KoreaderBackend for KoreaderBackendImpl {
	type Error = crate::errors::APIError;

	async fn check_authorized(&self) -> Result<Response, Self::Error> {
		Ok(sync::check_authorized().await?.into_response())
	}

	async fn get_progress(
		&self,
		auth: AuthContext,
		document: String,
	) -> Result<Response, Self::Error> {
		Ok(sync::get_progress(
			State(self.0.clone()),
			Extension(auth),
			Path(sync::KOReaderURLParams {
				params: sync::KOReaderDocumentURLParams { document },
				api_key: None,
			}),
		)
		.await?
		.into_response())
	}

	async fn put_progress(
		&self,
		auth: AuthContext,
		input: ProviderPutProgressInput,
	) -> Result<Response, Self::Error> {
		Ok(sync::put_progress(
			State(self.0.clone()),
			Extension(auth),
			Json(sync::PutProgressInput {
				document: input.document,
				progress: input.progress,
				percentage: input.percentage,
				device: input.device,
				device_id: input.device_id,
			}),
		)
		.await?
		.into_response())
	}
}

// The provider router leaves authentication and permissions to the server middleware above.
