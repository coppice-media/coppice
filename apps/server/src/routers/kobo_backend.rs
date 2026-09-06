mod comic_transform;
mod kepub;
mod kepub_cache;
mod router;
mod tags;
use std::sync::Arc;

use axum::{
	body::Bytes,
	extract::{Path, State},
	http::{HeaderMap, Method, Request},
	middleware::{self, Next},
	response::{IntoResponse, Response},
	Extension, Router,
};
use models::entity::kobo_sync_session;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use stump_auth::AuthContext;
use stump_kobo::{KoboBackend, ProviderHost};

use crate::{
	config::state::AppState,
	errors::APIResult,
	middleware::{
		auth::{api_key_middleware, auth_middleware},
		host::{HostDetails, HostExtractor},
	},
};

#[derive(Clone)]
pub(crate) struct KoboBackendImpl(pub(crate) AppState);

fn host_details(host: ProviderHost) -> HostExtractor {
	let (scheme, host) = host
		.url()
		.split_once("://")
		.map(|(scheme, host)| (scheme.to_string(), host.to_string()))
		.unwrap_or_else(|| ("http".to_string(), host.url().to_string()));
	HostExtractor(HostDetails { host, scheme })
}

async fn inject_host(
	State(_ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	mut request: Request<axum::body::Body>,
	next: Next,
) -> Response {
	request
		.extensions_mut()
		.insert(ProviderHost::new(host.url()));
	next.run(request).await
}

async fn delete_sync_sessions(
	Extension(req): Extension<AuthContext>,
	State(ctx): State<AppState>,
) -> APIResult<Response> {
	let user = req.user();
	kobo_sync_session::Entity::delete_many()
		.filter(kobo_sync_session::Column::UserId.eq(user.id.clone()))
		.exec(ctx.conn.as_ref())
		.await?;
	Ok(axum::http::StatusCode::NO_CONTENT.into_response())
}

pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	kepub_cache::start(app_state.clone());
	let backend = Arc::new(KoboBackendImpl(app_state.clone()));
	let kobo = stump_kobo::kobo_router::<AppState, _>(backend.clone())
		.layer(middleware::from_fn(router::authorize))
		.layer(middleware::from_fn_with_state(
			app_state.clone(),
			api_key_middleware,
		));
	let sessions = stump_kobo::session_router::<AppState, _>(backend).layer(
		middleware::from_fn_with_state(app_state.clone(), auth_middleware),
	);

	Router::new()
		.merge(kobo)
		.merge(sessions)
		.layer(middleware::from_fn_with_state(app_state, inject_host))
}

#[async_trait::async_trait]
impl KoboBackend for KoboBackendImpl {
	type Error = crate::errors::APIError;
	async fn auth_device(&self, body: Bytes) -> Result<Response, Self::Error> {
		Ok(router::auth_device(body).await?.into_response())
	}

	async fn stubbed_route(
		&self,
		method: Method,
		path: String,
		headers: HeaderMap,
		body: Bytes,
	) -> Result<Response, Self::Error> {
		Ok(router::stubbed_route_empty_success(
			method,
			Path((String::new(), path)),
			headers,
			body,
		)
		.await?
		.into_response())
	}

	async fn initialization(
		&self,
		host: ProviderHost,
		api_key: String,
	) -> Result<Response, Self::Error> {
		Ok(router::initialization(
			host_details(host),
			Path(router::KoboAPIKey { api_key }),
		)
		.await?
		.into_response())
	}

	async fn library_sync(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		api_key: String,
		headers: HeaderMap,
	) -> Result<Response, Self::Error> {
		Ok(router::library_sync(
			State(self.0.clone()),
			Extension(auth),
			host_details(host),
			Path(router::KoboAPIKey { api_key }),
			headers,
		)
		.await?
		.into_response())
	}
	async fn book_state(
		&self,
		auth: AuthContext,
		book_id: String,
	) -> Result<Response, Self::Error> {
		Ok(router::book_state(
			State(self.0.clone()),
			Extension(auth),
			Path(router::KoboAPIKeyAndBookId {
				api_key: String::new(),
				book_id,
			}),
		)
		.await?
		.into_response())
	}

	async fn update_book_state(
		&self,
		auth: AuthContext,
		book_id: String,
		headers: HeaderMap,
		body: Bytes,
	) -> Result<Response, Self::Error> {
		Ok(router::update_book_state(
			State(self.0.clone()),
			Extension(auth),
			Path(router::KoboAPIKeyAndBookId {
				api_key: String::new(),
				book_id,
			}),
			headers,
			body,
		)
		.await?
		.into_response())
	}

	async fn book_metadata(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		api_key: String,
		book_id: String,
	) -> Result<Response, Self::Error> {
		Ok(router::book_metadata(
			State(self.0.clone()),
			Extension(auth),
			host_details(host),
			Path(router::KoboAPIKeyAndBookId { api_key, book_id }),
		)
		.await?
		.into_response())
	}

	async fn book_thumbnail(
		&self,
		auth: AuthContext,
		book_id: String,
		width: u32,
		height: u32,
	) -> Result<Response, Self::Error> {
		Ok(router::book_thumbnail(
			State(self.0.clone()),
			Extension(auth),
			Path(router::KoboThumbnail {
				api_key: String::new(),
				book_id,
				width,
				height,
				is_greyscale: None,
			}),
		)
		.await?
		.into_response())
	}

	async fn book_download(
		&self,
		auth: AuthContext,
		book_id: String,
		headers: HeaderMap,
	) -> Result<Response, Self::Error> {
		Ok(router::book_download(
			State(self.0.clone()),
			Extension(auth),
			Path(router::KoboAPIKeyAndBookId {
				api_key: String::new(),
				book_id,
			}),
			headers,
		)
		.await?
		.into_response())
	}
	async fn book_file(
		&self,
		auth: AuthContext,
		book_id: String,
		headers: HeaderMap,
	) -> Result<Response, Self::Error> {
		kepub::book_file(self.0.clone(), auth, book_id, headers).await
	}

	async fn delete_sync_sessions(
		&self,
		auth: AuthContext,
	) -> Result<Response, Self::Error> {
		delete_sync_sessions(Extension(auth), State(self.0.clone())).await
	}

	async fn create_tag(
		&self,
		auth: AuthContext,
		request: stump_kobo::routes::TagCreateRequest,
	) -> Result<String, Self::Error> {
		tags::create_tag(self.0.clone(), auth, request).await
	}

	async fn rename_tag(
		&self,
		auth: AuthContext,
		tag_id: String,
		name: String,
	) -> Result<(), Self::Error> {
		tags::rename_tag(self.0.clone(), auth, tag_id, name).await
	}

	async fn delete_tag(
		&self,
		auth: AuthContext,
		tag_id: String,
	) -> Result<(), Self::Error> {
		tags::delete_tag(self.0.clone(), auth, tag_id).await
	}

	async fn add_tag_items(
		&self,
		auth: AuthContext,
		tag_id: String,
		revision_ids: Vec<String>,
	) -> Result<(), Self::Error> {
		tags::add_tag_items(self.0.clone(), auth, tag_id, revision_ids).await
	}

	async fn remove_tag_items(
		&self,
		auth: AuthContext,
		tag_id: String,
		revision_ids: Vec<String>,
	) -> Result<(), Self::Error> {
		tags::remove_tag_items(self.0.clone(), auth, tag_id, revision_ids).await
	}
}
