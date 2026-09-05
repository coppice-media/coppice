//! Kavita-shaped JWTs minted and verified by Stump.
//!
//! Kavita's `TokenService.CreateToken` signs an HS512 token carrying
//! `name` (username), `nameid` (user id), `role` (one claim per role), `nbf`,
//! `exp` (three days) and `iat`. The shape is preserved so clients that decode
//! the token (Kover reads the user from `Plugin/authenticate`) see the same
//! claims; only the signing key is Stump's own.

use chrono::{Duration, Utc};
use jsonwebtoken::{
	decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use serde::{Deserialize, Serialize};

/// Kavita access tokens live three days (`DateTime.UtcNow.AddDays(3)`).
pub const TOKEN_TTL: Duration = Duration::days(3);

/// Claims of a Kavita access token. `nameid` is the Kavita integer user id
/// (the `kavita_ids` mapping of the Stump user), never the Stump uuid.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KavitaClaims {
	pub name: String,
	pub nameid: String,
	pub role: Vec<String>,
	pub nbf: i64,
	pub exp: i64,
	pub iat: i64,
}

impl KavitaClaims {
	pub fn user_id(&self) -> Option<i32> {
		self.nameid.parse().ok()
	}
}

/// The Kavita roles a Stump user maps onto.
pub fn roles_for(is_server_owner: bool, can_download: bool) -> Vec<String> {
	let mut roles = Vec::with_capacity(3);
	if is_server_owner {
		roles.push("Admin".to_owned());
	}
	roles.push("Login".to_owned());
	if can_download || is_server_owner {
		roles.push("Download".to_owned());
	}
	roles
}

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
	#[error("failed to encode Kavita token: {0}")]
	Encode(jsonwebtoken::errors::Error),
	#[error("invalid Kavita token: {0}")]
	Invalid(jsonwebtoken::errors::Error),
}

/// Mint a Kavita access token for the given user.
pub fn mint_token(
	secret: &[u8],
	username: &str,
	kavita_user_id: i32,
	roles: &[String],
) -> Result<String, TokenError> {
	let now = Utc::now();
	let claims = KavitaClaims {
		name: username.to_owned(),
		nameid: kavita_user_id.to_string(),
		role: roles.to_vec(),
		nbf: now.timestamp(),
		exp: (now + TOKEN_TTL).timestamp(),
		iat: now.timestamp(),
	};
	encode(
		&Header::new(Algorithm::HS512),
		&claims,
		&EncodingKey::from_secret(secret),
	)
	.map_err(TokenError::Encode)
}

/// Verify a token minted by [`mint_token`], enforcing signature, `exp` and
/// `nbf`. Stump's own HS256 access tokens are rejected here and fall through
/// to the regular bearer handling in the server middleware.
pub fn verify_token(secret: &[u8], token: &str) -> Result<KavitaClaims, TokenError> {
	let mut validation = Validation::new(Algorithm::HS512);
	validation.validate_nbf = true;
	validation.set_required_spec_claims(&["exp", "nbf"]);
	decode::<KavitaClaims>(token, &DecodingKey::from_secret(secret), &validation)
		.map(|data| data.claims)
		.map_err(TokenError::Invalid)
}

/// Whether a bearer credential even looks like a JWT; used to skip the HMAC
/// for Stump API keys (`stump_...`) presented as bearer tokens.
pub fn looks_like_jwt(token: &str) -> bool {
	token.split('.').count() == 3
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn mint_and_verify_round_trip() {
		let secret = b"unit-test-secret";
		let roles = roles_for(true, false);
		let token = mint_token(secret, "admin", 7, &roles).unwrap();
		assert!(looks_like_jwt(&token));
		let header = jsonwebtoken::decode_header(&token).unwrap();
		assert_eq!(header.alg, Algorithm::HS512);
		let claims = verify_token(secret, &token).unwrap();
		assert_eq!(claims.name, "admin");
		assert_eq!(claims.user_id(), Some(7));
		assert_eq!(claims.role, vec!["Admin", "Login", "Download"]);
		assert_eq!(claims.exp - claims.iat, TOKEN_TTL.num_seconds());
		assert_eq!(claims.nbf, claims.iat);
	}

	#[test]
	fn rejects_foreign_secret_and_algorithm() {
		let token = mint_token(b"a", "admin", 1, &[]).unwrap();
		assert!(verify_token(b"b", &token).is_err());

		let hs256 = encode(
			&Header::default(),
			&KavitaClaims {
				name: "admin".into(),
				nameid: "1".into(),
				role: vec![],
				nbf: 0,
				exp: Utc::now().timestamp() + 60,
				iat: 0,
			},
			&EncodingKey::from_secret(b"a"),
		)
		.unwrap();
		assert!(verify_token(b"a", &hs256).is_err());
		assert!(!looks_like_jwt("stump_abcdef.ghijkl"));
	}

	#[test]
	fn roles_follow_stump_permissions() {
		assert_eq!(roles_for(false, false), vec!["Login"]);
		assert_eq!(roles_for(false, true), vec!["Login", "Download"]);
	}
}
