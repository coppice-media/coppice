use axum::{
	body::Body,
	extract::{Path, State},
	http::{HeaderMap, StatusCode},
	response::Response,
	routing::{get, patch},
	Extension, Json, Router,
};
use axum_extra::extract::Query;
use models::entity::{age_restriction, user, user_login_activity, user_preferences};
use sea_orm::{
	prelude::*, ActiveValue::NotSet, DatabaseTransaction, IntoActiveModel, QueryOrder,
	QuerySelect, Set, TransactionTrait,
};
use serde::Deserialize;
use stump_auth::AuthContext;
use stump_komga::{
	user::{
		AllowExclude, KomgaAuthenticationActivity, KomgaPasswordUpdateRequest, KomgaUser,
		KomgaUserCreateRequest, KomgaUserUpdateRequest,
	},
	KomgaAnnouncementId, KomgaJsonFeed, KomgaSettings, KomgaSettingsUpdateRequest,
	KomgaThumbnailSize, Page, PatchValue, SettingMultiSource,
};

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	utils::hash_password,
};

/// The extracted crate's ETag-aware JSON response, lifted into the server's
/// error type for the server-local Komga routes.
fn cached_json<T: serde::Serialize>(
	headers: &HeaderMap,
	value: &T,
) -> APIResult<Response<Body>> {
	stump_komga::routes::response::cached_json(headers, value).map_err(APIError::from)
}

const DEFAULT_PAGE_SIZE: i32 = 20;
const MAX_PAGE_SIZE: i32 = 200;
const SECONDS_PER_DAY: i64 = 86_400;

pub(crate) fn routes() -> Router<AppState> {
	Router::new()
		.route("/api/v1/settings", get(get_settings).patch(patch_settings))
		.route(
			"/api/v1/announcements",
			get(get_announcements).put(put_announcements),
		)
		.route("/api/v2/users", get(get_users).post(create_user))
		.route(
			"/api/v2/users/me/password",
			patch(update_current_user_password),
		)
		.route("/api/v2/users/{id}", patch(update_user).delete(delete_user))
		.route("/api/v2/users/{id}/password", patch(update_user_password))
		.route(
			"/api/v2/users/me/authentication-activity",
			get(get_authentication_activity),
		)
		.route(
			"/api/v2/users/authentication-activity",
			get(get_all_authentication_activity),
		)
		.route(
			"/api/v2/users/{id}/authentication-activity/latest",
			get(get_latest_authentication_activity),
		)
}

async fn get_settings(
	State(ctx): State<AppState>,
	Extension(_auth): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let remember_me_duration_days =
		i32::try_from(ctx.config.session_ttl / SECONDS_PER_DAY)?;
	let server_port = i32::from(ctx.config.port);
	let settings = KomgaSettings {
		// Stump does not have Komga's empty-collection deletion setting.
		delete_empty_collections: false,
		// Stump does not have Komga's empty-read-list deletion setting.
		delete_empty_read_lists: false,
		remember_me_duration_days,
		// Stump has no Komga thumbnail-size preference; DEFAULT is the neutral value.
		thumbnail_size: KomgaThumbnailSize::Default,
		// Stump has no configurable Komga task pool; one is the neutral value.
		task_pool_size: 1,
		server_port: SettingMultiSource {
			configuration_source: Some(server_port),
			database_source: None,
			effective_value: Some(server_port),
		},
		// Stump has no persisted server context path, so every source is absent.
		server_context_path: SettingMultiSource {
			configuration_source: None,
			database_source: None,
			effective_value: None,
		},
	};

	cached_json(&headers, &settings)
}
fn ensure_server_owner(auth: &AuthContext) -> APIResult<()> {
	if auth.user().is_server_owner {
		Ok(())
	} else {
		Err(APIError::forbidden_discreet())
	}
}

async fn get_users(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	ensure_server_owner(&auth)?;

	let users = user::LoginUser::find()
		.filter(user::Column::DeletedAt.is_null())
		.order_by_asc(user::Column::Username)
		.into_model::<user::LoginUser>()
		.all(ctx.conn.as_ref())
		.await?;
	let mut komga_users = Vec::with_capacity(users.len());

	for user in users {
		let email = user
			.oidc_email
			.clone()
			.unwrap_or_else(|| user.username.clone());
		let auth_user = user::AuthUser::from(user);
		let (shared_all_libraries, shared_libraries_ids) =
			super::identity::shared_libraries(&ctx, &auth_user).await?;
		komga_users.push(KomgaUser {
			id: auth_user.id.clone().into(),
			email,
			roles: super::identity::roles_for_user(&auth_user),
			shared_all_libraries,
			shared_libraries_ids,
			labels_allow: Default::default(),
			labels_exclude: Default::default(),
			age_restriction: super::identity::age_restriction_for_user(&auth_user),
		});
	}

	cached_json(&headers, &komga_users)
}

async fn create_user(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Json(request): Json<KomgaUserCreateRequest>,
) -> APIResult<(StatusCode, Json<KomgaUser>)> {
	ensure_server_owner(&auth)?;
	if request.email.is_empty() {
		return Err(APIError::BadRequest("email must not be empty".to_owned()));
	}
	if request.password.is_empty() {
		return Err(APIError::BadRequest(
			"password must not be empty".to_owned(),
		));
	}

	let conn = ctx.conn.as_ref();
	let email = request.email;
	if user::Entity::find()
		.filter(user::Column::Username.eq(email.clone()))
		.one(conn)
		.await?
		.is_some()
	{
		return Err(APIError::BadRequest(
			"A user with this email already exists".to_owned(),
		));
	}

	let hashed_password = hash_password(&request.password, &ctx.config)?;
	let txn = conn.begin().await?;
	let created_user = user::ActiveModel {
		username: Set(email.clone()),
		hashed_password: Set(hashed_password),
		is_server_owner: Set(false),
		permissions: Set(None),
		..Default::default()
	}
	.insert(&txn)
	.await?;

	let created_user_id = created_user.id.clone();
	let preferences = user_preferences::ActiveModel {
		user_id: Set(Some(created_user_id.clone())),
		..Default::default()
	}
	.insert(&txn)
	.await?;

	let mut created_user = created_user.into_active_model();
	created_user.user_preferences_id = Set(Some(preferences.id));
	created_user.update(&txn).await?;
	txn.commit().await?;

	let login_user = user::LoginUser::find_by_id(created_user_id)
		.into_model::<user::LoginUser>()
		.one(conn)
		.await?
		.ok_or_else(|| {
			APIError::InternalServerError("Failed to fetch created user".to_owned())
		})?;
	let auth_user = user::AuthUser::from(login_user);
	let (shared_all_libraries, shared_libraries_ids) =
		super::identity::shared_libraries(&ctx, &auth_user).await?;

	// Komga roles have no one-to-one mapping to Stump permissions. In
	// particular, ADMIN must never escalate a newly-created Stump user.
	Ok((
		StatusCode::CREATED,
		Json(KomgaUser {
			id: auth_user.id.clone().into(),
			email,
			roles: super::identity::roles_for_user(&auth_user),
			shared_all_libraries,
			shared_libraries_ids,
			labels_allow: Default::default(),
			labels_exclude: Default::default(),
			age_restriction: super::identity::age_restriction_for_user(&auth_user),
		}),
	))
}

async fn update_current_user_password(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Json(request): Json<KomgaPasswordUpdateRequest>,
) -> APIResult<StatusCode> {
	let user_id = auth.user().id.clone();
	update_password_for_user(&ctx, &auth, &user_id, request.password).await
}

async fn update_user_password(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	Json(request): Json<KomgaPasswordUpdateRequest>,
) -> APIResult<StatusCode> {
	update_password_for_user(&ctx, &auth, &id, request.password).await
}

async fn update_password_for_user(
	ctx: &AppState,
	auth: &AuthContext,
	target_id: &str,
	password: String,
) -> APIResult<StatusCode> {
	if !can_change_password(auth, target_id) {
		return Err(APIError::forbidden_discreet());
	}
	if password.is_empty() {
		return Err(APIError::BadRequest(
			"password must not be empty".to_owned(),
		));
	}

	let target = user::Entity::find_by_id(target_id.to_owned())
		.filter(user::Column::DeletedAt.is_null())
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| APIError::NotFound("User not found".to_owned()))?;
	let hashed_password = hash_password(&password, &ctx.config)?;
	let mut target = target.into_active_model();
	target.hashed_password = Set(hashed_password);
	target.update(ctx.conn.as_ref()).await?;

	Ok(StatusCode::NO_CONTENT)
}

fn can_change_password(auth: &AuthContext, target_id: &str) -> bool {
	target_id == auth.user().id || auth.user().is_server_owner
}

async fn update_user(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	Json(patch): Json<KomgaUserUpdateRequest>,
) -> APIResult<StatusCode> {
	ensure_server_owner(&auth)?;
	validate_user_update(&patch)?;

	let exists = user::Entity::find_by_id(id.clone())
		.filter(user::Column::DeletedAt.is_null())
		.one(ctx.conn.as_ref())
		.await?
		.is_some();
	if !exists {
		return Err(APIError::NotFound("User not found".to_owned()));
	}

	let txn = ctx.conn.as_ref().begin().await?;
	apply_age_restriction(&txn, &id, &patch.age_restriction).await?;
	txn.commit().await?;
	Ok(StatusCode::NO_CONTENT)
}

fn validate_user_update(patch: &KomgaUserUpdateRequest) -> APIResult<()> {
	let mut unsupported = Vec::new();
	if !patch.roles.is_unset() {
		unsupported.push("roles");
	}
	if !patch.labels_allow.is_unset() {
		unsupported.push("labelsAllow");
	}
	if !patch.labels_exclude.is_unset() {
		unsupported.push("labelsExclude");
	}
	if !patch.shared_libraries.is_unset() {
		unsupported.push("sharedLibraries");
	}

	if unsupported.is_empty() {
		Ok(())
	} else {
		Err(APIError::BadRequest(format!(
			"Unsupported user update fields: {}",
			unsupported.join(", ")
		)))
	}
}

async fn delete_age_restriction(
	txn: &DatabaseTransaction,
	user_id: &str,
) -> APIResult<()> {
	if let Some(existing) = age_restriction::Entity::find()
		.filter(age_restriction::Column::UserId.eq(user_id))
		.one(txn)
		.await?
	{
		age_restriction::Entity::delete_by_id(existing.id)
			.exec(txn)
			.await?;
	}
	Ok(())
}

async fn apply_age_restriction(
	txn: &DatabaseTransaction,
	user_id: &str,
	patch: &PatchValue<stump_komga::user::KomgaAgeRestriction>,
) -> APIResult<()> {
	match patch {
		PatchValue::Unset => {},
		PatchValue::None => delete_age_restriction(txn, user_id).await?,
		PatchValue::Some(restriction)
			if restriction.restriction == AllowExclude::None =>
		{
			delete_age_restriction(txn, user_id).await?;
		},
		PatchValue::Some(restriction) => {
			let existing = age_restriction::Entity::find()
				.filter(age_restriction::Column::UserId.eq(user_id))
				.one(txn)
				.await?;
			age_restriction::ActiveModel {
				id: existing.map_or(NotSet, |restriction| Set(restriction.id)),
				age: Set(restriction.age),
				restrict_on_unset: Set(matches!(
					restriction.restriction,
					AllowExclude::AllowOnly
				)),
				user_id: Set(user_id.to_owned()),
			}
			.save(txn)
			.await?;
		},
	}

	Ok(())
}

async fn delete_user(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
) -> APIResult<StatusCode> {
	ensure_server_owner(&auth)?;
	if auth.user().id == id {
		return Err(APIError::BadRequest(
			"You cannot delete your own account".to_owned(),
		));
	}

	let existing = user::Entity::find_by_id(id)
		.filter(user::Column::DeletedAt.is_null())
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| APIError::NotFound("User not found".to_owned()))?;
	if existing.is_server_owner {
		return Err(APIError::BadRequest(
			"You cannot delete the server owner".to_owned(),
		));
	}

	let mut existing = existing.into_active_model();
	existing.deleted_at = Set(Some(chrono::Utc::now().into()));
	existing.update(ctx.conn.as_ref()).await?;
	Ok(StatusCode::NO_CONTENT)
}

async fn patch_settings(
	Extension(auth): Extension<AuthContext>,
	Json(_request): Json<KomgaSettingsUpdateRequest>,
) -> APIResult<StatusCode> {
	if !auth.user().is_server_owner {
		return Err(APIError::forbidden_discreet());
	}

	tracing::warn!(
		"Accepted Komga settings update, but Stump does not persist Komga settings"
	);
	Ok(StatusCode::NO_CONTENT)
}

async fn get_announcements(
	Extension(_auth): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let feed = KomgaJsonFeed {
		version: "https://jsonfeed.org/version/1.1".to_owned(),
		title: "Stump".to_owned(),
		home_page_url: None,
		description: Some("Stump does not publish an announcements feed.".to_owned()),
		items: Vec::new(),
	};

	cached_json(&headers, &feed)
}

async fn put_announcements(
	Extension(_auth): Extension<AuthContext>,
	Json(_announcement_ids): Json<Vec<KomgaAnnouncementId>>,
) -> APIResult<StatusCode> {
	Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AuthenticationActivityPaginationQuery {
	page: i32,
	size: i32,
	unpaged: bool,
}

impl Default for AuthenticationActivityPaginationQuery {
	fn default() -> Self {
		Self {
			page: 0,
			size: DEFAULT_PAGE_SIZE,
			unpaged: false,
		}
	}
}

impl AuthenticationActivityPaginationQuery {
	fn validate(self) -> APIResult<AuthenticationActivityPagination> {
		if self.page < 0 {
			return Err(APIError::BadRequest(
				"page must be zero-based and non-negative".to_owned(),
			));
		}
		if self.size < 0 {
			return Err(APIError::BadRequest("size must be non-negative".to_owned()));
		}
		let size = if self.size > MAX_PAGE_SIZE {
			tracing::warn!(
				requested = self.size,
				clamped = MAX_PAGE_SIZE,
				"Clamping oversized Komga authentication activity page size"
			);
			MAX_PAGE_SIZE
		} else {
			self.size
		};
		Ok(AuthenticationActivityPagination {
			page: self.page,
			size,
			unpaged: self.unpaged,
		})
	}
}

#[derive(Debug, Clone, Copy)]
struct AuthenticationActivityPagination {
	page: i32,
	size: i32,
	unpaged: bool,
}

impl AuthenticationActivityPagination {
	fn offset(self) -> u64 {
		(self.page as u64).saturating_mul(self.size as u64)
	}
}

async fn get_authentication_activity(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<AuthenticationActivityPaginationQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let pagination = query.validate()?;
	let user = auth.user();
	let user_id = user.id.clone();
	let username = user.username.clone();
	let query = user_login_activity::Entity::find()
		.filter(user_login_activity::Column::UserId.eq(user_id))
		.order_by_desc(user_login_activity::Column::Timestamp)
		.order_by_desc(user_login_activity::Column::Id);
	let total = i32::try_from(query.clone().count(ctx.conn.as_ref()).await?)?;
	let query = if pagination.unpaged {
		query
	} else {
		query
			.offset(pagination.offset())
			.limit(pagination.size as u64)
	};
	let activities = query
		.all(ctx.conn.as_ref())
		.await?
		.into_iter()
		.map(|activity| map_authentication_activity(activity, Some(&username)))
		.collect::<Vec<_>>();

	cached_json(
		&headers,
		&Page::new(
			activities,
			pagination.page,
			pagination.size,
			total,
			pagination.unpaged,
		),
	)
}

async fn get_all_authentication_activity(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<AuthenticationActivityPaginationQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	ensure_server_owner(&auth)?;
	let pagination = query.validate()?;
	let base_query = user_login_activity::Entity::find()
		.order_by_desc(user_login_activity::Column::Timestamp)
		.order_by_desc(user_login_activity::Column::Id);
	let total = i32::try_from(base_query.clone().count(ctx.conn.as_ref()).await?)?;
	let query = if pagination.unpaged {
		base_query
	} else {
		base_query
			.offset(pagination.offset())
			.limit(pagination.size as u64)
	};
	let activities = query
		.find_also_related(user::Entity)
		.all(ctx.conn.as_ref())
		.await?
		.into_iter()
		.map(|(activity, user)| {
			map_authentication_activity(
				activity,
				user.as_ref().map(|user| user.username.as_str()),
			)
		})
		.collect::<Vec<_>>();

	cached_json(
		&headers,
		&Page::new(
			activities,
			pagination.page,
			pagination.size,
			total,
			pagination.unpaged,
		),
	)
}

async fn get_latest_authentication_activity(
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	ensure_server_owner(&auth)?;
	let user = user::Entity::find_by_id(id.clone())
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| APIError::NotFound("User not found".to_owned()))?;
	let activity = user_login_activity::Entity::find()
		.filter(user_login_activity::Column::UserId.eq(id))
		.order_by_desc(user_login_activity::Column::Timestamp)
		.order_by_desc(user_login_activity::Column::Id)
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| {
			APIError::NotFound("Authentication activity not found".to_owned())
		})?;

	cached_json(
		&headers,
		&map_authentication_activity(activity, Some(&user.username)),
	)
}

fn map_authentication_activity(
	activity: user_login_activity::Model,
	username: Option<&str>,
) -> KomgaAuthenticationActivity {
	KomgaAuthenticationActivity {
		user_id: Some(activity.user_id.into()),
		email: username.map(str::to_owned),
		ip: Some(activity.ip_address),
		user_agent: Some(activity.user_agent),
		success: activity.authentication_successful,
		error: None,
		date_time: activity.timestamp.with_timezone(&chrono::Utc),
		source: "PASSWORD".to_owned(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn pagination_defaults_to_zero_based_pages() {
		let query = AuthenticationActivityPaginationQuery::default();
		let pagination = query.validate().expect("default pagination is valid");
		assert_eq!(pagination.page, 0);
		assert_eq!(pagination.size, DEFAULT_PAGE_SIZE);
		assert!(!pagination.unpaged);
	}

	#[test]
	fn pagination_rejects_negative_values() {
		assert!(AuthenticationActivityPaginationQuery {
			page: -1,
			..Default::default()
		}
		.validate()
		.is_err());
		assert!(AuthenticationActivityPaginationQuery {
			size: -1,
			..Default::default()
		}
		.validate()
		.is_err());
	}

	#[test]
	fn password_hash_uses_configured_cost() {
		let mut config = stump_core::config::StumpConfig::debug();
		config.password_hash_cost = 4;
		let hash = hash_password("new-password", &config).expect("password should hash");
		let parts: bcrypt::HashParts = hash.parse().expect("hash should parse");
		assert_eq!(parts.get_cost(), 4);
		assert!(bcrypt::verify("new-password", &hash).expect("hash should verify"));
	}

	#[test]
	fn non_owner_cannot_change_another_users_password() {
		let auth = AuthContext {
			user: models::entity::user::AuthUser {
				id: "reader".to_owned(),
				..Default::default()
			},
			api_key: None,
		};
		assert!(!can_change_password(&auth, "another-user"));
		assert!(can_change_password(&auth, "reader"));
	}

	#[test]
	fn user_update_rejects_unpersistable_fields() {
		let patch: KomgaUserUpdateRequest = serde_json::from_value(serde_json::json!({
			"roles": ["ADMIN"],
			"ageRestriction": {
				"age": 13,
				"restriction": "ALLOW_ONLY"
			}
		}))
		.expect("user patch should deserialize");
		let error = validate_user_update(&patch).expect_err("roles must be rejected");
		assert!(matches!(
			error,
			APIError::BadRequest(message) if message.contains("roles")
		));
	}

	#[test]
	fn user_update_accepts_none_age_restriction() {
		let patch: KomgaUserUpdateRequest = serde_json::from_value(serde_json::json!({
			"ageRestriction": {
				"age": 0,
				"restriction": "NONE"
			}
		}))
		.expect("NONE age restriction should deserialize");
		validate_user_update(&patch).expect("age restriction should be supported");
		assert!(matches!(
			patch.age_restriction,
			PatchValue::Some(stump_komga::user::KomgaAgeRestriction {
				restriction: AllowExclude::None,
				..
			})
		));
	}
}
