use std::collections::{BTreeMap, BTreeSet};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use models::shared::enums::UserPermission;
use openidconnect::{
	core::{
		CoreAuthDisplay, CoreAuthPrompt, CoreAuthenticationFlow, CoreClient,
		CoreErrorResponseType, CoreGenderClaim, CoreIdToken, CoreJsonWebKey,
		CoreJweContentEncryptionAlgorithm, CoreProviderMetadata, CoreRevocableToken,
		CoreRevocationErrorResponse, CoreTokenIntrospectionResponse, CoreTokenResponse,
	},
	AdditionalClaims, AuthorizationCode, Client, ClientId, ClientSecret, CsrfToken,
	EmptyAdditionalClaims, EndpointMaybeSet, EndpointNotSet, EndpointSet, IssuerUrl,
	Nonce, OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope,
	StandardErrorResponse, TokenResponse, UserInfoClaims,
};
use serde::{Deserialize, Serialize};
use stump_core::config::OidcConfig;

use crate::errors::APIError;

// lol this is an absurd type alias
pub type StumpOidcClient = Client<
	EmptyAdditionalClaims,
	CoreAuthDisplay,
	CoreGenderClaim,
	CoreJweContentEncryptionAlgorithm,
	CoreJsonWebKey,
	CoreAuthPrompt,
	StandardErrorResponse<CoreErrorResponseType>,
	CoreTokenResponse,
	CoreTokenIntrospectionResponse,
	CoreRevocableToken,
	CoreRevocationErrorResponse,
	EndpointSet,
	EndpointNotSet,
	EndpointNotSet,
	EndpointNotSet,
	EndpointMaybeSet,
	EndpointMaybeSet,
>;
const ALL_PERMISSIONS: &[UserPermission] = &[
	UserPermission::AccessApiKeys,
	UserPermission::AccessKoreaderSync,
	UserPermission::AccessKoboSync,
	UserPermission::AcquireReleases,
	UserPermission::AccessWorker,
	UserPermission::AccessBookClub,
	UserPermission::CreateBookClub,
	UserPermission::ChangePassword,
	UserPermission::ChangeUsername,
	UserPermission::ChangeAvatar,
	UserPermission::EmailerRead,
	UserPermission::EmailerCreate,
	UserPermission::EmailerManage,
	UserPermission::EmailSend,
	UserPermission::EmailArbitrarySend,
	UserPermission::AccessSmartList,
	UserPermission::FileExplorer,
	UserPermission::UploadFile,
	UserPermission::DownloadFile,
	UserPermission::CreateLibrary,
	UserPermission::EditLibrary,
	UserPermission::ScanLibrary,
	UserPermission::ManageLibrary,
	UserPermission::EditThumbnails,
	UserPermission::EditMetadata,
	UserPermission::WriteBackMetadata,
	UserPermission::DeleteLibrary,
	UserPermission::ReadUsers,
	UserPermission::ManageUsers,
	UserPermission::ReadNotifier,
	UserPermission::CreateNotifier,
	UserPermission::ManageNotifier,
	UserPermission::DeleteNotifier,
	UserPermission::ReadJobs,
	UserPermission::ManageJobs,
	UserPermission::MetadataFetchRecordRead,
	UserPermission::MetadataFetchRecordManage,
	UserPermission::MetadataProviderRead,
	UserPermission::MetadataProviderManage,
	UserPermission::ReadPersistedLogs,
	UserPermission::ReadSystemLogs,
	UserPermission::ManageServer,
];

fn all_permissions() -> impl Iterator<Item = UserPermission> {
	ALL_PERMISSIONS.iter().copied()
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct OidcAdditionalClaims {
	#[serde(flatten)]
	claims: BTreeMap<String, serde_json::Value>,
}

impl AdditionalClaims for OidcAdditionalClaims {}

/// Cached OIDC client state, initialized once at server startup.
/// Holds the HTTP client and discovered provider metadata, avoiding
/// repeated metadata discovery on every OIDC login request.
#[derive(Clone)]
pub struct OidcProvider {
	pub http_client: oauth2_reqwest::ReqwestClient,
	provider_metadata: CoreProviderMetadata,
	client_id: String,
	client_secret: String,
}

impl OidcProvider {
	/// Build the HTTP client and discover provider metadata.
	/// This performs the expensive I/O (metadata discovery) so it should
	/// be called once at startup and reused.
	pub async fn new(config: &OidcConfig) -> Result<Self, APIError> {
		let issuer_url = IssuerUrl::new(config.issuer_url.clone()).map_err(|error| {
			tracing::error!(?error, "Invalid issuer URL for OIDC");
			APIError::OIDCConfigurationInvalid
		})?;

		let mut client_builder =
			reqwest::ClientBuilder::new().redirect(reqwest::redirect::Policy::none());

		if let Some(ca_cert_path) = &config.ca_cert_file {
			let cert_bytes = tokio::fs::read(ca_cert_path).await.map_err(|error| {
				tracing::error!(?error, path = %ca_cert_path, "Failed to read CA certificate file for OIDC");
				APIError::InternalServerError(format!(
					"Failed to read CA certificate file: {}",
					error
				))
			})?;
			let cert = reqwest::Certificate::from_pem(&cert_bytes).map_err(|error| {
				tracing::error!(?error, path = %ca_cert_path, "Failed to parse CA certificate file for OIDC");
				APIError::InternalServerError(format!(
					"Failed to parse CA certificate '{}': {}",
					ca_cert_path, error
				))
			})?;
			client_builder = client_builder.add_root_certificate(cert);
		}

		let http_client = oauth2_reqwest::ReqwestClient::from(
			client_builder.build().map_err(|error| {
				tracing::error!(?error, "Failed to create HTTP client for OIDC");
				APIError::InternalServerError(format!(
					"Failed to create HTTP client: {}",
					error
				))
			})?,
		);

		let provider_metadata =
			CoreProviderMetadata::discover_async(issuer_url, &http_client)
				.await
				.map_err(|e| {
					tracing::error!(?e, "OIDC discovery failed");
					APIError::InternalServerError(format!("OIDC discovery failed: {}", e))
				})?;

		Ok(Self {
			http_client,
			provider_metadata,
			client_id: config.client_id.clone(),
			client_secret: config.client_secret.clone(),
		})
	}

	/// Create a per-request [StumpOidcClient] from the cached state.
	/// This is cheap — no I/O, just constructs the client with the correct redirect URL.
	pub fn create_client(&self, frontend_url: &str) -> Result<StumpOidcClient, APIError> {
		let redirect_uri = format!("{}/api/v2/auth/oidc/callback", frontend_url);
		let redirect_url = RedirectUrl::new(redirect_uri).map_err(|e| {
			tracing::error!(?e, "Invalid redirect URI constructed from frontend URL");
			APIError::InternalServerError(format!(
				"Invalid redirect URI constructed from frontend URL: {}",
				frontend_url
			))
		})?;

		Ok(CoreClient::from_provider_metadata(
			self.provider_metadata.clone(),
			ClientId::new(self.client_id.clone()),
			Some(ClientSecret::new(self.client_secret.clone())),
		)
		.set_redirect_uri(redirect_url))
	}
}

/// Get the OIDC authorization URL to redirect the user to
pub fn get_oidc_authorize_url(
	client: &StumpOidcClient,
	scopes: &[String],
	state: &str,
	pkce_challenge: Option<PkceCodeChallenge>,
) -> String {
	let scope_vec: Vec<Scope> = scopes.iter().map(|s| Scope::new(s.clone())).collect();
	let state_owned = state.to_string();

	let mut auth_request = client
		.authorize_url(
			CoreAuthenticationFlow::AuthorizationCode,
			move || CsrfToken::new(state_owned),
			Nonce::new_random,
		)
		.add_scopes(scope_vec);

	if let Some(challenge) = pkce_challenge {
		auth_request = auth_request.set_pkce_challenge(challenge);
	}

	let (authorize_url, _, _) = auth_request.url();
	authorize_url.to_string()
}
/// Claims extracted from OIDC token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcClaims {
	/// The unique identifier for the user from the provider
	pub subject: String,
	/// The user's email address
	pub email: String,
	/// The user's full name, if the provider supplies it
	pub name: Option<String>,
	/// A URL to the user's profile picture, if one exists
	pub picture: Option<String>,
	/// Group names supplied by the provider
	pub groups: Vec<String>,
}

impl OidcClaims {
	/// Return the deterministic union of permissions mapped from this user's groups.
	///
	/// `None` means synchronization is disabled or no mapping exists. `Some(vec![])`
	/// means synchronization is enabled but none of the user's groups are mapped.
	pub fn mapped_permissions(&self, config: &OidcConfig) -> Option<Vec<UserPermission>> {
		if !config.sync_permissions
			|| (config.group_permissions.is_empty() && config.admin_group.is_none())
		{
			return None;
		}

		let groups = self.groups.iter().collect::<BTreeSet<_>>();
		let is_admin = config
			.admin_group
			.as_ref()
			.is_some_and(|admin_group| groups.contains(&admin_group));

		let mut permissions = Vec::new();
		if is_admin {
			permissions.extend(all_permissions());
		} else {
			for (group, mapped) in &config.group_permissions {
				if groups.contains(&group) {
					for permission in mapped {
						if !permissions.contains(permission) {
							permissions.push(*permission);
						}
					}
				}
			}
		}

		Some(permissions)
	}
}

fn extract_claim_groups(value: Option<&serde_json::Value>) -> Option<Vec<String>> {
	match value? {
		serde_json::Value::Array(groups) => Some(
			groups
				.iter()
				.filter_map(serde_json::Value::as_str)
				.map(str::to_owned)
				.collect(),
		),
		serde_json::Value::String(group) => Some(vec![group.clone()]),
		_ => None,
	}
}

fn extract_id_token_groups(
	id_token: &CoreIdToken,
	claim_name: &str,
) -> Option<Vec<String>> {
	let token = id_token.to_string();
	let payload = token.split('.').nth(1)?;
	let payload = URL_SAFE_NO_PAD.decode(payload).ok()?;
	let claims: serde_json::Value = serde_json::from_slice(&payload).ok()?;
	extract_claim_groups(claims.get(claim_name))
}

/// Exchange authorization code for tokens and extract claims
pub async fn exchange_code_for_claims(
	http_client: &oauth2_reqwest::ReqwestClient,
	client: &StumpOidcClient,
	code: String,
	extra_audiences: Vec<String>,
	groups_claim: &str,
	pkce_verifier: Option<PkceCodeVerifier>,
) -> Result<OidcClaims, APIError> {
	let mut request = client.exchange_code(AuthorizationCode::new(code))?;
	if let Some(verifier) = pkce_verifier {
		request = request.set_pkce_verifier(verifier);
	}
	let token_response = request.request_async(http_client).await.map_err(|error| {
		tracing::error!(?error, "Token exchange failed");
		APIError::OIDCTokenExchangeFailed(error.to_string())
	})?;

	let id_token = token_response
		.id_token()
		.ok_or(APIError::OIDCMissingToken)?;

	let token_verifier =
		client
			.id_token_verifier()
			.set_other_audience_verifier_fn(move |aud| {
				extra_audiences.iter().any(|a| a.as_str() == aud.as_str())
			});
	let id_token_claims = id_token.claims(&token_verifier, nonce_verifier)?;
	let id_token_groups = extract_id_token_groups(id_token, groups_claim);

	let access_token = token_response.access_token();
	let user_info: UserInfoClaims<OidcAdditionalClaims, CoreGenderClaim> = client
		.user_info(access_token.to_owned(), None)?
		.request_async(http_client)
		.await
		.map_err(|error| {
			tracing::error!(?error, "User info fetch failed");
			APIError::OIDCTokenExchangeFailed(error.to_string())
		})?;
	let groups = id_token_groups
		.or_else(|| {
			extract_claim_groups(user_info.additional_claims().claims.get(groups_claim))
		})
		.unwrap_or_default();

	Ok(OidcClaims {
		subject: id_token_claims.subject().to_string(),
		email: user_info
			.email()
			.ok_or(APIError::OIDCMissingEmail)?
			.to_string(),
		name: user_info
			.name()
			.and_then(|n| n.get(None))
			.map(|n| n.to_string()),
		picture: user_info
			.picture()
			.and_then(|p| p.get(None))
			.map(|p| p.to_string()),
		groups,
	})
}

fn nonce_verifier(_nonce: Option<&Nonce>) -> Result<(), String> {
	Ok(())
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use models::shared::enums::UserPermission;
	use serde_json::json;

	use super::{all_permissions, extract_claim_groups, OidcClaims};
	use stump_core::config::OidcConfig;

	fn claims(groups: &[&str]) -> OidcClaims {
		OidcClaims {
			subject: "subject".to_string(),
			email: "user@example.com".to_string(),
			name: None,
			picture: None,
			groups: groups.iter().map(|group| (*group).to_string()).collect(),
		}
	}

	fn mapped_config() -> OidcConfig {
		OidcConfig {
			group_permissions: BTreeMap::from([
				(
					"readers".to_string(),
					vec![UserPermission::ReadUsers, UserPermission::DownloadFile],
				),
				(
					"writers".to_string(),
					vec![UserPermission::DownloadFile, UserPermission::ManageUsers],
				),
			]),
			..Default::default()
		}
	}

	#[test]
	fn claim_groups_accepts_array_or_single_string() {
		assert_eq!(
			extract_claim_groups(Some(&json!(["readers", 42]))),
			Some(vec!["readers".to_string()])
		);
		assert_eq!(
			extract_claim_groups(Some(&json!("admins"))),
			Some(vec!["admins".to_string()])
		);
		assert_eq!(
			extract_claim_groups(Some(&json!({"group": "admins"}))),
			None
		);
	}

	#[test]
	fn mapped_permissions_union_known_groups_and_ignore_unknown_groups() {
		let mapped = claims(&["writers", "unknown", "readers"])
			.mapped_permissions(&mapped_config())
			.expect("non-empty mapping should synchronize");

		assert_eq!(
			mapped,
			vec![
				UserPermission::ReadUsers,
				UserPermission::DownloadFile,
				UserPermission::ManageUsers,
			]
		);
	}

	#[test]
	fn mapped_permissions_empty_for_unmapped_groups_and_disabled_sync() {
		let config = mapped_config();
		assert_eq!(
			claims(&["unknown"]).mapped_permissions(&config),
			Some(vec![])
		);

		let disabled = OidcConfig {
			sync_permissions: false,
			..config
		};
		assert_eq!(claims(&["readers"]).mapped_permissions(&disabled), None);
		assert_eq!(
			claims(&["readers"]).mapped_permissions(&OidcConfig::default()),
			None
		);
	}

	#[test]
	fn admin_group_receives_every_permission_without_owner_flag() {
		let config = OidcConfig {
			admin_group: Some("admins".to_string()),
			..Default::default()
		};
		let mapped = claims(&["admins"])
			.mapped_permissions(&config)
			.expect("admin group should synchronize");

		assert_eq!(mapped, all_permissions().collect::<Vec<_>>());
	}
}
