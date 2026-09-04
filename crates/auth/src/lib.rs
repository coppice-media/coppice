use models::{
	entity::user::AuthUser,
	shared::{enums::UserPermission, permission_set::user_has_all_permissions},
};

/// The error returned when an authenticated user is not authorized to perform an action.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AuthorizationError {
	/// The authenticated user's account is locked.
	#[error(
		"Your account is locked. Please contact an administrator to unlock your account."
	)]
	LockedAccount,
	/// The authenticated user does not have the required permission.
	#[error("You do not have permission to perform this action.")]
	ForbiddenAction,
}

/// The authenticated user and credentials associated with a request.
///
/// A user is authenticated if they meet one of the following criteria:
/// - They have a valid session
/// - They have a valid bearer token (session may not exist)
/// - They have valid basic auth credentials (session is created after successful authentication)
#[derive(Clone, Debug)]
pub struct AuthContext {
	pub user: AuthUser,
	pub api_key: Option<String>,
}

impl AuthContext {
	/// Get the current user.
	pub fn user(&self) -> AuthUser {
		self.user.clone()
	}

	/// Get the ID of the current user.
	pub fn id(&self) -> String {
		self.user.id.clone()
	}

	/// Get the API key associated with the current request, if any.
	pub fn api_key(&self) -> Option<String> {
		self.api_key.clone()
	}

	/// Enforce that the current user has all the permissions provided.
	///
	/// Server owners bypass permission checks. For all other users, a locked account is
	/// rejected before checking the requested permissions.
	#[tracing::instrument(skip(self))]
	pub fn enforce_permissions(
		&self,
		permissions: &[UserPermission],
	) -> Result<(), AuthorizationError> {
		if self.user.is_server_owner {
			return Ok(());
		}

		if self.user.is_locked {
			return Err(AuthorizationError::LockedAccount);
		}

		if user_has_all_permissions(&self.user, permissions) {
			Ok(())
		} else {
			Err(AuthorizationError::ForbiddenAction)
		}
	}

	/// Get the current user and enforce that they have all the permissions provided.
	#[tracing::instrument(skip(self))]
	pub fn user_and_enforce_permissions(
		&self,
		permissions: &[UserPermission],
	) -> Result<AuthUser, AuthorizationError> {
		self.enforce_permissions(permissions)?;
		Ok(self.user())
	}

	/// Enforce that the current user is the server owner.
	#[tracing::instrument(skip(self))]
	pub fn enforce_server_owner(&self) -> Result<(), AuthorizationError> {
		if self.user.is_server_owner {
			Ok(())
		} else {
			tracing::error!(
				username = &self.user.username,
				"User is not server owner, denying access"
			);
			Err(AuthorizationError::ForbiddenAction)
		}
	}

	/// Get the current user and enforce that they are the server owner.
	#[tracing::instrument(skip(self))]
	pub fn server_owner_user(&self) -> Result<AuthUser, AuthorizationError> {
		self.enforce_server_owner()?;
		Ok(self.user())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn context(user: AuthUser) -> AuthContext {
		AuthContext {
			user,
			api_key: None,
		}
	}

	#[test]
	fn accessors_clone_context_values() {
		let user = AuthUser {
			id: "user-id".to_string(),
			..Default::default()
		};
		let request_context = AuthContext {
			user: user.clone(),
			api_key: Some("api-key".to_string()),
		};

		assert!(user.is(&request_context.user()));
		assert_eq!(user.id, request_context.id());
		assert_eq!(Some("api-key".to_string()), request_context.api_key());
	}

	#[test]
	fn permission_check_allows_server_owner_without_permissions() {
		let request_context = context(AuthUser {
			is_server_owner: true,
			..Default::default()
		});

		assert_eq!(
			Ok(()),
			request_context.enforce_permissions(&[UserPermission::AccessBookClub])
		);
	}

	#[test]
	fn permission_check_allows_inherited_permissions() {
		let request_context = context(AuthUser {
			permissions: vec![UserPermission::CreateLibrary],
			..Default::default()
		});

		assert_eq!(
			Ok(()),
			request_context.enforce_permissions(&[UserPermission::EditLibrary])
		);
	}

	#[test]
	fn permission_check_rejects_locked_accounts_before_forbidden_action() {
		let request_context = context(AuthUser {
			is_locked: true,
			permissions: vec![UserPermission::AccessBookClub],
			..Default::default()
		});

		assert_eq!(
			Err(AuthorizationError::LockedAccount),
			request_context.enforce_permissions(&[UserPermission::CreateLibrary])
		);
	}

	#[test]
	fn permission_check_rejects_missing_permissions() {
		let request_context = context(AuthUser::default());

		assert_eq!(
			Err(AuthorizationError::ForbiddenAction),
			request_context.enforce_permissions(&[UserPermission::AccessBookClub])
		);
	}

	#[test]
	fn user_permission_helper_returns_user_only_when_authorized() {
		let user = AuthUser {
			permissions: vec![UserPermission::AccessBookClub],
			..Default::default()
		};
		let request_context = context(user.clone());

		assert!(user.is(&request_context
			.user_and_enforce_permissions(&[UserPermission::AccessBookClub])
			.unwrap()));
		assert!(matches!(
			request_context
				.user_and_enforce_permissions(&[UserPermission::CreateLibrary]),
			Err(AuthorizationError::ForbiddenAction)
		));
	}

	#[test]
	fn server_owner_helpers_enforce_and_return_owner() {
		let user = AuthUser {
			is_server_owner: true,
			..Default::default()
		};
		let request_context = context(user.clone());

		assert_eq!(Ok(()), request_context.enforce_server_owner());
		assert!(user.is(&request_context.server_owner_user().unwrap()));
	}

	#[test]
	fn server_owner_check_rejects_non_owner_with_forbidden_message() {
		let request_context = context(AuthUser::default());
		let error = request_context.enforce_server_owner().unwrap_err();

		assert_eq!(AuthorizationError::ForbiddenAction, error);
		assert_eq!(
			"You do not have permission to perform this action.",
			error.to_string()
		);
		assert!(matches!(
			request_context.server_owner_user(),
			Err(AuthorizationError::ForbiddenAction)
		));
	}

	#[test]
	fn authorization_error_preserves_current_messages() {
		assert_eq!(
			"Your account is locked. Please contact an administrator to unlock your account.",
			AuthorizationError::LockedAccount.to_string()
		);
		assert_eq!(
			"You do not have permission to perform this action.",
			AuthorizationError::ForbiddenAction.to_string()
		);
	}
}
