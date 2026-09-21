use chrono::{Duration, Utc};
use models::{
	domain::reading_state::{resolve, HeadState, Outcome, Projection, SourceProtocol},
	entity::reading_head,
};
use sea_orm::prelude::Decimal;

use crate::object::{
	media::merge_read_progress, resume_reading_cursor::ResumeReadingCursor,
};

#[test]
fn kobo_head_position_wins_over_older_session_position() {
	let now = Utc::now();
	let head = reading_head::Model {
		user_id: "42".to_owned(),
		media_id: "book".to_owned(),
		locator: None,
		progression: 0.8,
		page: Some(8),
		position_ms: None,
		track_index: None,
		completed: false,
		updated_at: now.into(),
		created_at: (now - Duration::minutes(5)).into(),
		changed_at: now.into(),
		source_protocol: SourceProtocol::Kobo,
		source_device_id: Some("kobo-device".to_owned()),
		revision: 2,
		event_id: 7,
	};
	let started_at: sea_orm::prelude::DateTimeWithTimeZone =
		(now - Duration::hours(1)).into();
	let session_updated_at = (now - Duration::minutes(10)).into();
	let session = ResumeReadingCursor {
		readthrough_number: 1,
		session_id: 3,
		page: Some(2),
		locator: None,
		position_ms: None,
		percentage_completed: Some(Decimal::new(2, 1)),
		elapsed_seconds: 91,
		started_at: Some(started_at.clone()),
		updated_at: Some(session_updated_at),
		media_id: "book".to_owned(),
	};
	let progress =
		merge_read_progress(Some(&head), Some(session)).expect("active progress");
	assert_eq!(progress.page, Some(8));
	assert_eq!(
		progress.percentage_completed.map(|value| value.round_dp(2)),
		Some(Decimal::new(8, 1))
	);
	assert_eq!(progress.elapsed_seconds, 91);
	assert_eq!(progress.started_at, Some(started_at));
	assert_eq!(progress.updated_at, Some(head.updated_at.clone()));
}

#[test]
fn stale_lower_update_is_provenance_only_and_preserves_head() {
	let now = Utc::now();
	let head = HeadState {
		updated_at: now,
		progression: 0.8,
		completed: false,
	};
	let resolved = resolve(
		Some(head),
		&Projection {
			progression: Some(0.2),
			..Projection::default()
		},
		now - Duration::seconds(61),
	);

	assert_eq!(resolved.outcome, Outcome::ProvenanceOnly);
	assert_eq!(resolved.progression, head.progression);
	assert_eq!(resolved.completed, head.completed);
}
