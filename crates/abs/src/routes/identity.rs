//! `POST /login`, `POST /auth/refresh`, `POST /api/authorize` and the three
//! unauthenticated probes a client hits before any of them.
//!
//! Lissen's login sequence is `GET /status` to pick an auth method
//! (`common/api/AudiobookshelfAuthService.kt:107-146`), then `POST /login`
//! with `x-return-tokens: true` (`AudiobookshelfApiClient.kt:164`), then
//! `POST /api/authorize` on every later app start to re-read the server
//! version (`:59`), and `POST /auth/refresh` with the refresh token in
//! `x-refresh-token` when the access token expires (`:170`).

use axum::{
	extract::Json,
	http::{HeaderMap, StatusCode},
	response::{IntoResponse, Response},
	Extension,
};
use serde::Deserialize;

use crate::{
	auth::{mint_tokens, verify_token, TokenType},
	dto::{LoginResponseDto, PingDto},
	errors::{AbsError, AbsResult},
	mapper::{self, TokenPresentation},
	routes::{me, AbsBackend, Backend, User},
};

/// The header a client sets to be handed a refresh token. abs-ref answers
/// `refreshToken: null` without it, and Lissen always sets it.
const RETURN_TOKENS_HEADER: &str = "x-return-tokens";
const REFRESH_TOKEN_HEADER: &str = "x-refresh-token";

#[derive(Debug, Deserialize)]
pub(crate) struct LoginRequest {
	username: String,
	password: String,
}

pub(crate) async fn ping() -> Json<PingDto> {
	Json(PingDto { success: true })
}

/// `GET /healthcheck` → `200` with the bare reason phrase
/// (`capture/healthcheck.txt`).
pub(crate) async fn healthcheck() -> Response {
	crate::routes::ok_text()
}

pub(crate) async fn status() -> Json<crate::dto::StatusDto> {
	Json(mapper::status())
}

/// `POST /logout`. The official app calls it when a server connection is
/// removed (`store/user.js:158`) and ignores the body; abs-ref answers
/// `{"redirect_url": null}`, which is what an OpenID install would fill in.
///
/// The profile mints stateless JWTs and keeps no server-side session to
/// destroy, so this is an acknowledgement, not a revocation — a token stays
/// valid until it expires. It is public for the same reason: a client whose
/// token has already expired must still be able to log out of it.
pub(crate) async fn logout() -> Json<crate::dto::LogoutDto> {
	Json(crate::dto::LogoutDto { redirect_url: None })
}

fn wants_tokens(headers: &HeaderMap) -> bool {
	headers
		.get(RETURN_TOKENS_HEADER)
		.and_then(|value| value.to_str().ok())
		.map(|value| value.eq_ignore_ascii_case("true"))
		.unwrap_or(false)
}

/// Build the login envelope: the user object, the server settings and the
/// default library.
///
/// The user is re-read here rather than passed in, because
/// `POST /auth/refresh` has nothing but a user id to go on and the other two
/// callers need `createdAt` from the same row anyway.
async fn envelope(
	backend: &dyn AbsBackend,
	user_id: &str,
	device: Option<&str>,
	tokens: TokenMode,
	return_tokens: bool,
) -> AbsResult<LoginResponseDto> {
	let (user, created_at) = backend.user(user_id).await?;
	if user.is_locked {
		return Err(AbsError::Unauthorized);
	}

	let secret = backend.token_secret().await?;
	let minted = mint_tokens(&secret, &user.id, &user.username, device)?;
	let refresh = return_tokens.then_some(minted.refresh_token);
	let presentation = match tokens {
		TokenMode::Login => TokenPresentation::Login {
			access: minted.access_token.clone(),
			refresh,
		},
		TokenMode::Refresh => TokenPresentation::Refresh {
			access: minted.access_token.clone(),
			refresh,
		},
		TokenMode::LegacyOnly => TokenPresentation::LegacyOnly,
	};

	let user_dto = mapper::user_dto(mapper::UserInput {
		user: &user,
		created_at,
		legacy_token: minted.token,
		tokens: presentation,
		media_progress: me::media_progress(backend, &user).await?,
		bookmarks: me::bookmarks(backend, &user).await?,
		libraries_accessible: user.device_library_scope.clone().unwrap_or_default(),
	});

	Ok(mapper::login_response(
		user_dto,
		me::default_library_id(backend, &user).await?,
		backend.is_docker(),
	))
}

/// Which of the three envelopes is being built.
#[derive(Debug, Clone, Copy)]
enum TokenMode {
	Login,
	Refresh,
	LegacyOnly,
}

pub(crate) async fn login(
	backend: Backend,
	headers: HeaderMap,
	Json(body): Json<LoginRequest>,
) -> AbsResult<Response> {
	let (user, device) = backend
		.authenticate_password(&body.username, &body.password)
		.await?;
	let envelope = envelope(
		&**backend,
		&user.id,
		device.as_deref(),
		TokenMode::Login,
		wants_tokens(&headers),
	)
	.await?;
	Ok((StatusCode::OK, Json(envelope)).into_response())
}

/// `POST /auth/refresh`. The refresh token arrives in `x-refresh-token`, not
/// in the body and not as a bearer, and must be a token of the refresh kind:
/// accepting an access token here would silently extend its lifetime by 30
/// days.
pub(crate) async fn refresh(backend: Backend, headers: HeaderMap) -> AbsResult<Response> {
	let token = headers
		.get(REFRESH_TOKEN_HEADER)
		.and_then(|value| value.to_str().ok())
		.ok_or(AbsError::Unauthorized)?;
	let secret = backend.token_secret().await?;
	let claims = verify_token(&secret, token).map_err(|_| AbsError::Unauthorized)?;
	if claims.token_type != Some(TokenType::Refresh) {
		return Err(AbsError::Unauthorized);
	}

	let envelope = envelope(
		&**backend,
		&claims.user_id,
		claims.device.as_deref(),
		TokenMode::Refresh,
		wants_tokens(&headers),
	)
	.await?;
	Ok((StatusCode::OK, Json(envelope)).into_response())
}

/// `POST /api/authorize`: the envelope for a client that already holds a
/// working credential, carrying `token` alone — abs-ref omits `accessToken`
/// and `refreshToken` here (`capture/authorize.json`). Lissen calls it on
/// every app start to re-read `serverSettings.version` and only needs
/// `user.username` (`common/model/connection/ConnectionInfoResponse.kt`).
pub(crate) async fn authorize(
	backend: Backend,
	Extension(user): User,
) -> AbsResult<Response> {
	let envelope =
		envelope(&**backend, &user.id, None, TokenMode::LegacyOnly, false).await?;
	Ok((StatusCode::OK, Json(envelope)).into_response())
}
