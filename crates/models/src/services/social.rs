//! Database helpers for social authorization, state transitions and signals.
//!
//! GraphQL and protocol adapters call these helpers so membership, recipient
//! scope and lifecycle rules do not drift between entry points.

use chrono::{DateTime, Utc};
use sea_orm::{prelude::*, ActiveValue::Set, DatabaseConnection, IntoActiveModel};

use crate::entity::user::AuthUser;
use crate::entity::{
	media, social_recommendation, social_recommendation_signal, social_share_grant,
	social_share_overlay,
};
use crate::shared::visibility::VisibilityScope;

pub type SocialResult<T> = Result<T, SocialError>;

#[derive(Debug, thiserror::Error)]
pub enum SocialError {
	#[error("social target is not visible to this user")]
	TargetNotVisible,
	#[error("social recipient is unavailable")]
	RecipientUnavailable,
	#[error("social state transition is not allowed")]
	InvalidTransition,
	#[error("social scope is invalid")]
	InvalidScope,
	#[error("social target snapshot is invalid")]
	InvalidTarget,
	#[error("database error: {0}")]
	Db(#[from] DbErr),
}

/// A persisted share scope must contain only known bits and at least one bit.
pub fn validate_scope(scope_mask: i32) -> SocialResult<()> {
	if scope_mask <= 0 || scope_mask & !social_share_grant::VALID_SCOPES != 0 {
		return Err(SocialError::InvalidScope);
	}
	Ok(())
}

pub fn recommendation_transition_allowed(current: &str, next: &str) -> bool {
	match (current, next) {
		(social_recommendation::PENDING, social_recommendation::ACCEPTED)
		| (social_recommendation::PENDING, social_recommendation::DECLINED)
		| (social_recommendation::PENDING, social_recommendation::REVOKED)
		| (social_recommendation::PENDING, social_recommendation::EXPIRED)
		| (social_recommendation::ACCEPTED, social_recommendation::DISMISSED)
		| (social_recommendation::ACCEPTED, social_recommendation::REVOKED) => true,
		_ => false,
	}
}

pub fn share_transition_allowed(current: &str, next: &str) -> bool {
	match (current, next) {
		(social_share_grant::PENDING, social_share_grant::ACTIVE)
		| (social_share_grant::PENDING, social_share_grant::DECLINED)
		| (social_share_grant::PENDING, social_share_grant::REVOKED)
		| (social_share_grant::PENDING, social_share_grant::EXPIRED)
		| (social_share_grant::ACTIVE, social_share_grant::REVOKED) => true,
		_ => false,
	}
}

pub fn is_expired(expires_at: Option<DateTimeWithTimeZone>, now: DateTime<Utc>) -> bool {
	expires_at.is_some_and(|deadline| deadline < DateTimeWithTimeZone::from(now))
}

/// A stable key for local recommendation learning. It never contains a user id.
pub fn target_key(
	target_kind: &str,
	source_provider: Option<&str>,
	remote_id: Option<&str>,
	external_key: Option<&str>,
	title: &str,
	authors: &str,
) -> String {
	if let Some(key) = external_key.filter(|value| !value.trim().is_empty()) {
		return format!("{target_kind}:key:{key}");
	}
	if let (Some(provider), Some(remote)) = (
		source_provider.filter(|value| !value.trim().is_empty()),
		remote_id.filter(|value| !value.trim().is_empty()),
	) {
		return format!("{target_kind}:{provider}:{remote}");
	}
	format!(
		"{target_kind}:text:{}:{}",
		normalize(title),
		normalize(authors)
	)
}

fn normalize(value: &str) -> String {
	value
		.chars()
		.filter_map(|character| {
			if character.is_alphanumeric() {
				Some(character.to_ascii_lowercase())
			} else if character.is_whitespace() {
				Some('-')
			} else {
				None
			}
		})
		.collect::<String>()
		.trim_matches('-')
		.to_owned()
}

/// Validate and load a media target under the sender's current visibility.
pub async fn visible_media(
	conn: &DatabaseConnection,
	user: &AuthUser,
	media_id: &str,
) -> SocialResult<media::ModelWithMetadata> {
	media::ModelWithMetadata::find_by_id_for_user(
		media_id.to_owned(),
		VisibilityScope::from(user),
	)
	.into_model::<media::ModelWithMetadata>()
	.one(conn)
	.await?
	.ok_or(SocialError::TargetNotVisible)
}

/// Insert one durable signal. Callers should use a public reason code, never a
/// peer identity or private review text.
pub async fn record_signal(
	conn: &DatabaseConnection,
	user_id: &str,
	target_key: &str,
	signal_kind: &str,
	weight: i32,
	reason_code: &str,
	recommendation_id: Option<&str>,
) -> SocialResult<social_recommendation_signal::Model> {
	let active = social_recommendation_signal::ActiveModel {
		id: Set(uuid::Uuid::new_v4().to_string()),
		user_id: Set(user_id.to_owned()),
		target_key: Set(target_key.to_owned()),
		signal_kind: Set(signal_kind.to_owned()),
		weight: Set(weight),
		reason_code: Set(reason_code.to_owned()),
		recommendation_id: Set(recommendation_id.map(str::to_owned)),
		created_at: Set(Utc::now().into()),
	};
	Ok(active.insert(conn).await?)
}

/// Progress is intentionally coarse; exact locators, elapsed time and devices
/// never cross a sharing boundary.
pub fn coarse_percentage(progression: f64) -> i32 {
	let bounded = progression.clamp(0.0, 1.0) * 100.0;
	((bounded / 5.0).round() as i32 * 5).clamp(0, 100)
}

pub fn overlay_color(source_user_id: &str, target_key: &str) -> &'static str {
	social_share_overlay::Entity::deterministic_color(source_user_id, target_key)
}

/// Build a recommendation active model from an immutable snapshot. The caller
/// must have already verified recipient availability and sender target access.
pub fn recommendation_model(
	sender_user_id: &str,
	recipient_user_id: &str,
	target_kind: &str,
	title: String,
	authors: String,
	source_provider: Option<String>,
	remote_id: Option<String>,
	external_key: Option<String>,
	cover_url: Option<String>,
	message: Option<String>,
	expires_at: Option<DateTimeWithTimeZone>,
	idempotency_key: Option<String>,
) -> social_recommendation::ActiveModel {
	social_recommendation::ActiveModel {
		id: Set(uuid::Uuid::new_v4().to_string()),
		sender_user_id: Set(sender_user_id.to_owned()),
		recipient_user_id: Set(recipient_user_id.to_owned()),
		target_kind: Set(target_kind.to_owned()),
		title: Set(title),
		authors: Set(authors),
		source_provider: Set(source_provider),
		remote_id: Set(remote_id),
		external_key: Set(external_key),
		cover_url: Set(cover_url),
		message: Set(message),
		state: Set(social_recommendation::PENDING.to_owned()),
		handoff_state: Set(social_recommendation::HANDOFF_NONE.to_owned()),
		expires_at: Set(expires_at),
		idempotency_key: Set(idempotency_key),
		..Default::default()
	}
}

/// Return a recommendation with an expired pending state materialized. This is
/// intentionally used on reads as well as workers so expiry is deterministic.
pub async fn expire_recommendation_if_needed(
	conn: &DatabaseConnection,
	model: social_recommendation::Model,
) -> SocialResult<social_recommendation::Model> {
	if model.state == social_recommendation::PENDING
		&& is_expired(model.expires_at, Utc::now())
	{
		let mut active = model.clone().into_active_model();
		active.state = Set(social_recommendation::EXPIRED.to_owned());
		return Ok(active.update(conn).await?);
	}
	Ok(model)
}

/// Return a share grant with an expired pending/active state materialized.
pub async fn expire_share_if_needed(
	conn: &DatabaseConnection,
	model: social_share_grant::Model,
) -> SocialResult<social_share_grant::Model> {
	if matches!(
		model.state.as_str(),
		social_share_grant::PENDING | social_share_grant::ACTIVE
	) && is_expired(model.expires_at, Utc::now())
	{
		let mut active = model.clone().into_active_model();
		active.state = Set(social_share_grant::EXPIRED.to_owned());
		return Ok(active.update(conn).await?);
	}
	Ok(model)
}
