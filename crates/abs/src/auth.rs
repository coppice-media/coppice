//! Audiobookshelf-shaped JWTs, minted and verified by Stump.
//!
//! abs-ref 2.36.0 issues three tokens from one login (captured in
//! `../komga-compat/abs/capture/login.json` and `refresh.json`):
//!
//! | field | claims | lifetime |
//! | --- | --- | --- |
//! | `user.token` | `{userId, username, iat}` | none (legacy, never expires) |
//! | `user.accessToken` | `{userId, username, jti, type:"access", iat, exp}` | 1 h (`exp - iat == 3600`) |
//! | `user.refreshToken` | `{userId, username, jti, type:"refresh", iat, exp}` | 30 d (`exp - iat == 2592000`) |
//!
//! All three are `HS256` (`{"alg":"HS256","typ":"JWT"}` in every captured
//! header) and `refreshToken` is only populated when the request carried
//! `x-return-tokens: true`; the key is always present and `null` otherwise.
//!
//! Stump signs with its own access-token secret and adds one claim ABS does
//! not have: `device`, the `devices.id` row a Lissen login registered, so the
//! device's library scope and transform profile apply to every later request
//! on that token. It is optional and ignored by clients.

use chrono::{Duration, Utc};
use jsonwebtoken::{
	decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use serde::{Deserialize, Serialize};

/// `TokenManager.accessTokenExpiry`: one hour.
pub const ACCESS_TOKEN_TTL: Duration = Duration::hours(1);
/// `TokenManager.refreshTokenExpiry`: thirty days.
pub const REFRESH_TOKEN_TTL: Duration = Duration::days(30);

/// Which of the two lifetimes a token carries. The legacy `token` has no
/// `type` claim at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenType {
	Access,
	Refresh,
}

/// Claims of an Audiobookshelf token. `user_id` is the Stump user uuid: ABS
/// user ids are uuids too, so no id mapping is needed for the subject.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AbsClaims {
	#[serde(rename = "userId")]
	pub user_id: String,
	pub username: String,
	/// Per-token id; present on the access/refresh pair, absent on the
	/// legacy token.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub jti: Option<String>,
	#[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
	pub token_type: Option<TokenType>,
	pub iat: i64,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub exp: Option<i64>,
	/// Stump extension: the `devices.id` this session is bound to.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub device: Option<String>,
}

impl AbsClaims {
	/// Whether the token may authenticate an API request. A refresh token
	/// only buys a new pair at `POST /auth/refresh`.
	pub fn is_access(&self) -> bool {
		!matches!(self.token_type, Some(TokenType::Refresh))
	}

	pub fn is_refresh(&self) -> bool {
		matches!(self.token_type, Some(TokenType::Refresh))
	}
}

/// The token trio a login or refresh hands back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintedTokens {
	/// `user.token`: the never-expiring legacy token.
	pub token: String,
	/// `user.accessToken`.
	pub access_token: String,
	/// `user.refreshToken`; the caller drops it unless the request asked for
	/// tokens.
	pub refresh_token: String,
}

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
	#[error("failed to encode Audiobookshelf token: {0}")]
	Encode(jsonwebtoken::errors::Error),
	#[error("invalid Audiobookshelf token: {0}")]
	Invalid(jsonwebtoken::errors::Error),
	#[error("Audiobookshelf token has expired")]
	Expired,
}

fn sign(secret: &[u8], claims: &AbsClaims) -> Result<String, TokenError> {
	encode(
		&Header::new(Algorithm::HS256),
		claims,
		&EncodingKey::from_secret(secret),
	)
	.map_err(TokenError::Encode)
}

/// Mint the legacy token, the access token and the refresh token for one
/// login, all bound to the same device row.
pub fn mint_tokens(
	secret: &[u8],
	user_id: &str,
	username: &str,
	device: Option<&str>,
) -> Result<MintedTokens, TokenError> {
	let now = Utc::now();
	let base = AbsClaims {
		user_id: user_id.to_owned(),
		username: username.to_owned(),
		jti: None,
		token_type: None,
		iat: now.timestamp(),
		exp: None,
		device: device.map(str::to_owned),
	};
	let token = sign(secret, &base)?;
	let access_token = sign(
		secret,
		&AbsClaims {
			jti: Some(uuid::Uuid::new_v4().to_string()),
			token_type: Some(TokenType::Access),
			exp: Some((now + ACCESS_TOKEN_TTL).timestamp()),
			..base.clone()
		},
	)?;
	let refresh_token = sign(
		secret,
		&AbsClaims {
			jti: Some(uuid::Uuid::new_v4().to_string()),
			token_type: Some(TokenType::Refresh),
			exp: Some((now + REFRESH_TOKEN_TTL).timestamp()),
			..base
		},
	)?;
	Ok(MintedTokens {
		token,
		access_token,
		refresh_token,
	})
}

/// Verify a token minted by [`mint_tokens`].
///
/// `exp` is enforced when present. The legacy token carries none, so the
/// signature is its only gate — the same trade abs-ref makes, since it hands
/// that token to every client that omits `x-return-tokens`.
pub fn verify_token(secret: &[u8], token: &str) -> Result<AbsClaims, TokenError> {
	let mut validation = Validation::new(Algorithm::HS256);
	// The legacy token has no `exp`; expiry is checked below so a token
	// without one is not rejected outright.
	validation.required_spec_claims.clear();
	validation.validate_exp = false;
	let claims =
		decode::<AbsClaims>(token, &DecodingKey::from_secret(secret), &validation)
			.map(|data| data.claims)
			.map_err(TokenError::Invalid)?;
	if let Some(exp) = claims.exp {
		if exp <= Utc::now().timestamp() {
			return Err(TokenError::Expired);
		}
	}
	Ok(claims)
}

/// Whether a bearer credential even looks like a JWT; used to skip the HMAC
/// for Stump API keys (`stump_...`) presented as bearer tokens.
pub fn looks_like_jwt(token: &str) -> bool {
	token.split('.').count() == 3
}

#[cfg(test)]
mod tests {
	use super::*;

	const SECRET: &[u8] = b"an-abs-profile-secret-of-some-length";

	#[test]
	fn minted_tokens_carry_the_abs_claim_shape() {
		let minted =
			mint_tokens(SECRET, "user-1", "ada", Some("device-9")).expect("mint tokens");

		let legacy = verify_token(SECRET, &minted.token).expect("legacy");
		assert_eq!(legacy.user_id, "user-1");
		assert_eq!(legacy.username, "ada");
		assert_eq!(legacy.device.as_deref(), Some("device-9"));
		assert_eq!(legacy.exp, None, "abs-ref's legacy token never expires");
		assert_eq!(legacy.jti, None);
		assert!(legacy.is_access());

		let access = verify_token(SECRET, &minted.access_token).expect("access");
		assert_eq!(access.token_type, Some(TokenType::Access));
		assert_eq!(
			access.exp.expect("exp") - access.iat,
			3600,
			"abs-ref accessToken lives one hour"
		);
		assert!(access.jti.is_some());
		assert!(access.is_access() && !access.is_refresh());

		let refresh = verify_token(SECRET, &minted.refresh_token).expect("refresh");
		assert_eq!(refresh.token_type, Some(TokenType::Refresh));
		assert_eq!(
			refresh.exp.expect("exp") - refresh.iat,
			2_592_000,
			"abs-ref refreshToken lives thirty days"
		);
		assert!(refresh.is_refresh() && !refresh.is_access());
		assert_ne!(access.jti, refresh.jti);
	}

	#[test]
	fn header_is_hs256_and_claims_use_abs_field_names() {
		let minted = mint_tokens(SECRET, "user-1", "ada", None).expect("mint");
		let mut parts = minted.access_token.split('.');
		let header = parts.next().expect("header");
		let payload = parts.next().expect("payload");
		let decode = |segment: &str| {
			use base64::Engine;
			let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
				.decode(segment)
				.expect("base64");
			serde_json::from_slice::<serde_json::Value>(&bytes).expect("json")
		};
		assert_eq!(
			decode(header),
			serde_json::json!({ "alg": "HS256", "typ": "JWT" })
		);
		let payload = decode(payload);
		assert_eq!(payload["userId"], "user-1");
		assert_eq!(payload["username"], "ada");
		assert_eq!(payload["type"], "access");
		assert!(payload.get("device").is_none(), "no device, no claim");
	}

	#[test]
	fn a_foreign_signature_is_rejected() {
		let minted = mint_tokens(SECRET, "user-1", "ada", None).expect("mint");
		assert!(matches!(
			verify_token(b"another-secret-entirely", &minted.access_token),
			Err(TokenError::Invalid(_))
		));
	}

	#[test]
	fn an_expired_access_token_is_rejected() {
		let now = Utc::now();
		let expired = sign(
			SECRET,
			&AbsClaims {
				user_id: "user-1".to_owned(),
				username: "ada".to_owned(),
				jti: Some("j".to_owned()),
				token_type: Some(TokenType::Access),
				iat: (now - Duration::hours(2)).timestamp(),
				exp: Some((now - Duration::hours(1)).timestamp()),
				device: None,
			},
		)
		.expect("sign");
		assert!(matches!(
			verify_token(SECRET, &expired),
			Err(TokenError::Expired)
		));
	}

	#[test]
	fn only_jwt_shaped_credentials_reach_the_hmac() {
		assert!(looks_like_jwt("a.b.c"));
		assert!(!looks_like_jwt("stump_abcd_efghijklmnop"));
	}
}
