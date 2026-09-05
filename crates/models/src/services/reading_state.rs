//! The unified reading-state service: one canonical head per `(user, media)`
//! plus append-only provenance, shared by every protocol adapter.
//!
//! Callers translate their wire payload into a
//! [`ProtocolUpdate`](crate::domain::reading_state::ProtocolUpdate) and call
//! [`apply`]; readers call [`head`]/[`heads`]. Sessions (`reading_sessions`)
//! stay the statistics history and are written by the routes that already do
//! so; this service never touches them.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sea_orm::{
	prelude::*, ActiveValue::Set, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder,
};

use crate::{
	domain::reading_state::{
		project, resolve, HeadState, Outcome, ProtocolUpdate, Publication,
		SourceProtocol, TimestampKind,
	},
	entity::{reading_head, reading_head_event},
};

/// The result of applying a protocol update.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
	/// The head after the update (unchanged when `outcome` is provenance only).
	pub head: reading_head::Model,
	/// The provenance row recorded for the update.
	pub event: reading_head_event::Model,
	pub outcome: Outcome,
}

impl Applied {
	pub fn accepted(&self) -> bool {
		self.outcome == Outcome::Accepted
	}
}

/// Apply a protocol update to the user's head for `publication`, recording
/// the update as provenance whether or not it wins the conflict rule.
///
/// The caller owns the transaction; `conn` may be a transaction handle.
pub async fn apply<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	publication: Publication<'_>,
	update: ProtocolUpdate,
) -> Result<Applied, DbErr> {
	let now = Utc::now();
	let (source_updated_at, timestamp_kind) = match update.updated_at {
		Some(at) => (at, TimestampKind::Device),
		None => (now, TimestampKind::Server),
	};
	let media_id = publication.media_id;
	let projection = project(&update, &publication);

	let existing = head(conn, user_id, media_id).await?;
	let resolved = resolve(
		existing.as_ref().map(|head| HeadState {
			updated_at: head.updated_at.to_utc(),
			progression: head.progression,
			completed: head.completed,
		}),
		&projection,
		source_updated_at,
	);

	let event = reading_head_event::ActiveModel {
		user_id: Set(user_id.to_string()),
		media_id: Set(media_id.to_string()),
		protocol: Set(update.protocol),
		device_id: Set(update.device_id.clone()),
		raw_payload: Set(update.raw_payload),
		locator: Set(projection.locator.clone()),
		progression: Set(projection.progression),
		page: Set(projection.page),
		completed: Set(projection.completed),
		timestamp_kind: Set(timestamp_kind),
		source_updated_at: Set(source_updated_at.into()),
		received_at: Set(now.into()),
		applied: Set(resolved.outcome == Outcome::Accepted),
		..Default::default()
	}
	.insert(conn)
	.await?;

	let head = match (existing, resolved.outcome) {
		(Some(head), Outcome::ProvenanceOnly) => head,
		(existing, Outcome::Accepted) => {
			let previous = existing.as_ref();
			let active = reading_head::ActiveModel {
				user_id: Set(user_id.to_string()),
				media_id: Set(media_id.to_string()),
				locator: Set(projection
					.locator
					.or_else(|| previous.and_then(|head| head.locator.clone()))),
				progression: Set(resolved.progression),
				page: Set(projection
					.page
					.or_else(|| previous.and_then(|head| head.page))),
				completed: Set(resolved.completed),
				updated_at: Set(source_updated_at.into()),
				created_at: Set(previous.map_or(now.into(), |head| head.created_at)),
				changed_at: Set(now.into()),
				source_protocol: Set(update.protocol),
				source_device_id: Set(update.device_id),
				revision: Set(previous.map_or(1, |head| head.revision + 1)),
				event_id: Set(event.id),
			};
			if previous.is_some() {
				active.update(conn).await?
			} else {
				active.insert(conn).await?
			}
		},
		(None, Outcome::ProvenanceOnly) => {
			unreachable!("an update without a head is always accepted")
		},
	};

	Ok(Applied {
		head,
		event,
		outcome: resolved.outcome,
	})
}

/// The user's head for one media item.
pub async fn head<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	media_id: &str,
) -> Result<Option<reading_head::Model>, DbErr> {
	reading_head::Entity::find_by_id((user_id.to_string(), media_id.to_string()))
		.one(conn)
		.await
}

/// The user's heads for a set of media items, keyed by media id.
pub async fn heads<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	media_ids: &[String],
) -> Result<HashMap<String, reading_head::Model>, DbErr> {
	if media_ids.is_empty() {
		return Ok(HashMap::new());
	}
	let rows = reading_head::Entity::find_for_user(user_id)
		.filter(reading_head::Column::MediaId.is_in(media_ids.to_vec()))
		.all(conn)
		.await?;
	Ok(rows
		.into_iter()
		.map(|head| (head.media_id.clone(), head))
		.collect())
}

/// Heads whose materialized state changed (server time) after `since`,
/// oldest change first. This is the cursor for feeds that mirror head
/// changes; it deliberately ignores the (device-supplied) `updated_at`.
pub async fn heads_since<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	since: DateTime<Utc>,
) -> Result<Vec<reading_head::Model>, DbErr> {
	reading_head::Entity::find_for_user(user_id)
		.filter(reading_head::Column::ChangedAt.gt(DateTimeWithTimeZone::from(since)))
		.order_by_asc(reading_head::Column::ChangedAt)
		.order_by_asc(reading_head::Column::MediaId)
		.all(conn)
		.await
}

/// The most recent provenance row a protocol recorded for the user's media,
/// whether or not it moved the head. Adapters use it to replay native fields
/// (Kobo statistics, a KOReader x-pointer) that the head does not project.
pub async fn latest_event<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	media_id: &str,
	protocol: SourceProtocol,
) -> Result<Option<reading_head_event::Model>, DbErr> {
	reading_head_event::Entity::find()
		.filter(reading_head_event::Column::UserId.eq(user_id))
		.filter(reading_head_event::Column::MediaId.eq(media_id))
		.filter(reading_head_event::Column::Protocol.eq(protocol))
		.order_by_desc(reading_head_event::Column::Id)
		.one(conn)
		.await
}

/// The provenance row that materialized `head`.
pub async fn winning_event<C: ConnectionTrait>(
	conn: &C,
	head: &reading_head::Model,
) -> Result<Option<reading_head_event::Model>, DbErr> {
	reading_head_event::Entity::find_by_id(head.event_id)
		.one(conn)
		.await
}

/// An explicit reset: delete the user's heads for `media_ids`, recording a
/// `{"cleared": true}` provenance row per deleted head so the reset is
/// attributable. Returns the number of heads removed.
pub async fn clear<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	media_ids: &[String],
	protocol: SourceProtocol,
	device_id: Option<String>,
) -> Result<u64, DbErr> {
	if media_ids.is_empty() {
		return Ok(0);
	}
	let heads = reading_head::Entity::find_for_user(user_id)
		.filter(reading_head::Column::MediaId.is_in(media_ids.to_vec()))
		.all(conn)
		.await?;
	if heads.is_empty() {
		return Ok(0);
	}
	let now = DateTimeWithTimeZone::from(Utc::now());
	for head in &heads {
		reading_head_event::ActiveModel {
			user_id: Set(user_id.to_string()),
			media_id: Set(head.media_id.clone()),
			protocol: Set(protocol),
			device_id: Set(device_id.clone()),
			raw_payload: Set(serde_json::json!({ "cleared": true })),
			locator: Set(None),
			progression: Set(None),
			page: Set(None),
			completed: Set(Some(false)),
			timestamp_kind: Set(TimestampKind::Server),
			source_updated_at: Set(now),
			received_at: Set(now),
			applied: Set(true),
			..Default::default()
		}
		.insert(conn)
		.await?;
	}
	let deleted = reading_head::Entity::delete_many()
		.filter(reading_head::Column::UserId.eq(user_id))
		.filter(
			reading_head::Column::MediaId
				.is_in(heads.iter().map(|head| head.media_id.clone())),
		)
		.exec(conn)
		.await?;
	Ok(deleted.rows_affected)
}
