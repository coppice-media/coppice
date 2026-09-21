use std::{collections::BTreeMap, env};

use models::shared::enums::UserPermission;
use serde::{Deserialize, Serialize};

use super::env_keys::*;
use crate::{CoreError, CoreResult};

const REQUIRED_SCOPES: &str = "email";

/// Configuration for OpenID Connect (OIDC) authentication
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "OidcConfig"))]
pub struct OidcConfig {
	/// Whether to enable OIDC authentication
	#[serde(default)]
	pub enabled: bool,
	/// The OIDC provider client ID
	#[serde(default)]
	pub client_id: String,
	/// The OIDC provider issuer URL (e.g., https://accounts.google.com)
	#[serde(default)]
	pub issuer_url: String,
	/// The client secret
	#[serde(default)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub client_secret: String,
	/// Additional scopes to request (comma-separated)
	/// Default: "openid,email,profile"
	#[serde(default = "default_oidc_scopes")]
	pub scopes: String,
	/// Allow automatic user registration via OIDC
	#[serde(default = "default_true")]
	pub allow_registration: bool,
	/// Disable local password authentication when OIDC is enabled
	#[serde(default)]
	pub disable_local_auth: bool,
	/// Additional trusted audiences for ID token verification
	#[serde(default)]
	pub extra_audiences: Vec<String>,
	/// Path to a CA certificate file (PEM-encoded) to trust when connecting to the OIDC issuer
	#[serde(default)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub ca_cert_file: Option<String>,
	/// Claim containing the user's OIDC groups
	#[serde(default = "default_groups_claim")]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub groups_claim: String,
	/// OIDC group names mapped to Coppice permissions
	#[serde(default)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub group_permissions: BTreeMap<String, Vec<UserPermission>>,
	/// Optional OIDC group that receives every Coppice permission
	#[serde(default)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub admin_group: Option<String>,
	/// Whether to synchronize mapped permissions on every OIDC login
	#[serde(default = "default_true")]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub sync_permissions: bool,
}

impl Default for OidcConfig {
	fn default() -> Self {
		Self {
			enabled: false,
			client_id: String::new(),
			issuer_url: String::new(),
			client_secret: String::new(),
			scopes: default_oidc_scopes(),
			allow_registration: true,
			disable_local_auth: false,
			extra_audiences: Vec::new(),
			ca_cert_file: None,
			groups_claim: default_groups_claim(),
			group_permissions: BTreeMap::new(),
			admin_group: None,
			sync_permissions: true,
		}
	}
}

fn default_oidc_scopes() -> String {
	"openid,email,profile".to_string()
}

fn default_groups_claim() -> String {
	"groups".to_string()
}

fn default_true() -> bool {
	true
}

impl std::fmt::Debug for OidcConfig {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("OidcConfig")
			.field("enabled", &self.enabled)
			.field("client_id", &self.client_id)
			.field("issuer_url", &self.issuer_url)
			.field("client_secret", &"[REDACTED]")
			.field("scopes", &self.scopes)
			.field("allow_registration", &self.allow_registration)
			.field("disable_local_auth", &self.disable_local_auth)
			.field("extra_audiences", &self.extra_audiences)
			.field("ca_cert_file", &self.ca_cert_file)
			.field("groups_claim", &self.groups_claim)
			.field("group_permissions", &self.group_permissions)
			.field("admin_group", &self.admin_group)
			.field("sync_permissions", &self.sync_permissions)
			.finish()
	}
}

impl OidcConfig {
	/// Load OIDC configuration from environment variables
	/// Returns None if OIDC is not enabled or not properly configured
	pub fn from_env() -> CoreResult<Option<Self>> {
		let enabled = env::var(OIDC_ENABLED_KEY)
			.ok()
			.and_then(|v| v.parse::<bool>().ok())
			.unwrap_or(false);

		if !enabled {
			return Ok(None);
		}

		let client_id = env::var(OIDC_CLIENT_ID_KEY).unwrap_or_default();
		let issuer_url = env::var(OIDC_ISSUER_URL_KEY).unwrap_or_default();
		let client_secret = env::var(OIDC_CLIENT_SECRET_KEY).unwrap_or_default();

		if client_id.is_empty() || issuer_url.is_empty() || client_secret.is_empty() {
			tracing::warn!(
				"OIDC is enabled but missing required configuration (client_id, issuer_url, or client_secret)"
			);
			return Ok(None);
		}

		let scopes = env::var(OIDC_SCOPES_KEY).unwrap_or_else(|_| default_oidc_scopes());

		let mut scopes_set: Vec<String> = scopes
			.split(',')
			.map(|s| s.trim().to_string())
			.filter(|s| !s.is_empty())
			.collect();
		// Ensure required scopes are included
		for required_scope in REQUIRED_SCOPES.split(',') {
			if !scopes_set.contains(&required_scope.to_string()) {
				scopes_set.push(required_scope.to_string());
			}
		}
		let scopes = scopes_set.join(",");

		let allow_registration = env::var(OIDC_ALLOW_REGISTRATION_KEY)
			.ok()
			.and_then(|v| v.parse::<bool>().ok())
			.unwrap_or(true);
		let disable_local_auth = env::var(OIDC_DISABLE_LOCAL_AUTH_KEY)
			.ok()
			.and_then(|v| v.parse::<bool>().ok())
			.unwrap_or(false);

		let extra_audiences = env::var(OIDC_EXTRA_AUDIENCES_KEY)
			.ok()
			.map(|v| {
				v.split(',')
					.map(|s| s.trim().to_string())
					.filter(|s| !s.is_empty())
					.collect()
			})
			.unwrap_or_default();

		let ca_cert_file = env::var(OIDC_CA_CERT_FILE_KEY)
			.ok()
			.filter(|v| !v.is_empty());
		if let Some(ca_cert_file_specified) = &ca_cert_file {
			tracing::info!("OIDC certificate specified as {}", ca_cert_file_specified);
		}

		let groups_claim =
			env::var(OIDC_GROUPS_CLAIM_KEY).unwrap_or_else(|_| default_groups_claim());
		let group_permissions = env::var(OIDC_GROUP_PERMISSIONS_KEY)
			.ok()
			.map(|value| {
				parse_group_permissions(&value).map_err(|error| {
					CoreError::InitializationError(format!(
						"Failed to parse {OIDC_GROUP_PERMISSIONS_KEY}: {error}"
					))
				})
			})
			.transpose()?
			.unwrap_or_default();
		let admin_group = env::var(OIDC_ADMIN_GROUP_KEY)
			.ok()
			.filter(|value| !value.is_empty());
		let sync_permissions = env::var(OIDC_SYNC_PERMISSIONS_KEY)
			.ok()
			.map(|value| {
				value.parse::<bool>().map_err(|error| {
					CoreError::InitializationError(format!(
						"Failed to parse {OIDC_SYNC_PERMISSIONS_KEY}: {error}"
					))
				})
			})
			.transpose()?
			.unwrap_or(true);

		Ok(Some(Self {
			enabled,
			client_id,
			issuer_url,
			client_secret,
			scopes,
			allow_registration,
			disable_local_auth,
			extra_audiences,
			ca_cert_file,
			groups_claim,
			group_permissions,
			admin_group,
			sync_permissions,
		}))
	}

	/// Check if OIDC is *properly* configured
	pub fn is_configured(&self) -> bool {
		let is_configured_properly = self.enabled
			&& !self.client_id.is_empty()
			&& !self.issuer_url.is_empty()
			&& !self.client_secret.is_empty();
		if !is_configured_properly && self.enabled {
			tracing::warn!(
				client_id = ?self.client_id,
				issuer_url = ?self.issuer_url,
				client_secret_set = !self.client_secret.is_empty(),
				"OIDC is enabled but not properly configured (client_id, issuer_url, and client_secret are required)"
			);
		}
		is_configured_properly
	}

	/// Get the scopes to use when requesting OIDC tokens
	pub fn get_scopes(&self) -> Vec<String> {
		self.scopes
			.split(',')
			.map(|s| s.trim().to_string())
			.filter(|s| !s.is_empty())
			.collect()
	}

	/// Get the extra trusted audiences for ID token verification, if any
	pub fn get_extra_audiences(&self) -> Vec<String> {
		self.extra_audiences.clone()
	}
}

fn parse_group_permissions(
	value: &str,
) -> Result<BTreeMap<String, Vec<UserPermission>>, serde_json::Error> {
	serde_json::from_str(value)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_oidc_config_validation() {
		let config = OidcConfig {
			enabled: true,
			client_id: "test-client".to_string(),
			issuer_url: "https://example.com".to_string(),
			client_secret: "test-secret".to_string(),
			..Default::default()
		};

		assert!(config.is_configured());

		let invalid_config = OidcConfig {
			enabled: true,
			client_id: String::new(),
			issuer_url: String::new(),
			..Default::default()
		};

		assert!(!invalid_config.is_configured());
	}

	#[test]
	fn test_oidc_scopes_parsing() {
		let config = OidcConfig {
			scopes: "openid,email,profile".to_string(),
			..Default::default()
		};

		let scopes = config.get_scopes();
		assert_eq!(scopes.len(), 3);
		assert_eq!(scopes, vec!["openid", "email", "profile"]);
	}

	#[test]
	fn test_group_permissions_json_env_parsing() {
		let config = temp_env::with_vars(
			[
				(OIDC_ENABLED_KEY, Some("true")),
				(OIDC_CLIENT_ID_KEY, Some("client")),
				(OIDC_ISSUER_URL_KEY, Some("https://issuer.example")),
				(OIDC_CLIENT_SECRET_KEY, Some("secret")),
				(OIDC_GROUPS_CLAIM_KEY, Some("roles")),
				(
					OIDC_GROUP_PERMISSIONS_KEY,
					Some(r#"{"reader":["READ_USERS","DOWNLOAD_FILE"]}"#),
				),
				(OIDC_ADMIN_GROUP_KEY, None::<&str>),
				(OIDC_SYNC_PERMISSIONS_KEY, None::<&str>),
			],
			|| OidcConfig::from_env(),
		)
		.expect("JSON environment configuration should parse")
		.expect("OIDC should be enabled");

		assert_eq!(config.groups_claim, "roles");
		assert_eq!(
			config.group_permissions.get("reader"),
			Some(&vec![
				UserPermission::ReadUsers,
				UserPermission::DownloadFile
			])
		);
		assert!(config.sync_permissions);
	}

	#[test]
	fn test_group_permissions_toml_parsing() {
		let config: OidcConfig = toml::from_str(
			r#"
groups_claim = "roles"
admin_group = "admins"
[group_permissions]
readers = ["READ_USERS", "DOWNLOAD_FILE"]
admins = ["MANAGE_USERS"]
"#,
		)
		.expect("TOML group permissions should parse");

		assert_eq!(config.groups_claim, "roles");
		assert_eq!(config.admin_group.as_deref(), Some("admins"));
		assert_eq!(
			config.group_permissions.get("readers"),
			Some(&vec![
				UserPermission::ReadUsers,
				UserPermission::DownloadFile
			])
		);
		assert!(config.sync_permissions);
	}

	#[test]
	fn test_unknown_group_permission_rejected() {
		let error = parse_group_permissions(r#"{"admins":["NOT_A_PERMISSION"]}"#)
			.expect_err("unknown permission should be rejected");
		assert!(error.to_string().contains("unknown variant"));

		let error = toml::from_str::<OidcConfig>(
			r#"
[group_permissions]
admins = ["NOT_A_PERMISSION"]
"#,
		)
		.expect_err("unknown TOML permission should be rejected");
		assert!(error.to_string().contains("unknown variant"));
	}
}
