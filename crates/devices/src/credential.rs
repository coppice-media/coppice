use chrono::Utc;
use models::{
	entity::api_key,
	shared::{
		api_key::{APIKeyPermissions, API_KEY_PREFIX},
		enums::{DeviceCredentialKind, DeviceKind, DeviceProtocol, UserPermission},
	},
};
use prefixed_api_key::{PrefixedApiKey, PrefixedApiKeyController};
use sea_orm::{prelude::*, ActiveValue::Set, ConnectionTrait};
use serde::Serialize;

use crate::error::{DeviceError, DeviceResult};

/// A freshly minted credential. `secret` is the plaintext and is returned
/// exactly once; only its hash is stored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "IssuedDeviceCredential"))]
pub struct IssuedCredential {
	pub kind: DeviceCredentialKind,
	pub protocol: DeviceProtocol,
	/// The stored reference: the API key's short token, or the liseur token id
	pub credential_ref: String,
	pub secret: String,
}

/// How an authenticated request identifies its credential.
#[derive(Clone, Copy, Debug)]
pub enum CredentialRef<'a> {
	/// The raw prefixed API key (`stump_<short>_<long>`) carried by the request
	ApiKey(&'a str),
	/// The `liseur_sync_tokens.id` resolved by the liseur bearer middleware
	LiseurToken(&'a str),
}

impl CredentialRef<'_> {
	/// The `(credential_kind, credential_ref)` pair stored on the credential row,
	/// or `None` when the reference cannot belong to a device credential.
	pub(crate) fn lookup_key(&self) -> Option<(DeviceCredentialKind, String)> {
		match self {
			Self::ApiKey(raw) => PrefixedApiKey::from_string(raw)
				.ok()
				.filter(|pak| pak.prefix() == API_KEY_PREFIX)
				.map(|pak| {
					(
						DeviceCredentialKind::ApiKey,
						pak.short_token().to_string(),
					)
				}),
			Self::LiseurToken(id) => {
				Some((DeviceCredentialKind::LiseurToken, (*id).to_string()))
			},
		}
	}
}

/// The protocol a device of `kind` speaks, i.e. what its credential is minted for
pub fn protocol_for(kind: DeviceKind) -> DeviceProtocol {
	match kind {
		DeviceKind::Kobo => DeviceProtocol::Kobo,
		DeviceKind::Koreader => DeviceProtocol::Koreader,
		DeviceKind::Mihon | DeviceKind::Komelia => DeviceProtocol::Komga,
		DeviceKind::Opds => DeviceProtocol::Opds,
		DeviceKind::Liseur => DeviceProtocol::Liseur,
		DeviceKind::Api | DeviceKind::Web => DeviceProtocol::Api,
	}
}

/// The credential storage a device of `kind` is issued
pub fn credential_kind_for(kind: DeviceKind) -> DeviceCredentialKind {
	match kind {
		DeviceKind::Liseur => DeviceCredentialKind::LiseurToken,
		_ => DeviceCredentialKind::ApiKey,
	}
}

/// The permissions the device's API key is narrowed to. Protocol kinds get the
/// protocol permission plus what the protocol needs to serve files; native API
/// kinds act as the user.
pub fn api_key_permissions_for(kind: DeviceKind) -> APIKeyPermissions {
	match kind {
		DeviceKind::Kobo => APIKeyPermissions::Custom(vec![
			UserPermission::AccessKoboSync,
			UserPermission::DownloadFile,
		]),
		DeviceKind::Koreader => {
			APIKeyPermissions::Custom(vec![UserPermission::AccessKoreaderSync])
		},
		DeviceKind::Mihon | DeviceKind::Komelia | DeviceKind::Opds => {
			APIKeyPermissions::Custom(vec![UserPermission::DownloadFile])
		},
		DeviceKind::Liseur | DeviceKind::Api | DeviceKind::Web => {
			APIKeyPermissions::inherit()
		},
	}
}

/// The permissions the owning user must hold before a device of `kind` can be
/// created: a key must never grant what the user does not have, and an API key
/// only authenticates for users allowed to use API keys.
pub fn required_permissions(kind: DeviceKind) -> Vec<UserPermission> {
	let mut required = match api_key_permissions_for(kind) {
		APIKeyPermissions::Custom(permissions) => permissions,
		APIKeyPermissions::Inherit(_) => Vec::new(),
	};
	if credential_kind_for(kind) == DeviceCredentialKind::ApiKey {
		required.push(UserPermission::AccessApiKeys);
	}
	required
}

/// Mints an API key row for `user_id` and returns `(short_token, plaintext)`.
pub(crate) async fn mint_api_key<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	name: &str,
	permissions: APIKeyPermissions,
) -> DeviceResult<(String, String)> {
	let controller = PrefixedApiKeyController::configure()
		.prefix(API_KEY_PREFIX.to_owned())
		.seam_defaults()
		.finalize()
		.map_err(|error| DeviceError::Credential(error.to_string()))?;
	let (pak, hash) = controller.generate_key_and_hash();
	api_key::ActiveModel {
		user_id: Set(user_id.to_string()),
		name: Set(name.to_string()),
		short_token: Set(pak.short_token().to_string()),
		long_token_hash: Set(hash),
		permissions: Set(permissions),
		created_at: Set(Utc::now().into()),
		expires_at: Set(None),
		last_used_at: Set(None),
		..Default::default()
	}
	.insert(conn)
	.await?;

	Ok((pak.short_token().to_string(), pak.to_string()))
}

pub mod liseur {
	//! liseur-sync device tokens. The secret format and hash are shared with
	//! the server's liseur-sync storage so tokens minted here authenticate
	//! through the protocol's bearer middleware.

	use chrono::{SecondsFormat, Utc};
	use models::entity::liseur_sync_token;
	use sea_orm::{prelude::*, ActiveValue::Set, ConnectionTrait};
	use sha2::{Digest, Sha256};

	use crate::error::{DeviceError, DeviceResult};

	/// The far-future expiry stored for an unbounded device token; the column
	/// predates nullable expiry values.
	pub const DEVICE_TOKEN_TTL_SECS: i64 = 10 * 365 * 24 * 60 * 60;

	/// The scopes a registered Liseur device is granted: progress sync plus
	/// read access to the catalog and reading insights.
	pub const DEVICE_SCOPES: [&str; 3] = ["sync", "read-insights", "library-read"];

	/// A new bearer secret in the `liseur-<uuid>` shape clients expect.
	pub fn new_secret() -> String {
		format!("liseur-{}", Uuid::new_v4())
	}

	/// The stored form of a bearer secret.
	pub fn hash_secret(secret: &str) -> String {
		let mut digest = Sha256::new();
		digest.update(secret.as_bytes());
		format!("{:x}", digest.finalize())
	}

	fn timestamp(value: chrono::DateTime<Utc>) -> String {
		value.to_rfc3339_opts(SecondsFormat::Nanos, true)
	}

	/// Mints a device token bound to `device_id` and returns `(token_id, secret)`.
	pub(crate) async fn mint<C: ConnectionTrait>(
		conn: &C,
		user_id: &str,
		device_id: &str,
		name: &str,
	) -> DeviceResult<(String, String)> {
		let token_id = Uuid::new_v4().to_string();
		let secret = new_secret();
		let now = Utc::now();
		let scopes = serde_json::to_string(&DEVICE_SCOPES)
			.map_err(|error| DeviceError::Credential(error.to_string()))?;

		liseur_sync_token::ActiveModel {
			id: Set(token_id.clone()),
			user_id: Set(user_id.to_string()),
			device_id: Set(device_id.to_string()),
			secret_hash: Set(hash_secret(&secret)),
			scopes: Set(scopes),
			created_at: Set(timestamp(now)),
			expires_at: Set(timestamp(
				now + chrono::Duration::seconds(DEVICE_TOKEN_TTL_SECS),
			)),
			last_used_at: Set(None),
			revoked_at: Set(None),
			name: Set(Some(name.to_string())),
			token_kind: Set(Some("device".to_string())),
		}
		.insert(conn)
		.await?;

		Ok((token_id, secret))
	}

	pub(crate) async fn revoke<C: ConnectionTrait>(
		conn: &C,
		token_id: &str,
	) -> DeviceResult<()> {
		liseur_sync_token::Entity::update_many()
			.filter(liseur_sync_token::Column::Id.eq(token_id))
			.filter(liseur_sync_token::Column::RevokedAt.is_null())
			.col_expr(
				liseur_sync_token::Column::RevokedAt,
				Expr::value(Some(timestamp(Utc::now()))),
			)
			.exec(conn)
			.await?;
		Ok(())
	}

	pub(crate) async fn rename<C: ConnectionTrait>(
		conn: &C,
		token_id: &str,
		name: &str,
	) -> DeviceResult<()> {
		liseur_sync_token::Entity::update_many()
			.filter(liseur_sync_token::Column::Id.eq(token_id))
			.col_expr(
				liseur_sync_token::Column::Name,
				Expr::value(Some(name.to_string())),
			)
			.exec(conn)
			.await?;
		Ok(())
	}

	#[cfg(test)]
	mod tests {
		use super::*;

		#[test]
		fn secret_shape_and_hash_match_liseur_storage() {
			let secret = new_secret();
			assert!(secret.starts_with("liseur-"));
			assert_eq!(secret.len(), "liseur-".len() + 36);
			// sha256("liseur-test"), lowercase hex: the exact form the liseur-sync
			// storage looks up in `liseur_sync_tokens.secret_hash`.
			assert_eq!(
				hash_secret("liseur-test"),
				"c93db9430cfd0f94d23fb4027c340d2c9e7f3c5f5247691dc5697eabd6db4b3b"
			);
		}
	}
}
