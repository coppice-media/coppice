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
use stump_kavita::{errors::APIError as KavitaError, routes::KavitaBackend};

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

/// Kavita's own token is a JWT this server minted. One minted from an API key
/// (`claims.api_key`) is exactly as good as that key and no better: the key
/// is validated again on every request — revoking or deleting the device, or
/// the key expiring, ends the session at once rather than at the token's
/// three-day `exp` — its narrowed permissions apply, and the request is bound
/// to the key's device. A password-login token acts as the user.
async fn authenticate_bearer(
	ctx: &AppState,
	token: &str,
) -> Result<AuthContext, Response> {
	if stump_kavita::auth::looks_like_jwt(token) {
		if let Ok(secret) = access_token_secret(ctx.conn.as_ref()).await {
			if let Ok(claims) = stump_kavita::verify_token(secret.as_bytes(), token) {
				if let Some(api_key) = claims.api_key.as_deref() {
					let auth = authenticate_api_key(ctx, api_key).await?;
					if auth.user.is_locked {
						return Err(KavitaError::Forbidden(
							"Your account has been locked by an administrator".to_owned(),
						)
						.into_response());
					}
					return Ok(auth);
				}
				let user = backend::user_by_kavita_id(ctx.conn.as_ref(), &claims)
					.await
					.map_err(|error| error.into_response())?;
				return Ok(AuthContext {
					user,
					api_key: None,
					device_id: None,
				});
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

	/// A Kavita token minted from a device key is only as good as the key:
	/// it carries the key's narrowed permissions and device binding, and
	/// revoking the device ends the session before the token's `exp`.
	#[tokio::test]
	async fn kavita_token_minted_from_a_device_key_dies_with_the_key() {
		use models::entity::{server_config, user::AuthUser};
		use models::shared::enums::UserPermission;
		use sea_orm::{ActiveModelTrait, ActiveValue::Set, ConnectionTrait, Statement};

		let database = ::tests::db::test_database().await;
		database
			.execute(Statement::from_string(
				DatabaseBackend::Sqlite,
				stump_kavita::ids::CREATE_KAVITA_IDS_SQL,
			))
			.await
			.unwrap();
		server_config::ActiveModel {
			initial_wal_setup_complete: Set(false),
			jwt_access_secret: Set(Some("kavita-device-token-secret".to_owned())),
			jwt_refresh_secret: Set(Some("kavita-device-refresh-secret".to_owned())),
			..Default::default()
		}
		.insert(&database)
		.await
		.unwrap();
		let user_row = ::tests::fake_data::User::new("kavita-owner")
			.insert(&database)
			.await;
		let owner = AuthUser {
			id: user_row.id.clone(),
			username: user_row.username.clone(),
			is_server_owner: true,
			..Default::default()
		};
		let ctx = stump_core::Ctx::for_testing(database).arced();
		let (device, credential) = ctx
			.devices()
			.create_device(
				&owner,
				stump_devices::CredentialIssuance::InteractiveSession,
				stump_devices::DeviceKind::Kavita,
				None,
			)
			.await
			.expect("Kavita device credential");
		let kavita_user_id = stump_kavita::KavitaIds::resolve(
			ctx.conn.as_ref(),
			stump_kavita::IdKind::User,
			&owner.id,
		)
		.await
		.unwrap();
		// The secret is process-wide (`OnceLock`); mint with whatever the
		// server will verify against.
		let secret = access_token_secret(ctx.conn.as_ref()).await.unwrap();
		let token = stump_kavita::mint_token(
			secret.as_bytes(),
			&owner.username,
			kavita_user_id,
			&["Login".to_owned()],
			Some(&credential.secret),
		)
		.unwrap();

		let auth = authenticate_bearer(&ctx, &token)
			.await
			.ok()
			.expect("a live device key authenticates its token");
		assert_eq!(auth.device_id.as_deref(), Some(device.id.as_str()));
		assert_eq!(auth.api_key.as_deref(), Some(credential.secret.as_str()));
		assert!(
			!auth.user.is_server_owner,
			"the token acts as the key, not as the owner behind it"
		);
		assert_eq!(auth.user.permissions, vec![UserPermission::DownloadFile]);

		ctx.devices()
			.revoke(&owner, &device.id)
			.await
			.expect("device revocation");
		let response = authenticate_bearer(&ctx, &token)
			.await
			.err()
			.expect("a revoked device key must not authenticate its token");
		assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
	}

	/// A password-login token carries no key, so it acts as the user — and
	/// the user is reloaded on every request: locking the account answers
	/// `403` for the rest of the token's three-day life.
	#[tokio::test]
	async fn kavita_password_login_token_stops_at_an_account_lock() {
		use models::entity::{server_config, user};
		use sea_orm::{ActiveModelTrait, ActiveValue::Set, ConnectionTrait, Statement};

		let database = ::tests::db::test_database().await;
		database
			.execute(Statement::from_string(
				DatabaseBackend::Sqlite,
				stump_kavita::ids::CREATE_KAVITA_IDS_SQL,
			))
			.await
			.unwrap();
		server_config::ActiveModel {
			initial_wal_setup_complete: Set(false),
			jwt_access_secret: Set(Some("kavita-password-token-secret".to_owned())),
			jwt_refresh_secret: Set(Some("kavita-password-refresh-secret".to_owned())),
			..Default::default()
		}
		.insert(&database)
		.await
		.unwrap();
		let user_row = ::tests::fake_data::User::new("kavita-password")
			.insert(&database)
			.await;
		let ctx = stump_core::Ctx::for_testing(database).arced();
		let kavita_user_id = stump_kavita::KavitaIds::resolve(
			ctx.conn.as_ref(),
			stump_kavita::IdKind::User,
			&user_row.id,
		)
		.await
		.unwrap();
		let secret = access_token_secret(ctx.conn.as_ref()).await.unwrap();
		let token = stump_kavita::mint_token(
			secret.as_bytes(),
			&user_row.username,
			kavita_user_id,
			&["Admin".to_owned(), "Login".to_owned()],
			None,
		)
		.unwrap();

		let auth = authenticate_bearer(&ctx, &token)
			.await
			.ok()
			.expect("a password-login token authenticates as the user");
		assert_eq!(auth.user.id, user_row.id);
		assert!(auth.api_key.is_none());
		assert!(auth.device_id.is_none());

		user::ActiveModel {
			id: Set(user_row.id.clone()),
			is_locked: Set(true),
			..Default::default()
		}
		.update(ctx.conn.as_ref())
		.await
		.unwrap();
		let response = authenticate_bearer(&ctx, &token)
			.await
			.err()
			.expect("a locked account must not authenticate its token");
		assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
	}
}
