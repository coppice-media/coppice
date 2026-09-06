//! `PluginController` and `AccountController` routes: API-key → JWT exchange
//! and username/password login.

use std::sync::Arc;

use axum::{
	extract::Query,
	http::header,
	response::{IntoResponse, Response},
	routing::{get, post},
	Extension, Json, Router,
};
use models::{
	entity::{age_restriction, user::AuthUser},
	shared::enums::UserPermission,
};
use serde::Deserialize;

use crate::{
	auth::{mint_token, roles_for, ALL_ROLES},
	dto::{
		default_user_preferences, AgeRating, AgeRestrictionDto, IdentityProvider,
		KavitaDateTime, LoginDto, UserDto,
	},
	errors::{APIError, APIResult},
	ids::{IdKind, KavitaIds},
	KAVITA_VERSION,
};
use stump_auth::AuthContext;

use super::{route_ci, KavitaBackend};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginAuthQuery {
	#[serde(default)]
	pub api_key: Option<String>,
	#[serde(default)]
	pub plugin_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyQuery {
	#[serde(default)]
	pub api_key: Option<String>,
}

/// How a login resolved, so the response can echo the credential used.
#[derive(Debug, Clone)]
pub struct LoginOutcome {
	pub user: AuthUser,
	pub api_key: Option<String>,
}

pub(crate) fn public_routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(
		router,
		"/api/Plugin/authenticate",
		post(plugin_authenticate),
	);
	let router = route_ci(router, "/api/Plugin/version", get(plugin_version));
	route_ci(router, "/api/Account/login", post(account_login))
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(router, "/api/Account", get(account_current));
	route_ci(router, "/api/Account/roles", get(account_roles))
}

/// `AccountController.GetCurrentUser`: the full `UserDto` of the
/// authenticated user, without a token. Stump cannot reproduce
/// `UserDto.apiKey` from the hashed key store, so it is only echoed when the
/// request itself authenticated with the key.
async fn account_current(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<Json<UserDto>> {
	let mut dto = user_dto(ctx.as_ref(), auth.user(), auth.api_key(), true).await?;
	dto.token = None;
	dto.refresh_token = None;
	Ok(Json(dto))
}

/// `AccountController.GetRoles`: the full Kavita `PolicyConstants` role list.
async fn account_roles() -> Json<Vec<String>> {
	Json(ALL_ROLES.iter().map(|role| (*role).to_owned()).collect())
}

async fn plugin_authenticate(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Query(query): Query<PluginAuthQuery>,
) -> APIResult<Json<UserDto>> {
	let api_key = query
		.api_key
		.filter(|key| !key.trim().is_empty())
		.ok_or_else(|| {
			APIError::BadRequest("The apiKey field is required.".to_owned())
		})?;
	let plugin_name = query.plugin_name.unwrap_or_default();
	let user = ctx.authenticate_api_key(&api_key).await.map_err(|error| {
		tracing::info!(
			plugin = %plugin_name,
			"A Kavita plugin tried to authenticate with an apiKey that doesn't match"
		);
		error
	})?;
	tracing::info!(plugin = %plugin_name, username = %user.username, "Kavita plugin authenticated");
	let dto = user_dto(ctx.as_ref(), user, Some(api_key), false).await?;
	Ok(Json(dto))
}

async fn plugin_version(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Query(query): Query<ApiKeyQuery>,
) -> APIResult<Response> {
	let api_key = query
		.api_key
		.filter(|key| !key.trim().is_empty())
		.ok_or_else(|| {
			APIError::BadRequest("The apiKey field is required.".to_owned())
		})?;
	ctx.authenticate_api_key(&api_key).await?;
	Ok((
		[(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
		KAVITA_VERSION,
	)
		.into_response())
}

async fn account_login(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Json(login): Json<LoginDto>,
) -> APIResult<Json<UserDto>> {
	let outcome = login_user(ctx.as_ref(), &login).await?;
	let dto = user_dto(ctx.as_ref(), outcome.user, outcome.api_key, true).await?;
	Ok(Json(dto))
}

/// `AccountController.Login`: an API key wins over username/password.
pub(crate) async fn login_user(
	ctx: &dyn KavitaBackend,
	login: &LoginDto,
) -> APIResult<LoginOutcome> {
	if let Some(api_key) = login
		.api_key
		.as_deref()
		.map(str::trim)
		.filter(|key| !key.is_empty())
	{
		let user = ctx.authenticate_api_key(api_key).await?;
		return Ok(LoginOutcome {
			user,
			api_key: Some(api_key.to_owned()),
		});
	}
	let username = login.username.as_deref().unwrap_or_default().trim();
	let password = login.password.as_deref().unwrap_or_default();
	if username.is_empty() || password.is_empty() {
		return Err(APIError::Unauthorized);
	}
	let user = ctx.authenticate_password(username, password).await?;
	Ok(LoginOutcome {
		user,
		api_key: None,
	})
}

fn age_restriction_dto(
	restriction: Option<&age_restriction::Model>,
) -> AgeRestrictionDto {
	match restriction {
		Some(restriction) => AgeRestrictionDto {
			age_rating: AgeRating::from_min_age(Some(restriction.age)),
			include_unknowns: !restriction.restrict_on_unset,
		},
		None => AgeRestrictionDto {
			age_rating: AgeRating::NotApplicable,
			include_unknowns: false,
		},
	}
}

/// Build the `UserDto` for a login. `full` selects the `Account/login` shape
/// (preferences, age restriction, roles) over the slimmer
/// `Plugin/authenticate` one; both carry a freshly minted token.
pub(crate) async fn user_dto(
	ctx: &dyn KavitaBackend,
	user: AuthUser,
	api_key: Option<String>,
	full: bool,
) -> APIResult<UserDto> {
	let kavita_user_id = KavitaIds::resolve(ctx.conn(), IdKind::User, &user.id).await?;
	let roles = roles_for(
		user.is_server_owner,
		user.has_permission(UserPermission::DownloadFile),
	);
	let secret = ctx.token_secret().await?;
	let token = mint_token(
		&secret,
		&user.username,
		kavita_user_id,
		&roles,
		api_key.as_deref(),
	)
	.map_err(|error| APIError::InternalServerError(error.to_string()))?;
	Ok(UserDto {
		id: kavita_user_id,
		oidc_id: None,
		username: user.username.clone(),
		email: None,
		roles: if full { roles } else { Vec::new() },
		token: Some(token),
		refresh_token: None,
		api_key,
		preferences: full.then(default_user_preferences),
		age_restriction: full.then(|| age_restriction_dto(user.age_restriction.as_ref())),
		kavita_version: Some(KAVITA_VERSION.to_owned()),
		identity_provider: IdentityProvider::Kavita,
		created: KavitaDateTime::default(),
		created_utc: KavitaDateTime::default(),
		auth_keys: full.then(Vec::new),
		cover_image: None,
		primary_color: None,
		secondary_color: None,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_support::{auth_user, db, TestBackend};
	use ::tests::fake_data;
	use models::entity::user;

	/// The `UserDto` wire shape must match Kavita's field set: Kamigura decodes
	/// `username`/`roles`/`token` and Inkita reads the login record as-is.
	#[tokio::test]
	async fn user_dto_carries_the_kavita_field_set() {
		let conn = db().await;
		let user_row = fake_data::User::new("kate").insert(&conn).await;
		let backend = TestBackend::new(conn);
		let dto = user_dto(
			&backend,
			auth_user(&user_row),
			Some("stump_key".to_owned()),
			true,
		)
		.await
		.unwrap();

		assert_eq!(dto.username, "kate");
		assert_eq!(dto.api_key.as_deref(), Some("stump_key"));
		assert!(dto.token.is_some(), "login mints a fresh token");
		assert_eq!(dto.kavita_version.as_deref(), Some(KAVITA_VERSION));
		assert!(dto.roles.contains(&"Login".to_owned()));
		assert!(dto.roles.contains(&"Admin".to_owned()));
		assert!(dto.preferences.is_some());
		assert!(dto.age_restriction.is_some());

		let value = serde_json::to_value(&dto).unwrap();
		for key in [
			"id",
			"oidcId",
			"username",
			"email",
			"roles",
			"token",
			"refreshToken",
			"apiKey",
			"preferences",
			"ageRestriction",
			"kavitaVersion",
			"identityProvider",
			"created",
			"createdUtc",
			"authKeys",
			"coverImage",
			"primaryColor",
			"secondaryColor",
		] {
			assert!(value.get(key).is_some(), "missing UserDto key {key}");
		}
		assert_eq!(value["preferences"]["locale"], "en");
		assert_eq!(value["ageRestriction"]["ageRating"], -1);
		let _ = user::Entity;
	}

	#[test]
	fn roles_route_reports_the_full_policy_list() {
		assert_eq!(ALL_ROLES.len(), 9);
		assert!(ALL_ROLES.contains(&"Admin"));
		assert!(ALL_ROLES.contains(&"Login"));
		assert!(ALL_ROLES.contains(&"Download"));
	}
}
