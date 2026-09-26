//! Privacy-scoped BookClub/social GraphQL operations.
//!
//! Every lookup starts from the authenticated user's participant/recipient
//! scope. Cross-user records are rendered from immutable snapshots; sender
//! media/work identifiers are never dereferenced for the recipient.

use std::collections::{BTreeMap, HashMap};

use async_graphql::{Enum, InputObject, Object, SimpleObject, ID};
use chrono::{DateTime, FixedOffset, Utc};
use models::{
	entity::{
		liseur_sync_work, social_preferences, social_recommendation,
		social_recommendation_signal, social_share_grant, social_share_overlay,
		social_share_overlay_preference, user,
	},
	services::social::{
		coarse_percentage, expire_recommendation_if_needed, expire_share_if_needed,
		recommendation_model, record_signal, target_key, validate_scope, visible_media,
	},
};
use sea_orm::{
	prelude::*, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
	QueryOrder, QuerySelect,
};
use stump_notify::{resolve_targets, Audience, Notification, NotificationKind, Rule};

use crate::{
	data::CoreContext,
	input::book_request::{ExternalWorkReferenceInput, RequestFormat},
};

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum RecommendationTargetKind {
	#[graphql(name = "INTERNAL_MEDIA")]
	InternalMedia,
	#[graphql(name = "INTERNAL_WORK")]
	InternalWork,
	#[graphql(name = "EXTERNAL_WORK")]
	ExternalWork,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum RecommendationState {
	#[graphql(name = "PENDING")]
	Pending,
	#[graphql(name = "ACCEPTED")]
	Accepted,
	#[graphql(name = "DECLINED")]
	Declined,
	#[graphql(name = "REVOKED")]
	Revoked,
	#[graphql(name = "EXPIRED")]
	Expired,
	#[graphql(name = "DISMISSED")]
	Dismissed,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum RecommendationDirection {
	#[graphql(name = "INCOMING")]
	Incoming,
	#[graphql(name = "OUTGOING")]
	Outgoing,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ShareScope {
	#[graphql(name = "METADATA")]
	Metadata,
	#[graphql(name = "RATING")]
	Rating,
	#[graphql(name = "REVIEW_TEXT")]
	ReviewText,
	#[graphql(name = "COMPLETION")]
	Completion,
	#[graphql(name = "PERCENTAGE")]
	Percentage,
	#[graphql(name = "ANNOTATIONS")]
	Annotations,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ShareState {
	#[graphql(name = "PENDING")]
	Pending,
	#[graphql(name = "ACTIVE")]
	Active,
	#[graphql(name = "DECLINED")]
	Declined,
	#[graphql(name = "REVOKED")]
	Revoked,
	#[graphql(name = "EXPIRED")]
	Expired,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ShareOverlayKind {
	#[graphql(name = "HIGHLIGHT")]
	Highlight,
	#[graphql(name = "NOTE")]
	Note,
	#[graphql(name = "BOOKMARK")]
	Bookmark,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum RecommendationHandoffState {
	#[graphql(name = "NONE")]
	None,
	#[graphql(name = "REQUESTED")]
	Requested,
	#[graphql(name = "LINKED")]
	Linked,
	#[graphql(name = "FAILED")]
	Failed,
}

#[derive(InputObject, Clone)]
pub struct SendRecommendationInput {
	pub recipient_user_id: ID,
	pub target_kind: RecommendationTargetKind,
	pub media_id: Option<ID>,
	pub work_id: Option<ID>,
	pub title: String,
	pub authors: String,
	pub message: Option<String>,
	pub source_provider: Option<String>,
	pub remote_id: Option<String>,
	pub external_key: Option<String>,
	pub cover_url: Option<String>,
}

#[derive(InputObject, Clone)]
pub struct RecommendationResponseInput {
	pub accept: bool,
}

#[derive(InputObject, Clone)]
pub struct RequestDestinationInput {
	pub shelf_id: Option<ID>,
	pub device_id: Option<ID>,
}

#[derive(InputObject, Clone)]
pub struct CreateShareGrantInput {
	pub recipient_user_id: ID,
	pub target_key: String,
	pub title: String,
	pub authors: String,
	pub scopes: Vec<ShareScope>,
	pub source_provider: Option<String>,
	pub remote_id: Option<String>,
	pub external_key: Option<String>,
	pub cover_url: Option<String>,
	pub target_media_id: Option<ID>,
	pub target_work_id: Option<ID>,
	pub recommendation_id: Option<ID>,
	pub expires_at: Option<DateTime<FixedOffset>>,
}

#[derive(InputObject, Clone)]
pub struct CreateShareOverlayInput {
	pub grant_id: ID,
	pub kind: ShareOverlayKind,
	pub locator: Option<async_graphql::Json<serde_json::Value>>,
	pub progression: Option<f64>,
	pub percentage: Option<i32>,
	pub excerpt: Option<String>,
	pub body: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct SocialUser {
	pub id: ID,
	pub username: String,
}

#[derive(SimpleObject, Clone)]
pub struct SocialPreferences {
	pub recommendations_opt_out: bool,
	pub sharing_opt_out: bool,
	pub updated_at: DateTime<FixedOffset>,
}

#[derive(SimpleObject, Clone)]
pub struct SocialRecommendation {
	pub id: ID,
	pub direction: RecommendationDirection,
	pub state: RecommendationState,
	pub target_kind: RecommendationTargetKind,
	pub title: String,
	pub authors: String,
	pub source_provider: Option<String>,
	pub remote_id: Option<String>,
	pub external_key: Option<String>,
	pub cover_url: Option<String>,
	pub message: Option<String>,
	pub handoff_state: RecommendationHandoffState,
	pub request_id: Option<String>,
	pub destination_shelf_id: Option<String>,
	pub created_at: DateTime<FixedOffset>,
	pub expires_at: Option<DateTime<FixedOffset>>,
	pub accepted_at: Option<DateTime<FixedOffset>>,
}

#[derive(SimpleObject, Clone)]
pub struct AdaptiveRecommendation {
	pub target_key: String,
	pub title: String,
	pub authors: String,
	pub source_provider: Option<String>,
	pub remote_id: Option<String>,
	pub external_key: Option<String>,
	pub cover_url: Option<String>,
	pub score: i32,
	pub reason_code: String,
}

#[derive(SimpleObject, Clone)]
pub struct SocialShareGrant {
	pub id: ID,
	pub target_key: String,
	pub title: String,
	pub authors: String,
	pub source_provider: Option<String>,
	pub remote_id: Option<String>,
	pub external_key: Option<String>,
	pub cover_url: Option<String>,
	pub scopes: Vec<ShareScope>,
	pub state: ShareState,
	pub expires_at: Option<DateTime<FixedOffset>>,
	pub accepted_at: Option<DateTime<FixedOffset>>,
}

#[derive(SimpleObject, Clone)]
pub struct SocialOverlay {
	pub id: ID,
	pub target_key: String,
	pub kind: ShareOverlayKind,
	pub locator: Option<async_graphql::Json<serde_json::Value>>,
	pub progression: Option<f64>,
	pub percentage: Option<i32>,
	pub excerpt: Option<String>,
	pub body: Option<String>,
	pub color: Option<String>,
	pub captured_at: DateTime<FixedOffset>,
	pub hidden: bool,
}

#[derive(Default)]
pub struct SocialQuery;

#[Object]
impl SocialQuery {
	async fn social_recommendations(
		&self,
		ctx: &async_graphql::Context<'_>,
		#[graphql(default = false)] recipient_only: bool,
		direction: Option<RecommendationDirection>,
		state: Option<RecommendationState>,
	) -> async_graphql::Result<Vec<SocialRecommendation>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let incoming_only = recipient_only
			|| matches!(direction, Some(RecommendationDirection::Incoming));
		let outgoing_only = matches!(direction, Some(RecommendationDirection::Outgoing));
		let rows = if incoming_only {
			social_recommendation::Entity::find_for_recipient(&user.id)
				.all(conn)
				.await?
		} else if outgoing_only {
			social_recommendation::Entity::find_for_sender(&user.id)
				.all(conn)
				.await?
		} else {
			social_recommendation::Entity::find_for_participant(&user.id)
				.all(conn)
				.await?
		};

		let mut result = Vec::with_capacity(rows.len());
		for row in rows {
			let row = expire_recommendation_if_needed(conn, row).await?;
			let row_state = recommendation_state(&row.state)?;
			if state.is_some_and(|requested| requested != row_state) {
				continue;
			}
			let row_direction = if row.recipient_user_id == user.id {
				RecommendationDirection::Incoming
			} else {
				RecommendationDirection::Outgoing
			};
			result.push(SocialRecommendation::from_model(row, row_direction)?);
		}
		Ok(result)
	}

	async fn adaptive_recommendations(
		&self,
		ctx: &async_graphql::Context<'_>,
		#[graphql(default = 20)] limit: i32,
	) -> async_graphql::Result<Vec<AdaptiveRecommendation>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		if social_preferences::Entity::find_by_id(&user.id)
			.one(conn)
			.await?
			.is_some_and(|preferences| preferences.recommendations_opt_out)
		{
			return Ok(vec![]);
		}

		let signals = social_recommendation_signal::Entity::find_for_user(&user.id)
			.all(conn)
			.await?;
		let mut scores: HashMap<String, (i32, String, bool)> = HashMap::new();
		for signal in signals {
			let entry = scores.entry(signal.target_key).or_insert((
				0,
				signal.reason_code.clone(),
				false,
			));
			entry.0 += signal.weight;
			if signal.signal_kind == social_recommendation_signal::DISMISS {
				entry.2 = true;
			}
			if signal.weight > 0 {
				entry.1 = signal.reason_code;
			}
		}

		let incoming = social_recommendation::Entity::find_for_recipient(&user.id)
			.order_by_desc(social_recommendation::Column::CreatedAt)
			.limit(250)
			.all(conn)
			.await?;
		let mut candidates: BTreeMap<
			String,
			(social_recommendation::Model, i32, String),
		> = BTreeMap::new();
		for row in incoming {
			if matches!(
				row.state.as_str(),
				social_recommendation::REVOKED
					| social_recommendation::DECLINED
					| social_recommendation::EXPIRED
					| social_recommendation::DISMISSED
			) {
				continue;
			}
			if row.target_kind == social_recommendation::TARGET_INTERNAL_MEDIA {
				if let Some(media_id) = &row.media_id {
					if visible_media(conn, user, media_id).await.is_ok() {
						continue;
					}
				}
			}
			if row.target_kind == social_recommendation::TARGET_INTERNAL_WORK
				&& row.work_id.is_some()
				&& liseur_sync_work::Entity::find_by_id(row.work_id.clone().unwrap())
					.filter(liseur_sync_work::Column::UserId.eq(&user.id))
					.one(conn)
					.await?
					.is_some()
			{
				continue;
			}
			let key = target_key(
				&row.target_kind,
				row.source_provider.as_deref(),
				row.remote_id.as_deref(),
				row.external_key.as_deref(),
				&row.title,
				&row.authors,
			);
			if let Some((score, reason, dismissed)) = scores.get(&key) {
				if *dismissed {
					continue;
				}
				candidates
					.entry(key)
					.or_insert((row, *score, reason.clone()));
			} else {
				candidates
					.entry(key)
					.or_insert((row, 0, "UNREAD_CATALOG".to_owned()));
			}
		}

		let mut values: Vec<_> = candidates
			.into_values()
			.map(|(row, score, reason_code)| AdaptiveRecommendation {
				target_key: target_key(
					&row.target_kind,
					row.source_provider.as_deref(),
					row.remote_id.as_deref(),
					row.external_key.as_deref(),
					&row.title,
					&row.authors,
				),
				title: row.title,
				authors: row.authors,
				source_provider: row.source_provider,
				remote_id: row.remote_id,
				external_key: row.external_key,
				cover_url: row.cover_url,
				score,
				reason_code,
			})
			.collect();
		values.sort_by(|left, right| {
			right
				.score
				.cmp(&left.score)
				.then_with(|| left.target_key.cmp(&right.target_key))
		});
		values.truncate(limit.clamp(1, 100) as usize);
		Ok(values)
	}

	async fn social_share_grants(
		&self,
		ctx: &async_graphql::Context<'_>,
		#[graphql(default = true)] incoming: bool,
	) -> async_graphql::Result<Vec<SocialShareGrant>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let rows = if incoming {
			social_share_grant::Entity::find_for_recipient(&user.id)
				.all(conn)
				.await?
		} else {
			social_share_grant::Entity::find_for_source(&user.id)
				.all(conn)
				.await?
		};
		let mut result = Vec::with_capacity(rows.len());
		for row in rows {
			let row = expire_share_if_needed(conn, row).await?;
			result.push(SocialShareGrant::from_model(row)?);
		}
		Ok(result)
	}

	async fn social_overlays(
		&self,
		ctx: &async_graphql::Context<'_>,
		target_key_filter: Option<String>,
	) -> async_graphql::Result<Vec<SocialOverlay>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let grants = social_share_grant::Entity::find_for_recipient(&user.id)
			.all(conn)
			.await?;
		let mut result = Vec::new();
		for grant in grants {
			let grant = expire_share_if_needed(conn, grant).await?;
			if grant.state != social_share_grant::ACTIVE
				|| target_key_filter
					.as_deref()
					.is_some_and(|key| key != grant.target_key)
			{
				continue;
			}
			let overlays = social_share_overlay::Entity::find()
				.filter(social_share_overlay::Column::GrantId.eq(&grant.id))
				.filter(social_share_overlay::Column::TombstonedAt.is_null())
				.all(conn)
				.await?;
			for overlay in overlays {
				let preference = social_share_overlay_preference::Entity::find_by_id((
					user.id.clone(),
					overlay.id.clone(),
				))
				.one(conn)
				.await?;
				result.push(SocialOverlay::from_model(
					overlay,
					preference.is_some_and(|value| {
						value.hidden_at.is_some() && value.restored_at.is_none()
					}),
				)?);
			}
		}
		Ok(result)
	}

	async fn social_user_search(
		&self,
		ctx: &async_graphql::Context<'_>,
		query: String,
	) -> async_graphql::Result<Vec<SocialUser>> {
		let stump_auth::AuthContext { user: current, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		if query.trim().len() < 2 {
			return Ok(vec![]);
		}
		let rows = user::Entity::find()
			.filter(user::Column::Username.contains(query.trim()))
			.filter(user::Column::DeletedAt.is_null())
			.filter(user::Column::IsLocked.eq(false))
			.filter(user::Column::Id.ne(&current.id))
			.order_by_asc(user::Column::Username)
			.limit(20)
			.all(conn)
			.await?;
		Ok(rows
			.into_iter()
			.map(|row| SocialUser {
				id: ID::from(row.id),
				username: row.username,
			})
			.collect())
	}

	async fn social_preferences(
		&self,
		ctx: &async_graphql::Context<'_>,
	) -> async_graphql::Result<SocialPreferences> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let model = social_preferences::Entity::find_by_id(&user.id)
			.one(conn)
			.await?;
		Ok(preferences_output(model))
	}
}

#[derive(Default)]
pub struct SocialMutation;

#[Object]
impl SocialMutation {
	async fn send_recommendation(
		&self,
		ctx: &async_graphql::Context<'_>,
		input: SendRecommendationInput,
	) -> async_graphql::Result<SocialRecommendation> {
		let stump_auth::AuthContext { user: sender, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		if sender.is_locked {
			return Err("Locked users cannot send recommendations".into());
		}
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();
		let recipient_id = input.recipient_user_id.to_string();
		if recipient_id == sender.id {
			return Err("You cannot recommend a work to yourself".into());
		}
		let recipient = user::Entity::find_by_id(&recipient_id)
			.filter(user::Column::DeletedAt.is_null())
			.filter(user::Column::IsLocked.eq(false))
			.one(conn)
			.await?
			.ok_or("Recipient not found")?;
		if social_preferences::Entity::find_by_id(&recipient.id)
			.one(conn)
			.await?
			.is_some_and(|preferences| preferences.recommendations_opt_out)
		{
			return Err("This user does not accept social recommendations".into());
		}

		let mut title = input.title.trim().to_owned();
		let mut authors = input.authors.trim().to_owned();
		let mut source_provider = input.source_provider.clone();
		let mut remote_id = input.remote_id.clone();
		let mut external_key = input.external_key.clone();
		let mut cover_url = input.cover_url.clone();
		let mut media_id = None;
		let mut work_id = None;
		let target_kind = match input.target_kind {
			RecommendationTargetKind::InternalMedia => {
				let id = input.media_id.as_ref().ok_or("mediaId is required")?;
				let visible = visible_media(conn, sender, id.as_ref())
					.await
					.map_err(|_| "Media is not visible to you")?;
				media_id = Some(id.to_string());
				if title.is_empty() {
					title = visible
						.metadata
						.as_ref()
						.and_then(|metadata| metadata.title.clone())
						.unwrap_or(visible.media.name.clone());
				}
				if authors.is_empty() {
					authors = visible
						.metadata
						.as_ref()
						.and_then(|metadata| metadata.writers.clone())
						.unwrap_or_default();
				}
				social_recommendation::TARGET_INTERNAL_MEDIA
			},
			RecommendationTargetKind::InternalWork => {
				let id = input.work_id.as_ref().ok_or("workId is required")?;
				let work = liseur_sync_work::Entity::find_by_id(id.to_string())
					.filter(liseur_sync_work::Column::UserId.eq(&sender.id))
					.one(conn)
					.await?
					.ok_or("Work is not visible to you")?;
				work_id = Some(id.to_string());
				if title.is_empty() {
					title = work.title;
				}
				if authors.is_empty() {
					authors = work.author;
				}
				social_recommendation::TARGET_INTERNAL_WORK
			},
			RecommendationTargetKind::ExternalWork => {
				if input.media_id.is_some() || input.work_id.is_some() {
					return Err(
						"External recommendations cannot contain internal IDs".into()
					);
				}
				if source_provider.as_deref().is_none_or(str::is_empty) {
					return Err(
						"sourceProvider is required for external recommendations".into(),
					);
				}
				social_recommendation::TARGET_EXTERNAL_WORK
			},
		};
		if title.is_empty() || authors.is_empty() {
			return Err("Title and authors are required".into());
		}
		if title.len() > 512 || authors.len() > 512 {
			return Err("Title and authors are too long".into());
		}
		if let Some(message) = &input.message {
			if message.len() > 2000 {
				return Err("Recommendation message is too long".into());
			}
		}
		let model = recommendation_model(
			&sender.id,
			&recipient.id,
			target_kind,
			title,
			authors,
			source_provider.take(),
			remote_id.take(),
			external_key.take(),
			cover_url.take(),
			input.message,
			Some((Utc::now() + chrono::Duration::days(30)).into()),
			None,
		);
		let mut model = model;
		model.media_id = Set(media_id);
		model.work_id = Set(work_id);
		let created = model.insert(conn).await?;
		let output = SocialRecommendation::from_model(
			created.clone(),
			RecommendationDirection::Outgoing,
		)?;
		let notification = Notification::new(
			NotificationKind::RecommendationReceived,
			"New recommendation",
			format!("{} recommended a title for you.", sender.username),
		);
		enqueue_recipient_notification(core, &recipient.id, notification).await;
		Ok(output)
	}

	async fn respond_to_recommendation(
		&self,
		ctx: &async_graphql::Context<'_>,
		id: ID,
		response: RecommendationResponseInput,
	) -> async_graphql::Result<SocialRecommendation> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();
		let row = social_recommendation::Entity::find_for_recipient(&user.id)
			.filter(social_recommendation::Column::Id.eq(id.to_string()))
			.one(conn)
			.await?
			.ok_or("Recommendation not found")?;
		let row = expire_recommendation_if_needed(conn, row).await?;
		if row.state != social_recommendation::PENDING {
			return Err("Recommendation is no longer pending".into());
		}
		let next = if response.accept {
			social_recommendation::ACCEPTED
		} else {
			social_recommendation::DECLINED
		};
		let mut active = row.clone().into_active_model();
		active.state = Set(next.to_owned());
		if response.accept {
			active.accepted_at = Set(Some(Utc::now().into()));
		} else {
			active.declined_at = Set(Some(Utc::now().into()));
		}
		let updated = active.update(conn).await?;
		let key = target_key(
			&updated.target_kind,
			updated.source_provider.as_deref(),
			updated.remote_id.as_deref(),
			updated.external_key.as_deref(),
			&updated.title,
			&updated.authors,
		);
		record_signal(
			conn,
			&user.id,
			&key,
			if response.accept {
				social_recommendation_signal::ACCEPT
			} else {
				social_recommendation_signal::DECLINE
			},
			if response.accept { 10 } else { -10 },
			"DIRECT_RECOMMENDATION",
			Some(&updated.id),
		)
		.await?;
		let kind = if response.accept {
			NotificationKind::RecommendationAccepted
		} else {
			NotificationKind::RecommendationDeclined
		};
		enqueue_recipient_notification(
			core,
			&updated.sender_user_id,
			Notification::new(
				kind,
				"Recommendation updated",
				"A recommendation response was recorded.",
			),
		)
		.await;
		SocialRecommendation::from_model(updated, RecommendationDirection::Incoming)
	}

	async fn revoke_recommendation(
		&self,
		ctx: &async_graphql::Context<'_>,
		id: ID,
	) -> async_graphql::Result<SocialRecommendation> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();
		let row = social_recommendation::Entity::find_for_sender(&user.id)
			.filter(social_recommendation::Column::Id.eq(id.to_string()))
			.one(conn)
			.await?
			.ok_or("Recommendation not found")?;
		if !matches!(
			row.state.as_str(),
			social_recommendation::PENDING | social_recommendation::ACCEPTED
		) {
			return Err("Recommendation cannot be revoked from its current state".into());
		}
		let recipient_id = row.recipient_user_id.clone();
		let mut active = row.into_active_model();
		active.state = Set(social_recommendation::REVOKED.to_owned());
		active.revoked_at = Set(Some(Utc::now().into()));
		let updated = active.update(conn).await?;
		enqueue_recipient_notification(
			core,
			&recipient_id,
			Notification::new(
				NotificationKind::RecommendationRevoked,
				"Recommendation revoked",
				"A recommendation is no longer available.",
			),
		)
		.await;
		SocialRecommendation::from_model(updated, RecommendationDirection::Outgoing)
	}

	async fn dismiss_recommendation(
		&self,
		ctx: &async_graphql::Context<'_>,
		id: ID,
	) -> async_graphql::Result<SocialRecommendation> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let row = social_recommendation::Entity::find_for_recipient(&user.id)
			.filter(social_recommendation::Column::Id.eq(id.to_string()))
			.one(conn)
			.await?
			.ok_or("Recommendation not found")?;
		if row.state != social_recommendation::ACCEPTED {
			return Err("Only an accepted recommendation can be dismissed".into());
		}
		let mut active = row.clone().into_active_model();
		active.state = Set(social_recommendation::DISMISSED.to_owned());
		active.dismissed_at = Set(Some(Utc::now().into()));
		let updated = active.update(conn).await?;
		let key = target_key(
			&updated.target_kind,
			updated.source_provider.as_deref(),
			updated.remote_id.as_deref(),
			updated.external_key.as_deref(),
			&updated.title,
			&updated.authors,
		);
		record_signal(
			conn,
			&user.id,
			&key,
			social_recommendation_signal::DISMISS,
			-20,
			"DIRECT_RECOMMENDATION_DISMISSED",
			Some(&updated.id),
		)
		.await?;
		SocialRecommendation::from_model(updated, RecommendationDirection::Incoming)
	}

	async fn request_recommendation(
		&self,
		ctx: &async_graphql::Context<'_>,
		id: ID,
		destination: Option<RequestDestinationInput>,
	) -> async_graphql::Result<SocialRecommendation> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();
		let row = social_recommendation::Entity::find_for_recipient(&user.id)
			.filter(social_recommendation::Column::Id.eq(id.to_string()))
			.one(conn)
			.await?
			.ok_or("Recommendation not found")?;
		if row.state != social_recommendation::ACCEPTED {
			return Err("Accept the recommendation before requesting it".into());
		}
		if row.handoff_state == social_recommendation::HANDOFF_LINKED {
			return SocialRecommendation::from_model(
				row,
				RecommendationDirection::Incoming,
			);
		}
		let destination_shelf_id = destination
			.as_ref()
			.and_then(|value| value.shelf_id.as_ref().map(|id| id.as_ref().to_owned()));
		let destination_device_id = destination
			.as_ref()
			.and_then(|value| value.device_id.as_ref().map(|id| id.as_ref().to_owned()));
		let external = if row.target_kind == social_recommendation::TARGET_EXTERNAL_WORK {
			Some(ExternalWorkReferenceInput {
				source_provider: row.source_provider.clone().ok_or("Missing provider")?,
				remote_id: row.remote_id.clone().ok_or("Missing remote ID")?,
				external_key: row.external_key.clone(),
				title: row.title.clone(),
				authors: Some(row.authors.clone()),
				cover_url: row.cover_url.clone(),
			})
		} else {
			None
		};
		let request = crate::mutation::create_request_from_recommendation(
			core,
			&user.id,
			row.media_id.clone(),
			row.work_id.clone(),
			external,
			Some(row.title.clone()),
			Some(row.authors.clone()),
			row.cover_url.clone(),
			destination_shelf_id.clone(),
			destination_device_id.clone(),
			RequestFormat::Any,
			None,
			None,
		)
		.await?;
		let mut active = row.into_active_model();
		active.handoff_state = Set(social_recommendation::HANDOFF_LINKED.to_owned());
		active.request_id = Set(Some(request.id.clone()));
		active.destination_shelf_id = Set(destination_shelf_id);
		active.destination_device_id = Set(destination_device_id);
		let updated = active.update(conn).await?;
		record_signal(
			conn,
			&user.id,
			&target_key(
				&updated.target_kind,
				updated.source_provider.as_deref(),
				updated.remote_id.as_deref(),
				updated.external_key.as_deref(),
				&updated.title,
				&updated.authors,
			),
			social_recommendation_signal::REQUEST,
			5,
			"DIRECT_RECOMMENDATION_REQUESTED",
			Some(&updated.id),
		)
		.await?;
		enqueue_recipient_notification(
			core,
			&updated.sender_user_id,
			Notification::new(
				NotificationKind::RequestStatusUpdated,
				"Recommendation requested",
				"A recipient requested an item from your recommendation.",
			),
		)
		.await;
		SocialRecommendation::from_model(updated, RecommendationDirection::Incoming)
	}

	async fn set_recommendation_opt_out(
		&self,
		ctx: &async_graphql::Context<'_>,
		opt_out: bool,
	) -> async_graphql::Result<SocialPreferences> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let preferences = social_preferences::Entity::find_by_id(&user.id)
			.one(conn)
			.await?;
		let model = if let Some(model) = preferences {
			let mut active = model.into_active_model();
			active.recommendations_opt_out = Set(opt_out);
			active.updated_at = Set(Utc::now().into());
			active.update(conn).await?
		} else {
			social_preferences::ActiveModel {
				user_id: Set(user.id.clone()),
				recommendations_opt_out: Set(opt_out),
				sharing_opt_out: Set(false),
				updated_at: Set(Utc::now().into()),
			}
			.insert(conn)
			.await?
		};
		Ok(preferences_output(Some(model)))
	}

	async fn create_share_grant(
		&self,
		ctx: &async_graphql::Context<'_>,
		input: CreateShareGrantInput,
	) -> async_graphql::Result<SocialShareGrant> {
		let stump_auth::AuthContext { user: source, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();
		let recipient_id = input.recipient_user_id.to_string();
		if recipient_id == source.id {
			return Err("You cannot share with yourself".into());
		}
		let recipient = user::Entity::find_by_id(&recipient_id)
			.filter(user::Column::DeletedAt.is_null())
			.filter(user::Column::IsLocked.eq(false))
			.one(conn)
			.await?
			.ok_or("Recipient not found")?;
		if social_preferences::Entity::find_by_id(&recipient.id)
			.one(conn)
			.await?
			.is_some_and(|preferences| preferences.sharing_opt_out)
		{
			return Err("This user does not accept shared reading data".into());
		}
		let scope_mask = input
			.scopes
			.iter()
			.fold(0, |mask, scope| mask | scope.bit());
		validate_scope(scope_mask).map_err(|error| error.to_string())?;
		if input.target_key.trim().is_empty() || input.title.trim().is_empty() {
			return Err("A target key and title are required".into());
		}
		if let Some(media_id) = &input.target_media_id {
			visible_media(conn, source, media_id.as_ref())
				.await
				.map_err(|_| "Media is not visible to you")?;
		}
		if let Some(work_id) = &input.target_work_id {
			if liseur_sync_work::Entity::find_by_id(work_id.to_string())
				.filter(liseur_sync_work::Column::UserId.eq(&source.id))
				.one(conn)
				.await?
				.is_none()
			{
				return Err("Work is not visible to you".into());
			}
		}
		if let Some(recommendation_id) = &input.recommendation_id {
			let valid = social_recommendation::Entity::find_for_sender(&source.id)
				.filter(
					social_recommendation::Column::Id.eq(recommendation_id.to_string()),
				)
				.filter(social_recommendation::Column::RecipientUserId.eq(&recipient.id))
				.one(conn)
				.await?
				.is_some();
			if !valid {
				return Err(
					"Recommendation is not owned by this sender/recipient pair".into()
				);
			}
		}
		let model = social_share_grant::ActiveModel {
			id: Set(uuid::Uuid::new_v4().to_string()),
			target_key: Set(input.target_key),
			target_media_id: Set(input.target_media_id.map(|id| id.to_string())),
			target_work_id: Set(input.target_work_id.map(|id| id.to_string())),
			source_provider: Set(input.source_provider),
			remote_id: Set(input.remote_id),
			external_key: Set(input.external_key),
			title: Set(input.title),
			authors: Set(input.authors),
			cover_url: Set(input.cover_url),
			source_user_id: Set(source.id.clone()),
			recipient_user_id: Set(recipient.id.clone()),
			recommendation_id: Set(input.recommendation_id.map(|id| id.to_string())),
			scope_mask: Set(scope_mask),
			state: Set(social_share_grant::PENDING.to_owned()),
			expires_at: Set(input.expires_at.map(Into::into)),
			created_at: Set(Utc::now().into()),
			..Default::default()
		};
		let created = model.insert(conn).await?;
		enqueue_recipient_notification(
			core,
			&recipient.id,
			Notification::new(
				NotificationKind::RecommendationReceived,
				"Shared reading request",
				"Someone requested a scoped reading share.",
			),
		)
		.await;
		SocialShareGrant::from_model(created)
	}

	async fn respond_to_share_grant(
		&self,
		ctx: &async_graphql::Context<'_>,
		id: ID,
		accept: bool,
	) -> async_graphql::Result<SocialShareGrant> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();
		let row = social_share_grant::Entity::find_for_recipient(&user.id)
			.filter(social_share_grant::Column::Id.eq(id.to_string()))
			.one(conn)
			.await?
			.ok_or("Share grant not found")?;
		let row = expire_share_if_needed(conn, row).await?;
		if row.state != social_share_grant::PENDING {
			return Err("Share grant is no longer pending".into());
		}
		let source_id = row.source_user_id.clone();
		let mut active = row.into_active_model();
		active.state = Set(if accept {
			social_share_grant::ACTIVE.to_owned()
		} else {
			social_share_grant::DECLINED.to_owned()
		});
		if accept {
			active.accepted_at = Set(Some(Utc::now().into()));
		} else {
			active.declined_at = Set(Some(Utc::now().into()));
		}
		let updated = active.update(conn).await?;
		enqueue_recipient_notification(
			core,
			&source_id,
			Notification::new(
				if accept {
					NotificationKind::RecommendationAccepted
				} else {
					NotificationKind::RecommendationDeclined
				},
				"Share grant updated",
				"A scoped reading share was updated.",
			),
		)
		.await;
		SocialShareGrant::from_model(updated)
	}

	async fn revoke_share_grant(
		&self,
		ctx: &async_graphql::Context<'_>,
		id: ID,
	) -> async_graphql::Result<SocialShareGrant> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();
		let row = social_share_grant::Entity::find_for_participant(&user.id)
			.filter(social_share_grant::Column::Id.eq(id.to_string()))
			.one(conn)
			.await?
			.ok_or("Share grant not found")?;
		if !matches!(
			row.state.as_str(),
			social_share_grant::PENDING | social_share_grant::ACTIVE
		) {
			return Err("Share grant cannot be revoked from its current state".into());
		}
		let recipient_id = row.recipient_user_id.clone();
		let mut active = row.into_active_model();
		active.state = Set(social_share_grant::REVOKED.to_owned());
		active.revoked_at = Set(Some(Utc::now().into()));
		active.revoked_by_user_id = Set(Some(user.id.clone()));
		let updated = active.update(conn).await?;
		social_share_overlay::Entity::update_many()
			.col_expr(
				social_share_overlay::Column::TombstonedAt,
				sea_orm::sea_query::Expr::value(Utc::now().fixed_offset()),
			)
			.filter(social_share_overlay::Column::GrantId.eq(&updated.id))
			.exec(conn)
			.await?;
		enqueue_recipient_notification(
			core,
			&recipient_id,
			Notification::new(
				NotificationKind::RecommendationRevoked,
				"Shared reading revoked",
				"A shared reading view is no longer available.",
			),
		)
		.await;
		SocialShareGrant::from_model(updated)
	}

	async fn create_share_overlay(
		&self,
		ctx: &async_graphql::Context<'_>,
		input: CreateShareOverlayInput,
	) -> async_graphql::Result<SocialOverlay> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let grant = social_share_grant::Entity::find_for_source(&user.id)
			.filter(social_share_grant::Column::Id.eq(input.grant_id.to_string()))
			.one(conn)
			.await?
			.ok_or("Share grant not found")?;
		if grant.state != social_share_grant::ACTIVE {
			return Err("Share grant is not writable".into());
		}
		if grant.scope_mask & social_share_grant::SCOPE_ANNOTATIONS == 0 {
			return Err("The grant does not include annotations".into());
		}
		let (kind, color) = match input.kind {
			ShareOverlayKind::Highlight => (
				social_share_overlay::HIGHLIGHT,
				Some(
					social_share_overlay::Entity::deterministic_color(
						&user.id,
						&grant.target_key,
					)
					.to_owned(),
				),
			),
			ShareOverlayKind::Note => (social_share_overlay::NOTE, None),
			ShareOverlayKind::Bookmark => (social_share_overlay::BOOKMARK, None),
		};
		let progression = if grant.scope_mask
			& (social_share_grant::SCOPE_COMPLETION
				| social_share_grant::SCOPE_PERCENTAGE)
			!= 0
		{
			input
				.progression
				.map(|value| f64::from(coarse_percentage(value)) / 100.0)
		} else {
			None
		};
		let percentage = if grant.scope_mask & social_share_grant::SCOPE_PERCENTAGE != 0 {
			input
				.percentage
				.or_else(|| input.progression.map(coarse_percentage))
				.map(|value| value.clamp(0, 100) / 5 * 5)
		} else {
			None
		};
		let locator = if grant.scope_mask & social_share_grant::SCOPE_ANNOTATIONS != 0 {
			input.locator.map(|value| value.0)
		} else {
			None
		};
		let target_key_for_overlay = grant.target_key.clone();
		let grant_id = grant.id.clone();
		let model = social_share_overlay::ActiveModel {
			id: Set(uuid::Uuid::new_v4().to_string()),
			grant_id: Set(grant_id),
			source_user_id: Set(user.id.clone()),
			target_key: Set(target_key_for_overlay),
			kind: Set(kind.to_owned()),
			locator: Set(locator),
			progression: Set(progression),
			percentage: Set(percentage),
			excerpt: Set(input
				.excerpt
				.map(|value| value.chars().take(2000).collect())),
			body: Set(input.body.map(|value| value.chars().take(4000).collect())),
			color: Set(color),
			captured_at: Set(Utc::now().into()),
			revision: Set(1),
			tombstoned_at: Set(None),
		};
		let created = model.insert(conn).await?;
		SocialOverlay::from_model(created, false)
	}

	async fn set_overlay_visibility(
		&self,
		ctx: &async_graphql::Context<'_>,
		overlay_id: ID,
		hidden: bool,
	) -> async_graphql::Result<SocialOverlay> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let overlay = social_share_overlay::Entity::find_by_id(overlay_id.to_string())
			.one(conn)
			.await?
			.ok_or("Overlay not found")?;
		let grant = social_share_grant::Entity::find_for_recipient(&user.id)
			.filter(social_share_grant::Column::Id.eq(&overlay.grant_id))
			.one(conn)
			.await?
			.ok_or("Overlay not found")?;
		if grant.state != social_share_grant::ACTIVE {
			return Err("Overlay is no longer available".into());
		}
		let key = (user.id.clone(), overlay.id.clone());
		let preference = social_share_overlay_preference::Entity::find_by_id(key.clone())
			.one(conn)
			.await?;
		if let Some(preference) = preference {
			let mut active = preference.into_active_model();
			if hidden {
				active.hidden_at = Set(Some(Utc::now().into()));
				active.restored_at = Set(None);
			} else {
				active.restored_at = Set(Some(Utc::now().into()));
			}
			active.update(conn).await?;
		} else {
			social_share_overlay_preference::ActiveModel {
				recipient_user_id: Set(user.id.clone()),
				overlay_id: Set(overlay.id.clone()),
				hidden_at: Set(hidden.then(|| Utc::now().into())),
				restored_at: Set(None),
			}
			.insert(conn)
			.await?;
		}
		SocialOverlay::from_model(overlay, hidden)
	}
}

impl ShareScope {
	fn bit(self) -> i32 {
		match self {
			Self::Metadata => social_share_grant::SCOPE_METADATA,
			Self::Rating => social_share_grant::SCOPE_RATING,
			Self::ReviewText => social_share_grant::SCOPE_REVIEW_TEXT,
			Self::Completion => social_share_grant::SCOPE_COMPLETION,
			Self::Percentage => social_share_grant::SCOPE_PERCENTAGE,
			Self::Annotations => social_share_grant::SCOPE_ANNOTATIONS,
		}
	}
}

fn recommendation_state(value: &str) -> async_graphql::Result<RecommendationState> {
	match value {
		social_recommendation::PENDING => Ok(RecommendationState::Pending),
		social_recommendation::ACCEPTED => Ok(RecommendationState::Accepted),
		social_recommendation::DECLINED => Ok(RecommendationState::Declined),
		social_recommendation::REVOKED => Ok(RecommendationState::Revoked),
		social_recommendation::EXPIRED => Ok(RecommendationState::Expired),
		social_recommendation::DISMISSED => Ok(RecommendationState::Dismissed),
		_ => Err("Invalid recommendation state".into()),
	}
}

fn recommendation_kind(value: &str) -> async_graphql::Result<RecommendationTargetKind> {
	match value {
		social_recommendation::TARGET_INTERNAL_MEDIA => {
			Ok(RecommendationTargetKind::InternalMedia)
		},
		social_recommendation::TARGET_INTERNAL_WORK => {
			Ok(RecommendationTargetKind::InternalWork)
		},
		social_recommendation::TARGET_EXTERNAL_WORK => {
			Ok(RecommendationTargetKind::ExternalWork)
		},
		_ => Err("Invalid recommendation target kind".into()),
	}
}

fn recommendation_handoff(
	value: &str,
) -> async_graphql::Result<RecommendationHandoffState> {
	match value {
		social_recommendation::HANDOFF_NONE => Ok(RecommendationHandoffState::None),
		social_recommendation::HANDOFF_REQUESTED => {
			Ok(RecommendationHandoffState::Requested)
		},
		social_recommendation::HANDOFF_LINKED => Ok(RecommendationHandoffState::Linked),
		social_recommendation::HANDOFF_FAILED => Ok(RecommendationHandoffState::Failed),
		_ => Err("Invalid recommendation handoff state".into()),
	}
}

fn share_state(value: &str) -> async_graphql::Result<ShareState> {
	match value {
		social_share_grant::PENDING => Ok(ShareState::Pending),
		social_share_grant::ACTIVE => Ok(ShareState::Active),
		social_share_grant::DECLINED => Ok(ShareState::Declined),
		social_share_grant::REVOKED => Ok(ShareState::Revoked),
		social_share_grant::EXPIRED => Ok(ShareState::Expired),
		_ => Err("Invalid share state".into()),
	}
}

fn overlay_kind(value: &str) -> async_graphql::Result<ShareOverlayKind> {
	match value {
		social_share_overlay::HIGHLIGHT => Ok(ShareOverlayKind::Highlight),
		social_share_overlay::NOTE => Ok(ShareOverlayKind::Note),
		social_share_overlay::BOOKMARK => Ok(ShareOverlayKind::Bookmark),
		_ => Err("Invalid overlay kind".into()),
	}
}

impl SocialRecommendation {
	fn from_model(
		model: social_recommendation::Model,
		direction: RecommendationDirection,
	) -> async_graphql::Result<Self> {
		Ok(Self {
			id: ID::from(model.id),
			direction,
			state: recommendation_state(&model.state)?,
			target_kind: recommendation_kind(&model.target_kind)?,
			title: model.title,
			authors: model.authors,
			source_provider: model.source_provider,
			remote_id: model.remote_id,
			external_key: model.external_key,
			cover_url: model.cover_url,
			message: model.message,
			handoff_state: recommendation_handoff(&model.handoff_state)?,
			request_id: model.request_id,
			destination_shelf_id: model.destination_shelf_id,
			created_at: model.created_at.fixed_offset(),
			expires_at: model.expires_at.map(|value| value.fixed_offset()),
			accepted_at: model.accepted_at.map(|value| value.fixed_offset()),
		})
	}
}

impl SocialShareGrant {
	fn from_model(model: social_share_grant::Model) -> async_graphql::Result<Self> {
		Ok(Self {
			id: ID::from(model.id),
			target_key: model.target_key,
			title: model.title,
			authors: model.authors,
			source_provider: model.source_provider,
			remote_id: model.remote_id,
			external_key: model.external_key,
			cover_url: model.cover_url,
			scopes: scopes(model.scope_mask),
			state: share_state(&model.state)?,
			expires_at: model.expires_at.map(|value| value.fixed_offset()),
			accepted_at: model.accepted_at.map(|value| value.fixed_offset()),
		})
	}
}

impl SocialOverlay {
	fn from_model(
		model: social_share_overlay::Model,
		hidden: bool,
	) -> async_graphql::Result<Self> {
		Ok(Self {
			id: ID::from(model.id),
			target_key: model.target_key,
			kind: overlay_kind(&model.kind)?,
			locator: model.locator.map(async_graphql::Json),
			progression: model.progression,
			percentage: model.percentage,
			excerpt: model.excerpt,
			body: model.body,
			color: model.color,
			captured_at: model.captured_at.fixed_offset(),
			hidden,
		})
	}
}

fn scopes(mask: i32) -> Vec<ShareScope> {
	[
		(social_share_grant::SCOPE_METADATA, ShareScope::Metadata),
		(social_share_grant::SCOPE_RATING, ShareScope::Rating),
		(
			social_share_grant::SCOPE_REVIEW_TEXT,
			ShareScope::ReviewText,
		),
		(social_share_grant::SCOPE_COMPLETION, ShareScope::Completion),
		(social_share_grant::SCOPE_PERCENTAGE, ShareScope::Percentage),
		(
			social_share_grant::SCOPE_ANNOTATIONS,
			ShareScope::Annotations,
		),
	]
	.into_iter()
	.filter_map(|(bit, scope)| (mask & bit != 0).then_some(scope))
	.collect()
}

fn preferences_output(model: Option<social_preferences::Model>) -> SocialPreferences {
	match model {
		Some(model) => SocialPreferences {
			recommendations_opt_out: model.recommendations_opt_out,
			sharing_opt_out: model.sharing_opt_out,
			updated_at: model.updated_at.fixed_offset(),
		},
		None => SocialPreferences {
			recommendations_opt_out: false,
			sharing_opt_out: false,
			updated_at: DateTime::<FixedOffset>::from(Utc::now()).fixed_offset(),
		},
	}
}

async fn enqueue_recipient_notification(
	core: &CoreContext,
	user_id: &str,
	notification: Notification,
) {
	let result = async {
		let rules = models::entity::notification_rule::Entity::find()
			.all(core.conn.as_ref())
			.await?
			.into_iter()
			.map(|model| Rule {
				user_id: model.user_id,
				event_kind: model.event_kind,
				channel_id: model.channel_id,
				enabled: model.enabled,
			})
			.collect::<Vec<_>>();
		let targets = resolve_targets(
			&rules,
			&Audience::user(user_id.to_owned()),
			notification.kind,
		);
		if targets.is_empty() {
			return Ok::<(), sea_orm::DbErr>(());
		}
		let deliveries = targets
			.into_iter()
			.map(|target| stump_core::job::notification::QueuedDelivery {
				user_id: target.user_id,
				channel_id: target.channel_id,
				notification: notification.clone(),
			})
			.collect();
		core.enqueue(stump_core::job::stump_job::StumpJob::NotificationDispatch {
			deliveries,
		})
		.await
		.map_err(|error| sea_orm::DbErr::Custom(error.to_string()))
		.map(|_| ())
	};
	if let Err(error) = result.await {
		tracing::warn!(?error, user_id, "failed to enqueue social notification");
	}
}
