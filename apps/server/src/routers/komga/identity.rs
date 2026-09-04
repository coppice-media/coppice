use std::collections::BTreeSet;

use axum::{
	body::Body,
	extract::{Query, State},
	http::{header, HeaderMap, HeaderValue, StatusCode},
	response::Response,
	routing::{get, head, post},
	Extension, Router,
};
use chrono::{Duration as ChronoDuration, Utc};
use models::{
	entity::{
		library, library_exclusion, session as session_entity,
		user::{self, AuthUser},
	},
	shared::enums::UserPermission,
};
use sea_orm::{prelude::*, ColumnTrait, QuerySelect};
use serde::Deserialize;
use stump_auth::AuthContext;
use tower_sessions::{
	cookie::{Cookie, SameSite},
	Session,
};

use crate::{
	config::{
		session::{delete_cookie_header, StumpSessionStore},
		state::AppState,
	},
	errors::{APIError, APIResult},
	middleware::auth::{
		komga_remember_me_cookie, KomgaBasicAuthSuccess, KOMGA_REMEMBER_ME_COOKIE_NAME,
	},
	routers::enforce_max_sessions,
};
use stump_komga::routes::response::cached_json;
use stump_komga::{
	user::{
		AllowExclude, KomgaAgeRestriction, ROLE_ADMIN, ROLE_FILE_DOWNLOAD,
		ROLE_PAGE_STREAMING, ROLE_USER,
	},
	KomgaLibraryId, KomgaUser, KomgaUserId,
};

const ROLE_KOBO_SYNC: &str = "KOBO_SYNC";
const ROLE_KOREADER_SYNC: &str = "KOREADER_SYNC";

/// Komga's `komga.remember-me.validity` default is `P14D` (14 days).
const KOMGA_REMEMBER_ME_VALIDITY_DAYS: i64 = 14;

#[derive(Debug, Default, Deserialize)]
struct CurrentUserQuery {
	#[serde(rename = "remember-me", default)]
	remember_me: bool,
}

fn build_komga_remember_me_cookie(token: String) -> Cookie<'static> {
	Cookie::build((KOMGA_REMEMBER_ME_COOKIE_NAME, token))
		.path("/")
		.http_only(true)
		.same_site(SameSite::Lax)
		.max_age(time::Duration::days(KOMGA_REMEMBER_ME_VALIDITY_DAYS))
		.build()
}
fn build_komga_remember_me_clear_cookie() -> Cookie<'static> {
	Cookie::build((KOMGA_REMEMBER_ME_COOKIE_NAME, ""))
		.path("/")
		.http_only(true)
		.same_site(SameSite::Lax)
		.max_age(time::Duration::ZERO)
		.build()
}

/// Routes that Komelia probes before authenticating.
pub(crate) fn public_routes() -> Router<AppState> {
	Router::new().route("/login", head(login_probe))
}

pub(crate) fn routes() -> Router<AppState> {
	routes_without_current_user().route("/api/v2/users/me", get(current_user))
}

/// Identity routes minus `GET /api/v2/users/me`, which Grimmory's shim replaces
/// with its own response shape under the `/komga` alias.
pub(crate) fn routes_without_current_user() -> Router<AppState> {
	Router::new().route("/api/logout", post(logout))
}

async fn login_probe() -> StatusCode {
	StatusCode::OK
}
async fn logout(
	State(ctx): State<AppState>,
	session: Session,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	if let Some(token) = komga_remember_me_cookie(&headers) {
		delete_komga_remember_me_session(token, ctx.conn.as_ref()).await?;
	}

	session.delete().await?;

	// Always clear both cookies. tower-sessions only emits its own clear when
	// it still holds a loaded session at response time; after `delete()` it
	// does not, so the explicit clear is the only one the client receives.
	let (session_cookie_name, session_cookie) = delete_cookie_header();
	Response::builder()
		.status(StatusCode::NO_CONTENT)
		.header(session_cookie_name, session_cookie)
		.header(
			header::SET_COOKIE,
			build_komga_remember_me_clear_cookie().to_string(),
		)
		.body(Body::empty())
		.map_err(|error| APIError::InternalServerError(error.to_string()))
}

async fn delete_komga_remember_me_session(
	token: &str,
	conn: &DatabaseConnection,
) -> APIResult<()> {
	session_entity::Entity::delete_many()
		.filter(session_entity::Column::SessionId.eq(token))
		.exec(conn)
		.await?;
	Ok(())
}

async fn current_user(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<CurrentUserQuery>,
	_basic_auth: Option<Extension<KomgaBasicAuthSuccess>>,
	headers: HeaderMap,
) -> APIResult<axum::response::Response<Body>> {
	let user = auth.user();
	let remember_me_cookie = if query.remember_me && _basic_auth.is_some() {
		let login_user = user::LoginUser::find_by_id(user.id.clone())
			.filter(user::Column::DeletedAt.is_null())
			.into_model::<user::LoginUser>()
			.one(ctx.conn.as_ref())
			.await?
			.ok_or(APIError::Unauthorized)?;
		if login_user.is_locked {
			return Err(APIError::Unauthorized);
		}

		let expiry_time: DateTimeWithTimeZone =
			(Utc::now() + ChronoDuration::days(KOMGA_REMEMBER_ME_VALIDITY_DAYS)).into();
		let store = StumpSessionStore::new(ctx.conn.clone(), ctx.config.clone());
		let token = store.create_for_user(&login_user.id, expiry_time).await?;
		enforce_max_sessions(&login_user, ctx.conn.as_ref()).await?;
		Some(build_komga_remember_me_cookie(token).to_string())
	} else {
		None
	};

	let oidc_email = user::Entity::find_by_id(user.id.clone())
		.select_only()
		.column(user::Column::OidcEmail)
		.into_tuple::<Option<String>>()
		.one(ctx.conn.as_ref())
		.await?
		.flatten();

	let (shared_all_libraries, shared_libraries_ids) =
		shared_libraries(&ctx, &user).await?;
	let roles = roles_for_user(&user);
	let age_restriction = age_restriction_for_user(&user);
	let komga_user = KomgaUser {
		id: KomgaUserId::from(user.id),
		email: oidc_email.unwrap_or(user.username),
		roles,
		shared_all_libraries,
		shared_libraries_ids,
		// Stump has no sharing labels; Komga clients treat empty sets as "no
		// restriction", which matches the library visibility already applied.
		labels_allow: Default::default(),
		labels_exclude: Default::default(),
		age_restriction,
	};

	let mut response = cached_json(&headers, &komga_user)?;
	if let Some(cookie) = remember_me_cookie {
		let cookie = HeaderValue::from_str(&cookie)
			.map_err(|error| APIError::InternalServerError(error.to_string()))?;
		response.headers_mut().append(header::SET_COOKIE, cookie);
	}

	Ok(response)
}

pub(super) fn roles_for_user(user: &AuthUser) -> BTreeSet<String> {
	let mut roles =
		BTreeSet::from([ROLE_USER.to_owned(), ROLE_PAGE_STREAMING.to_owned()]);

	if user.is_server_owner || user.permissions.contains(&UserPermission::DownloadFile) {
		roles.insert(ROLE_FILE_DOWNLOAD.to_owned());
	}
	if user.is_server_owner || user.permissions.contains(&UserPermission::AccessKoboSync)
	{
		roles.insert(ROLE_KOBO_SYNC.to_owned());
	}
	if user.is_server_owner
		|| user
			.permissions
			.contains(&UserPermission::AccessKoreaderSync)
	{
		roles.insert(ROLE_KOREADER_SYNC.to_owned());
	}
	if user.is_server_owner {
		roles.insert(ROLE_ADMIN.to_owned());
	}

	roles
}
pub(super) fn age_restriction_for_user(user: &AuthUser) -> Option<KomgaAgeRestriction> {
	user.age_restriction
		.as_ref()
		.map(|restriction| KomgaAgeRestriction {
			age: restriction.age,
			restriction: if restriction.restrict_on_unset {
				AllowExclude::AllowOnly
			} else {
				AllowExclude::Exclude
			},
		})
}

pub(super) async fn shared_libraries(
	ctx: &AppState,
	user: &AuthUser,
) -> APIResult<(bool, BTreeSet<KomgaLibraryId>)> {
	if user.is_server_owner {
		return Ok((true, BTreeSet::new()));
	}

	let hidden_library_ids = library_exclusion::Entity::find()
		.select_only()
		.column(library_exclusion::Column::LibraryId)
		.filter(library_exclusion::Column::UserId.eq(user.id.clone()))
		.into_tuple::<String>()
		.all(ctx.conn.as_ref())
		.await?;

	if hidden_library_ids.is_empty() {
		return Ok((true, BTreeSet::new()));
	}

	let visible_library_ids = library::Entity::find_for_user(user)
		.select_only()
		.column(library::Column::Id)
		.into_tuple::<String>()
		.all(ctx.conn.as_ref())
		.await?
		.into_iter()
		.map(KomgaLibraryId::from)
		.collect();

	Ok((false, visible_library_ids))
}

#[cfg(test)]
mod tests {
	use super::*;
	use models::entity::age_restriction;

	fn user(is_server_owner: bool, permissions: Vec<UserPermission>) -> AuthUser {
		AuthUser {
			id: "user-id".to_owned(),
			username: "reader".to_owned(),
			is_server_owner,
			permissions,
			..AuthUser::default()
		}
	}

	#[test]
	fn owner_receives_administrator_and_optional_protocol_roles() {
		let roles = roles_for_user(&user(true, Vec::new()));

		assert_eq!(roles.len(), 6);
		assert!(roles.contains(ROLE_USER));
		assert!(roles.contains(ROLE_PAGE_STREAMING));
		assert!(roles.contains(ROLE_ADMIN));
		assert!(roles.contains(ROLE_FILE_DOWNLOAD));
		assert!(roles.contains(ROLE_KOBO_SYNC));
		assert!(roles.contains(ROLE_KOREADER_SYNC));
	}

	#[test]
	fn restricted_user_receives_only_matching_protocol_roles() {
		let roles = roles_for_user(&user(
			false,
			vec![UserPermission::DownloadFile, UserPermission::AccessKoboSync],
		));

		assert_eq!(roles.len(), 4);
		assert!(roles.contains(ROLE_USER));
		assert!(roles.contains(ROLE_PAGE_STREAMING));
		assert!(roles.contains(ROLE_FILE_DOWNLOAD));
		assert!(roles.contains(ROLE_KOBO_SYNC));
		assert!(!roles.contains(ROLE_ADMIN));
		assert!(!roles.contains(ROLE_KOREADER_SYNC));
	}

	#[test]
	fn remember_me_cookie_uses_komga_attributes_and_lifetime() {
		let cookie = build_komga_remember_me_cookie("opaque-token".to_owned());

		assert_eq!(cookie.name(), KOMGA_REMEMBER_ME_COOKIE_NAME);
		assert_eq!(cookie.value(), "opaque-token");
		assert_eq!(cookie.path(), Some("/"));
		assert_eq!(cookie.http_only(), Some(true));
		assert_eq!(cookie.same_site(), Some(SameSite::Lax));
		assert_eq!(
			cookie.max_age(),
			Some(time::Duration::days(KOMGA_REMEMBER_ME_VALIDITY_DAYS))
		);
	}

	#[test]
	fn age_restriction_maps_to_komga_wire_values() {
		let mut user = user(false, Vec::new());
		user.age_restriction = Some(age_restriction::Model {
			id: 1,
			age: 13,
			restrict_on_unset: true,
			user_id: user.id.clone(),
		});
		let mapped = age_restriction_for_user(&user).expect("restriction should map");
		assert_eq!(mapped.age, 13);
		assert_eq!(mapped.restriction, AllowExclude::AllowOnly);

		user.age_restriction.as_mut().unwrap().restrict_on_unset = false;
		assert_eq!(
			age_restriction_for_user(&user)
				.expect("restriction should map")
				.restriction,
			AllowExclude::Exclude
		);
	}

	#[test]
	fn logout_clears_session_and_remember_me_cookies() {
		let (_, session_cookie) = delete_cookie_header();
		assert!(session_cookie.starts_with("stump_session=;"));
		let remember_cookie = build_komga_remember_me_clear_cookie().to_string();
		assert!(remember_cookie.starts_with("komga-remember-me=;"));
		assert!(remember_cookie.contains("Max-Age=0"));
	}

	#[tokio::test]
	async fn logout_deletes_current_and_remember_me_rows() {
		use sea_orm::{
			ActiveModelTrait, ConnectionTrait, Database, DatabaseBackend, EntityTrait,
			Set, Statement,
		};
		use tower_sessions::session::Id;

		let conn = std::sync::Arc::new(
			Database::connect("sqlite::memory:")
				.await
				.expect("database should connect"),
		);
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
		.expect("sessions table should be created");

		let current_id = Id::default();
		let remember_id = Id::default();
		for session_id in [current_id.to_string(), remember_id.to_string()] {
			session_entity::ActiveModel {
				session_id: Set(session_id),
				user_id: Set("user-id".to_owned()),
				expiry_time: Set((Utc::now() + ChronoDuration::days(1)).into()),
				..Default::default()
			}
			.insert(conn.as_ref())
			.await
			.expect("session row should be inserted");
		}

		let store = StumpSessionStore::new(
			conn.clone(),
			std::sync::Arc::new(stump_core::config::StumpConfig::debug()),
		);
		let session = Session::new(Some(current_id), std::sync::Arc::new(store), None);
		delete_komga_remember_me_session(&remember_id.to_string(), conn.as_ref())
			.await
			.expect("remember-me row should be deleted");
		session
			.delete()
			.await
			.expect("current row should be deleted");

		assert_eq!(
			session_entity::Entity::find()
				.count(conn.as_ref())
				.await
				.expect("session rows should be countable"),
			0
		);
	}

	#[test]
	fn komga_user_serializes_and_deserializes_with_typed_contract() {
		let original = KomgaUser {
			id: KomgaUserId::from("user-id"),
			email: "reader@example.com".to_owned(),
			roles: roles_for_user(&user(false, Vec::new())),
			shared_all_libraries: true,
			shared_libraries_ids: BTreeSet::new(),
			labels_allow: BTreeSet::new(),
			labels_exclude: BTreeSet::new(),
			age_restriction: None,
		};
		let serialized =
			serde_json::to_string(&original).expect("Komga user should serialize");
		let parsed: KomgaUser =
			serde_json::from_str(&serialized).expect("Komelia DTO should parse");

		assert_eq!(parsed, original);
	}
}
