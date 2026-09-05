//! The unified reading state: one canonical head per `(user, media)` with
//! append-only provenance, shared by every protocol.
//!
//! The projection and conflict rules live in
//! [`models::domain::reading_state`], the persistence in
//! [`models::services::reading_state`]; both are re-exported here. This module
//! adds the process-wide change notification that protocol adapters (Komga
//! SSE) subscribe to through [`Ctx::reading_state_events`].

pub use models::domain::reading_state::{
	page_locator, page_progression, project, resolve, HeadState, Outcome, Position,
	Projection, ProtocolUpdate, Publication, Resolved, SourceProtocol, TimestampKind,
	STALE_TOLERANCE_SECS,
};
pub use models::services::reading_state::{
	apply, clear, head, heads, heads_since, latest_event, winning_event, Applied,
};

use models::entity::media;

use crate::Ctx;

/// An accepted change to a user's reading head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadingHeadChanged {
	pub user_id: String,
	pub media_id: String,
	pub series_id: Option<String>,
	pub protocol: SourceProtocol,
	/// The head was removed by an explicit reset rather than moved.
	pub cleared: bool,
}

/// Publish an applied update once its transaction has committed. Updates that
/// were recorded as provenance only did not change the head and are not
/// announced.
pub fn announce(ctx: &Ctx, media: &media::Model, applied: &Applied) {
	if !applied.accepted() {
		return;
	}
	ctx.emit_reading_head_changed(ReadingHeadChanged {
		user_id: applied.head.user_id.clone(),
		media_id: media.id.clone(),
		series_id: media.series_id.clone(),
		protocol: applied.head.source_protocol,
		cleared: false,
	});
}

/// Publish an explicit reset of the user's head for `media`.
pub fn announce_cleared(
	ctx: &Ctx,
	user_id: &str,
	media: &media::Model,
	protocol: SourceProtocol,
) {
	ctx.emit_reading_head_changed(ReadingHeadChanged {
		user_id: user_id.to_string(),
		media_id: media.id.clone(),
		series_id: media.series_id.clone(),
		protocol,
		cleared: true,
	});
}

#[cfg(test)]
mod tests {
	use super::*;
	use ::tests::{db::test_database, fake_data};
	use chrono::{DateTime, Duration, Utc};
	use models::{
		entity::{media, reading_head_event, user},
		shared::readium::{ReadiumLocation, ReadiumLocator},
	};
	use sea_orm::{
		prelude::Decimal, ColumnTrait, DbConn, EntityTrait, PaginatorTrait, QueryFilter,
	};
	use serde_json::json;

	async fn setup(pages: i32) -> (DbConn, user::Model, media::Model) {
		let db = test_database().await;
		let user = fake_data::User::new("reader").insert(&db).await;
		let series = fake_data::Series::default().insert(&db).await;
		let media = fake_data::Media {
			series_id: series.id,
			pages: Some(pages),
			..Default::default()
		}
		.insert(&db)
		.await;
		(db, user, media)
	}

	fn update(
		protocol: SourceProtocol,
		position: Position,
		progression: Option<f64>,
		completed: Option<bool>,
		updated_at: Option<DateTime<Utc>>,
	) -> ProtocolUpdate {
		ProtocolUpdate {
			protocol,
			device_id: Some(format!("{protocol}-device")),
			updated_at,
			position,
			progression,
			completed,
			raw_payload: json!({ "protocol": protocol.to_string() }),
		}
	}

	fn epub_locator(progression: f64, total: f64) -> ReadiumLocator {
		ReadiumLocator {
			href: "OEBPS/chapter3.xhtml".to_string(),
			title: Some("Chapter 3".to_string()),
			locations: Some(ReadiumLocation {
				fragments: None,
				progression: Some(Decimal::try_from(progression).unwrap()),
				position: None,
				total_progression: Some(Decimal::try_from(total).unwrap()),
				css_selector: None,
				partial_cfi: None,
			}),
			kobo_span: Some("kobo.12.1".to_string()),
			..Default::default()
		}
	}

	fn t0() -> DateTime<Utc> {
		DateTime::parse_from_rfc3339("2026-09-05T10:00:00Z")
			.unwrap()
			.to_utc()
	}

	#[tokio::test]
	async fn komga_page_patch_projects_page_progression_and_synthesized_locator() {
		let (db, user, media) = setup(10).await;
		let applied = apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(SourceProtocol::Komga, Position::Page(3), None, None, None),
		)
		.await
		.unwrap();

		assert_eq!(applied.outcome, Outcome::Accepted);
		let head = applied.head;
		assert_eq!(head.page, Some(3));
		assert!((head.progression - 0.3).abs() < f64::EPSILON);
		assert!(!head.completed);
		assert_eq!(head.revision, 1);
		assert_eq!(head.source_protocol, SourceProtocol::Komga);
		assert_eq!(head.source_device_id.as_deref(), Some("komga-device"));
		let locator = head.locator.expect("page heads synthesize a locator");
		assert_eq!(locator.href, format!("/api/v2/media/{}/page/3", media.id));
		let locations = locator.locations.expect("locations");
		assert_eq!(locations.position, Some(3));
		assert_eq!(
			locations.total_progression,
			Some(Decimal::try_from(0.3).unwrap())
		);
		assert_eq!(applied.event.timestamp_kind, TimestampKind::Server);
		assert!(applied.event.applied);
		assert_eq!(head.event_id, applied.event.id);
	}

	#[tokio::test]
	async fn opds_page_delivery_marks_last_page_complete() {
		let (db, user, media) = setup(4).await;
		let applied = apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Opds,
				Position::Page(4),
				None,
				Some(true),
				None,
			),
		)
		.await
		.unwrap();
		assert_eq!(applied.head.page, Some(4));
		assert!((applied.head.progression - 1.0).abs() < f64::EPSILON);
		assert!(applied.head.completed);
	}

	#[tokio::test]
	async fn kobo_reading_state_stores_locator_verbatim_with_device_time() {
		let (db, user, media) = setup(0).await;
		let locator = epub_locator(0.42, 0.73);
		let applied = apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Kobo,
				Position::Locator(locator.clone()),
				Some(0.73),
				None,
				Some(t0()),
			),
		)
		.await
		.unwrap();

		let head = applied.head;
		assert_eq!(head.locator, Some(locator));
		assert!((head.progression - 0.73).abs() < f64::EPSILON);
		assert_eq!(head.page, None);
		assert_eq!(head.updated_at.to_utc(), t0());
		assert_eq!(applied.event.timestamp_kind, TimestampKind::Device);
		assert_eq!(applied.event.source_updated_at.to_utc(), t0());
	}

	#[tokio::test]
	async fn komga_r2_progression_derives_progression_from_locator_total() {
		let (db, user, media) = setup(0).await;
		let applied = apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Komga,
				Position::Locator(epub_locator(0.1, 0.55)),
				None,
				None,
				Some(t0()),
			),
		)
		.await
		.unwrap();
		assert!((applied.head.progression - 0.55).abs() < f64::EPSILON);
	}

	#[tokio::test]
	async fn opds_v2_position_without_total_progression_uses_page_count() {
		let (db, user, media) = setup(20).await;
		let locator = ReadiumLocator {
			href: "/opds/v2.0/books/x/pages/5".to_string(),
			locations: Some(ReadiumLocation {
				fragments: None,
				progression: None,
				position: Some(5),
				total_progression: None,
				css_selector: None,
				partial_cfi: None,
			}),
			..Default::default()
		};
		let applied = apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Opds,
				Position::Locator(locator),
				None,
				None,
				Some(t0()),
			),
		)
		.await
		.unwrap();
		assert_eq!(applied.head.page, Some(5));
		assert!((applied.head.progression - 0.25).abs() < f64::EPSILON);
	}

	#[tokio::test]
	async fn koreader_xpointer_keeps_locator_and_page_but_moves_progression() {
		let (db, user, media) = setup(0).await;
		let locator = epub_locator(0.2, 0.3);
		apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Komga,
				Position::Locator(locator.clone()),
				None,
				None,
				Some(t0()),
			),
		)
		.await
		.unwrap();

		let applied = apply(
			&db,
			&user.id,
			Publication::from(&media),
			ProtocolUpdate {
				raw_payload: json!({
					"document": "abc",
					"progress": "/body/DocFragment[7]/body/p[3]/text().0",
					"percentage": 0.5
				}),
				..update(
					SourceProtocol::Koreader,
					Position::None,
					Some(0.5),
					None,
					None,
				)
			},
		)
		.await
		.unwrap();

		assert_eq!(applied.outcome, Outcome::Accepted);
		assert_eq!(applied.head.locator, Some(locator));
		assert!((applied.head.progression - 0.5).abs() < f64::EPSILON);
		assert_eq!(applied.head.source_protocol, SourceProtocol::Koreader);
		assert_eq!(applied.head.revision, 2);
		assert_eq!(applied.event.locator, None);
		assert_eq!(
			applied.event.raw_payload["progress"],
			json!("/body/DocFragment[7]/body/p[3]/text().0")
		);
	}

	#[tokio::test]
	async fn koreader_numeric_progress_is_a_page() {
		let (db, user, media) = setup(200).await;
		let applied = apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Koreader,
				Position::Page(50),
				Some(0.26),
				None,
				None,
			),
		)
		.await
		.unwrap();
		assert_eq!(applied.head.page, Some(50));
		// The device's own percentage wins over page / pages.
		assert!((applied.head.progression - 0.26).abs() < f64::EPSILON);
	}

	#[tokio::test]
	async fn liseur_position_op_projects_opaque_locator_when_it_parses() {
		let (db, user, media) = setup(0).await;
		let raw = json!({"href": "chapter.xhtml", "locations": {"progression": 0.25}});
		let locator: ReadiumLocator = serde_json::from_value(raw.clone()).unwrap();
		let applied = apply(
			&db,
			&user.id,
			Publication::from(&media),
			ProtocolUpdate {
				raw_payload: json!({ "op_id": "op-1", "locator": raw }),
				..update(
					SourceProtocol::Liseur,
					Position::Locator(locator.clone()),
					Some(0.25),
					None,
					Some(t0()),
				)
			},
		)
		.await
		.unwrap();
		assert_eq!(applied.head.locator, Some(locator));
		assert!((applied.head.progression - 0.25).abs() < f64::EPSILON);
	}

	#[tokio::test]
	async fn newest_update_wins_even_when_it_moves_backwards() {
		let (db, user, media) = setup(10).await;
		apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Komga,
				Position::Page(5),
				None,
				None,
				Some(t0()),
			),
		)
		.await
		.unwrap();
		let applied = apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Kobo,
				Position::Page(3),
				None,
				None,
				Some(t0() + Duration::seconds(10)),
			),
		)
		.await
		.unwrap();
		assert_eq!(applied.outcome, Outcome::Accepted);
		assert_eq!(applied.head.page, Some(3));
		assert_eq!(applied.head.source_protocol, SourceProtocol::Kobo);
	}

	#[tokio::test]
	async fn stale_lower_update_is_provenance_only() {
		let (db, user, media) = setup(10).await;
		let first = apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Komga,
				Position::Page(5),
				None,
				None,
				Some(t0()),
			),
		)
		.await
		.unwrap();
		let applied = apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Kobo,
				Position::Page(3),
				None,
				None,
				Some(t0() - Duration::seconds(STALE_TOLERANCE_SECS + 1)),
			),
		)
		.await
		.unwrap();

		assert_eq!(applied.outcome, Outcome::ProvenanceOnly);
		assert_eq!(applied.head, first.head);
		assert!(!applied.event.applied);
		assert_eq!(applied.event.page, Some(3));
		let stored = head(&db, &user.id, &media.id).await.unwrap().unwrap();
		assert_eq!(stored.page, Some(5));
		assert_eq!(stored.revision, 1);
		let events = reading_head_event::Entity::find()
			.filter(reading_head_event::Column::MediaId.eq(media.id.clone()))
			.count(&db)
			.await
			.unwrap();
		assert_eq!(events, 2);
	}

	#[tokio::test]
	async fn stale_but_further_update_wins() {
		let (db, user, media) = setup(10).await;
		apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Komga,
				Position::Page(5),
				None,
				None,
				Some(t0()),
			),
		)
		.await
		.unwrap();
		let applied = apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Koreader,
				Position::Page(8),
				None,
				None,
				Some(t0() - Duration::minutes(30)),
			),
		)
		.await
		.unwrap();
		assert_eq!(applied.outcome, Outcome::Accepted);
		assert_eq!(applied.head.page, Some(8));
	}

	#[tokio::test]
	async fn slightly_older_lower_update_is_within_tolerance() {
		let (db, user, media) = setup(10).await;
		apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Komga,
				Position::Page(5),
				None,
				None,
				Some(t0()),
			),
		)
		.await
		.unwrap();
		let applied = apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Opds,
				Position::Page(3),
				None,
				None,
				Some(t0() - Duration::seconds(STALE_TOLERANCE_SECS)),
			),
		)
		.await
		.unwrap();
		assert_eq!(applied.outcome, Outcome::Accepted);
		assert_eq!(applied.head.page, Some(3));
	}

	#[tokio::test]
	async fn completion_is_sticky_until_explicit_unread() {
		let (db, user, media) = setup(10).await;
		let finished = apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Komga,
				Position::None,
				None,
				Some(true),
				Some(t0()),
			),
		)
		.await
		.unwrap();
		assert!(finished.head.completed);
		assert_eq!(finished.head.page, Some(10));
		assert!((finished.head.progression - 1.0).abs() < f64::EPSILON);

		let reread = apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Kobo,
				Position::Page(2),
				None,
				None,
				Some(t0() + Duration::minutes(5)),
			),
		)
		.await
		.unwrap();
		assert!(reread.head.completed, "completion survives a re-read");
		assert_eq!(reread.head.page, Some(2));

		let unread = apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Komga,
				Position::None,
				None,
				Some(false),
				Some(t0() + Duration::minutes(6)),
			),
		)
		.await
		.unwrap();
		assert!(!unread.head.completed);
		assert_eq!(unread.head.page, Some(2), "an un-read keeps the position");
	}

	#[tokio::test]
	async fn status_only_update_keeps_position_and_progression() {
		let (db, user, media) = setup(10).await;
		apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Komga,
				Position::Page(4),
				None,
				None,
				Some(t0()),
			),
		)
		.await
		.unwrap();
		let applied = apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Kobo,
				Position::None,
				None,
				None,
				Some(t0() + Duration::seconds(1)),
			),
		)
		.await
		.unwrap();
		assert_eq!(applied.head.page, Some(4));
		assert!((applied.head.progression - 0.4).abs() < f64::EPSILON);
		assert_eq!(applied.head.source_protocol, SourceProtocol::Kobo);
	}

	#[tokio::test]
	async fn clear_removes_head_and_records_provenance() {
		let (db, user, media) = setup(10).await;
		apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(SourceProtocol::Komga, Position::Page(4), None, None, None),
		)
		.await
		.unwrap();
		let removed = clear(
			&db,
			&user.id,
			&[media.id.clone()],
			SourceProtocol::Komga,
			None,
		)
		.await
		.unwrap();
		assert_eq!(removed, 1);
		assert!(head(&db, &user.id, &media.id).await.unwrap().is_none());
		let last = latest_event(&db, &user.id, &media.id, SourceProtocol::Komga)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(last.raw_payload, json!({ "cleared": true }));
		assert_eq!(last.completed, Some(false));

		let again = clear(
			&db,
			&user.id,
			&[media.id.clone()],
			SourceProtocol::Komga,
			None,
		)
		.await
		.unwrap();
		assert_eq!(again, 0);
	}

	#[tokio::test]
	async fn heads_since_uses_server_change_time_not_device_time() {
		let (db, user, media) = setup(10).await;
		let before = Utc::now() - Duration::seconds(1);
		// Device time far in the past; the change still happened now.
		let applied = apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Kobo,
				Position::Page(4),
				None,
				None,
				Some(t0() - Duration::days(30)),
			),
		)
		.await
		.unwrap();
		let changed = heads_since(&db, &user.id, before).await.unwrap();
		assert_eq!(changed.len(), 1);
		assert_eq!(changed[0].media_id, media.id);
		let later = heads_since(&db, &user.id, applied.head.changed_at.to_utc())
			.await
			.unwrap();
		assert!(later.is_empty());
	}

	#[tokio::test]
	async fn latest_event_returns_protocol_provenance_even_when_it_lost() {
		let (db, user, media) = setup(10).await;
		apply(
			&db,
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Komga,
				Position::Page(9),
				None,
				None,
				Some(t0()),
			),
		)
		.await
		.unwrap();
		let stale = apply(
			&db,
			&user.id,
			Publication::from(&media),
			ProtocolUpdate {
				raw_payload: json!({ "Statistics": { "SpentReadingMinutes": 12 } }),
				..update(
					SourceProtocol::Kobo,
					Position::Page(2),
					None,
					None,
					Some(t0() - Duration::hours(1)),
				)
			},
		)
		.await
		.unwrap();
		assert_eq!(stale.outcome, Outcome::ProvenanceOnly);
		let kobo = latest_event(&db, &user.id, &media.id, SourceProtocol::Kobo)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(
			kobo.raw_payload["Statistics"]["SpentReadingMinutes"],
			json!(12)
		);
		assert!(!kobo.applied);
		let winner = winning_event(&db, &stale.head).await.unwrap().unwrap();
		assert_eq!(winner.protocol, SourceProtocol::Komga);
	}

	#[tokio::test]
	async fn bulk_heads_are_keyed_by_media() {
		let (db, user, media) = setup(10).await;
		let other = fake_data::Media {
			series_id: media.series_id.clone().unwrap(),
			pages: Some(10),
			..Default::default()
		}
		.insert(&db)
		.await;
		for (target, page) in [(&media, 1), (&other, 7)] {
			apply(
				&db,
				&user.id,
				Publication::from(target),
				update(
					SourceProtocol::Stump,
					Position::Page(page),
					None,
					None,
					None,
				),
			)
			.await
			.unwrap();
		}
		let map = heads(&db, &user.id, &[media.id.clone(), other.id.clone()])
			.await
			.unwrap();
		assert_eq!(map[&media.id].page, Some(1));
		assert_eq!(map[&other.id].page, Some(7));
		assert!(heads(&db, &user.id, &[]).await.unwrap().is_empty());
	}

	#[tokio::test]
	async fn announce_publishes_only_accepted_updates() {
		let (db, user, media) = setup(10).await;
		let ctx = Ctx::for_testing(db);
		let mut events = ctx.reading_state_events();
		let first = apply(
			ctx.conn.as_ref(),
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Komga,
				Position::Page(5),
				None,
				None,
				Some(t0()),
			),
		)
		.await
		.unwrap();
		announce(&ctx, &media, &first);
		let stale = apply(
			ctx.conn.as_ref(),
			&user.id,
			Publication::from(&media),
			update(
				SourceProtocol::Kobo,
				Position::Page(1),
				None,
				None,
				Some(t0() - Duration::hours(1)),
			),
		)
		.await
		.unwrap();
		announce(&ctx, &media, &stale);
		announce_cleared(&ctx, &user.id, &media, SourceProtocol::Stump);

		let changed = events.try_recv().unwrap();
		assert_eq!(changed.protocol, SourceProtocol::Komga);
		assert!(!changed.cleared);
		assert_eq!(changed.series_id, media.series_id);
		let cleared = events.try_recv().unwrap();
		assert!(cleared.cleared);
		assert!(events.try_recv().is_err());
	}
}
