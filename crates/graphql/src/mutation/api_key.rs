use crate::{
	data::CoreContext,
	guard::PermissionGuard,
	input::api_key::APIKeyInput,
	object::api_key::{APIKey, CreatedAPIKey},
};
use async_graphql::{Context, Object, Result};
use models::entity::api_key;
use models::shared::{api_key::APIKeyPermissions, enums::UserPermission};
use sea_orm::{prelude::*, ActiveModelTrait};

#[derive(Default)]
pub struct APIKeyMutation;

#[Object]
impl APIKeyMutation {
	#[graphql(guard = "PermissionGuard::one(UserPermission::AccessApiKeys)")]
	async fn create_api_key(
		&self,
		ctx: &Context<'_>,
		input: APIKeyInput,
	) -> Result<CreatedAPIKey> {
		let req_ctx = ctx.data::<stump_auth::AuthContext>()?;
		let core_ctx = ctx.data::<CoreContext>()?;
		let conn = core_ctx.conn.as_ref();

		check_permissions(req_ctx, &input.permissions)?;

		let (active_model, secret) = input.into_create(&req_ctx.user)?;
		let result = active_model.insert(conn).await?;

		Ok(CreatedAPIKey {
			api_key: APIKey::from(result),
			secret,
		})
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::AccessApiKeys)")]
	async fn update_api_key(
		&self,
		ctx: &Context<'_>,
		id: i32,
		input: APIKeyInput,
	) -> Result<APIKey> {
		let req_ctx = ctx.data::<stump_auth::AuthContext>()?;
		let core_ctx = ctx.data::<CoreContext>()?;
		let conn = core_ctx.conn.as_ref();
		let user = &req_ctx.user;

		check_permissions(req_ctx, &input.permissions)?;

		let model = api_key::Entity::find_for_user(user)
			.filter(api_key::Column::Id.eq(id))
			.one(conn)
			.await?
			.ok_or_else(|| async_graphql::Error::new("API key not found"))?;

		let active_model = input.apply_updates(model)?;
		let result = active_model.update(conn).await?;

		Ok(APIKey::from(result))
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::AccessApiKeys)")]
	async fn delete_api_key(&self, ctx: &Context<'_>, id: i32) -> Result<APIKey> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core_ctx = ctx.data::<CoreContext>()?;
		let conn = core_ctx.conn.as_ref();

		let model = api_key::Entity::find_for_user(user)
			.filter(api_key::Column::Id.eq(id))
			.one(conn)
			.await?
			.ok_or_else(|| async_graphql::Error::new("API key not found"))?;
		let _ = model.clone().delete(conn).await?;

		Ok(APIKey::from(model))
	}
}

fn check_permissions(
	req_ctx: &stump_auth::AuthContext,
	permissions: &APIKeyPermissions,
) -> Result<()> {
	match permissions {
		APIKeyPermissions::Inherit(_) => {
			if req_ctx.api_key().is_some() {
				return Err(
					"You cannot use an API key to create another API key with inherited permissions"
						.into(),
				);
			}
		},
		APIKeyPermissions::Custom(permissions) => {
			req_ctx.enforce_permissions(permissions).map_err(|e| {
				tracing::trace!(?e, "User does not have requested permissions");
				"You lack the required permissions".to_string()
			})?;
		},
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use models::{entity::user::AuthUser, shared::api_key::InheritPermissionValue};

	fn context(api_key: Option<&str>) -> stump_auth::AuthContext {
		stump_auth::AuthContext {
			user: AuthUser {
				is_server_owner: true,
				..Default::default()
			},
			api_key: api_key.map(str::to_string),
			device_id: None,
		}
	}

	#[test]
	fn inherited_permissions_require_an_interactive_session() {
		let inherit = APIKeyPermissions::Inherit(InheritPermissionValue::Inherit);

		assert!(check_permissions(&context(None), &inherit).is_ok());

		let error = check_permissions(&context(Some("api-key")), &inherit).unwrap_err();
		assert_eq!(
			error.message,
			"You cannot use an API key to create another API key with inherited permissions"
		);
	}
}
