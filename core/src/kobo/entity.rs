use models::{
	entity::{media, media_metadata, reading_session, user::AuthUser},
	prefixer::{parse_query_to_model, parse_query_to_model_optional},
	shared::{
		enums::ReadingStatus,
		readium::{ReadiumLocation, ReadiumLocator},
	},
};
use rust_decimal::prelude::ToPrimitive;
use sea_orm::{
	prelude::*,
	sea_query::{Condition, Expr, Query, SimpleExpr, SubQueryStatement},
	ActiveModelTrait, ActiveValue, ConnectionTrait, EntityTrait, FromQueryResult,
	IntoActiveModel, JoinType, QueryFilter, QuerySelect, Select, Set,
};

use crate::kobo::sync_types::*;
use chrono::Utc;

#[derive(Debug, Clone, FromQueryResult)]
pub struct ReadingSession {
	pub created_at: DateTimeWithTimeZone,
	pub updated_at: Option<DateTimeWithTimeZone>,
	pub end_percentage: Option<Decimal>,
	pub end_locator: Option<ReadiumLocator>,
	pub status: ReadingStatus,
	pub kobo_state: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct MediaWithMetadataAndReadingSessions {
	pub media: media::Model,
	pub metadata: Option<media_metadata::Model>,
	pub reading_session: Option<ReadingSession>,
	pub finished_reading_session_count: u32,
	pub finished_reading_session_last_completed_at: Option<DateTimeWithTimeZone>,
}

fn apply_reading_session_joins(
	query: Select<media::Entity>,
	user: &AuthUser,
) -> Select<media::Entity> {
	// it would be nice to use `.select_also` here instead of manually selecting columns, but
	// that doesn't work with `.into_model`.
	//
	// we're using a custom `ReadingSession` struct to insulate us from changes to
	// `reading_session`: if the entity requires columns that aren't selected here, then
	// `parse_query_to_model_optional` will silently return None.
	let user_id = user.id.clone();

	// IN (select max(created_at) where user_id=user.id AND media_id=media.id
	let latest_subq = Query::select()
		.expr(
			Expr::col((reading_session::Entity, reading_session::Column::CreatedAt))
				.max(),
		)
		.from(reading_session::Entity)
		.and_where(reading_session::Column::UserId.eq(user_id.clone()))
		// where media_id = media.id
		.and_where(
			Expr::col((reading_session::Entity, reading_session::Column::MediaId))
				.equals((media::Entity, media::Column::Id)),
		)
		.to_owned();

	let completed_count_subq = Query::select()
		.expr(Expr::col((reading_session::Entity, reading_session::Column::Id)).count())
		.from(reading_session::Entity)
		.and_where(reading_session::Column::UserId.eq(user_id.clone()))
		.and_where(
			Expr::col((reading_session::Entity, reading_session::Column::MediaId))
				.equals((media::Entity, media::Column::Id)),
		)
		.and_where(reading_session::Column::Status.eq(ReadingStatus::Finished))
		.to_owned();

	let last_completed_subq = Query::select()
		.expr(
			Expr::col((reading_session::Entity, reading_session::Column::UpdatedAt))
				.max(),
		)
		.from(reading_session::Entity)
		.and_where(reading_session::Column::UserId.eq(user_id.clone()))
		.and_where(
			Expr::col((reading_session::Entity, reading_session::Column::MediaId))
				.equals((media::Entity, media::Column::Id)),
		)
		.and_where(reading_session::Column::Status.eq(ReadingStatus::Finished))
		.to_owned();

	query
		.column_as(
			Expr::col((reading_session::Entity, reading_session::Column::Id)),
			"reading_sessionsid",
		)
		.column_as(
			Expr::col((reading_session::Entity, reading_session::Column::CreatedAt)),
			"reading_sessionscreated_at",
		)
		.column_as(
			Expr::col((reading_session::Entity, reading_session::Column::UpdatedAt)),
			"reading_sessionsupdated_at",
		)
		.column_as(
			Expr::col((
				reading_session::Entity,
				reading_session::Column::EndPercentage,
			)),
			"reading_sessionsend_percentage",
		)
		.column_as(
			Expr::col((reading_session::Entity, reading_session::Column::EndLocator)),
			"reading_sessionsend_locator",
		)
		.column_as(
			Expr::col((reading_session::Entity, reading_session::Column::KoboState)),
			"reading_sessionskobo_state",
		)
		.column_as(
			Expr::col((reading_session::Entity, reading_session::Column::Status)),
			"reading_sessionsstatus",
		)
		// LEFT JOIN reading_sessions on media.id = reading_sessions.media_id
		//  AND reading_sessions.user_id = $user_id AND reading_sessions.created_at IN (latest_subq)
		.join_rev(
			JoinType::LeftJoin,
			reading_session::Entity::belongs_to(media::Entity)
				.from(reading_session::Column::MediaId)
				.to(media::Column::Id)
				.on_condition({
					let user_id = user_id.clone();
					let latest_subq = latest_subq.clone();
					move |_left, _right| {
						Condition::all()
							.add(reading_session::Column::UserId.eq(user_id.clone()))
							.add(
								Expr::col((
									reading_session::Entity,
									reading_session::Column::CreatedAt,
								))
								.in_subquery(latest_subq.clone()),
							)
					}
				})
				.into(),
		)
		.column_as(
			SimpleExpr::SubQuery(
				None,
				Box::new(SubQueryStatement::SelectStatement(completed_count_subq)),
			),
			"finished_reading_session_count",
		)
		.column_as(
			SimpleExpr::SubQuery(
				None,
				Box::new(SubQueryStatement::SelectStatement(last_completed_subq)),
			),
			"finished_reading_session_last_completed_at",
		)
		.group_by(media::Column::Id)
}

impl MediaWithMetadataAndReadingSessions {
	pub fn find_by_id_for_user(id: String, user: &AuthUser) -> Select<media::Entity> {
		let select = media::ModelWithMetadata::find_by_id_for_user(id, user);
		apply_reading_session_joins(select, user)
	}

	pub fn find_by_ids_for_user(
		ids: &[String],
		user: &AuthUser,
	) -> Select<media::Entity> {
		let select = media::ModelWithMetadata::find_for_user(user)
			.filter(media::Column::Id.is_in(ids));
		apply_reading_session_joins(select, user)
	}
}

impl FromQueryResult for MediaWithMetadataAndReadingSessions {
	fn from_query_result(
		res: &sea_orm::QueryResult,
		_pre: &str,
	) -> Result<Self, sea_orm::DbErr> {
		let media = parse_query_to_model::<media::Model, media::Entity>(res)?;
		let metadata = parse_query_to_model_optional::<
			media_metadata::Model,
			media_metadata::Entity,
		>(res)?;
		let reading_session = parse_query_to_model_optional::<
			ReadingSession,
			reading_session::Entity,
		>(res)?;
		Ok(Self {
			media,
			metadata,
			reading_session,
			finished_reading_session_count: res
				.try_get("", "finished_reading_session_count")?,
			finished_reading_session_last_completed_at: res
				.try_get("", "finished_reading_session_last_completed_at")?,
		})
	}
}

// a UUID that we can use when we don't have an ID that is more appropriate.
const DUMMY_UUID: &str = "00000000-0000-0000-0000-000000000001";

impl BookMetadata {
	pub fn from_media(m: &MediaWithMetadataAndReadingSessions, book_url: String) -> Self {
		Self::from_media_with_format(m, book_url, Format::EPUB3)
	}

	pub fn from_media_with_format(
		m: &MediaWithMetadataAndReadingSessions,
		book_url: String,
		format: Format,
	) -> Self {
		let media_id = &m.media.id;

		let writers = m.metadata.as_ref().and_then(|mm| mm.writers.clone());
		let publication_date =
			m.metadata
				.as_ref()
				.and_then(|mm| match (mm.year, mm.month, mm.day) {
					(Some(year), month, day) => Date::from_ymd_opt(
						year,
						month.and_then(|v| u32::try_from(v).ok()).unwrap_or(1),
						day.and_then(|v| u32::try_from(v).ok()).unwrap_or(1),
					),
					_ => None,
				});

		let series = m.metadata.as_ref().and_then(|mm| {
			match (m.media.series_id.clone(), mm.series.clone(), mm.number) {
				(Some(series_id), Some(series), series_number) => Some(Series {
					id: series_id,
					name: series,
					number: series_number
						.map(|n| n.to_string())
						.unwrap_or("1".to_string()),
					number_float: series_number.and_then(|n| n.to_f32()).unwrap_or(1.0),
				}),
				_ => None,
			}
		});

		BookMetadata {
			categories: vec![DUMMY_UUID.to_string()],
			contributor_roles: writers
				.clone()
				.into_iter()
				.map(|w| ContributorRole { name: w })
				.collect(),
			contributors: writers.clone().into_iter().collect(),
			cover_image_id: media_id.clone(),
			cross_revision_id: media_id.clone(),
			current_display_price: DisplayPrice {
				currency_code: "USD".to_string(),
				total_amount: 0,
			},
			current_love_display_price: LoveDisplayPrice { total_amount: 0 },
			description: m.metadata.as_ref().and_then(|mm| mm.summary.clone()),
			download_urls: vec![DownloadUrl {
				drm_type: "None".to_string(),
				// this seems to be unrelated to the EPUB 3 spec.
				// the Kobo ignores books with format: "EPUB".
				format,
				size: u64::try_from(m.media.size).unwrap_or(0),
				platform: "Generic".to_string(),
				url: book_url,
			}],
			entitlement_id: media_id.clone(),
			external_ids: vec![],
			genre: DUMMY_UUID.to_string(),
			is_eligible_for_kobo_love: false,
			is_internet_archive: false,
			is_pre_order: false,
			is_social_enabled: true,
			isbn: m
				.metadata
				.as_ref()
				.and_then(|mm| mm.identifier_isbn.clone()),
			language: "en".to_string(),
			phonetic_pronunciations: Empty {},
			publication_date: publication_date
				.and_then(|pd| pd.and_hms_opt(0, 0, 0))
				.map(|pd| pd.and_utc()),
			publisher: m.metadata.as_ref().and_then(|mm| mm.publisher.clone()).map(
				|mp| Publisher {
					imprint: "".to_string(),
					name: mp,
				},
			),
			revision_id: media_id.clone(),
			series,
			title: m
				.metadata
				.as_ref()
				.and_then(|mm| mm.title.clone())
				.unwrap_or(m.media.name.clone()),
			work_id: media_id.clone(),
		}
	}
}

/// The normalized projection of a Kobo `CurrentBookmark` and `StatusInfo`.
#[derive(Debug, Clone, PartialEq)]
pub struct KoboReadingStateProjection {
	pub locator: Option<ReadiumLocator>,
	pub progression: Option<Decimal>,
	pub total_progression: Option<Decimal>,
	pub status: Option<ReadingStatus>,
}

fn percent_to_progression(value: Option<f32>) -> Option<Decimal> {
	value
		.and_then(|value| Decimal::try_from(value as f64).ok())
		.map(|value| value / Decimal::new(100, 0))
}

/// Map the fields understood by the Kobo ReadingState API onto Stump's
/// canonical Readium locator and reading-session values.
///
/// Kobo spans are only projected when their location explicitly declares the
/// `KoboSpan` type. Unknown location types remain in the retained raw payload
/// rather than being presented as a Readium anchor.
pub fn map_kobo_reading_state(
	update: &ReadingStateUpdate,
) -> Result<KoboReadingStateProjection, String> {
	let (progression, total_progression, locator) =
		if let Some(bookmark) = update.current_bookmark.as_ref() {
			let progression =
				percent_to_progression(bookmark.content_source_progress_percent);
			let total_progression = percent_to_progression(bookmark.progress_percent);

			let locator = bookmark.location.as_ref().and_then(|location| {
				let source = location.source.as_deref()?.trim();
				if source.is_empty() {
					return None;
				}

				let kobo_span = location
					.type_
					.as_deref()
					.filter(|kind| kind.eq_ignore_ascii_case("KoboSpan"))
					.and_then(|_| location.value.clone());

				Some(ReadiumLocator {
					chapter_title: String::new(),
					href: source.to_string(),
					title: None,
					locations: Some(ReadiumLocation {
						fragments: None,
						progression,
						position: None,
						total_progression,
						css_selector: None,
						partial_cfi: None,
					}),
					text: None,
					kobo_span,
					r#type: "application/xhtml+xml".to_string(),
				})
			});

			(progression, total_progression, locator)
		} else {
			(None, None, None)
		};

	let status = update
		.status_info
		.as_ref()
		.and_then(|status_info| status_info.status.as_deref())
		.map(|status| match status {
			"ReadyToRead" => Ok(ReadingStatus::NotStarted),
			"Reading" => Ok(ReadingStatus::Reading),
			"Finished" => Ok(ReadingStatus::Finished),
			other => Err(format!("unsupported Kobo reading status: {other}")),
		})
		.transpose()?;

	Ok(KoboReadingStateProjection {
		locator,
		progression,
		total_progression,
		status,
	})
}

#[cfg(test)]
mod tests {
	use super::map_kobo_reading_state;
	use crate::kobo::sync_types::ReadingStateUpdateRequest;
	use models::shared::enums::ReadingStatus;
	use rust_decimal::Decimal;

	#[test]
	fn maps_kobo_span_location_and_percentages() {
		let update: super::ReadingStateUpdate =
			serde_json::from_value(serde_json::json!({
				"CurrentBookmark": {
					"ProgressPercent": 73.0,
					"ContentSourceProgressPercent": 42.0,
					"Location": {
						"Value": "kobo.3.7",
						"Type": "KoboSpan",
						"Source": "chapter.xhtml"
					}
				},
				"StatusInfo": { "Status": "Reading" }
			}))
			.expect("valid Kobo state");

		let projection = map_kobo_reading_state(&update).expect("state maps");
		assert_eq!(projection.progression, Some(Decimal::new(42, 2)));
		assert_eq!(projection.total_progression, Some(Decimal::new(73, 2)));
		assert_eq!(projection.status, Some(ReadingStatus::Reading));
		let locator = projection.locator.expect("locator maps");
		assert_eq!(locator.href, "chapter.xhtml");
		assert_eq!(locator.kobo_span.as_deref(), Some("kobo.3.7"));
	}

	#[test]
	fn maps_plain_location_without_kobo_span_anchor() {
		let update: super::ReadingStateUpdate =
			serde_json::from_value(serde_json::json!({
				"CurrentBookmark": {
					"ProgressPercent": 20.0,
					"ContentSourceProgressPercent": 11.0,
					"Location": {
						"Value": "epubcfi(/6/4)",
						"Type": "ContentCFI",
						"Source": "chapter.xhtml"
					}
				},
				"StatusInfo": { "Status": "Finished" }
			}))
			.expect("valid Kobo state");

		let projection = map_kobo_reading_state(&update).expect("state maps");
		assert_eq!(projection.status, Some(ReadingStatus::Finished));
		let locator = projection.locator.expect("locator maps");
		assert_eq!(locator.href, "chapter.xhtml");
		assert_eq!(locator.kobo_span, None);
		assert_eq!(
			locator
				.locations
				.and_then(|locations| locations.progression),
			Some(Decimal::new(11, 2))
		);
	}

	#[test]
	fn rejects_unknown_status_without_silent_projection() {
		let update: super::ReadingStateUpdate =
			serde_json::from_value(serde_json::json!({
				"StatusInfo": { "Status": "Paused" }
			}))
			.expect("valid Kobo state");
		let error = map_kobo_reading_state(&update).expect_err("unknown status rejected");
		assert!(error.contains("Paused"));
	}

	#[test]
	fn update_request_reads_pascal_case_sections() {
		let request: ReadingStateUpdateRequest =
			serde_json::from_value(serde_json::json!({
				"ReadingStates": [{ "StatusInfo": { "Status": "Reading" } }]
			}))
			.expect("valid Kobo request");
		assert_eq!(request.reading_states.len(), 1);
	}
}

fn add_device_id(active: &mut reading_session::ActiveModel, device_id: Option<String>) {
	let Some(device_id) = device_id.filter(|id| !id.is_empty()) else {
		return;
	};

	let current = match &active.device_ids {
		ActiveValue::Set(value) | ActiveValue::Unchanged(value) => value.as_ref(),
		ActiveValue::NotSet => None,
	};
	let mut ids = current
		.map(|reading_session::DeviceIds(ids)| ids.clone())
		.unwrap_or_default();
	if !ids.contains(&device_id) {
		ids.push(device_id);
		active.device_ids = Set(Some(reading_session::DeviceIds(ids)));
	}
}

/// Persist a Kobo state request in the existing reading-session model.
///
/// The complete request body is retained in `kobo_state`; normalized
/// progression and the Readium locator are updated independently so later
/// non-Kobo clients can still consume the session.
pub async fn persist_kobo_reading_state<C: ConnectionTrait>(
	conn: &C,
	user: &AuthUser,
	media_id: &str,
	update: &ReadingStateUpdate,
	raw_payload: serde_json::Value,
	device_id: Option<String>,
) -> Result<reading_session::Model, DbErr> {
	let projection = map_kobo_reading_state(update).map_err(DbErr::Custom)?;
	let latest = reading_session::Entity::find_latest_for_user_and_media(user, media_id)
		.one(conn)
		.await?;

	let incoming_status = projection.status.unwrap_or(ReadingStatus::Reading);
	let starts_new_readthrough = latest.as_ref().is_some_and(|session| {
		session.is_finalized() && incoming_status == ReadingStatus::Reading
	});

	// A finalized session followed by fresh reading activity starts a new
	// readthrough; otherwise the latest session is continued.
	let continued = match latest {
		Some(session) if !starts_new_readthrough => Some(session),
		_ => None,
	};

	match continued {
		None => {
			let readthrough_number =
				models::services::reading_progress::derive_readthrough_number(
					conn, &user.id, media_id,
				)
				.await?;
			let locator = projection.locator.clone();
			let mut active = reading_session::ActiveModel {
				session_date: Set(Utc::now().date_naive()),
				start_locator: Set(locator.clone()),
				end_locator: Set(locator),
				start_percentage: Set(Some(Decimal::new(0, 0))),
				end_percentage: Set(projection.total_progression),
				elapsed_seconds: Set(Some(0)),
				readthrough_number: Set(readthrough_number),
				status: Set(incoming_status),
				kobo_state: Set(Some(raw_payload)),
				media_id: Set(media_id.to_string()),
				user_id: Set(user.id.clone()),
				..Default::default()
			};
			add_device_id(&mut active, device_id);
			active.insert(conn).await
		},
		Some(session) => {
			let mut active = session.into_active_model();
			if let Some(locator) = projection.locator {
				active.end_locator = Set(Some(locator));
			}
			if let Some(total_progression) = projection.total_progression {
				active.end_percentage = Set(Some(total_progression));
			}
			if let Some(status) = projection.status {
				active.status = Set(status);
			}
			active.kobo_state = Set(Some(raw_payload));
			add_device_id(&mut active, device_id);
			active.update(conn).await
		},
	}
}

impl ReadingState {
	fn unread(media_id: String) -> Self {
		let now = Utc::now();

		ReadingState {
			created: now,
			current_bookmark: CurrentBookmark {
				last_modified: now,
				progress_percent: None,
				content_source_progress_percent: None,
				location: None,
			},
			entitlement_id: media_id,
			last_modified: now,
			priority_timestamp: now,
			statistics: Statistics { last_modified: now },
			status_info: StatusInfo {
				last_modified: now,
				status: Status::ReadyToRead,
				times_started_reading: 0,
			},
		}
	}

	fn finished(media_id: String, last_completed_at: DateTimeWithTimeZone) -> Self {
		let utc_completed_at = last_completed_at.to_utc();

		ReadingState {
			created: Utc::now(),
			current_bookmark: CurrentBookmark {
				last_modified: utc_completed_at,
				progress_percent: None,
				content_source_progress_percent: None,
				location: None,
			},
			entitlement_id: media_id,
			last_modified: utc_completed_at,
			priority_timestamp: utc_completed_at,
			statistics: Statistics {
				last_modified: utc_completed_at,
			},
			status_info: StatusInfo {
				last_modified: utc_completed_at,
				status: Status::Finished,
				times_started_reading: 1,
			},
		}
	}

	pub fn from_active_reading_session(media_id: String, rs: &ReadingSession) -> Self {
		let updated_or_started_at = rs.updated_at.unwrap_or(rs.created_at).to_utc();
		let stored_percent = rs
			.end_percentage
			.and_then(|pc| pc.to_f32().map(|pc| pc * 100.0));
		let stored_content_source_percent = rs
			.end_locator
			.as_ref()
			.and_then(|locator| locator.locations.as_ref())
			.and_then(|locations| locations.progression)
			.and_then(|progression| progression.to_f32().map(|value| value * 100.0));

		let raw_update = rs
			.kobo_state
			.as_ref()
			.and_then(|raw| {
				serde_json::from_value::<ReadingStateUpdateRequest>(raw.clone()).ok()
			})
			.and_then(|request| request.reading_states.into_iter().next());

		let percent_complete = raw_update
			.as_ref()
			.and_then(|update| {
				update
					.current_bookmark
					.as_ref()
					.and_then(|bookmark| bookmark.progress_percent)
			})
			.or(stored_percent);
		let content_source_progress_percent = raw_update
			.as_ref()
			.and_then(|update| {
				update
					.current_bookmark
					.as_ref()
					.and_then(|bookmark| bookmark.content_source_progress_percent)
			})
			.or(stored_content_source_percent)
			.or(percent_complete);

		let location = raw_update
			.as_ref()
			.and_then(|update| update.current_bookmark.as_ref())
			.and_then(|bookmark| bookmark.location.as_ref())
			.map(|location| Location {
				value: location.value.clone(),
				type_: location.type_.clone(),
				source: location.source.clone().unwrap_or_default(),
			})
			.or_else(|| {
				rs.end_locator.as_ref().and_then(|locator| {
					locator.kobo_span.as_ref().map(|span| Location {
						value: Some(span.clone()),
						type_: Some("KoboSpan".to_string()),
						source: locator.href.clone(),
					})
				})
			});

		let status = match rs.status {
			ReadingStatus::Finished => Status::Finished,
			ReadingStatus::NotStarted | ReadingStatus::Abandoned => Status::ReadyToRead,
			ReadingStatus::Reading => Status::Reading,
		};

		ReadingState {
			created: rs.created_at.to_utc(),
			current_bookmark: CurrentBookmark {
				last_modified: updated_or_started_at,
				progress_percent: percent_complete,
				content_source_progress_percent,
				location,
			},
			entitlement_id: media_id,
			last_modified: updated_or_started_at,
			priority_timestamp: updated_or_started_at,
			statistics: Statistics {
				last_modified: updated_or_started_at,
			},
			status_info: StatusInfo {
				last_modified: updated_or_started_at,
				status,
				times_started_reading: if matches!(rs.status, ReadingStatus::Reading) {
					1
				} else {
					0
				},
			},
		}
	}
}

impl BookEntitlementContainer {
	pub fn from_media(m: MediaWithMetadataAndReadingSessions, book_url: String) -> Self {
		let media_id = &m.media.id;

		let reading_state = match (
			m.reading_session.as_ref(),
			m.finished_reading_session_last_completed_at,
		) {
			// Kobo's ReadyToRead state maps to the explicit NotStarted
			// session status rather than an active reading session.
			(Some(rs), _) if rs.status == ReadingStatus::NotStarted => {
				ReadingState::unread(media_id.to_string())
			},
			// latest session was abandoned but there is a prior completion
			(Some(rs), Some(last_completed_at))
				if rs.status == ReadingStatus::Abandoned =>
			{
				ReadingState::finished(media_id.to_string(), last_completed_at)
			},
			// TODO(kobo): determine whether this is ideal outcome. if a book was abandoned, it wasn't
			// really `unread` but think for now this is acceptable.
			(Some(rs), None) if rs.status == ReadingStatus::Abandoned => {
				ReadingState::unread(media_id.to_string())
			},
			// latest session is completed
			(Some(rs), _) if rs.status == ReadingStatus::Finished => {
				ReadingState::finished(
					media_id.to_string(),
					m.finished_reading_session_last_completed_at
						.unwrap_or_else(|| chrono::Utc::now().into()),
				)
			},
			// latest session is in-progress
			(Some(active_reading_session), _) => {
				ReadingState::from_active_reading_session(
					media_id.to_string(),
					active_reading_session,
				)
			},
			// no active session but has a past completion
			(_, Some(last_completed_at)) => {
				ReadingState::finished(media_id.to_string(), last_completed_at)
			},
			_ => ReadingState::unread(media_id.to_string()),
		};

		BookEntitlementContainer {
			book_entitlement: BookEntitlement {
				accessibility: "Full".to_string(),
				active_period: Period { from: Utc::now() },
				created: m.media.created_at.to_utc(),
				cross_revision_id: media_id.clone(),
				id: media_id.clone(),
				is_hidden_from_archive: false,
				is_locked: false,
				is_removed: false,
				last_modified: m
					.media
					.modified_at
					.map(|t| t.to_utc())
					.unwrap_or(Utc::now()),
				origin_category: "Imported".to_string(),
				revision_id: media_id.clone(),
				status: "Active".to_string(),
			},
			book_metadata: BookMetadata::from_media(&m, book_url),
			reading_state: Some(reading_state),
		}
	}
}

#[cfg(test)]
mod persistence_tests {
	use models::entity::user;
	use sea_orm::DbConn;
	use tests::db::test_database;
	use tests::fake_data;

	use crate::kobo::entity::MediaWithMetadataAndReadingSessions;
	use crate::kobo::sync_types::{BookEntitlementContainer, Status};

	async fn load_media(
		db: &DbConn,
		user: &user::AuthUser,
		id: String,
	) -> MediaWithMetadataAndReadingSessions {
		MediaWithMetadataAndReadingSessions::find_by_id_for_user(id, user)
			.into_model::<MediaWithMetadataAndReadingSessions>()
			.one(db)
			.await
			.expect("book not found")
			.unwrap()
	}

	#[tokio::test]
	async fn test_reading_state_unread() {
		let db = test_database().await;

		let user = fake_data::User::default().insert(&db).await;
		let user = user::AuthUser {
			id: user.id,
			permissions: vec![],
			..Default::default()
		};

		let series = fake_data::Series::default().insert(&db).await;
		let media = fake_data::Media {
			series_id: series.id.clone(),
			id: Some("don-quixote".to_string()),
			name: Some("Don Quixote".to_string()),
			created_at: Some("1605-01-16T00:00:00Z".parse().unwrap()),
			..Default::default()
		}
		.insert(&db)
		.await;

		// this book has no reading sessions.

		let m = load_media(&db, &user, media.id).await;

		let entitlement =
			BookEntitlementContainer::from_media(m, "https://example.org/".to_string());

		// this is an unread book.
		let reading_state = entitlement.reading_state.unwrap();
		assert_eq!(Status::ReadyToRead, reading_state.status_info.status);

		let bookmark = reading_state.current_bookmark;
		assert_eq!(None, bookmark.progress_percent);
		assert_eq!(None, bookmark.content_source_progress_percent);
		assert_eq!(None, bookmark.location);
	}

	#[tokio::test]
	async fn test_reading_state_currently_reading() {
		let db = test_database().await;

		let user = fake_data::User::default().insert(&db).await;
		let user = user::AuthUser {
			id: user.id,
			permissions: vec![],
			..Default::default()
		};

		let series = fake_data::Series::default().insert(&db).await;
		let media = fake_data::Media {
			series_id: series.id.clone(),
			id: Some("don-quixote".to_string()),
			name: Some("Don Quixote".to_string()),
			created_at: Some("1605-01-16T00:00:00Z".parse().unwrap()),
			..Default::default()
		}
		.insert(&db)
		.await;

		// this book has a single active reading session

		fake_data::ReadingSession {
			media_id: media.id.clone(),
			user_id: user.id.clone(),
			end_percentage: 0.5,
			..Default::default()
		}
		.insert(&db)
		.await;

		let m = load_media(&db, &user, media.id).await;

		let entitlement =
			BookEntitlementContainer::from_media(m, "https://example.org/".to_string());

		// we're partway through this book.
		let reading_state = entitlement.reading_state.unwrap();
		assert_eq!(Status::Reading, reading_state.status_info.status);

		let bookmark = reading_state.current_bookmark;
		assert_eq!(Some(50.0), bookmark.progress_percent);
		assert_eq!(Some(50.0), bookmark.content_source_progress_percent);
		assert_eq!(None, bookmark.location);
	}

	#[tokio::test]
	async fn test_reading_state_abandoned_no_prior_completion() {
		let db = test_database().await;

		let user = fake_data::User::default().insert(&db).await;
		let user = user::AuthUser {
			id: user.id,
			permissions: vec![],
			..Default::default()
		};

		let series = fake_data::Series::default().insert(&db).await;
		let media = fake_data::Media {
			series_id: series.id.clone(),
			id: Some("don-quixote".to_string()),
			name: Some("Don Quixote".to_string()),
			created_at: Some("1605-01-16T00:00:00Z".parse().unwrap()),
			..Default::default()
		}
		.insert(&db)
		.await;

		// abandoned without ever having finished it
		fake_data::ReadingSession {
			media_id: media.id.clone(),
			user_id: user.id.clone(),
			end_percentage: 0.4,
			status: models::shared::enums::ReadingStatus::Abandoned,
			..Default::default()
		}
		.insert(&db)
		.await;

		let m = load_media(&db, &user, media.id).await;

		let entitlement =
			BookEntitlementContainer::from_media(m, "https://example.org/".to_string());

		// TODO(kobo): see above re: whether abandoned + no prior complete = unread is ideal
		let reading_state = entitlement.reading_state.unwrap();
		assert_eq!(Status::ReadyToRead, reading_state.status_info.status);
	}

	#[tokio::test]
	async fn test_reading_state_abandoned_after_prior_completion() {
		let db = test_database().await;

		let user = fake_data::User::default().insert(&db).await;
		let user = user::AuthUser {
			id: user.id,
			permissions: vec![],
			..Default::default()
		};

		let series = fake_data::Series::default().insert(&db).await;
		let media = fake_data::Media {
			series_id: series.id.clone(),
			id: Some("don-quixote".to_string()),
			name: Some("Don Quixote".to_string()),
			created_at: Some("1605-01-16T00:00:00Z".parse().unwrap()),
			..Default::default()
		}
		.insert(&db)
		.await;

		// first readthrough was completed, then the re-read was abandoned
		fake_data::ReadingSession {
			media_id: media.id.clone(),
			user_id: user.id.clone(),
			end_percentage: 1.0,
			status: models::shared::enums::ReadingStatus::Finished,
			created_at: Some("2026-05-26T00:00:00Z".parse().unwrap()),
		}
		.insert(&db)
		.await;

		fake_data::ReadingSession {
			media_id: media.id.clone(),
			user_id: user.id.clone(),
			end_percentage: 0.3,
			status: models::shared::enums::ReadingStatus::Abandoned,
			created_at: Some("2026-05-27T00:00:00Z".parse().unwrap()),
		}
		.insert(&db)
		.await;

		let m = load_media(&db, &user, media.id).await;

		let entitlement =
			BookEntitlementContainer::from_media(m, "https://example.org/".to_string());

		// non-dnf should always take precendence over dnf if newer
		let reading_state = entitlement.reading_state.unwrap();
		assert_eq!(Status::Finished, reading_state.status_info.status);
	}

	#[tokio::test]
	async fn test_reading_state_rereading() {
		let db = test_database().await;

		let user = fake_data::User::default().insert(&db).await;
		let user = user::AuthUser {
			id: user.id,
			permissions: vec![],
			..Default::default()
		};

		let series = fake_data::Series::default().insert(&db).await;
		let media = fake_data::Media {
			series_id: series.id.clone(),
			id: Some("don-quixote".to_string()),
			name: Some("Don Quixote".to_string()),
			created_at: Some("1605-01-16T00:00:00Z".parse().unwrap()),
			..Default::default()
		}
		.insert(&db)
		.await;

		// first readthrough is complete
		fake_data::ReadingSession {
			media_id: media.id.clone(),
			user_id: user.id.clone(),
			end_percentage: 1.0,
			status: models::shared::enums::ReadingStatus::Finished,
			created_at: Some("2026-05-26T00:00:00Z".parse().unwrap()),
		}
		.insert(&db)
		.await;

		// second readthrough is in-progress
		fake_data::ReadingSession {
			media_id: media.id.clone(),
			user_id: user.id.clone(),
			end_percentage: 0.35,
			status: models::shared::enums::ReadingStatus::Reading,
			created_at: Some("2026-05-27T00:00:00Z".parse().unwrap()),
		}
		.insert(&db)
		.await;

		let m = load_media(&db, &user, media.id).await;

		let entitlement =
			BookEntitlementContainer::from_media(m, "https://example.org/".to_string());

		// the re-read in-progress should take precedence
		let reading_state = entitlement.reading_state.unwrap();
		assert_eq!(Status::Reading, reading_state.status_info.status);

		let bookmark = reading_state.current_bookmark;
		assert_eq!(Some(35.0), bookmark.progress_percent);
		assert_eq!(Some(35.0), bookmark.content_source_progress_percent);
		assert_eq!(None, bookmark.location);
	}

	#[tokio::test]
	async fn test_reading_state_finished_multiple_readthroughs() {
		let db = test_database().await;

		let user = fake_data::User::default().insert(&db).await;
		let user = user::AuthUser {
			id: user.id,
			permissions: vec![],
			..Default::default()
		};

		let series = fake_data::Series::default().insert(&db).await;
		let media = fake_data::Media {
			series_id: series.id.clone(),
			id: Some("don-quixote".to_string()),
			name: Some("Don Quixote".to_string()),
			created_at: Some("1605-01-16T00:00:00Z".parse().unwrap()),
			..Default::default()
		}
		.insert(&db)
		.await;

		fake_data::ReadingSession {
			media_id: media.id.clone(),
			user_id: user.id.clone(),
			end_percentage: 1.0,
			status: models::shared::enums::ReadingStatus::Finished,
			created_at: Some("2026-05-26T00:00:00Z".parse().unwrap()),
		}
		.insert(&db)
		.await;

		fake_data::ReadingSession {
			media_id: media.id.clone(),
			user_id: user.id.clone(),
			end_percentage: 1.0,
			status: models::shared::enums::ReadingStatus::Finished,
			created_at: Some("2026-05-27T00:00:00Z".parse().unwrap()),
		}
		.insert(&db)
		.await;

		let m = load_media(&db, &user, media.id).await;

		assert_eq!(2, m.finished_reading_session_count);

		let entitlement =
			BookEntitlementContainer::from_media(m, "https://example.org/".to_string());

		let reading_state = entitlement.reading_state.unwrap();
		assert_eq!(Status::Finished, reading_state.status_info.status);
	}

	#[tokio::test]
	async fn test_reading_state_finished() {
		let db = test_database().await;

		let user = fake_data::User::default().insert(&db).await;
		let user = user::AuthUser {
			id: user.id,
			permissions: vec![],
			..Default::default()
		};

		let series = fake_data::Series::default().insert(&db).await;
		let media = fake_data::Media {
			series_id: series.id.clone(),
			id: Some("don-quixote".to_string()),
			name: Some("Don Quixote".to_string()),
			created_at: Some("1605-01-16T00:00:00Z".parse().unwrap()),
			..Default::default()
		}
		.insert(&db)
		.await;

		// this book has a single finished reading session

		fake_data::ReadingSession::completed(media.id.clone(), user.id.clone())
			.insert(&db)
			.await;

		let m = load_media(&db, &user, media.id).await;

		let entitlement =
			BookEntitlementContainer::from_media(m, "https://example.org/".to_string());

		// we finished this book.
		let reading_state = entitlement.reading_state.unwrap();
		assert_eq!(Status::Finished, reading_state.status_info.status);

		let bookmark = reading_state.current_bookmark;
		assert_eq!(None, bookmark.progress_percent);
		assert_eq!(None, bookmark.content_source_progress_percent);
		assert_eq!(None, bookmark.location);
	}
}
