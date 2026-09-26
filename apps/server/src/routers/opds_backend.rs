mod v1_2;
mod v2_0;

use std::sync::Arc;

use axum::{
	extract::{Path, Query, State},
	http::{HeaderMap, Request},
	middleware::{self, Next},
	response::{IntoResponse, Response},
	Extension, Json, Router,
};
use stump_auth::AuthContext;
use stump_core::opds::v2_0::progression::OPDSProgressionInput;
use stump_opds::{BrowseParams, OpdsBackend, ProviderHost};

use crate::{
	config::state::AppState,
	middleware::{
		auth::{api_key_middleware, auth_middleware},
		host::{HostDetails, HostExtractor},
	},
};

#[derive(Clone)]
pub(crate) struct OpdsBackendImpl(pub(crate) AppState);

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

pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	let backend = Arc::new(OpdsBackendImpl(app_state.clone()));
	let v1 = stump_opds::v1_router::<AppState, _>(backend.clone());
	let v2 = stump_opds::v2_router::<AppState, _>(backend);

	Router::new()
		.nest(
			"/opds/v1.2",
			v1.clone().layer(middleware::from_fn_with_state(
				app_state.clone(),
				auth_middleware,
			)),
		)
		.nest(
			"/opds/{api_key}/v1.2",
			v1.layer(middleware::from_fn_with_state(
				app_state.clone(),
				api_key_middleware,
			)),
		)
		.nest(
			"/opds/v2.0",
			v2.layer(middleware::from_fn_with_state(
				app_state.clone(),
				auth_middleware,
			)),
		)
		.layer(middleware::from_fn_with_state(app_state, inject_host))
}
#[async_trait::async_trait]
impl OpdsBackend for OpdsBackendImpl {
	type Error = crate::errors::APIError;

	async fn v1_catalog(&self, auth: AuthContext) -> Result<Response, Self::Error> {
		Ok(v1_2::catalog(Extension(auth)).await?.into_response())
	}

	async fn v1_search_description(
		&self,
		auth: AuthContext,
	) -> Result<Response, Self::Error> {
		Ok(v1_2::search_description(Extension(auth))
			.await?
			.into_response())
	}

	async fn v1_search_feed(
		&self,
		auth: AuthContext,
		search: Option<String>,
	) -> Result<Response, Self::Error> {
		Ok(v1_2::search_feed(
			State(self.0.clone()),
			Query(v1_2::OPDSSearchQuery { search }),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v1_keep_reading(&self, auth: AuthContext) -> Result<Response, Self::Error> {
		Ok(v1_2::keep_reading(State(self.0.clone()), Extension(auth))
			.await?
			.into_response())
	}

	async fn v1_get_libraries(
		&self,
		auth: AuthContext,
		search: Option<String>,
	) -> Result<Response, Self::Error> {
		Ok(v1_2::get_libraries(
			State(self.0.clone()),
			Query(v1_2::OPDSSearchQuery { search }),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v1_get_library_by_id(
		&self,
		auth: AuthContext,
		id: String,
		pagination: stump_api_types::OffsetPagination,
		search: Option<String>,
	) -> Result<Response, Self::Error> {
		Ok(v1_2::get_library_by_id(
			State(self.0.clone()),
			Path(v1_2::OPDSURLParams {
				params: v1_2::OPDSIDURLParams { id },
				api_key: auth.api_key(),
			}),
			Query(pagination),
			Query(v1_2::OPDSSearchQuery { search }),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v1_get_series(
		&self,
		auth: AuthContext,
		search: Option<String>,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v1_2::get_series(
			State(self.0.clone()),
			Query(pagination),
			Query(v1_2::OPDSSearchQuery { search }),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v1_get_latest_series(
		&self,
		auth: AuthContext,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v1_2::get_latest_series(
			State(self.0.clone()),
			Query(pagination),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v1_get_series_by_id(
		&self,
		auth: AuthContext,
		id: String,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v1_2::get_series_by_id(
			Path(v1_2::OPDSURLParams {
				params: v1_2::OPDSIDURLParams { id },
				api_key: auth.api_key(),
			}),
			State(self.0.clone()),
			Query(pagination),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v1_get_books(
		&self,
		auth: AuthContext,
		search: Option<String>,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v1_2::get_books(
			State(self.0.clone()),
			Query(pagination),
			Query(v1_2::OPDSSearchQuery { search }),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v1_get_latest_books(
		&self,
		auth: AuthContext,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v1_2::get_latest_books(
			State(self.0.clone()),
			Query(pagination),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v1_get_book_thumbnail(
		&self,
		auth: AuthContext,
		id: String,
	) -> Result<Response, Self::Error> {
		Ok(v1_2::get_book_thumbnail(
			Path(v1_2::OPDSURLParams {
				params: v1_2::OPDSIDURLParams { id },
				api_key: auth.api_key(),
			}),
			State(self.0.clone()),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v1_get_book_page(
		&self,
		auth: AuthContext,
		id: String,
		page: i32,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v1_2::get_book_page(
			Path(v1_2::OPDSURLParams {
				params: v1_2::OPDSPageURLParams { id, page },
				api_key: auth.api_key(),
			}),
			State(self.0.clone()),
			Query(pagination),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v1_download_book(
		&self,
		auth: AuthContext,
		id: String,
		headers: HeaderMap,
	) -> Result<Response, Self::Error> {
		Ok(v1_2::download_book(
			Path(v1_2::OPDSURLParams {
				params: v1_2::OPDSFilenameURLParams {
					id,
					filename: String::new(),
				},
				api_key: auth.api_key(),
			}),
			State(self.0.clone()),
			Extension(auth),
			headers,
		)
		.await?
		.into_response())
	}

	async fn v2_auth(&self, _host: ProviderHost) -> Result<Response, Self::Error> {
		Ok(v2_0::auth().await?.into_response())
	}

	async fn v2_catalog(
		&self,
		auth: AuthContext,
		host: ProviderHost,
	) -> Result<Response, Self::Error> {
		Ok(
			v2_0::catalog(State(self.0.clone()), host_details(host), Extension(auth))
				.await?
				.into_response(),
		)
	}

	async fn v2_search(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		query: Option<String>,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v2_0::search(
			State(self.0.clone()),
			host_details(host),
			Query(v2_0::OPDSSearchQuery { query }),
			Query(pagination),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v2_search_libraries(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		query: Option<String>,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v2_0::search_libraries(
			State(self.0.clone()),
			host_details(host),
			Query(v2_0::OPDSSearchQuery { query }),
			Query(pagination),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v2_search_series(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		query: Option<String>,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v2_0::search_series(
			State(self.0.clone()),
			host_details(host),
			Query(v2_0::OPDSSearchQuery { query }),
			Query(pagination),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v2_search_books(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		query: Option<String>,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v2_0::search_books(
			State(self.0.clone()),
			host_details(host),
			Query(v2_0::OPDSSearchQuery { query }),
			Query(pagination),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v2_browse_libraries(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v2_0::browse_libraries(
			State(self.0.clone()),
			host_details(host),
			Query(pagination),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v2_browse_library_by_id(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		id: String,
	) -> Result<Response, Self::Error> {
		Ok(v2_0::browse_library_by_id(
			State(self.0.clone()),
			host_details(host),
			Path(id),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v2_browse_library_books(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		id: String,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v2_0::browse_library_books(
			State(self.0.clone()),
			host_details(host),
			Path(id),
			Query(pagination),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v2_latest_library_books(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		id: String,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v2_0::latest_library_books(
			State(self.0.clone()),
			host_details(host),
			Path(id),
			Query(pagination),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v2_browse_series(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v2_0::browse_series(
			State(self.0.clone()),
			host_details(host),
			Query(pagination),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v2_browse_series_by_id(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		id: String,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v2_0::browse_series_by_id(
			State(self.0.clone()),
			host_details(host),
			Query(pagination),
			Path(id),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v2_browse_books(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		params: BrowseParams,
	) -> Result<Response, Self::Error> {
		let pagination = params.pagination();
		let BrowseParams {
			author,
			penciler,
			colorist,
			inker,
			letterer,
			editor,
			cover_artist,
			subject,
			characters,
			teams,
			..
		} = params;
		Ok(v2_0::browse_books(
			State(self.0.clone()),
			host_details(host),
			Query(v2_0::OPDSBrowseParams {
				pagination,
				filter: v2_0::OPDSBrowseFilter {
					author,
					penciler,
					colorist,
					inker,
					letterer,
					editor,
					cover_artist,
					subject,
					characters,
					teams,
				},
			}),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v2_latest_books(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v2_0::latest_books(
			State(self.0.clone()),
			host_details(host),
			Query(pagination),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v2_keep_reading(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		pagination: stump_api_types::OffsetPagination,
	) -> Result<Response, Self::Error> {
		Ok(v2_0::keep_reading(
			State(self.0.clone()),
			host_details(host),
			Query(pagination),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v2_get_book_by_id(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		id: String,
	) -> Result<Response, Self::Error> {
		Ok(v2_0::get_book_by_id(
			Path(id),
			host_details(host),
			State(self.0.clone()),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v2_get_book_thumbnail(
		&self,
		auth: AuthContext,
		id: String,
	) -> Result<Response, Self::Error> {
		Ok(
			v2_0::get_book_thumbnail(Path(id), State(self.0.clone()), Extension(auth))
				.await?
				.into_response(),
		)
	}

	async fn v2_get_book_page(
		&self,
		auth: AuthContext,
		id: String,
		page: i32,
	) -> Result<Response, Self::Error> {
		Ok(
			v2_0::get_book_page(Path((id, page)), State(self.0.clone()), Extension(auth))
				.await?
				.into_response(),
		)
	}

	async fn v2_get_book_progression(
		&self,
		auth: AuthContext,
		host: ProviderHost,
		id: String,
	) -> Result<Response, Self::Error> {
		Ok(v2_0::get_book_progression(
			Path(id),
			State(self.0.clone()),
			host_details(host),
			Extension(auth),
		)
		.await?
		.into_response())
	}

	async fn v2_update_book_progression(
		&self,
		auth: AuthContext,
		id: String,
		input: OPDSProgressionInput,
	) -> Result<Response, Self::Error> {
		Ok(v2_0::update_book_progression(
			Path(id),
			State(self.0.clone()),
			Extension(auth),
			Json(input),
		)
		.await?
		.into_response())
	}

	async fn v2_download_book(
		&self,
		auth: AuthContext,
		id: String,
		headers: HeaderMap,
	) -> Result<Response, Self::Error> {
		Ok(
			v2_0::download_book(
				Path(id),
				State(self.0.clone()),
				Extension(auth),
				headers,
			)
			.await?
			.into_response(),
		)
	}
}
