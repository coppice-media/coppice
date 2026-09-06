//! Kavita compatibility profile: the extracted `stump_kavita` router over a
//! server-side backend adapter, authenticated by Kavita's credential rules.
//!
//! Kavita authenticates every `/api/*` request with one of:
//! - `Authorization: Bearer <jwt>` where the JWT was minted by
//!   `POST /api/Plugin/authenticate` or `POST /api/Account/login`;
//! - the `x-api-key` header carrying the user's API key;
//! - an `apiKey` query parameter (image, PDF and, in Kavita, any route).
//!
//! Stump API keys (`stump_...`) stand in for Kavita auth keys, and a Stump
//! access token presented as a bearer is accepted too, so first-party clients
//! can drive the profile with their existing credentials.

use std::sync::Arc;

use axum::{
	extract::{Request, State},
	http::header,
	middleware::{self, Next},
	response::{IntoResponse, Response},
	Router,
};
use prefixed_api_key::PrefixedApiKey;
use stump_api_types::RequestOrigin;
use stump_auth::AuthContext;
use stump_devices::{CredentialRef, Protocol};
use stump_kavita::routes::KavitaBackend;

use crate::{
	config::{jwt::access_token_secret, state::AppState},
	errors::APIError,
	middleware::{
		auth::{bind_device, handle_bearer_auth, inject_avatar_url, validate_api_key},
		host::HostExtractor,
	},
};

mod backend;

pub(crate) const KAVITA_API_KEY_HEADER: &str = "x-api-key";

pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	let backend: Arc<dyn KavitaBackend> =
		Arc::new(backend::KavitaBackendAdapter::new(app_state.clone()));
	compose(app_state, backend)
}

fn compose(app_state: AppState, backend: Arc<dyn KavitaBackend>) -> Router<AppState> {
	let public = stump_kavita::routes::public_router::<AppState>(backend.clone());
	let protected = stump_kavita::routes::router::<AppState>(backend).layer(
		middleware::from_fn_with_state(app_state, kavita_auth_middleware),
	);
	public.merge(protected)
}

/// The `apiKey` query parameter, if present.
fn api_key_query(query: Option<&str>) -> Option<String> {
	query?
		.split('&')
		.filter_map(|pair| pair.split_once('='))
		.find(|(key, _)| key.eq_ignore_ascii_case("apiKey"))
		.map(|(_, value)| {
			urlencoding::decode(value)
				.map(|value| value.into_owned())
				.unwrap_or_else(|_| value.to_owned())
		})
		.filter(|value| !value.is_empty())
}

async fn authenticate_api_key(
	ctx: &AppState,
	api_key: &str,
) -> Result<AuthContext, Response> {
	let pak = PrefixedApiKey::from_string(api_key)
		.map_err(|_| APIError::Unauthorized.into_response())?;
	let user = validate_api_key(pak, ctx.conn.as_ref())
		.await
		.map_err(|error| error.into_response())?;
	let mut auth = AuthContext {
		user,
		api_key: Some(api_key.to_owned()),
		device_id: None,
	};
	bind_kavita_device(ctx, &mut auth).await?;
	Ok(auth)
}

/// Kavita's own token is a JWT this server minted, and it carries the API key
/// the client logged in with (`claims.api_key`), so a Kavita session stays
/// bound to the device that key belongs to for its whole lifetime.
async fn authenticate_bearer(
	ctx: &AppState,
	token: &str,
) -> Result<AuthContext, Response> {
	if stump_kavita::auth::looks_like_jwt(token) {
		if let Ok(secret) = access_token_secret(ctx.conn.as_ref()).await {
			if let Ok(claims) = stump_kavita::verify_token(secret.as_bytes(), token) {
				let user = backend::user_by_kavita_id(ctx.conn.as_ref(), &claims)
					.await
					.map_err(|error| error.into_response())?;
				let mut auth = AuthContext {
					user,
					api_key: claims.api_key.clone(),
					device_id: None,
				};
				bind_kavita_device(ctx, &mut auth).await?;
				return Ok(auth);
			}
		}
	}
	let mut auth = handle_bearer_auth(token.to_owned(), ctx.conn.as_ref())
		.await
		.map_err(|error| error.into_response())?;
	bind_kavita_device(ctx, &mut auth).await?;
	Ok(auth)
}

/// Narrows the request to the library scope of the device its API key belongs
/// to. Kavita is a compatibility surface rather than a protocol the device
/// registry mints credentials for, so a sighting is recorded under the
/// generic [`Protocol::Api`] bucket.
async fn bind_kavita_device(
	ctx: &AppState,
	auth: &mut AuthContext,
) -> Result<(), Response> {
	let Some(api_key) = auth.api_key.clone() else {
		return Ok(());
	};
	bind_device(ctx, auth, CredentialRef::ApiKey(&api_key), Protocol::Api)
		.await
		.map_err(|error| error.into_response())
}

/// Authenticate a Kavita request: `apiKey` query, then `x-api-key`, then
/// `Authorization: Bearer`. A present credential that fails is a `401`; it
/// never falls through to the next one, like Kavita's handlers.
async fn kavita_auth_middleware(
	State(ctx): State<AppState>,
	HostExtractor(host_details): HostExtractor,
	mut req: Request,
	next: Next,
) -> Result<Response, Response> {
	let service =
		RequestOrigin::new(host_details.host.clone(), host_details.scheme.clone());
	let headers = req.headers().clone();
	let query_key = api_key_query(req.uri().query());
	let header_key = headers
		.get(KAVITA_API_KEY_HEADER)
		.and_then(|value| value.to_str().ok())
		.map(str::trim)
		.filter(|value| !value.is_empty())
		.map(str::to_owned);
	let bearer = headers
		.get(header::AUTHORIZATION)
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.strip_prefix("Bearer "))
		.map(str::trim)
		.filter(|value| !value.is_empty())
		.map(str::to_owned);

	let mut auth = if let Some(key) = query_key {
		authenticate_api_key(&ctx, &key).await?
	} else if let Some(key) = header_key {
		authenticate_api_key(&ctx, &key).await?
	} else if let Some(token) = bearer {
		authenticate_bearer(&ctx, &token).await?
	} else {
		return Err(APIError::Unauthorized.into_response());
	};
	auth.user = inject_avatar_url(auth.user, service);
	req.extensions_mut().insert(auth);
	Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
	use super::*;
	use sea_orm::{DatabaseBackend, MockDatabase};

	/// Axum panics on overlapping routes when the router is built; catch it
	/// here rather than at server start.
	#[tokio::test]
	async fn kavita_router_composes_without_route_collisions() {
		let ctx = Arc::new(stump_core::Ctx::mock_sea(MockDatabase::new(
			DatabaseBackend::Sqlite,
		)));
		let backend: Arc<dyn KavitaBackend> =
			Arc::new(backend::KavitaBackendAdapter::new(ctx.clone()));
		let _router: Router<()> = compose(ctx.clone(), backend).with_state(ctx);
	}

	#[test]
	fn api_key_query_is_case_insensitive_and_decoded() {
		assert_eq!(
			api_key_query(Some("chapterId=1&apiKey=stump_a%2Bb")),
			Some("stump_a+b".to_owned())
		);
		assert_eq!(api_key_query(Some("apikey=x")), Some("x".to_owned()));
		assert_eq!(api_key_query(Some("apiKey=")), None);
		assert_eq!(api_key_query(None), None);
	}
}
