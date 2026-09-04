use std::sync::Arc;

use crate::middleware::{auth::auth_middleware, host::HostExtractor};
use async_graphql::http::{Credentials, GraphiQLSource, ALL_WEBSOCKET_PROTOCOLS};
use async_graphql_axum::{
	GraphQLProtocol, GraphQLRequest, GraphQLResponse, GraphQLWebSocket,
};
use axum::{
	extract::{ws::WebSocketUpgrade, State},
	http::StatusCode,
	middleware,
	response::{Html, IntoResponse, Response},
	routing::{get, post},
	Extension, Router,
};

use graphql::schema::{build_schema, AppSchema};
use stump_api_types::RequestOrigin;
use stump_auth::AuthContext;
use tokio::sync::OnceCell;
use tower_sessions::Session;

use crate::{config::state::AppState, errors::APIError};

type SharedSchema = Arc<OnceCell<AppSchema>>;

pub(crate) async fn mount(app_state: AppState) -> Router<AppState> {
	let schema: SharedSchema = Arc::new(OnceCell::new());

	let mut method_router = post(graphql_handler);
	if cfg!(feature = "webui")
		&& app_state.config.enable_webui
		&& (app_state.config.enable_playground || cfg!(debug_assertions))
	{
		method_router = method_router.get(playground);
	}

	Router::new()
		.route("/", method_router)
		.route("/ws", get(graphql_subscription_handler))
		.layer(middleware::from_fn_with_state(app_state, auth_middleware))
		.layer(Extension(schema))
}

// TODO: Consider new user permission
async fn playground(
	Extension(req_ctx): Extension<AuthContext>,
) -> Result<impl IntoResponse, APIError> {
	if !req_ctx.user().is_server_owner {
		return Err(APIError::forbidden_discreet());
	}

	Ok(Html(
		GraphiQLSource::build()
			.endpoint("/api/graphql")
			.subscription_endpoint("/api/graphql/ws")
			.credentials(Credentials::Include)
			.finish(),
	))
}

fn graphql_status(response: &async_graphql::Response) -> StatusCode {
	if response.errors.iter().any(|error| {
		error
			.extensions
			.as_ref()
			.and_then(|extensions| extensions.get("code"))
			.is_some_and(|code| {
				matches!(code, async_graphql::Value::String(value) if value == "SERVICE_UNAVAILABLE")
			})
	}) {
		StatusCode::SERVICE_UNAVAILABLE
	} else {
		StatusCode::OK
	}
}

fn graphql_into_response(response: async_graphql::Response) -> Response {
	let status = graphql_status(&response);
	let mut response = GraphQLResponse::from(response).into_response();
	*response.status_mut() = status;
	response
}

async fn graphql_handler(
	schema: Extension<SharedSchema>,
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	HostExtractor(details): HostExtractor,
	session: Session,
	req: GraphQLRequest,
) -> Response {
	let schema = schema
		.get_or_init(|| async move { build_schema(ctx).await })
		.await;
	let mut req = req.into_inner();
	req = req.data(auth);
	req = req.data(RequestOrigin::new(details.host, details.scheme));
	req = req.data(session);
	graphql_into_response(schema.execute(req).await)
}

async fn graphql_subscription_handler(
	schema: Extension<SharedSchema>,
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	HostExtractor(details): HostExtractor,
	protocol: GraphQLProtocol,
	websocket: WebSocketUpgrade,
) -> impl IntoResponse {
	let schema_ctx = ctx.clone();
	let schema = schema
		.get_or_init(|| async move { build_schema(schema_ctx).await })
		.await
		.clone();
	let mut data = async_graphql::Data::default();
	data.insert(auth);
	data.insert(RequestOrigin::new(details.host, details.scheme));
	data.insert(ctx);

	websocket
		.protocols(ALL_WEBSOCKET_PROTOCOLS)
		.on_upgrade(move |stream| {
			GraphQLWebSocket::new(stream, schema, protocol)
				.with_data(data)
				.serve()
		})
}

#[cfg(test)]
mod tests {
	use super::*;

	fn response_with_error_code(code: Option<&str>) -> async_graphql::Response {
		let mut error = async_graphql::ServerError::new("resolver error", None);
		if let Some(code) = code {
			let mut extensions = async_graphql::ErrorExtensionValues::default();
			extensions.set("code", code);
			error.extensions = Some(extensions);
		}
		async_graphql::Response::from_errors(vec![error])
	}

	#[test]
	fn successful_graphql_response_uses_ok() {
		assert_eq!(
			graphql_status(&async_graphql::Response::new(async_graphql::Value::Null)),
			StatusCode::OK
		);
	}

	#[test]
	fn ordinary_graphql_error_uses_ok() {
		assert_eq!(
			graphql_status(&response_with_error_code(None)),
			StatusCode::OK
		);
	}

	#[test]
	fn service_unavailable_graphql_error_uses_503() {
		assert_eq!(
			graphql_status(&response_with_error_code(Some("SERVICE_UNAVAILABLE"))),
			StatusCode::SERVICE_UNAVAILABLE
		);
	}
}
