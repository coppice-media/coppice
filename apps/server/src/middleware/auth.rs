#[cfg(any(feature = "opds", feature = "kobo", feature = "koreader"))]
use std::collections::HashMap;

#[cfg(feature = "komga")]
use std::str::FromStr;

#[cfg(feature = "komga")]
use chrono::Utc;

#[cfg(feature = "komga")]
use models::entity::session as session_entity;

#[cfg(feature = "komga")]
use tower_sessions::session::Id;

#[cfg(any(feature = "opds", feature = "kobo", feature = "koreader"))]
use axum::extract::Path;
use axum::{
	body::Body,
	extract::{OriginalUri, Request, State},
	http::{header, StatusCode},
	middleware::Next,
	response::{IntoResponse, Redirect, Response},
	Json,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use models::{
	entity::{
		api_key::{self, APIKeyWithUser},
		user::{self, AuthUser},
	},
	shared::{
		api_key::{APIKeyPermissions, API_KEY_PREFIX},
		enums::UserPermission,
		image::ImageRef,
	},
};
use prefixed_api_key::{PrefixedApiKey, PrefixedApiKeyController};
use reqwest::Method;
use sea_orm::{prelude::*, Condition, DatabaseConnection};
#[cfg(any(feature = "opds", feature = "kobo", feature = "koreader"))]
use serde::Deserialize;
use stump_api_types::RequestOrigin;
use stump_auth::AuthContext;
use stump_core::opds::v2_0::{
	authentication::{
		OPDSAuthenticationDocumentBuilder, OPDSSupportedAuthFlow,
		OPDS_AUTHENTICATION_DOCUMENT_REL, OPDS_AUTHENTICATION_DOCUMENT_TYPE,
	},
	link::OPDSLink,
};
use stump_devices::{CredentialRef, Protocol};
use tower_sessions::Session;

#[cfg(feature = "komga")]
use crate::config::session::SESSION_NAME;
use crate::{
	config::{
		jwt::extract_user_from_jwt,
		session::{delete_cookie_header, SESSION_USER_KEY},
		state::AppState,
	},
	errors::{api_error_message, APIError, APIResult},
	routers::enforce_max_sessions,
	utils::{
		current_utc_time, decode_base64_credentials, fetch_session_user, verify_password,
	},
};

use super::host::HostExtractor;

pub const STUMP_SAVE_BASIC_SESSION_HEADER: &str = "X-Stump-Save-Session";
pub const KOMGA_API_KEY_HEADER: &str = "X-API-Key";
#[cfg(feature = "komga")]
pub(crate) const KOMGA_REMEMBER_ME_COOKIE_NAME: &str = "komga-remember-me";

#[cfg(feature = "komga")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct KomgaBasicAuthSuccess;

pub(crate) fn inject_avatar_url(mut user: AuthUser, service: RequestOrigin) -> AuthUser {
	user.avatar = ImageRef {
		url: service.cache_friendly_url(
			format!("/api/v2/users/{}/avatar", user.id),
			&user.avatar.last_modified,
		),
		..user.avatar
	};
	user
}

#[cfg(feature = "komga")]
fn is_komga_basic_auth_path(path: &str) -> bool {
	let is_v1_path = path.starts_with("/api/v1/") || path.starts_with("/komga/api/v1/");
	let is_komf_path = path.starts_with("/api/komga/");
	#[cfg(feature = "komf")]
	let is_komf_global_path =
		path == "/api/config" || path == "/api/jobs" || path.starts_with("/api/jobs/");
	#[cfg(not(feature = "komf"))]
	let is_komf_global_path = false;
	let is_identity_path = matches!(
		path,
		"/api/logout"
			| "/api/v2/users"
			| "/api/v2/users/me"
			| "/api/v2/users/me/authentication-activity"
			| "/api/v2/users/authentication-activity"
			| "/api/v2/authors"
			| "/komga/api/v2/users/me"
	);
	let is_latest_activity_path = path.starts_with("/api/v2/users/")
		&& path.ends_with("/authentication-activity/latest");
	// Mihon's Komga tracker: `/api/v2/series/{id}/read-progress/tachiyomi`.
	let is_tracker_path = (path.starts_with("/api/v2/series/")
		|| path.starts_with("/komga/api/v2/series/"))
		&& path.ends_with("/read-progress/tachiyomi");

	is_v1_path
		|| is_komf_path
		|| is_komf_global_path
		|| is_identity_path
		|| is_latest_activity_path
		|| is_tracker_path
		|| path == "/sse/v1/events"
}

/// Matches only paths registered by `stump_komf::routes::router`; Kavita's
/// separate `apiKey` query lane must not authenticate on its broader `/api/*` surface.
#[cfg(feature = "komf")]
fn is_komf_auth_path(path: &str) -> bool {
	if matches!(
		path,
		"/api/config"
			| "/api/jobs"
			| "/api/jobs/all"
			| "/api/komga/metadata/providers"
			| "/api/komga/metadata/search"
			| "/api/komga/metadata/series-cover"
			| "/api/komga/metadata/identify"
			| "/api/komga/media-server/connected"
			| "/api/komga/media-server/libraries"
	) {
		return true;
	}

	let is_job_route = path.strip_prefix("/api/jobs/").is_some_and(|suffix| {
		let mut segments = suffix.split('/');
		if !segments.next().is_some_and(|segment| !segment.is_empty()) {
			return false;
		}
		match segments.next() {
			None => true,
			Some("events") => segments.next().is_none(),
			Some(_) => false,
		}
	});
	let is_metadata_library_route = |prefix: &str| {
		path.strip_prefix(prefix).is_some_and(|suffix| {
			let mut segments = suffix.split('/');
			if !segments.next().is_some_and(|segment| !segment.is_empty()) {
				return false;
			}
			match segments.next() {
				None => true,
				Some("series") => {
					segments.next().is_some_and(|segment| !segment.is_empty())
						&& segments.next().is_none()
				},
				Some(_) => false,
			}
		})
	};

	is_job_route
		|| is_metadata_library_route("/api/komga/metadata/match/library/")
		|| is_metadata_library_route("/api/komga/metadata/reset/library/")
}

#[cfg(feature = "komf")]
fn komf_api_key_query(query: Option<&str>) -> Option<String> {
	query?.split('&').find_map(|pair| {
		let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
		key.eq_ignore_ascii_case("apiKey").then(|| {
			urlencoding::decode(value)
				.map(|value| value.into_owned())
				.unwrap_or_else(|_| value.to_owned())
		})
	})
}

#[cfg(feature = "komf")]
fn require_komf_device(auth: &AuthContext) -> APIResult<()> {
	if auth.device_id.is_some() {
		Ok(())
	} else {
		Err(APIError::Unauthorized)
	}
}

#[cfg(feature = "komga")]
fn is_komga_logout_path(path: &str) -> bool {
	path == "/api/logout"
}

#[cfg(feature = "komga")]
pub(crate) fn komga_remember_me_cookie(headers: &header::HeaderMap) -> Option<&str> {
	for header_value in headers.get_all(header::COOKIE).iter() {
		let Ok(cookie_header) = header_value.to_str() else {
			continue;
		};

		for cookie in cookie_header.split(';') {
			let Some((name, value)) = cookie.trim().split_once('=') else {
				continue;
			};
			if name.trim() == KOMGA_REMEMBER_ME_COOKIE_NAME {
				return Some(value.trim());
			}
		}
	}

	None
}

#[cfg(feature = "komga")]
fn should_accept_komga_remember_me(path: &str, headers: &header::HeaderMap) -> bool {
	is_komga_basic_auth_path(path)
		&& !headers.contains_key(header::AUTHORIZATION)
		&& !headers.contains_key(KOMGA_API_KEY_HEADER)
		&& komga_remember_me_cookie(headers).is_some()
}

#[cfg(feature = "komga")]
fn parse_komga_remember_me_token(token: &str) -> APIResult<Id> {
	Id::from_str(token).map_err(|_| APIError::Unauthorized)
}

#[cfg(feature = "komga")]
async fn load_komga_remember_me_session(
	token: &str,
	conn: &DatabaseConnection,
) -> APIResult<Option<session_entity::Model>> {
	let session_id = parse_komga_remember_me_token(token)?;
	let now: DateTimeWithTimeZone = Utc::now().into();
	session_entity::Entity::find()
		.filter(
			session_entity::Column::SessionId
				.eq(session_id.to_string())
				.and(session_entity::Column::ExpiryTime.gt(now)),
		)
		.one(conn)
		.await
		.map_err(Into::into)
}

#[cfg(feature = "komga")]
async fn authenticate_komga_remember_me(
	token: &str,
	conn: &DatabaseConnection,
) -> APIResult<user::LoginUser> {
	let session = load_komga_remember_me_session(token, conn)
		.await?
		.ok_or(APIError::Unauthorized)?;

	let user = user::LoginUser::find_by_id(session.user_id)
		.filter(user::Column::DeletedAt.is_null())
		.into_model::<user::LoginUser>()
		.one(conn)
		.await?
		.ok_or(APIError::Unauthorized)?;

	if user.is_locked {
		return Err(APIError::Unauthorized);
	}

	Ok(user)
}

/// Komga-compatible clients (Komelia) run several HTTP clients (REST, SSE, image
/// loaders) against one shared cookie jar. Komga never clears the session cookie
/// on a 401, so a stale cookie on any one client is harmless there. Stump attaches
/// a `Max-Age=0` `Set-Cookie` to every 401 (both [`APIErrorResponse`](crate::errors::APIErrorResponse)
/// and tower-sessions), which lets one client's dead cookie wipe the live session
/// of the others. Strip those clears on Komga-scoped paths only; every other Stump
/// surface keeps the existing behavior.
#[cfg(feature = "komga")]
pub(crate) async fn strip_komga_unauthorized_cookie_clears(
	req: Request,
	next: Next,
) -> Response {
	let is_komga_path = is_komga_basic_auth_path(req.uri().path());
	let mut response = next.run(req).await;

	if is_komga_path && response.status() == StatusCode::UNAUTHORIZED {
		strip_session_cookie_clears(response.headers_mut());
	}

	response
}

/// Removes every `Set-Cookie` that expires the session cookie, keeping all
/// other cookies (including a fresh session issued on the same response).
#[cfg(feature = "komga")]
fn strip_session_cookie_clears(headers: &mut header::HeaderMap) {
	let kept: Vec<_> = headers
		.get_all(header::SET_COOKIE)
		.iter()
		.filter(|value| !value.to_str().is_ok_and(is_session_cookie_clear))
		.cloned()
		.collect();
	headers.remove(header::SET_COOKIE);
	for value in kept {
		headers.append(header::SET_COOKIE, value);
	}
}

#[cfg(feature = "komga")]
fn is_session_cookie_clear(cookie: &str) -> bool {
	let Some(rest) = cookie.strip_prefix(SESSION_NAME) else {
		return false;
	};
	rest.starts_with('=')
		&& rest.split(';').skip(1).any(|attribute| {
			attribute
				.trim()
				.split_once('=')
				.is_some_and(|(name, value)| {
					name.eq_ignore_ascii_case("max-age") && value.trim() == "0"
				})
		})
}

#[cfg(feature = "komga")]
async fn authenticate_komga_api_key(
	api_key_header: Option<&str>,
	conn: &DatabaseConnection,
) -> APIResult<AuthContext> {
	let Some(api_key_header) = api_key_header else {
		return Err(APIError::Unauthorized);
	};
	let pak = PrefixedApiKey::from_string(api_key_header)
		.map_err(|_| APIError::Unauthorized)?;
	if pak.prefix() != API_KEY_PREFIX {
		return Err(APIError::Unauthorized);
	}
	let user = validate_api_key(pak, conn).await?;
	Ok(AuthContext {
		user,
		api_key: Some(api_key_header.to_owned()),
		// The caller binds the device: see `bind_device`.
		device_id: None,
	})
}

/// Basic credentials are accepted on OPDS paths unconditionally (OPDS readers
/// send them on every request and never opt into sessions) and on the Komga
/// compatibility paths. Everywhere else Basic is rejected.
fn basic_auth_accepted(is_opds: bool, is_komga_basic_auth: bool) -> bool {
	is_opds || is_komga_basic_auth
}

/// Whether a successful Basic login should also persist a session: OPDS clients
/// opt in with `X-Stump-Save-Session: true`; Komga clients always get one,
/// except on the logout route.
fn basic_auth_saves_session(
	is_opds: bool,
	save_basic_session: bool,
	is_komga_basic_auth: bool,
	is_komga_logout: bool,
) -> bool {
	(is_opds && save_basic_session) || (is_komga_basic_auth && !is_komga_logout)
}

/// A middleware to authenticate a user by one of the three methods:
/// - Bearer token (JWT or API key)
/// - Session cookie
/// - Basic auth (only for OPDS and the Komga compatibility paths)
/// This middleware should be used broadly across the application, however in instances where
/// a router is scoped to an API key, the `api_key_middlware` should be used instead.
///
/// If the user is authenticated, the middleware will insert the user into the request
/// extensions.
///
/// Note: It is important that this middlware is placed _after_ any middleware/handlers which access the
/// request extensions, as the user is inserted into the request extensions dynamically here.
#[tracing::instrument(skip_all)]
pub async fn auth_middleware(
	State(ctx): State<AppState>,
	HostExtractor(host_details): HostExtractor,
	mut session: Session,
	mut req: Request,
	next: Next,
) -> Result<Response, impl IntoResponse> {
	let req_headers = req.headers().clone();
	let auth_header = req_headers
		.get(header::AUTHORIZATION)
		.and_then(|header| header.to_str().ok());
	let save_basic_session = req_headers
		.get(STUMP_SAVE_BASIC_SESSION_HEADER)
		.and_then(|header| header.to_str().ok())
		.is_some_and(|header| header == "true");

	let request_uri = req.extensions().get::<OriginalUri>().cloned().map_or_else(
		|| req.uri().path().to_owned(),
		|path| path.0.path().to_owned(),
	);
	#[cfg(feature = "komf")]
	let komf_api_key = if is_komf_auth_path(&request_uri) {
		let query = req
			.extensions()
			.get::<OriginalUri>()
			.map_or_else(|| req.uri().query(), |uri| uri.0.query());
		komf_api_key_query(query)
	} else {
		None
	};

	let service =
		RequestOrigin::new(host_details.host.clone(), host_details.scheme.clone());
	// A present Komf query key is authoritative: invalid or unbound keys
	// return 401 before cookie, Basic, or Bearer authentication can fall back.
	#[cfg(feature = "komf")]
	if let Some(api_key) = komf_api_key {
		let mut req_ctx = authenticate_komga_api_key(Some(&api_key), ctx.conn.as_ref())
			.await
			.map_err(|error| error.into_response())?;
		bind_device(
			&ctx,
			&mut req_ctx,
			CredentialRef::ApiKey(&api_key),
			Protocol::Komga,
		)
		.await
		.map_err(|error| error.into_response())?;
		require_komf_device(&req_ctx).map_err(|error| error.into_response())?;
		req_ctx.user = inject_avatar_url(req_ctx.user, service);
		req.extensions_mut().insert(req_ctx);
		return Ok(next.run(req).await);
	}

	let session_user = fetch_session_user(&session, ctx.conn.as_ref())
		.await
		.map_err(|e| {
			tracing::error!(error = ?e, "Failed to fetch user from session");
			APIError::Unauthorized.into_response()
		})?;

	if let Some(user) = session_user {
		req.extensions_mut().insert(AuthContext {
			user: inject_avatar_url(user, service),
			api_key: None,
			// A session never carries a device: a device credential is
			// deliberately never upgraded to one (see `handle_basic_auth`),
			// so a session request inherits its user's visibility.
			device_id: None,
		});
		return Ok(next.run(req).await);
	}

	if cfg!(debug_assertions) {
		tracing::debug!("No session in middleware, falling back to auth header");
	}

	let is_opds = request_uri.starts_with("/opds");
	let is_playground_request =
		matches!(request_uri.as_str(), "/api/graphql" | "/api/graphql/")
			&& *req.method() == Method::GET;
	let is_playground_allowed = cfg!(feature = "webui")
		&& ctx.config.protocols.enable_webui
		&& (ctx.config.protocols.enable_playground || cfg!(debug_assertions));
	let is_playground = is_playground_request && is_playground_allowed;
	#[cfg(feature = "komga")]
	let is_komga_basic_auth = is_komga_basic_auth_path(&request_uri);
	#[cfg(not(feature = "komga"))]
	let is_komga_basic_auth = false;
	#[cfg(feature = "komga")]
	let is_komga_logout = is_komga_logout_path(&request_uri);
	#[cfg(not(feature = "komga"))]
	let is_komga_logout = false;

	let Some(auth_header) = auth_header else {
		// Komga API-key clients (e.g. Liseur) send no Authorization header at
		// all; accept `X-API-Key` only on the Komga-scoped paths.
		#[cfg(feature = "komga")]
		if is_komga_basic_auth {
			if let Some(api_key) = req_headers
				.get(KOMGA_API_KEY_HEADER)
				.and_then(|header| header.to_str().ok())
			{
				let mut req_ctx =
					authenticate_komga_api_key(Some(api_key), ctx.conn.as_ref())
						.await
						.map_err(|error| error.into_response())?;
				bind_device(
					&ctx,
					&mut req_ctx,
					CredentialRef::ApiKey(api_key),
					Protocol::Komga,
				)
				.await
				.map_err(|error| error.into_response())?;
				req_ctx.user = inject_avatar_url(req_ctx.user, service);
				req.extensions_mut().insert(req_ctx);
				return Ok(next.run(req).await);
			}
		}

		#[cfg(feature = "komga")]
		if should_accept_komga_remember_me(&request_uri, &req_headers) {
			let token = komga_remember_me_cookie(&req_headers)
				.expect("remember-me acceptance requires a cookie");
			let user = authenticate_komga_remember_me(token, ctx.conn.as_ref())
				.await
				.map_err(|error| error.into_response())?;

			// Keep the browser session short-lived, just as for a successful
			// Komga Basic-auth request.
			enforce_max_sessions(&user, ctx.conn.as_ref())
				.await
				.map_err(|error| error.into_response())?;
			if !is_komga_logout {
				if let Err(error) =
					session.insert(SESSION_USER_KEY, user.id.clone()).await
				{
					tracing::error!(error = ?error, "Failed to save session");
				}
			}

			req.extensions_mut().insert(AuthContext {
				user: inject_avatar_url(AuthUser::from(user), service),
				api_key: None,
				// A remember-me token acts as the user, not as a device.
				device_id: None,
			});
			return Ok(next.run(req).await);
		}

		if is_opds {
			// If we are access the OPDS auth document, we allow it
			if request_uri.ends_with("/opds/v2.0/auth") {
				return Ok(next.run(req).await);
			}

			let opds_version = request_uri
				.split('/')
				.nth(2)
				.map_or("1.2".to_string(), |v| v.replace('v', ""));

			return Err(
				OPDSBasicAuth::new(opds_version, host_details.url()).into_response()
			);
		} else if is_playground {
			// Sign in through Home before opening the server-side playground.
			return Err(
				Redirect::to("/app/login?returnTo=%2Fapi%2Fgraphql").into_response()
			);
		}

		return Err(APIError::Unauthorized.into_response());
	};

	let mut req_ctx = match auth_header {
		_ if auth_header.starts_with("Bearer ") && auth_header.len() > 7 => {
			let token = auth_header[7..].to_owned();

			handle_bearer_auth(token, ctx.conn.as_ref())
				.await
				.map_err(|e| e.into_response())?
		},
		_ if auth_header.starts_with("Basic ")
			&& auth_header.len() > 6
			&& basic_auth_accepted(is_opds, is_komga_basic_auth) =>
		{
			let encoded_credentials = &auth_header[6..];
			handle_basic_auth(
				encoded_credentials,
				ctx.conn.as_ref(),
				&mut session,
				basic_auth_saves_session(
					is_opds,
					save_basic_session,
					is_komga_basic_auth,
					is_komga_logout,
				),
			)
			.await
			.map_err(|e| e.into_response())?
		},
		_ => return Err(APIError::Unauthorized.into_response()),
	};
	// A Komga remember-me token acts as the user; a device key must not be
	// upgraded to one.
	#[cfg(feature = "komga")]
	if auth_header.starts_with("Basic ")
		&& is_komga_basic_auth
		&& req_ctx.api_key.is_none()
	{
		req.extensions_mut().insert(KomgaBasicAuthSuccess);
	}

	req_ctx.user = inject_avatar_url(req_ctx.user, service);

	if let Some(api_key) = req_ctx.api_key.clone() {
		let protocol = if is_komga_basic_auth {
			Protocol::Komga
		} else if is_opds {
			Protocol::Opds
		} else {
			Protocol::Api
		};
		bind_device(
			&ctx,
			&mut req_ctx,
			CredentialRef::ApiKey(&api_key),
			protocol,
		)
		.await
		.map_err(|error| error.into_response())?;
	}

	req.extensions_mut().insert(req_ctx);

	Ok(next.run(req).await)
}

#[cfg(any(feature = "opds", feature = "kobo", feature = "koreader"))]
#[derive(Debug, Deserialize)]
pub struct APIKeyPath(HashMap<String, String>);

#[cfg(any(feature = "opds", feature = "kobo", feature = "koreader"))]
impl APIKeyPath {
	fn get_key(&self) -> Option<String> {
		self.0.get("api_key").cloned()
	}
}

/// A middleware to authenticate a user by an API key in a *very* specific way. This middleware
/// assumes that a fully qualified API key is provided in the path. This is used for three features today:
///
/// 1. An alternative for bearer token on the OPDS v1.2 API
/// 2. A way to authenticate users for the KoReader sync API
/// 3. A way to authenticate users for the Kobo sync API
///
/// This isn't necessary for OPDS v2.0 as it has a more robust authentication mechanism. The koreader
/// frontend app will send an md5 hash of whatever password you provide. Stump does not use the same
/// hashing algorithm, therefore the default auth method would not work.
#[cfg(any(feature = "opds", feature = "kobo", feature = "koreader"))]
pub async fn api_key_middleware(
	State(ctx): State<AppState>,
	Path(params): Path<APIKeyPath>,
	mut req: Request,
	next: Next,
) -> Result<Response, impl IntoResponse> {
	let Some(api_key) = params.get_key() else {
		tracing::error!("No API key provided");
		return Err(APIError::Unauthorized.into_response());
	};

	let Ok(pak) = PrefixedApiKey::from_string(api_key.as_str()) else {
		tracing::error!("Failed to parse API key");
		return Err(APIError::Unauthorized.into_response());
	};

	let user = validate_api_key(pak, ctx.conn.as_ref())
		.await
		.map_err(|e| e.into_response())?;

	// The middleware is mounted on the path-key routers only, so the mount
	// prefix names the protocol.
	let request_path = req.extensions().get::<OriginalUri>().map_or_else(
		|| req.uri().path().to_owned(),
		|uri| uri.0.path().to_owned(),
	);
	let protocol = match request_path.trim_start_matches('/').split('/').next() {
		Some("kobo") => Protocol::Kobo,
		Some("koreader") => Protocol::Koreader,
		_ => Protocol::Opds,
	};
	let mut req_ctx = AuthContext {
		user,
		api_key: Some(api_key.clone()),
		device_id: None,
	};
	bind_device(
		&ctx,
		&mut req_ctx,
		CredentialRef::ApiKey(&api_key),
		protocol,
	)
	.await
	.map_err(|error| error.into_response())?;

	req.extensions_mut().insert(req_ctx);

	Ok(next.run(req).await)
}

/// Binds an authenticated request to the device its credential belongs to:
/// records the sighting and narrows the request's visibility to the device's
/// library scope.
///
/// One device lookup serves both, and nothing caches the result, so a
/// `setDeviceLibraryScope` write is in force on the device's very next
/// request. The sighting itself stays best-effort inside the registry; an
/// error here means the device behind the credential could not be
/// determined, and a request whose visible library set is unknown is refused
/// rather than served with the user's full visibility.
pub(crate) async fn bind_device(
	ctx: &AppState,
	auth: &mut AuthContext,
	credential: CredentialRef<'_>,
	protocol: Protocol,
) -> APIResult<()> {
	let device = ctx
		.devices()
		.authenticate(credential, protocol)
		.await
		.map_err(|error| {
			tracing::error!(?error, ?protocol, "Failed to resolve the request's device");
			APIError::InternalServerError(
				"Could not resolve the device for this credential".to_string(),
			)
		})?;
	if let Some(device) = device {
		auth.user.device_library_scope =
			device.library_scope.library_ids().map(<[String]>::to_vec);
		auth.device_id = Some(device.device_id);
	}
	Ok(())
}

pub async fn validate_api_key(
	pak: PrefixedApiKey,
	conn: &DatabaseConnection,
) -> APIResult<AuthUser> {
	let controller = PrefixedApiKeyController::configure()
		.prefix(API_KEY_PREFIX.to_owned())
		.seam_defaults()
		.finalize()?;

	let long_token_hash = controller.long_token_hashed(&pak);
	let validation_start = DateTimeWithTimeZone::from(current_utc_time());

	let APIKeyWithUser { api_key, user } = APIKeyWithUser::find()
		.filter(
			Condition::all()
				.add(
					api_key::Column::ShortToken
						.eq(pak.short_token().to_string())
						.and(api_key::Column::LongTokenHash.eq(long_token_hash)),
				)
				.add(
					Condition::any()
						.add(api_key::Column::ExpiresAt.gte(validation_start))
						.add(api_key::Column::ExpiresAt.is_null()),
				),
		)
		.into_model::<APIKeyWithUser>()
		.one(conn)
		.await?
		.ok_or(APIError::Unauthorized)?;

	let api_key_permissions = api_key.permissions.clone();

	// Note: we check as a precaution. If a user had the permission revoked, that logic should also
	// clean up keys.
	let can_use_key =
		user.is_server_owner || user.permissions.contains(&UserPermission::AccessApiKeys);

	if !can_use_key || !controller.check_hash(&pak, &api_key.long_token_hash) {
		tracing::error!(?can_use_key, "API key validation failed!");
		// TODO(security): track?
		return Err(APIError::Unauthorized);
	}

	let update_result = api_key::Entity::update_many()
		.filter(api_key::Column::Id.eq(api_key.id))
		.col_expr(api_key::Column::LastUsedAt, Expr::value(validation_start))
		.exec(conn)
		.await;
	if let Err(e) = update_result {
		// IMO we shouldn't fail the request if we can't update the last used at field
		tracing::error!(error = ?e, "Failed to update API key");
	}

	let constructed_user = match api_key_permissions {
		APIKeyPermissions::Inherit(_) => AuthUser::from(user),
		// TODO(permissions): server owner going away
		// Note: we don't construct permission sets for inferred permissions. What you
		// give to your API key is what it gets, however the server owner flag
		// will be set to false always.
		APIKeyPermissions::Custom(permissions) => AuthUser {
			permissions,
			is_server_owner: false,
			..AuthUser::from(user)
		},
	};

	Ok(constructed_user)
}

/// A function to handle bearer token authentication. This function will verify the token and
/// return the user if the token is valid.
#[tracing::instrument(skip_all)]
pub(crate) async fn handle_bearer_auth(
	token: String,
	conn: &DatabaseConnection,
) -> APIResult<AuthContext> {
	match PrefixedApiKey::from_string(token.as_str()) {
		Ok(api_key) if api_key.prefix() == API_KEY_PREFIX => {
			return validate_api_key(api_key, conn)
				.await
				.map(|user| AuthContext {
					user,
					api_key: Some(token),
					// The caller binds the device: see `bind_device`.
					device_id: None,
				});
		},
		_ => (),
	};

	let user_id = extract_user_from_jwt(&token, conn).await?;

	let fetched_user = user::LoginUser::find()
		.filter(user::Column::Id.eq(user_id.clone()))
		.filter(user::Column::DeletedAt.is_null())
		.into_model::<user::LoginUser>()
		.one(conn)
		.await?;

	let Some(user) = fetched_user else {
		tracing::error!(?user_id, "No user found for ID");
		return Err(APIError::Unauthorized);
	};

	if user.is_locked {
		tracing::error!(
			username = &user.username,
			"User is locked, denying authentication"
		);
		return Err(APIError::Forbidden(
			api_error_message::LOCKED_ACCOUNT.to_string(),
		));
	}

	Ok(AuthContext {
		user: AuthUser::from(user),
		api_key: None,
		// A JWT acts as the user, not as a device.
		device_id: None,
	})
}

fn login_user_by_username_query(username: &str) -> sea_orm::Select<user::Entity> {
	user::LoginUser::find()
		.filter(user::Column::Username.eq(username.to_owned()))
		.filter(user::Column::DeletedAt.is_null())
}

fn parse_basic_credentials(
	encoded_credentials: &str,
) -> APIResult<crate::utils::DecodedCredentials> {
	let decoded_bytes = STANDARD
		.decode(encoded_credentials.as_bytes())
		.map_err(|_| APIError::Unauthorized)?;

	decode_base64_credentials(decoded_bytes).map_err(|_| APIError::Unauthorized)
}

/// Bounded cache of recently verified Basic credentials.
///
/// Compatibility clients (Komga profile, OPDS readers) send `Authorization:
/// Basic` on many requests in a burst; bcrypt verification at the configured
/// cost is the dominant per-request cost (~250 ms at cost 12), which serialises
/// a sync burst into seconds of CPU. The cache keys on a SHA-256 of the raw
/// header value (never the plaintext or the bcrypt hash) and stores only the
/// user id plus the password hash that was verified. A hit still re-reads the
/// user row, so password changes, lock, and deletion take effect immediately:
/// only the bcrypt compare is skipped, and only while the stored hash still
/// matches. Failed verifications are never cached.
mod basic_auth_cache {
	use std::{
		collections::HashMap,
		sync::{LazyLock, Mutex},
		time::{Duration, Instant},
	};

	use sha2::{Digest, Sha256};

	const TTL: Duration = Duration::from_secs(60);
	const CAPACITY: usize = 256;

	#[derive(Clone)]
	pub(super) struct Verified {
		pub user_id: String,
		pub hashed_password: String,
		verified_at: Instant,
	}

	static CACHE: LazyLock<Mutex<HashMap<[u8; 32], Verified>>> =
		LazyLock::new(|| Mutex::new(HashMap::with_capacity(CAPACITY)));

	fn key(encoded_credentials: &str) -> [u8; 32] {
		Sha256::digest(encoded_credentials.as_bytes()).into()
	}

	pub(super) fn get(encoded_credentials: &str) -> Option<Verified> {
		let key = key(encoded_credentials);
		let mut cache = CACHE
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		match cache.get(&key) {
			Some(entry) if entry.verified_at.elapsed() < TTL => Some(entry.clone()),
			Some(_) => {
				cache.remove(&key);
				None
			},
			None => None,
		}
	}

	pub(super) fn insert(
		encoded_credentials: &str,
		user_id: &str,
		hashed_password: &str,
	) {
		let mut cache = CACHE
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		if cache.len() >= CAPACITY {
			cache.retain(|_, entry| entry.verified_at.elapsed() < TTL);
			if cache.len() >= CAPACITY {
				// Still full of live entries: drop the oldest rather than grow.
				if let Some(oldest) = cache
					.iter()
					.min_by_key(|(_, entry)| entry.verified_at)
					.map(|(key, _)| *key)
				{
					cache.remove(&oldest);
				}
			}
		}
		cache.insert(
			key(encoded_credentials),
			Verified {
				user_id: user_id.to_owned(),
				hashed_password: hashed_password.to_owned(),
				verified_at: Instant::now(),
			},
		);
	}

	pub(super) fn invalidate(encoded_credentials: &str) {
		let mut cache = CACHE
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		cache.remove(&key(encoded_credentials));
	}

	#[cfg(test)]
	pub(super) fn len() -> usize {
		CACHE
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
			.len()
	}

	#[cfg(test)]
	pub(super) fn clear() {
		CACHE
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
			.clear();
	}
}

/// What a `Basic` header verified as: the user's password, or a device API
/// key standing in for it.
enum BasicVerified {
	Password(user::LoginUser),
	DeviceKey { user: AuthUser, api_key: String },
}

async fn verify_basic_credentials(
	encoded_credentials: &str,
	conn: &DatabaseConnection,
) -> APIResult<BasicVerified> {
	let decoded_credentials = parse_basic_credentials(encoded_credentials)?;

	// A device API key stands in for the password, so Basic-only clients
	// (Mihon, Komelia, OPDS 1.2 readers) can act as their registered device
	// with the key's narrowed permissions. A key that fails to validate is
	// tried as a password below, so a password that merely looks like a key
	// still works.
	if let Some(user) = verify_basic_api_key(&decoded_credentials, conn).await {
		return Ok(BasicVerified::DeviceKey {
			user,
			api_key: decoded_credentials.password,
		});
	}

	let fetched_user = login_user_by_username_query(&decoded_credentials.username)
		.into_model::<user::LoginUser>()
		.one(conn)
		.await?;

	let Some(user) = fetched_user else {
		basic_auth_cache::invalidate(encoded_credentials);
		return Err(APIError::Unauthorized);
	};

	// A cache hit is valid only for the same user and the same stored hash;
	// any password change falls through to a full bcrypt verification.
	let cached_match = basic_auth_cache::get(encoded_credentials).is_some_and(|hit| {
		hit.user_id == user.id && hit.hashed_password == user.hashed_password
	});
	let is_match = cached_match
		|| verify_password(&user.hashed_password, &decoded_credentials.password)?;

	if is_match && user.is_locked {
		basic_auth_cache::invalidate(encoded_credentials);
		tracing::error!(
			username = &user.username,
			"User is locked, denying authentication"
		);
		return Err(APIError::Forbidden(
			api_error_message::LOCKED_ACCOUNT.to_string(),
		));
	} else if !is_match {
		basic_auth_cache::invalidate(encoded_credentials);
		return Err(APIError::Unauthorized);
	}

	if !cached_match {
		basic_auth_cache::insert(encoded_credentials, &user.id, &user.hashed_password);
	}

	Ok(BasicVerified::Password(user))
}

/// The user a Basic password authenticates as when it is a prefixed API key
/// belonging to the named user; `None` when it is not a key or the key does
/// not validate for that user.
async fn verify_basic_api_key(
	credentials: &crate::utils::DecodedCredentials,
	conn: &DatabaseConnection,
) -> Option<AuthUser> {
	let pak = PrefixedApiKey::from_string(&credentials.password)
		.ok()
		.filter(|pak| pak.prefix() == API_KEY_PREFIX)?;
	let user = validate_api_key(pak, conn).await.ok()?;
	(user.username == credentials.username).then_some(user)
}

/// A function to handle basic authentication. If the user is authenticated, an optional session
/// will be created for the user. Session creation is used by OPDS only; compatibility clients
/// send Basic credentials on each request. A device key never gets a session: the session would
/// act as the user, widening the key's permissions.
#[tracing::instrument(skip_all)]
async fn handle_basic_auth(
	encoded_credentials: &str,
	conn: &DatabaseConnection,
	session: &mut Session,
	save_session: bool,
) -> APIResult<AuthContext> {
	let user = match verify_basic_credentials(encoded_credentials, conn).await? {
		BasicVerified::Password(user) => user,
		BasicVerified::DeviceKey { user, api_key } => {
			return Ok(AuthContext {
				user,
				api_key: Some(api_key),
				// The caller binds the device: see `bind_device`.
				device_id: None,
			});
		},
	};

	if save_session {
		tracing::trace!("Saving session for user");
		enforce_max_sessions(&user, conn).await?;
		if let Err(error) = session.insert(SESSION_USER_KEY, user.id.clone()).await {
			tracing::error!(error = ?error, "Failed to save session");
		}
	}

	Ok(AuthContext {
		user: AuthUser::from(user),
		api_key: None,
		// A password acts as the user, not as a device.
		device_id: None,
	})
}

/// A struct used to hold the details required to generate an OPDS basic auth response
pub struct OPDSBasicAuth {
	version: String,
	service_url: String,
}

impl OPDSBasicAuth {
	pub fn new(version: String, service_url: String) -> Self {
		Self {
			version,
			service_url,
		}
	}
}

impl IntoResponse for OPDSBasicAuth {
	fn into_response(self) -> Response {
		if self.version == "2.0" {
			let links = vec![OPDSLink::help()];

			let document = match OPDSAuthenticationDocumentBuilder::default()
				.id(format!("{}/opds/v2.0/auth", self.service_url))
				.description(OPDSSupportedAuthFlow::Basic.description().to_string())
				.links(links)
				.build()
			{
				Ok(document) => document,
				Err(e) => {
					tracing::error!(error = ?e, "Failed to build OPDS authentication document");
					return APIError::InternalServerError(e.to_string()).into_response();
				},
			};
			let json_response = Json(document).into_response();
			let body = json_response.into_body();

			// We want to encourage the client to delete any existing session cookies when the current
			// is no longer valid
			let delete_cookie = delete_cookie_header();

			Response::builder()
				.status(StatusCode::UNAUTHORIZED)
				.header("Authorization", "Basic")
				.header(
					"WWW-Authenticate",
					format!("Basic realm=\"stump OPDS v{}\"", self.version),
				)
				.header(
					"Content-Type",
					format!("{OPDS_AUTHENTICATION_DOCUMENT_TYPE}; charset=utf-8"),
				)
				.header(
					"Link",
					format!(
						"<{}{}>; rel=\"{OPDS_AUTHENTICATION_DOCUMENT_REL}\"; type=\"{OPDS_AUTHENTICATION_DOCUMENT_TYPE}\"",
						self.service_url,
						"/opds/v2.0/auth"
					),
				)
				.header(delete_cookie.0, delete_cookie.1)
				.body(body)
				.unwrap_or_else(|e| {
					tracing::error!(error = ?e, "Failed to build response");
					StatusCode::INTERNAL_SERVER_ERROR.into_response()
				})
		} else {
			Response::builder()
				.status(StatusCode::UNAUTHORIZED)
				.header("Authorization", "Basic")
				.header(
					"WWW-Authenticate",
					format!("Basic realm=\"stump OPDS v{}\"", self.version),
				)
				.body(Body::default())
				.unwrap_or_else(|e| {
					tracing::error!(error = ?e, "Failed to build response");
					APIError::InternalServerError(e.to_string()).into_response()
				})
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The Basic-auth cache is process-global; every test that touches it
	/// holds this lock so the parallel test runner cannot interleave them.
	static CACHE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

	fn cache_test_guard() -> std::sync::MutexGuard<'static, ()> {
		CACHE_TEST_LOCK
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
	}

	#[test]
	fn test_request_context_user() {
		let user = AuthUser::default();
		let request_context = AuthContext {
			user: user.clone(),
			api_key: None,
			device_id: None,
		};
		assert!(user.is(&request_context.user()));
	}

	#[test]
	fn test_request_context_id() {
		let user = AuthUser::default();
		let request_context = AuthContext {
			user: user.clone(),
			api_key: None,
			device_id: None,
		};
		assert_eq!(user.id, request_context.id());
	}

	#[test]
	fn test_request_context_enforce_permissions_when_server_owner() {
		let user = AuthUser {
			is_server_owner: true,
			..Default::default()
		};
		let request_context = AuthContext {
			user: user.clone(),
			api_key: None,
			device_id: None,
		};
		assert!(request_context
			.enforce_permissions(&[UserPermission::AccessBookClub])
			.is_ok());
	}

	#[test]
	fn test_request_context_enforce_permissions_when_permitted() {
		let user = AuthUser {
			permissions: vec![UserPermission::AccessBookClub],
			..Default::default()
		};
		let request_context = AuthContext {
			user: user.clone(),
			api_key: None,
			device_id: None,
		};
		assert!(request_context
			.enforce_permissions(&[UserPermission::AccessBookClub])
			.is_ok());
	}

	#[test]
	fn test_request_context_enforce_permissions_when_denied() {
		let user = AuthUser::default();
		let request_context = AuthContext {
			user: user.clone(),
			api_key: None,
			device_id: None,
		};
		assert!(request_context
			.enforce_permissions(&[UserPermission::AccessBookClub])
			.is_err());
	}

	#[test]
	fn test_request_context_enforce_permissions_when_denied_partial() {
		let user = AuthUser {
			permissions: vec![UserPermission::AccessBookClub],
			..Default::default()
		};
		let request_context = AuthContext {
			user: user.clone(),
			api_key: None,
			device_id: None,
		};
		assert!(request_context
			.enforce_permissions(&[
				UserPermission::AccessBookClub,
				UserPermission::CreateLibrary
			])
			.is_err());
	}

	#[test]
	fn test_request_context_user_and_enforce_permissions_when_permitted() {
		let user = AuthUser {
			permissions: vec![UserPermission::AccessBookClub],
			..Default::default()
		};
		let request_context = AuthContext {
			user: user.clone(),
			api_key: None,
			device_id: None,
		};
		assert!(user.is(&request_context
			.user_and_enforce_permissions(&[UserPermission::AccessBookClub])
			.unwrap()));
	}

	#[test]
	fn test_request_context_user_and_enforce_permissions_when_denied() {
		let user = AuthUser::default();
		let request_context = AuthContext {
			user: user.clone(),
			api_key: None,
			device_id: None,
		};
		assert!(request_context
			.user_and_enforce_permissions(&[UserPermission::AccessBookClub])
			.is_err());
	}

	#[test]
	fn test_request_context_enforce_server_owner_when_server_owner() {
		let user = AuthUser {
			is_server_owner: true,
			..Default::default()
		};
		let request_context = AuthContext {
			user: user.clone(),
			api_key: None,
			device_id: None,
		};
		assert!(request_context.enforce_server_owner().is_ok());
	}

	#[test]
	fn test_request_context_enforce_server_owner_when_not_server_owner() {
		let user = AuthUser::default();
		let request_context = AuthContext {
			user: user.clone(),
			api_key: None,
			device_id: None,
		};
		assert!(request_context.enforce_server_owner().is_err());
	}

	#[cfg(feature = "komga")]
	#[test]
	fn test_komga_basic_auth_path_scope_is_exact() {
		assert_eq!(KOMGA_API_KEY_HEADER, "X-API-Key");
		assert!(is_komga_basic_auth_path("/api/v1/libraries"));
		assert!(is_komga_basic_auth_path("/api/v1/libraries/123"));
		assert!(is_komga_basic_auth_path("/api/v2/users"));
		assert!(is_komga_basic_auth_path("/api/v2/users/me"));
		assert!(is_komga_basic_auth_path(
			"/api/v2/users/me/authentication-activity"
		));
		assert!(is_komga_basic_auth_path(
			"/api/v2/users/authentication-activity"
		));
		assert!(is_komga_basic_auth_path(
			"/api/v2/users/user-id/authentication-activity/latest"
		));
		assert!(is_komga_basic_auth_path("/api/v2/authors"));
		assert!(is_komga_basic_auth_path("/sse/v1/events"));
		assert!(is_komga_basic_auth_path("/api/logout"));
		assert!(is_komga_basic_auth_path("/komga/api/v1/libraries"));
		assert!(is_komga_basic_auth_path("/komga/api/v2/users/me"));
		assert!(is_komga_basic_auth_path("/api/komga/v1/providers"));
		assert!(is_komga_basic_auth_path("/api/komga/v1/config"));
		assert!(is_komga_basic_auth_path(
			"/api/v2/series/series-id/read-progress/tachiyomi"
		));
		assert!(is_komga_basic_auth_path(
			"/komga/api/v2/series/series-id/read-progress/tachiyomi"
		));

		assert!(!is_komga_basic_auth_path("/api/v1"));
		assert!(!is_komga_basic_auth_path("/api/v1x/libraries"));
		assert!(!is_komga_basic_auth_path("/api/v2/users/"));
		assert!(!is_komga_basic_auth_path("/api/v2/users/me/books"));
		assert!(!is_komga_basic_auth_path(
			"/api/v2/series/series-id/thumbnail"
		));
		assert!(!is_komga_basic_auth_path(
			"/api/v2/users/user-id/authentication-activity"
		));
		assert!(!is_komga_basic_auth_path(
			"/api/v2/users/user-id/authentication-activity/latest/extra"
		));
		assert!(!is_komga_basic_auth_path("/api/v2/users/user-id/avatar"));
		assert!(!is_komga_basic_auth_path("/api/graphql"));
		assert!(!is_komga_basic_auth_path("/sse/v1/events/other"));
		assert!(!is_komga_basic_auth_path("/komga/api/v1"));
		assert!(!is_komga_basic_auth_path("/komga/api/v2/users"));
		assert!(!is_komga_basic_auth_path("/api/logout/other"));
		assert!(!is_komga_basic_auth_path("/api/komga"));
		assert!(!is_komga_basic_auth_path("/api/komgaish/v1/providers"));
		#[cfg(feature = "komf")]
		{
			assert!(is_komga_basic_auth_path("/api/config"));
			assert!(is_komga_basic_auth_path("/api/jobs"));
			assert!(is_komga_basic_auth_path("/api/jobs/all"));
			assert!(is_komga_basic_auth_path("/api/jobs/43/events"));
			assert!(!is_komga_basic_auth_path("/api/config/"));
			assert!(!is_komga_basic_auth_path("/api/metadata/providers"));
		}
	}

	#[cfg(feature = "komf")]
	#[test]
	fn komf_api_key_auth_is_limited_to_registered_routes() {
		for path in [
			"/api/config",
			"/api/jobs",
			"/api/jobs/all",
			"/api/jobs/job-id",
			"/api/jobs/job-id/events",
			"/api/komga/metadata/providers",
			"/api/komga/metadata/search",
			"/api/komga/metadata/series-cover",
			"/api/komga/metadata/identify",
			"/api/komga/metadata/match/library/library-id",
			"/api/komga/metadata/match/library/library-id/series/series-id",
			"/api/komga/metadata/reset/library/library-id",
			"/api/komga/metadata/reset/library/library-id/series/series-id",
			"/api/komga/media-server/connected",
			"/api/komga/media-server/libraries",
		] {
			assert!(is_komf_auth_path(path), "expected Komf route: {path}");
		}

		for path in [
			"/api/config/",
			"/api/configure",
			"/api/jobs/job-id/events/extra",
			"/api/jobsfoo/job-id",
			"/api/komga/metadata/search/extra",
			"/api/komga/v1/providers",
			"/api/graphql",
			"/api/v1/libraries",
			"/api/v2/libraries",
			"/api/Plugin/authenticate",
			"/api/items/item-id/cover",
			"/api/series",
		] {
			assert!(!is_komf_auth_path(path), "unexpected Komf route: {path}");
		}
	}

	#[cfg(feature = "komf")]
	#[test]
	fn komf_api_key_query_is_case_insensitive_and_decoded() {
		assert_eq!(
			komf_api_key_query(Some("page=1&apikey=stump_a%2Bb")),
			Some("stump_a+b".to_owned())
		);
		assert_eq!(komf_api_key_query(Some("apiKey=")), Some(String::new()));
		assert_eq!(komf_api_key_query(Some("apiKey")), Some(String::new()));
		assert_eq!(komf_api_key_query(None), None);
	}

	#[cfg(feature = "komf")]
	#[tokio::test]
	async fn valid_unbound_komf_api_key_is_unauthorized() {
		use sea_orm::{ActiveModelTrait, Set};

		let conn = ::tests::db::test_database().await;
		let owner = ::tests::fake_data::User::new("komf-key-owner")
			.insert(&conn)
			.await;
		let (key, hash) = stump_core::api_key::create_prefixed_key().unwrap();
		let key_value = key.to_string();
		api_key::ActiveModel {
			user_id: Set(owner.id),
			name: Set("unbound Komf key".to_owned()),
			short_token: Set(key.short_token().to_owned()),
			long_token_hash: Set(hash),
			permissions: Set(APIKeyPermissions::Custom(vec![
				UserPermission::AccessApiKeys,
			])),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();

		let auth = authenticate_komga_api_key(Some(&key_value), &conn)
			.await
			.unwrap();
		assert!(auth.device_id.is_none());
		assert_eq!(
			require_komf_device(&auth).unwrap_err().status_code(),
			StatusCode::UNAUTHORIZED
		);
	}

	#[cfg(feature = "komf")]
	#[tokio::test]
	async fn komf_device_auth_binds_its_library_scope() {
		let database = ::tests::db::test_database().await;
		let user = ::tests::fake_data::User::new("komf-scope-owner")
			.insert(&database)
			.await;
		let owner = AuthUser {
			id: user.id,
			username: user.username,
			is_server_owner: true,
			..Default::default()
		};
		let library = ::tests::fake_data::Library::default()
			.insert(&database)
			.await;
		let _hidden_library = ::tests::fake_data::Library::default()
			.insert(&database)
			.await;
		let ctx = stump_core::Ctx::for_testing(database).arced();
		let conn = ctx.conn.clone();
		let devices = ctx.devices();
		let (device, credential) = devices
			.create_device(
				&owner,
				stump_devices::CredentialIssuance::InteractiveSession,
				stump_devices::DeviceKind::Komelia,
				None,
			)
			.await
			.expect("Komelia device credential");
		devices
			.set_library_scope(
				&owner,
				&device.id,
				stump_devices::LibraryScope::Only(vec![library.id.clone()]),
			)
			.await
			.expect("device library scope");

		let mut auth =
			authenticate_komga_api_key(Some(&credential.secret), conn.as_ref())
				.await
				.expect("valid Komelia device key");
		bind_device(
			&ctx,
			&mut auth,
			CredentialRef::ApiKey(&credential.secret),
			Protocol::Komga,
		)
		.await
		.expect("bind the device");
		let visible_libraries =
			models::entity::library::Entity::find_for_user(auth.scope())
				.all(conn.as_ref())
				.await
				.expect("query visible libraries");

		assert_eq!(visible_libraries.len(), 1);
		assert_eq!(visible_libraries[0].id, library.id);

		assert_eq!(auth.device_id.as_deref(), Some(device.id.as_str()));
		assert_eq!(auth.user.device_library_scope, Some(vec![library.id]));
	}

	#[cfg(feature = "komf")]
	#[tokio::test]
	async fn unknown_komf_api_key_is_unauthorized() {
		let conn = ::tests::db::test_database().await;
		let key = PrefixedApiKey::new(
			API_KEY_PREFIX.to_owned(),
			"unknown-token".to_owned(),
			"unknown-secret".to_owned(),
		)
		.to_string();

		let error = authenticate_komga_api_key(Some(&key), &conn)
			.await
			.err()
			.expect("an unknown key must be rejected");
		assert_eq!(error.status_code(), StatusCode::UNAUTHORIZED);
	}

	#[cfg(feature = "komf")]
	#[tokio::test]
	async fn revoked_komelia_api_key_is_unauthorized() {
		let conn = std::sync::Arc::new(::tests::db::test_database().await);
		let user = ::tests::fake_data::User::new("komf-revoked-owner")
			.insert(conn.as_ref())
			.await;
		let owner = AuthUser {
			id: user.id,
			username: user.username,
			is_server_owner: true,
			..Default::default()
		};
		let devices = stump_devices::DeviceService::new(conn.clone());
		let (device, credential) = devices
			.create_device(
				&owner,
				stump_devices::CredentialIssuance::InteractiveSession,
				stump_devices::DeviceKind::Komelia,
				None,
			)
			.await
			.expect("Komelia device credential");
		devices
			.revoke(&owner, &device.id)
			.await
			.expect("device revocation");

		let error = authenticate_komga_api_key(Some(&credential.secret), conn.as_ref())
			.await
			.err()
			.expect("a revoked device key must be rejected");
		assert_eq!(error.status_code(), StatusCode::UNAUTHORIZED);
	}

	#[cfg(feature = "komf")]
	#[tokio::test]
	async fn malformed_komf_api_key_is_unauthorized() {
		use sea_orm::{DatabaseBackend, MockDatabase};

		let conn = MockDatabase::new(DatabaseBackend::Sqlite).into_connection();
		let error = authenticate_komga_api_key(Some(""), &conn)
			.await
			.err()
			.expect("an empty key must be rejected");
		assert_eq!(error.status_code(), StatusCode::UNAUTHORIZED);
	}

	#[cfg(feature = "komf")]
	#[tokio::test]
	async fn invalid_komf_query_key_does_not_fall_back_to_a_valid_session() {
		use sea_orm::{ActiveModelTrait, Set};

		let database = ::tests::db::test_database().await;
		let user = ::tests::fake_data::User::new("komf-query-session")
			.insert(&database)
			.await;
		let session_id = Id::default();
		session_entity::ActiveModel {
			session_id: Set(session_id.to_string()),
			user_id: Set(user.id),
			expiry_time: Set((chrono::Utc::now() + chrono::Duration::minutes(1)).into()),
			..Default::default()
		}
		.insert(&database)
		.await
		.expect("valid session");

		let mut config = stump_core::config::StumpConfig::debug();
		config.auth.expired_session_cleanup_interval = 0;
		let ctx = std::sync::Arc::new(stump_core::Ctx::for_testing_with_config(
			database, config,
		));
		let app = axum::Router::new()
			.route(
				"/api/config",
				axum::routing::get(|| async { StatusCode::OK }),
			)
			.route_layer(axum::middleware::from_fn_with_state(
				ctx.clone(),
				auth_middleware,
			))
			.with_state(ctx.clone())
			.layer(crate::config::session::get_session_layer(ctx));
		let server = axum_test::TestServer::new(app).expect("test server");
		let cookie = format!("{SESSION_NAME}={session_id}");

		let invalid_key = server
			.get("/api/config?apiKey=")
			.add_header(header::COOKIE, cookie.clone())
			.add_header("user-agent", "stump-server-tests")
			.await;
		assert_eq!(invalid_key.status_code(), StatusCode::UNAUTHORIZED);

		let session_only = server
			.get("/api/config")
			.add_header(header::COOKIE, cookie)
			.add_header("user-agent", "stump-server-tests")
			.await;
		assert_eq!(session_only.status_code(), StatusCode::OK);
	}

	#[cfg(feature = "komga")]
	#[test]
	fn test_komga_remember_me_token_and_path_gate() {
		use axum::http::HeaderValue;

		let missing_cookie = header::HeaderMap::new();
		assert!(!should_accept_komga_remember_me(
			"/api/v1/libraries",
			&missing_cookie
		));

		let mut garbage_cookie = header::HeaderMap::new();
		garbage_cookie.insert(
			header::COOKIE,
			HeaderValue::from_static("komga-remember-me=garbage"),
		);
		assert!(should_accept_komga_remember_me(
			"/api/v1/libraries",
			&garbage_cookie
		));
		assert_eq!(komga_remember_me_cookie(&garbage_cookie), Some("garbage"));
		assert_eq!(
			parse_komga_remember_me_token("garbage")
				.unwrap_err()
				.status_code(),
			StatusCode::UNAUTHORIZED
		);
		assert!(!should_accept_komga_remember_me(
			"/api/graphql",
			&garbage_cookie
		));
		garbage_cookie.insert(
			header::AUTHORIZATION,
			HeaderValue::from_static("Basic dXNlcjpwYXNz"),
		);
		assert!(!should_accept_komga_remember_me(
			"/api/v1/libraries",
			&garbage_cookie
		));
		garbage_cookie.remove(header::AUTHORIZATION);
		garbage_cookie.insert(
			KOMGA_API_KEY_HEADER,
			HeaderValue::from_static("garbage-api-key"),
		);
		assert!(!should_accept_komga_remember_me(
			"/api/v1/libraries",
			&garbage_cookie
		));
	}

	#[cfg(feature = "komga")]
	#[tokio::test]
	async fn test_komga_remember_me_expired_row_is_rejected() {
		use chrono::Duration;
		use sea_orm::{
			ActiveModelTrait, ConnectionTrait, Database, DatabaseBackend, Set, Statement,
		};

		let conn = Database::connect("sqlite::memory:").await.unwrap();
		conn.execute(Statement::from_string(
			DatabaseBackend::Sqlite,
			"CREATE TABLE sessions (
				id INTEGER PRIMARY KEY AUTOINCREMENT,
				session_id TEXT NOT NULL,
				user_id TEXT NOT NULL,
				created_at DATETIME NOT NULL,
				expiry_time DATETIME NOT NULL
			)"
			.to_owned(),
		))
		.await
		.unwrap();

		let token = Id::default().to_string();
		session_entity::ActiveModel {
			session_id: Set(token.clone()),
			user_id: Set("user-id".to_owned()),
			expiry_time: Set((Utc::now() - Duration::minutes(1)).into()),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();

		assert!(load_komga_remember_me_session(&token, &conn)
			.await
			.unwrap()
			.is_none());
	}

	#[cfg(feature = "komga")]
	#[test]
	fn test_komga_strips_only_session_cookie_clears() {
		use axum::http::HeaderValue;

		let (_, stump_clear) = delete_cookie_header();
		let tower_clear = format!(
			"{SESSION_NAME}=; Path=/; Max-Age=0; Expires=Wed, 03 Sep 2025 16:47:29 GMT"
		);
		let fresh = format!(
			"{SESSION_NAME}=abc123; HttpOnly; SameSite=Lax; Path=/; Max-Age=259200"
		);
		let other_zero = "other=; Path=/; Max-Age=0";
		let prefix_collision = format!("{SESSION_NAME}_x=; Path=/; Max-Age=0");

		assert!(is_session_cookie_clear(&stump_clear));
		assert!(is_session_cookie_clear(&tower_clear));
		assert!(is_session_cookie_clear(&format!(
			"{SESSION_NAME}=; Path=/; MAX-AGE=0"
		)));
		assert!(!is_session_cookie_clear(&fresh));
		assert!(!is_session_cookie_clear(other_zero));
		assert!(!is_session_cookie_clear(&prefix_collision));

		let mut headers = header::HeaderMap::new();
		headers.append(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
		for value in [
			stump_clear.as_str(),
			fresh.as_str(),
			tower_clear.as_str(),
			other_zero,
		] {
			headers.append(header::SET_COOKIE, HeaderValue::from_str(value).unwrap());
		}

		strip_session_cookie_clears(&mut headers);

		let cookies: Vec<_> = headers
			.get_all(header::SET_COOKIE)
			.iter()
			.map(|value| value.to_str().unwrap())
			.collect();
		assert_eq!(cookies, vec![fresh.as_str(), other_zero]);
		assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), "text/plain");
	}

	/// OPDS readers send Basic on every request and never the save-session
	/// header; requiring that header once broke every OPDS client.
	#[test]
	fn test_opds_basic_auth_never_requires_save_session_header() {
		let (is_opds, no_header) = (true, false);
		assert!(basic_auth_accepted(is_opds, false));
		assert!(!basic_auth_saves_session(is_opds, no_header, false, false));
		assert!(basic_auth_saves_session(is_opds, true, false, false));

		// Komga paths: always accepted, session saved except on logout.
		assert!(basic_auth_accepted(false, true));
		assert!(basic_auth_saves_session(false, no_header, true, false));
		assert!(!basic_auth_saves_session(false, no_header, true, true));

		// Everything else rejects Basic regardless of the header.
		assert!(!basic_auth_accepted(false, false));
	}

	#[test]
	fn test_malformed_basic_credentials_are_unauthorized() {
		let malformed_credentials = vec![
			"not-base64".to_owned(),
			STANDARD.encode("username"),
			STANDARD.encode(":password"),
			STANDARD.encode("username:"),
			STANDARD.encode([0xff_u8, 0xfe]),
		];

		for credentials in malformed_credentials {
			let error = parse_basic_credentials(&credentials).unwrap_err();
			assert_eq!(error.status_code(), StatusCode::UNAUTHORIZED);
		}
	}

	#[test]
	fn test_basic_credential_errors_do_not_echo_secrets() {
		let password = "secret-password";
		let credentials = STANDARD.encode(format!(":{password}"));
		let error = parse_basic_credentials(&credentials).unwrap_err();

		assert_eq!(error.status_code(), StatusCode::UNAUTHORIZED);
		assert!(!error.to_string().contains(password));
	}

	#[test]
	fn test_basic_user_query_filters_soft_deleted_users() {
		use sea_orm::{sea_query::SqliteQueryBuilder, QueryTrait};

		let query = login_user_by_username_query("komelia");
		let sql = query.into_query().to_string(SqliteQueryBuilder);

		assert!(
			sql.contains(r#""users"."deleted_at" IS NULL"#),
			"basic-auth query must exclude soft-deleted users: {sql}"
		);
	}

	#[test]
	fn test_opds_basic_auth_v1_2_into_response() {
		let response =
			OPDSBasicAuth::new("1.2".to_string(), "http://localhost".to_string())
				.into_response();
		assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
		assert_eq!(response.headers().get("Authorization").unwrap(), "Basic");
		assert_eq!(
			response.headers().get("WWW-Authenticate").unwrap(),
			"Basic realm=\"stump OPDS v1.2\""
		);
	}

	#[test]
	fn test_opds_basic_auth_v2_0_into_response() {
		let response =
			OPDSBasicAuth::new("2.0".to_string(), "http://localhost".to_string())
				.into_response();
		assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
		assert_eq!(response.headers().get("Authorization").unwrap(), "Basic");
		assert_eq!(
			response.headers().get("WWW-Authenticate").unwrap(),
			"Basic realm=\"stump OPDS v2.0\""
		);
		assert_eq!(
			response.headers().get("Link").unwrap(),
			"<http://localhost/opds/v2.0/auth>; rel=\"http://opds-spec.org/auth/document\"; type=\"application/opds-authentication+json\""
		);
	}

	/// The cache must only ever skip the bcrypt compare, never the checks that
	/// make a credential invalid: hash rotation, wrong user, or a failed attempt.
	/// One test body: the cache is process-global, so the scenarios must not
	/// run concurrently with each other.
	#[test]
	fn test_basic_auth_cache_safety_and_bound() {
		let _guard = cache_test_guard();
		basic_auth_cache::clear();
		let header = STANDARD.encode("alice:correct");

		assert!(basic_auth_cache::get(&header).is_none());
		basic_auth_cache::insert(&header, "user-1", "$2b$12$hash-v1");

		let hit = basic_auth_cache::get(&header).expect("fresh entry hits");
		assert_eq!(hit.user_id, "user-1");
		// The caller compares this against the user's current hash; a rotated
		// password therefore falls through to bcrypt instead of matching.
		assert_eq!(hit.hashed_password, "$2b$12$hash-v1");

		// Different credentials never share an entry.
		assert!(basic_auth_cache::get(&STANDARD.encode("alice:wrong")).is_none());

		basic_auth_cache::invalidate(&header);
		assert!(basic_auth_cache::get(&header).is_none());

		// Bounded: inserting past capacity evicts rather than grows, and the
		// newest entry always survives.
		for i in 0..300 {
			basic_auth_cache::insert(
				&format!("bounded-{i}"),
				&format!("user-{i}"),
				"hash",
			);
		}
		assert!(basic_auth_cache::len() <= 256);
		assert!(basic_auth_cache::get("bounded-299").is_some());
		basic_auth_cache::clear();
	}

	/// Drives `verify_basic_credentials` against a real (in-memory) schema to
	/// prove the cache never outlives the checks that revoke a credential:
	/// a rotated password hash and an account lock are enforced immediately,
	/// and a wrong password is rejected while the good one is cached.
	// The guard is held across awaits on purpose: it serialises this test
	// against the other cache test, and a current-thread runtime means no
	// other task can be starved by it.
	#[allow(clippy::await_holding_lock)]
	#[tokio::test(flavor = "current_thread")]
	async fn test_basic_auth_cache_respects_hash_rotation_and_lock() {
		use sea_orm::{ActiveModelTrait, Set};

		let _guard = cache_test_guard();
		basic_auth_cache::clear();
		let conn = stump_core::database::connect_at("sqlite::memory:")
			.await
			.unwrap();

		let config = stump_core::config::StumpConfig::debug();
		let hash_v1 = crate::utils::hash_password("pw-v1", &config).unwrap();
		user::ActiveModel {
			id: Set("user-1".to_owned()),
			username: Set("alice".to_owned()),
			hashed_password: Set(hash_v1),
			is_server_owner: Set(false),
			is_locked: Set(false),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();

		let good = STANDARD.encode("alice:pw-v1");
		let wrong = STANDARD.encode("alice:pw-wrong");

		// First call verifies with bcrypt and populates the cache.
		verify_basic_credentials(&good, &conn).await.unwrap();
		assert!(basic_auth_cache::get(&good).is_some());
		// Cached hit still authenticates; a wrong password never does.
		verify_basic_credentials(&good, &conn).await.unwrap();
		assert!(verify_basic_credentials(&wrong, &conn).await.is_err());

		// Rotating the stored hash must invalidate the cached credential.
		let hash_v2 = crate::utils::hash_password("pw-v2", &config).unwrap();
		user::ActiveModel {
			id: Set("user-1".to_owned()),
			hashed_password: Set(hash_v2),
			..Default::default()
		}
		.update(&conn)
		.await
		.unwrap();
		assert!(verify_basic_credentials(&good, &conn).await.is_err());
		assert!(basic_auth_cache::get(&good).is_none());

		// Locking the account is enforced even for a cached credential.
		let new_good = STANDARD.encode("alice:pw-v2");
		verify_basic_credentials(&new_good, &conn).await.unwrap();
		user::ActiveModel {
			id: Set("user-1".to_owned()),
			is_locked: Set(true),
			..Default::default()
		}
		.update(&conn)
		.await
		.unwrap();
		match verify_basic_credentials(&new_good, &conn).await {
			Err(error) => assert_eq!(error.status_code(), StatusCode::FORBIDDEN),
			Ok(_) => panic!("locked account must not authenticate"),
		}
		assert!(basic_auth_cache::get(&new_good).is_none());
		basic_auth_cache::clear();
	}

	#[tokio::test]
	async fn custom_api_key_from_server_owner_never_grants_owner_status() {
		use sea_orm::{ActiveModelTrait, Set};

		let conn = ::tests::db::test_database().await;
		let owner = ::tests::fake_data::User::new("owner").insert(&conn).await;
		let (key, hash) = stump_core::api_key::create_prefixed_key().unwrap();
		let custom_permissions = vec![UserPermission::AccessApiKeys];

		api_key::ActiveModel {
			user_id: Set(owner.id),
			name: Set("restricted owner key".to_string()),
			short_token: Set(key.short_token().to_string()),
			long_token_hash: Set(hash),
			permissions: Set(APIKeyPermissions::Custom(custom_permissions.clone())),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();

		let authenticated_user = validate_api_key(key, &conn).await.unwrap();

		assert!(!authenticated_user.is_server_owner);
		assert_eq!(authenticated_user.permissions, custom_permissions);
		assert_eq!(
			AuthContext {
				user: authenticated_user,
				api_key: Some("restricted-owner-key".to_string()),
				device_id: None,
			}
			.enforce_server_owner(),
			Err(stump_auth::AuthorizationError::ForbiddenAction)
		);
	}
}
