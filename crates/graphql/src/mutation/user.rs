use crate::{
	data::CoreContext,
	error_message::FORBIDDEN_ACTION,
	guard::{PermissionGuard, ServerOwnerGuard},
	input::user::{AgeRestrictionInput, CreateUserInput, UpdateUserInput},
	object::user::User,
};
use async_graphql::{Context, Object, Result, ID};
use chrono::Utc;
#[cfg(feature = "web")]
use models::entity::user_preferences;
use models::txn::begin_write;
use models::{
	entity::{
		age_restriction, api_key, book_club_invitation, book_club_reader_participant,
		book_club_reader_session, book_request, book_request_approval, custom_emoji,
		device, device_credential,
		device_pairing::{self, DevicePairingStatus},
		emailer_send_record, ingest_drop_item, ingest_metadata_application,
		ingest_plugin_setting, kindle_delivery, known_duplicate_page, liseur_sync_token,
		notification_channel_setting, notification_rule, reading_list, refresh_token,
		session, smart_list, social_share_grant,
		user::{self, AuthUser},
		user_login_activity, user_preferences as preferences,
	},
	shared::{
		enums::{EntityVisibility, UserPermission},
		permission_set::PermissionSet,
	},
};
use sea_orm::{
	prelude::*,
	sea_query::{Expr, Query},
	ActiveValue::NotSet,
	ColumnTrait, Condition, DatabaseTransaction, IntoActiveModel, Set, TryIntoModel,
};
use stump_core::config::StumpConfig;

#[cfg(feature = "web")]
use crate::{
	input::user::{
		HomeArrangementInput, NavigationArrangementInput, UpdateUserPreferencesInput,
	},
	object::user_preferences::UserPreferences,
	utils::save_user_session,
};
#[cfg(feature = "web")]
use models::shared::arrangement::{Arrangement, HomeArrangement};
#[cfg(feature = "web")]
use tower_sessions::Session;

#[derive(Default)]
pub struct UserMutation;

#[Object]
impl UserMutation {
	#[graphql(guard = "ServerOwnerGuard")]
	async fn delete_login_activity(&self, ctx: &Context<'_>) -> Result<u64> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		let deleted_rows = user_login_activity::Entity::delete_many()
			.exec(conn)
			.await?;
		tracing::debug!("Deleted login activity entries");

		Ok(deleted_rows.rows_affected)
	}

	/// Delete the avatar for the authenticated viewer, or for any user if
	/// called by a server owner (by passing `id`).
	async fn delete_user_avatar(
		&self,
		ctx: &Context<'_>,
		id: Option<ID>,
	) -> Result<User> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();

		let target_id = match &id {
			Some(id) => {
				if id.as_str() != user.id && !user.is_server_owner {
					return Err(FORBIDDEN_ACTION.into());
				}
				id.to_string()
			},
			None => user.id.clone(),
		};

		let existing = user::Entity::find()
			.filter(user::Column::Id.eq(&target_id))
			.one(conn)
			.await?
			.ok_or("User not found")?;

		if let Some(ref path_str) = existing.avatar_path {
			let _ = tokio::fs::remove_file(path_str).await;
		}

		let mut active = existing.into_active_model();
		active.avatar_path = Set(None);
		active.avatar_meta = Set(None);
		active.avatar_updated_at = Set(Some(Utc::now().into()));
		let result = active.update(conn).await?;

		Ok(User::from(result))
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageUsers)")]
	#[tracing::instrument(skip(self, ctx, input), fields(username = ?input.username))]
	async fn create_user(
		&self,
		ctx: &Context<'_>,
		input: CreateUserInput,
	) -> Result<User> {
		if input.max_sessions_allowed.is_some_and(|limit| limit <= 0) {
			return Err("max_sessions_allowed must be greater than 0 when set".into());
		}
		let core_ctx = ctx.data::<CoreContext>()?;
		let hashed_password =
			bcrypt::hash(input.password, core_ctx.config.auth.password_hash_cost)?;

		let conn = core_ctx.conn.as_ref();

		let permissions = PermissionSet::new(input.permissions);

		let user = user::ActiveModel {
			id: NotSet,
			is_server_owner: Set(false),
			created_at: Set(chrono::Utc::now().into()),
			username: Set(input.username),
			hashed_password: Set(hashed_password),
			permissions: Set(permissions.resolve_into_string()),
			max_sessions_allowed: Set(input.max_sessions_allowed),
			..Default::default()
		};

		let txn = begin_write(conn).await?;
		let user_model = user
			.save(&txn)
			.await
			.map_err(|error| {
				tracing::error!(?error, "Failed to create user");
				"Failed to create user"
			})?
			.try_into_model()?;
		tracing::debug!(?user_model, "Created user");

		if let Some(ar) = input.age_restriction {
			let created_restriction = age_restriction::ActiveModel {
				id: NotSet,
				user_id: Set(user_model.id.clone()),
				age: Set(ar.age),
				restrict_on_unset: Set(ar.restrict_on_unset),
			}
			.save(&txn)
			.await
			.map_err(|error| {
				tracing::error!(?error, "Failed to create age restriction");
				"Failed to create age restriction"
			})?;
			tracing::trace!(?created_restriction, "Created age restriction");
		}

		let user_preferences = models::entity::user_preferences::ActiveModel {
			id: NotSet,
			user_id: Set(Some(user_model.id.clone())),
			..Default::default()
		};

		let user_preferences = user_preferences
			.save(&txn)
			.await
			.map_err(|e| {
				tracing::error!(error = ?e, "Failed to create user preferences");
				"Failed to create user preferences"
			})?
			.try_into_model()?;

		let mut user_model = user_model.into_active_model();
		user_model.user_preferences_id = Set(Some(user_preferences.id));
		let user_model = user_model.update(&txn).await?;

		txn.commit().await?;

		Ok(User::from(user_model.try_into_model()?))
	}

	async fn update_viewer(
		&self,
		ctx: &Context<'_>,
		input: UpdateUserInput,
	) -> Result<User> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core_ctx = ctx.data::<CoreContext>()?;
		let config = core_ctx.config.as_ref();
		let conn = core_ctx.conn.as_ref();

		let updated_user =
			update_user(user, user.id.clone(), conn, config, &input).await?;

		Ok(updated_user)
	}

	#[cfg(feature = "web")]
	async fn update_viewer_preferences(
		&self,
		ctx: &Context<'_>,
		input: UpdateUserPreferencesInput,
	) -> Result<UserPreferences> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let session = ctx.data::<Session>()?;
		let core_ctx = ctx.data::<CoreContext>()?;
		let conn = core_ctx.conn.as_ref();

		let user_preferences = user_preferences::Entity::find()
			.filter(user_preferences::Column::UserId.eq(user.id.clone()))
			.one(conn)
			.await?;

		if let Some(user_preferences_model) = user_preferences {
			tracing::trace!(user_id = ?user.id, ?user_preferences_model, updates = ?input, "Updating viewer's preferences");

			let updated_user_preferences = update_user_preferences_by_id(
				user_preferences_model.id,
				user.id.clone(),
				input,
				conn,
			)
			.await?;

			save_user_session(
				session,
				AuthUser {
					preferences: Some(updated_user_preferences.model.clone()),
					..user.clone()
				},
			)
			.await;

			Ok(updated_user_preferences)
		} else {
			Err("User preferences not found".into())
		}
	}

	async fn update_user(
		&self,
		ctx: &Context<'_>,
		id: ID,
		input: UpdateUserInput,
	) -> Result<User> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core_ctx = ctx.data::<CoreContext>()?;
		let config = core_ctx.config.as_ref();
		let conn = core_ctx.conn.as_ref();

		let is_self = user.id == id.to_string();
		let can_manage_users =
			user.is_server_owner || user.has_permission(UserPermission::ManageUsers);

		if !is_self && !can_manage_users {
			return Err(FORBIDDEN_ACTION.into());
		}

		// TODO(permissions): server owner goes away
		// nobody can update the server owner
		if !is_self && !user.is_server_owner {
			let target = user::Entity::find_by_id(id.to_string())
				.one(conn)
				.await?
				.ok_or("User not found")?;
			if target.is_server_owner {
				return Err(FORBIDDEN_ACTION.into());
			}
		}

		let updated_user =
			update_user(user, id.to_string(), conn, config, &input).await?;
		tracing::debug!(?updated_user, "Updated user");

		if !is_self {
			remove_all_session_for_user(id.to_string(), conn).await?;
		}

		Ok(updated_user)
	}

	#[graphql(guard = "ServerOwnerGuard")]
	async fn delete_user(
		&self,
		ctx: &Context<'_>,
		id: ID,
		hard_delete: Option<bool>,
	) -> Result<User> {
		let auth = ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		Ok(User::from(
			delete_user_account(
				conn,
				&auth.user,
				id.as_ref(),
				hard_delete.unwrap_or(false),
			)
			.await?,
		))
	}

	#[graphql(guard = "ServerOwnerGuard")]
	async fn delete_user_sessions(&self, ctx: &Context<'_>, id: ID) -> Result<u64> {
		let core_ctx = ctx.data::<CoreContext>()?;
		let conn = core_ctx.conn.as_ref();

		let removed_sessions = remove_all_session_for_user(id.to_string(), conn).await?;
		Ok(removed_sessions.len().try_into()?)
	}

	#[graphql(guard = "ServerOwnerGuard")]
	async fn update_user_lock_status(
		&self,
		ctx: &Context<'_>,
		id: ID,
		lock: bool,
	) -> Result<User> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core_ctx = ctx.data::<CoreContext>()?;
		let conn = core_ctx.conn.as_ref();

		if id.to_string() == user.id {
			return Err("You cannot lock your own account".into());
		}

		let model = user::Entity::find()
			.filter(user::Column::Id.eq(id.to_string()))
			.one(conn)
			.await?
			.ok_or("User not found")?;
		let mut active_model = model.into_active_model();
		active_model.is_locked = Set(lock);

		if lock {
			// Delete all sessions for this user if they are being locked
			remove_all_session_for_user(id.to_string(), conn).await?;
		}

		let updated_user = active_model.update(conn).await?;

		Ok(User::from(updated_user))
	}

	#[cfg(feature = "web")]
	/// Replace the authenticated user's home sections
	async fn update_home_arrangement(
		&self,
		ctx: &Context<'_>,
		input: HomeArrangementInput,
	) -> Result<HomeArrangement> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let arrangement = HomeArrangement::new(input.sections);

		let preferences = user_preferences::Entity::find()
			.filter(user_preferences::Column::UserId.eq(&user.id))
			.one(conn)
			.await?
			.ok_or("User preferences not found")?;

		let mut active_model = preferences.into_active_model();
		active_model.home_arrangement = Set(Some(arrangement.clone().into()));
		active_model.update(conn).await?;

		Ok(arrangement)
	}
	#[cfg(feature = "web")]
	async fn update_navigation_arrangement_lock(
		&self,
		ctx: &Context<'_>,
		locked: bool,
	) -> Result<Arrangement> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;

		let preferences = user_preferences::Entity::find()
			.filter(user_preferences::Column::UserId.eq(&user.id))
			.one(conn)
			.await?
			.ok_or("User preferences not found")?;

		let updated_arrangement = Arrangement {
			locked,
			..preferences
				.navigation_arrangement
				.clone()
				.unwrap_or_else(Arrangement::default_navigation)
		};

		let mut active_model = preferences.into_active_model();
		active_model.navigation_arrangement = Set(Some(updated_arrangement.clone()));
		active_model.update(conn).await?;

		Ok(updated_arrangement)
	}

	#[cfg(feature = "web")]
	async fn update_navigation_arrangement(
		&self,
		ctx: &Context<'_>,
		input: NavigationArrangementInput,
	) -> Result<Arrangement> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;

		let preferences = user_preferences::Entity::find()
			.filter(user_preferences::Column::UserId.eq(&user.id))
			.one(conn)
			.await?
			.ok_or("User preferences not found")?;

		let arrangement = preferences
			.navigation_arrangement
			.clone()
			.unwrap_or_else(Arrangement::default_navigation);

		if arrangement.locked {
			return Err("Navigation arrangement is locked".into());
		}

		let updated_arrangement = Arrangement {
			locked: arrangement.locked,
			sections: input.sections,
		};

		let mut active_model = preferences.into_active_model();
		active_model.navigation_arrangement = Set(Some(updated_arrangement.clone()));

		active_model.update(conn).await?;

		Ok(updated_arrangement)
	}
}

/// Revocation and deletion must share a write transaction: an approved pairing
/// or guest capability must never outlive the account row it was issued for.
async fn delete_user_account(
	conn: &DatabaseConnection,
	actor: &AuthUser,
	id: &str,
	hard_delete: bool,
) -> Result<user::Model> {
	if !actor.is_server_owner {
		return Err(FORBIDDEN_ACTION.into());
	}
	if actor.id == id {
		return Err("You cannot delete your own account".into());
	}

	let txn = begin_write(conn).await?;
	let existing = user::Entity::find_by_id(id)
		.one(&txn)
		.await?
		.ok_or("User not found")?;
	if existing.is_server_owner {
		return Err("You cannot delete the server owner".into());
	}

	let now = Utc::now().fixed_offset();
	// A reader credential is independently usable without a login. Preserve
	// other participants' messages/progress, but close publications issued by
	// this account and revoke their guest tokens. This also covers a guest
	// credential created by the account in somebody else's session.
	book_club_reader_participant::Entity::update_many()
		.col_expr(
			book_club_reader_participant::Column::TokenDigest,
			Expr::value(None::<String>),
		)
		.col_expr(
			book_club_reader_participant::Column::RevokedAt,
			Expr::value(Some(now)),
		)
		.col_expr(
			book_club_reader_participant::Column::CredentialExpiresAt,
			Expr::value(Some(now)),
		)
		.col_expr(
			book_club_reader_participant::Column::UpdatedAt,
			Expr::value(now),
		)
		.filter(
			Condition::any()
				.add(book_club_reader_participant::Column::LinkedUserId.eq(id))
				.add(book_club_reader_participant::Column::CreatedByUserId.eq(id))
				.add(
					book_club_reader_participant::Column::SessionId.in_subquery(
						Query::select()
							.column(book_club_reader_session::Column::Id)
							.from(book_club_reader_session::Entity)
							.and_where(
								book_club_reader_session::Column::CreatedByUserId.eq(id),
							)
							.to_owned(),
					),
				),
		)
		.exec(&txn)
		.await?;
	book_club_reader_session::Entity::update_many()
		.col_expr(
			book_club_reader_session::Column::ClosedAt,
			Expr::value(Some(now)),
		)
		.col_expr(
			book_club_reader_session::Column::UpdatedAt,
			Expr::value(now),
		)
		.filter(book_club_reader_session::Column::CreatedByUserId.eq(id))
		.exec(&txn)
		.await?;
	// A pending invitation can grant a new club membership without rechecking
	// its issuer. Retire such invitations and active/pending social grants while
	// leaving resolved invitations and shared history intact.
	book_club_invitation::Entity::update_many()
		.col_expr(
			book_club_invitation::Column::Status,
			Expr::value(book_club_invitation::REVOKED),
		)
		.col_expr(
			book_club_invitation::Column::RevokedAt,
			Expr::value(Some(now)),
		)
		.filter(book_club_invitation::Column::CreatedByUserId.eq(id))
		.filter(book_club_invitation::Column::Status.eq(book_club_invitation::PENDING))
		.exec(&txn)
		.await?;
	social_share_grant::Entity::update_many()
		.col_expr(
			social_share_grant::Column::State,
			Expr::value(social_share_grant::REVOKED),
		)
		.col_expr(
			social_share_grant::Column::RevokedAt,
			Expr::value(Some(now)),
		)
		.filter(
			Condition::any()
				.add(social_share_grant::Column::SourceUserId.eq(id))
				.add(social_share_grant::Column::RecipientUserId.eq(id)),
		)
		.filter(
			social_share_grant::Column::State
				.is_in([social_share_grant::PENDING, social_share_grant::ACTIVE]),
		)
		.exec(&txn)
		.await?;

	// Basic, bearer/JWT, and Komga remember-me auth re-read the user row;
	// separately minted device and Liseur credentials must also be retired.
	device_credential::Entity::delete_many()
		.filter(
			device_credential::Column::DeviceId.in_subquery(
				Query::select()
					.column(device::Column::Id)
					.from(device::Entity)
					.and_where(device::Column::UserId.eq(id))
					.to_owned(),
			),
		)
		.exec(&txn)
		.await?;
	device::Entity::update_many()
		.col_expr(device::Column::RevokedAt, Expr::value(Some(now)))
		.filter(device::Column::UserId.eq(id))
		.exec(&txn)
		.await?;
	device_pairing::Entity::update_many()
		.col_expr(
			device_pairing::Column::Status,
			Expr::value(DevicePairingStatus::Denied),
		)
		.filter(device_pairing::Column::UserId.eq(id))
		.filter(device_pairing::Column::Status.eq(DevicePairingStatus::Approved))
		.filter(device_pairing::Column::CredentialIssued.eq(false))
		.exec(&txn)
		.await?;
	session::Entity::delete_many()
		.filter(session::Column::UserId.eq(id))
		.exec(&txn)
		.await?;
	refresh_token::Entity::delete_many()
		.filter(refresh_token::Column::UserId.eq(id))
		.exec(&txn)
		.await?;
	api_key::Entity::delete_many()
		.filter(api_key::Column::UserId.eq(id))
		.exec(&txn)
		.await?;
	liseur_sync_token::Entity::delete_many()
		.filter(liseur_sync_token::Column::UserId.eq(id))
		.exec(&txn)
		.await?;

	let deleted_user = if hard_delete {
		// User-owned records cascade, but shared catalog objects and reader
		// history must survive their creator. A closed reader session is retained
		// under the server owner as custodian, never as a live authority.
		book_club_reader_session::Entity::update_many()
			.col_expr(
				book_club_reader_session::Column::CreatedByUserId,
				Expr::value(&actor.id),
			)
			.filter(book_club_reader_session::Column::CreatedByUserId.eq(id))
			.exec(&txn)
			.await?;
		reading_list::Entity::update_many()
			.col_expr(reading_list::Column::CreatingUserId, Expr::value(&actor.id))
			.filter(reading_list::Column::CreatingUserId.eq(id))
			.filter(reading_list::Column::Visibility.is_in(["PUBLIC", "SHARED"]))
			.exec(&txn)
			.await?;
		smart_list::Entity::update_many()
			.col_expr(smart_list::Column::CreatorId, Expr::value(&actor.id))
			.filter(smart_list::Column::CreatorId.eq(id))
			.filter(
				smart_list::Column::Visibility
					.is_in([EntityVisibility::Public, EntityVisibility::Shared]),
			)
			.exec(&txn)
			.await?;
		custom_emoji::Entity::update_many()
			.col_expr(custom_emoji::Column::CreatedById, Expr::value(&actor.id))
			.filter(custom_emoji::Column::CreatedById.eq(id))
			.exec(&txn)
			.await?;

		// These user references have no FK or have the FK in the opposite
		// direction; cascades alone leave secrets, orphan settings, or stale
		// references in other accounts' audit/history rows.
		notification_rule::Entity::delete_many()
			.filter(notification_rule::Column::UserId.eq(id))
			.exec(&txn)
			.await?;
		notification_channel_setting::Entity::delete_many()
			.filter(notification_channel_setting::Column::UserId.eq(id))
			.exec(&txn)
			.await?;
		ingest_plugin_setting::Entity::delete_many()
			.filter(ingest_plugin_setting::Column::UserId.eq(id))
			.exec(&txn)
			.await?;
		kindle_delivery::Entity::delete_many()
			.filter(kindle_delivery::Column::UserId.eq(id))
			.exec(&txn)
			.await?;
		book_request_approval::Entity::delete_many()
			.filter(book_request_approval::Column::ApproverId.eq(id))
			.exec(&txn)
			.await?;
		book_request::Entity::update_many()
			.col_expr(
				book_request::Column::ApprovedBy,
				Expr::value(None::<String>),
			)
			.filter(book_request::Column::ApprovedBy.eq(id))
			.exec(&txn)
			.await?;
		book_request::Entity::update_many()
			.col_expr(
				book_request::Column::RejectedBy,
				Expr::value(None::<String>),
			)
			.filter(book_request::Column::RejectedBy.eq(id))
			.exec(&txn)
			.await?;
		book_club_invitation::Entity::update_many()
			.col_expr(
				book_club_invitation::Column::CreatedByUserId,
				Expr::value(None::<String>),
			)
			.filter(book_club_invitation::Column::CreatedByUserId.eq(id))
			.exec(&txn)
			.await?;
		book_club_invitation::Entity::update_many()
			.col_expr(
				book_club_invitation::Column::RevokedByUserId,
				Expr::value(None::<String>),
			)
			.filter(book_club_invitation::Column::RevokedByUserId.eq(id))
			.exec(&txn)
			.await?;
		social_share_grant::Entity::update_many()
			.col_expr(
				social_share_grant::Column::RevokedByUserId,
				Expr::value(None::<String>),
			)
			.filter(social_share_grant::Column::RevokedByUserId.eq(id))
			.exec(&txn)
			.await?;
		emailer_send_record::Entity::update_many()
			.col_expr(
				emailer_send_record::Column::SentByUserId,
				Expr::value(None::<String>),
			)
			.filter(emailer_send_record::Column::SentByUserId.eq(id))
			.exec(&txn)
			.await?;
		ingest_drop_item::Entity::update_many()
			.col_expr(
				ingest_drop_item::Column::CreatedBy,
				Expr::value(None::<String>),
			)
			.filter(ingest_drop_item::Column::CreatedBy.eq(id))
			.exec(&txn)
			.await?;
		known_duplicate_page::Entity::update_many()
			.col_expr(
				known_duplicate_page::Column::CreatedBy,
				Expr::value(None::<String>),
			)
			.filter(known_duplicate_page::Column::CreatedBy.eq(id))
			.exec(&txn)
			.await?;
		ingest_metadata_application::Entity::update_many()
			.col_expr(
				ingest_metadata_application::Column::Actor,
				Expr::value("deleted account"),
			)
			.filter(ingest_metadata_application::Column::Actor.eq(id))
			.exec(&txn)
			.await?;

		// `users.user_preferences_id` points *to* preferences; the preferences
		// row's own user_id is not a FK and otherwise becomes an orphan.
		user::Entity::update_many()
			.col_expr(user::Column::UserPreferencesId, Expr::value(None::<i32>))
			.filter(user::Column::Id.eq(id))
			.exec(&txn)
			.await?;
		let mut preferences_filter =
			Condition::any().add(preferences::Column::UserId.eq(id));
		if let Some(pref_id) = existing.user_preferences_id {
			preferences_filter =
				preferences_filter.add(preferences::Column::Id.eq(pref_id));
		}
		preferences::Entity::delete_many()
			.filter(preferences_filter)
			.exec(&txn)
			.await?;
		user::Entity::delete_by_id(id).exec(&txn).await?;
		existing
	} else {
		let mut active = existing.into_active_model();
		active.deleted_at = Set(Some(now));
		active.update(&txn).await?
	};

	txn.commit().await?;
	Ok(deleted_user)
}

async fn remove_all_session_for_user(
	id: String,
	conn: &DatabaseConnection,
) -> Result<Vec<session::Model>> {
	let removed_sessions = session::Entity::delete_many()
		.filter(session::Column::UserId.eq(id.clone()))
		.exec_with_returning(conn)
		.await?;

	tracing::debug!(?removed_sessions, "Removed sessions for user");
	Ok(removed_sessions)
}

#[cfg(feature = "web")]
async fn update_user_preferences_by_id(
	id: i32,
	user_id: String,
	user_preferences: UpdateUserPreferencesInput,
	conn: &DatabaseConnection,
) -> Result<UserPreferences> {
	// FIXME(graphql): I think this will overwrite the NotSet values if they are
	// currently set
	let updated_user_preferences = user_preferences::ActiveModel {
		id: Set(id),
		user_id: Set(Some(user_id)),
		locale: Set(user_preferences.locale),
		preferred_layout_mode: Set(user_preferences.preferred_layout_mode),
		app_theme: Set(user_preferences.app_theme),
		enable_gradients: Set(user_preferences.enable_gradients),
		app_font: Set(user_preferences.app_font),
		primary_navigation_mode: Set(user_preferences.primary_navigation_mode),
		layout_max_width_px: Set(user_preferences.layout_max_width_px),
		show_query_indicator: Set(user_preferences.show_query_indicator),
		enable_live_refetch: Set(user_preferences.enable_live_refetch),
		enable_discord_presence: Set(user_preferences.enable_discord_presence),
		enable_compact_display: Set(user_preferences.enable_compact_display),
		enable_double_sidebar: Set(user_preferences.enable_double_sidebar),
		enable_replace_primary_sidebar: Set(
			user_preferences.enable_replace_primary_sidebar
		),
		enable_hide_scrollbar: Set(user_preferences.enable_hide_scrollbar),
		enable_job_overlay: Set(user_preferences.enable_job_overlay),
		enable_fancy_animations: Set(user_preferences.enable_fancy_animations),
		prefer_accent_color: Set(user_preferences.prefer_accent_color),
		thumbnail_ratio: Set(user_preferences.thumbnail_ratio),
		thumbnail_placeholder_style: Set(user_preferences.thumbnail_placeholder_style),
		enable_alphabet_select: Set(user_preferences.enable_alphabet_select),
		enable_reading_journal: Set(user_preferences.enable_reading_journal),
		day_reset_hour_offset: Set(user_preferences.day_reset_hour_offset),
		reading_session_grace_period_secs: Set(
			user_preferences.reading_session_grace_period_secs
		),
		interface_roundness: Set(user_preferences.interface_roundness),
		thumbnail_roundness: Set(user_preferences.thumbnail_roundness),
		home_arrangement: NotSet,
		navigation_arrangement: NotSet,
	};

	let updated_user_prefs = updated_user_preferences.update(conn).await?;

	Ok(UserPreferences::from(updated_user_prefs))
}

async fn update_user(
	by_user: &AuthUser,
	for_user_id: String,
	conn: &DatabaseConnection,
	config: &StumpConfig,
	input: &UpdateUserInput,
) -> Result<User> {
	// NOTE: there are other mechanisms in place to effectively disable logging in,
	// so I am making this a bad request. In the future, perhaps this can change.
	match input.max_sessions_allowed {
		Some(max_sessions_allowed) if max_sessions_allowed <= 0 => {
			return Err("max_sessions_allowed must be greater than 0 when set".into());
		},
		Some(max_sessions_allowed) => {
			tracing::trace!(?max_sessions_allowed, "The max sessions allowed is set");
		},
		_ => {},
	}

	let is_self_update = by_user.id == for_user_id;

	let is_different_username = input.username != by_user.username;
	if is_self_update
		&& is_different_username
		&& !by_user.has_permission(UserPermission::ChangeUsername)
	{
		return Err("You do not have permission to change the username".into());
	}

	let mut update_user = user::ActiveModel {
		id: Set(for_user_id.clone()),
		username: Set(input.username.clone()),
		..Default::default()
	};

	if let Some(password) = input.password.clone() {
		if is_self_update && !by_user.has_permission(UserPermission::ChangePassword) {
			return Err("You do not have permission to change the password".into());
		}
		let hashed_password = bcrypt::hash(password, config.auth.password_hash_cost)?;
		update_user.hashed_password = Set(hashed_password);
	}

	let txn = begin_write(conn).await?;

	// TODO(permissions): server owner goes away
	// only a server owner or a user with ManageUsers may set another user's
	// permissions, age restriction, and session cap.
	let can_manage_privileged_fields = (by_user.is_server_owner
		|| by_user.has_permission(UserPermission::ManageUsers))
		&& !is_self_update;
	if can_manage_privileged_fields {
		update_user.max_sessions_allowed = Set(input.max_sessions_allowed);
		update_user_age_restriction(&for_user_id, &input.age_restriction, &txn).await?;
		let permissions = PermissionSet::new(input.permissions.clone());
		update_user.permissions = Set(permissions.resolve_into_string());
	}

	let updated_user_entity = update_user.update(&txn).await?;

	txn.commit().await?;

	Ok(User::from(updated_user_entity))
}

async fn update_user_age_restriction(
	user_id: &str,
	age_restriction: &Option<AgeRestrictionInput>,
	txn: &DatabaseTransaction,
) -> Result<()> {
	let existing_age_restriction = age_restriction::Entity::find()
		.filter(age_restriction::Column::UserId.eq(user_id))
		.one(txn)
		.await?;

	if let Some(age_restriction) = age_restriction {
		let set_age_restriction_id = if let Some(restriction) = existing_age_restriction {
			Set(restriction.id)
		} else {
			NotSet
		};

		let _ = age_restriction::ActiveModel {
			id: set_age_restriction_id,
			user_id: Set(user_id.to_string()),
			age: Set(age_restriction.age),
			restrict_on_unset: Set(age_restriction.restrict_on_unset),
		}
		.save(txn)
		.await
		.map_err(|e| {
			tracing::error!("Failed to save age restriction: {:?}", e);
			"Failed to save age restriction"
		})?;

		Ok(())
	} else if let Some(existing_restriction) = existing_age_restriction {
		// delete age restriction
		let result = age_restriction::Entity::delete_by_id(existing_restriction.id)
			.exec(txn)
			.await
			.map_err(|e| {
				tracing::error!("Failed to delete age restriction: {:?}", e);
				"Failed to delete age restriction"
			})?;

		if result.rows_affected != 1 {
			return Err("Failed to delete age restriction".into());
		}

		Ok(())
	} else {
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use sea_orm::{ConnectionTrait, DatabaseBackend, DbConn, Statement};

	async fn database() -> (tempfile::TempDir, DbConn) {
		let dir = tempfile::tempdir().unwrap();
		let url = format!(
			"sqlite://{}?mode=rwc",
			dir.path().join("users.db").display()
		);
		let conn = stump_core::database::connect_at(&url).await.unwrap();
		for sql in [
			"INSERT INTO users (id, username, hashed_password, is_server_owner, created_at, is_locked) VALUES ('owner', 'owner', 'hash', 1, CURRENT_TIMESTAMP, 0)",
			"INSERT INTO users (id, username, hashed_password, is_server_owner, created_at, is_locked) VALUES ('member', 'member', 'hash', 0, CURRENT_TIMESTAMP, 0)",
			"INSERT INTO users (id, username, hashed_password, is_server_owner, created_at, is_locked) VALUES ('other', 'other', 'hash', 0, CURRENT_TIMESTAMP, 0)",
		] {
			execute(&conn, sql).await;
		}
		(dir, conn)
	}

	async fn execute(conn: &DbConn, sql: &str) {
		conn.execute(Statement::from_string(
			DatabaseBackend::Sqlite,
			sql.to_owned(),
		))
		.await
		.unwrap_or_else(|error| panic!("{sql}: {error}"));
	}

	async fn count(conn: &DbConn, sql: &str) -> i64 {
		conn.query_one(Statement::from_string(
			DatabaseBackend::Sqlite,
			sql.to_owned(),
		))
		.await
		.unwrap()
		.unwrap()
		.try_get("", "total")
		.unwrap()
	}

	fn owner() -> AuthUser {
		AuthUser {
			id: "owner".into(),
			is_server_owner: true,
			..Default::default()
		}
	}

	async fn credentials(conn: &DbConn) {
		for sql in [
			"INSERT INTO sessions (session_id, user_id, created_at, expiry_time) VALUES ('session-1', 'member', CURRENT_TIMESTAMP, '2030-01-01')",
			"INSERT INTO refresh_tokens (id, user_id, created_at, expires_at) VALUES ('refresh-1', 'member', CURRENT_TIMESTAMP, '2030-01-01')",
			"INSERT INTO api_keys (name, short_token, long_token_hash, user_id) VALUES ('key', 'short', 'hash', 'member')",
			"INSERT INTO devices (id, user_id, name, kind) VALUES ('reader-1', 'member', 'Reader', 'koreader')",
			"INSERT INTO liseur_sync_tokens (id, user_id, device_id, secret_hash, scopes, created_at, expires_at) VALUES ('token-1', 'member', 'reader-1', 'hash', '[\"sync\"]', '2026-01-01', '2030-01-01')",
			"INSERT INTO device_credentials (device_id, protocol, credential_kind, credential_ref) VALUES ('reader-1', 'koreader', 'api_key', 'short')",
		] {
			execute(conn, sql).await;
		}
	}

	async fn club_reader(conn: &DbConn) {
		for sql in [
			"INSERT INTO book_clubs (id, name, slug, is_private, created_at) VALUES ('club-1', 'Readers', 'readers', 0, CURRENT_TIMESTAMP)",
			"INSERT INTO book_club_reader_sessions (id, book_club_id, created_by_user_id, name, created_at, updated_at) VALUES ('club-session', 'club-1', 'member', 'Reading together', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
			"INSERT INTO book_club_reader_participants (id, session_id, linked_user_id, created_by_user_id, display_name, share_progress, token_digest, created_at, updated_at) VALUES ('guest-1', 'club-session', 'other', 'member', 'Guest', 1, 'guest-secret-digest', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
			"INSERT INTO book_club_reader_messages (id, session_id, participant_id, body, created_at) VALUES ('message-1', 'club-session', 'guest-1', 'Shared discussion', CURRENT_TIMESTAMP)",
		] {
			execute(conn, sql).await;
		}
	}

	#[tokio::test]
	async fn soft_delete_keeps_private_data_but_retires_all_credentials() {
		let (_dir, conn) = database().await;
		credentials(&conn).await;
		club_reader(&conn).await;
		execute(&conn, "INSERT INTO notification_channel_settings (user_id, channel_id, settings) VALUES ('member', 'ntfy', '{\"token\":\"secret\"}')").await;
		execute(&conn, "INSERT INTO device_pairings (id, kind, code_hash, nonce, remote_ip, user_id, status, failed_attempts, credential_issued, created_at, expires_at) VALUES ('pair-1', 'koreader', 'hash', 'nonce', '127.0.0.1', 'member', 'APPROVED', 0, 0, CURRENT_TIMESTAMP, '2030-01-01')").await;
		let result = delete_user_account(&conn, &owner(), "member", false)
			.await
			.unwrap();
		assert!(result.deleted_at.is_some());
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM users WHERE id='member' AND deleted_at IS NOT NULL").await, 1);
		for table in [
			"sessions",
			"refresh_tokens",
			"api_keys",
			"liseur_sync_tokens",
			"device_credentials",
		] {
			assert_eq!(
				count(&conn, &format!("SELECT COUNT(*) AS total FROM {table}")).await,
				0,
				"{table}"
			);
		}
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM notification_channel_settings WHERE user_id='member'").await, 1);
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM devices WHERE id='reader-1' AND revoked_at IS NOT NULL").await, 1);
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM device_pairings WHERE id='pair-1' AND status='DENIED'").await, 1);
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM book_club_reader_sessions WHERE id='club-session' AND closed_at IS NOT NULL AND created_by_user_id='member'").await, 1);
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM book_club_reader_participants WHERE id='guest-1' AND token_digest IS NULL AND revoked_at IS NOT NULL").await, 1);
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM book_club_reader_messages WHERE id='message-1'").await, 1);
	}

	#[tokio::test]
	async fn permanent_delete_purges_private_records_and_preserves_shared_history() {
		let (_dir, conn) = database().await;
		credentials(&conn).await;
		club_reader(&conn).await;
		let preferences = preferences::ActiveModel {
			user_id: Set(Some("member".into())),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();
		execute(
			&conn,
			&format!(
				"UPDATE users SET user_preferences_id = {} WHERE id='member'",
				preferences.id
			),
		)
		.await;
		for sql in [
			"INSERT INTO notification_channel_settings (user_id, channel_id, settings) VALUES ('member', 'ntfy', '{\"token\":\"secret\"}')",
			"INSERT INTO reading_lists (id, name, updated_at, visibility, ordering, creating_user_id) VALUES ('private-list', 'Personal', CURRENT_TIMESTAMP, 'PRIVATE', 'MANUAL', 'member')",
			"INSERT INTO reading_lists (id, name, updated_at, visibility, ordering, creating_user_id) VALUES ('shared-list', 'Together', CURRENT_TIMESTAMP, 'SHARED', 'MANUAL', 'member')",
			"INSERT INTO reading_lists (id, name, updated_at, visibility, ordering, creating_user_id) VALUES ('unknown-list', 'Unknown visibility', CURRENT_TIMESTAMP, 'LEGACY_PRIVATE', 'MANUAL', 'member')",
			"INSERT INTO collections (id, name, updated_at, ordered, creating_user_id) VALUES ('collection-1', 'Personal shelf', CURRENT_TIMESTAMP, 0, 'member')",
			"INSERT INTO custom_emojis (name, created_by_id) VALUES ('wave', 'member')",
			"INSERT INTO book_requests (id, requester_id, title, format, status, approval_policy) VALUES ('personal-request', 'member', 'My Book', 'EBOOK', 'PENDING', 'REQUIRED')",
			"INSERT INTO book_requests (id, requester_id, title, format, status, approval_policy, approved_by) VALUES ('shared-request', 'other', 'Another Book', 'EBOOK', 'APPROVED', 'REQUIRED', 'member')",
			"INSERT INTO book_request_approvals (id, request_id, approver_id, decision, created_at) VALUES ('approval-1', 'shared-request', 'member', 'APPROVED', CURRENT_TIMESTAMP)",
		] {
			execute(&conn, sql).await;
		}

		let result = delete_user_account(&conn, &owner(), "member", true)
			.await
			.unwrap();
		assert_eq!(result.id, "member");
		for (table, field) in [
			("users", "id"),
			("user_preferences", "user_id"),
			("sessions", "user_id"),
			("refresh_tokens", "user_id"),
			("api_keys", "user_id"),
			("liseur_sync_tokens", "user_id"),
			("devices", "user_id"),
			("notification_channel_settings", "user_id"),
			("book_requests", "requester_id"),
		] {
			assert_eq!(
				count(
					&conn,
					&format!(
						"SELECT COUNT(*) AS total FROM {table} WHERE {field}='member'"
					)
				)
				.await,
				0,
				"{table}"
			);
		}
		assert_eq!(
			count(
				&conn,
				"SELECT COUNT(*) AS total FROM reading_lists WHERE id='private-list'"
			)
			.await,
			0
		);
		assert_eq!(
			count(
				&conn,
				"SELECT COUNT(*) AS total FROM reading_lists WHERE id='unknown-list'"
			)
			.await,
			0,
			"only explicit public/shared lists transfer"
		);
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM reading_lists WHERE id='shared-list' AND creating_user_id='owner'").await, 1);
		assert_eq!(
			count(
				&conn,
				"SELECT COUNT(*) AS total FROM collections WHERE id='collection-1'"
			)
			.await,
			0
		);
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM custom_emojis WHERE name='wave' AND created_by_id='owner'").await, 1);
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM book_club_reader_sessions WHERE id='club-session' AND created_by_user_id='owner' AND closed_at IS NOT NULL").await, 1);
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM book_club_reader_messages WHERE id='message-1' AND body='Shared discussion'").await, 1);
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM book_club_reader_participants WHERE id='guest-1' AND token_digest IS NULL AND revoked_at IS NOT NULL").await, 1);
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM book_requests WHERE id='shared-request' AND status='APPROVED' AND approved_by IS NULL").await, 1);
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM book_request_approvals WHERE id='approval-1'").await, 0);
		assert_eq!(
			count(
				&conn,
				"SELECT COUNT(*) AS total FROM book_clubs WHERE id='club-1'"
			)
			.await,
			1
		);
		let violations = conn
			.query_all(Statement::from_string(
				DatabaseBackend::Sqlite,
				"PRAGMA foreign_key_check".to_owned(),
			))
			.await
			.unwrap();
		assert!(
			violations.is_empty(),
			"permanent deletion must preserve foreign keys"
		);
	}

	#[tokio::test]
	async fn permanent_delete_rolls_back_revocation_when_the_account_delete_fails() {
		let (_dir, conn) = database().await;
		credentials(&conn).await;
		club_reader(&conn).await;
		execute(&conn, "CREATE TRIGGER block_member_deletion BEFORE DELETE ON users WHEN OLD.id = 'member' BEGIN SELECT RAISE(ABORT, 'blocked'); END").await;

		assert!(delete_user_account(&conn, &owner(), "member", true)
			.await
			.is_err());
		assert_eq!(
			count(
				&conn,
				"SELECT COUNT(*) AS total FROM users WHERE id='member'"
			)
			.await,
			1
		);
		assert_eq!(
			count(
				&conn,
				"SELECT COUNT(*) AS total FROM sessions WHERE user_id='member'"
			)
			.await,
			1
		);
		assert_eq!(
			count(
				&conn,
				"SELECT COUNT(*) AS total FROM api_keys WHERE user_id='member'"
			)
			.await,
			1
		);
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM devices WHERE user_id='member' AND revoked_at IS NULL").await, 1);
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM book_club_reader_sessions WHERE created_by_user_id='member' AND closed_at IS NULL").await, 1);
		assert_eq!(count(&conn, "SELECT COUNT(*) AS total FROM book_club_reader_participants WHERE id='guest-1' AND token_digest IS NOT NULL AND revoked_at IS NULL").await, 1);
	}

	#[tokio::test]
	async fn deletion_rejects_self_owner_and_non_owner_without_changing_rows() {
		let (_dir, conn) = database().await;
		assert!(delete_user_account(&conn, &owner(), "owner", true)
			.await
			.is_err());
		assert!(delete_user_account(
			&conn,
			&AuthUser {
				id: "member".into(),
				is_server_owner: true,
				..Default::default()
			},
			"member",
			false
		)
		.await
		.is_err());
		assert!(delete_user_account(
			&conn,
			&AuthUser {
				id: "other".into(),
				..Default::default()
			},
			"member",
			true
		)
		.await
		.is_err());
		assert!(delete_user_account(&conn, &owner(), "missing", true)
			.await
			.is_err());
		assert_eq!(
			count(
				&conn,
				"SELECT COUNT(*) AS total FROM users WHERE deleted_at IS NULL"
			)
			.await,
			3
		);
	}
}
