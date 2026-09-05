// data types used in the Kobo sync API.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

fn serialize_kobo_progress<S>(
	value: &Option<f32>,
	serializer: S,
) -> Result<S::Ok, S::Error>
where
	S: serde::Serializer,
{
	match value {
		Some(value) if value.fract() == 0.0 => serializer.serialize_i64(*value as i64),
		Some(value) => serializer.serialize_f32(*value),
		None => serializer.serialize_none(),
	}
}

fn is_none_or_zero_i64(value: &Option<i64>) -> bool {
	value.is_none_or(|value| value == 0)
}

#[derive(Serialize)]
pub enum SyncItem {
	NewEntitlement(BookEntitlementContainer),
	ChangedEntitlement(BookEntitlementContainer),
	ChangedProductMetadata(BookMetadata),
	ChangedReadingState(ReadingStateContainer),
	/// A shelf the device has never seen; the payload wraps the tag.
	NewTag(KoboTagContainer),
	/// A shelf whose name, membership, or timestamps changed.
	ChangedTag(KoboTagContainer),
	/// A shelf deleted since the previous sync; only the id survives.
	DeletedTag(KoboDeletedTag),
}

/// Wire shape pinned to Calibre-Web `create_kobo_tag`
/// (`cps/kobo.py:707-737@a97826402f1b39c45b7ea8d906efddc9f1750934`):
/// `{"Tag": {...}}` with `Items` keyed by `RevisionId` and typed
/// `ProductRevisionTagItem`, and the tag itself typed `UserTag`.
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct KoboTagContainer {
	pub tag: KoboTag,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct KoboTag {
	pub id: String,
	pub name: String,
	pub last_modified: String,
	pub tag_type: String,
	#[serde(rename = "Type")]
	pub r#type: String,
	pub items: Vec<KoboTagItem>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct KoboTagItem {
	pub revision_id: String,
	#[serde(rename = "Type")]
	pub r#type: String,
}

/// Wire shape for `DeletedTag`: `{"Tag": {"Id", "LastModified"}}`
/// (`cps/kobo.py:655-681@a97826402f1b39c45b7ea8d906efddc9f1750934`).
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct KoboDeletedTag {
	pub tag: KoboDeletedTagId,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct KoboDeletedTagId {
	pub id: String,
	pub last_modified: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct BookEntitlementContainer {
	pub book_entitlement: BookEntitlement,
	pub book_metadata: BookMetadata,
	pub reading_state: Option<ReadingState>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReadingStateContainer {
	pub reading_state: ReadingState,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct BookEntitlement {
	pub accessibility: String,
	pub active_period: Period,
	pub created: DateTime<Utc>,
	pub cross_revision_id: String,
	pub id: String,
	pub is_hidden_from_archive: bool,
	pub is_locked: bool,
	pub is_removed: bool,
	pub last_modified: DateTime<Utc>,
	pub origin_category: String,
	pub revision_id: String,
	pub status: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Period {
	pub from: DateTime<Utc>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct BookMetadata {
	pub categories: Vec<String>,
	pub contributor_roles: Vec<ContributorRole>,
	pub contributors: Vec<String>,
	pub cover_image_id: String,
	pub cross_revision_id: String,
	pub current_display_price: DisplayPrice,
	pub current_love_display_price: LoveDisplayPrice,
	pub description: Option<String>,
	pub download_urls: Vec<DownloadUrl>,
	pub entitlement_id: String,
	pub external_ids: Vec<String>,
	pub genre: String,
	pub is_eligible_for_kobo_love: bool,
	pub is_internet_archive: bool,
	pub is_pre_order: bool,
	pub is_social_enabled: bool,
	pub isbn: Option<String>,
	pub language: String,
	// according to Komga this is a Map<String, String>.
	pub phonetic_pronunciations: Empty,
	pub publication_date: Option<DateTime<Utc>>,
	pub publisher: Option<Publisher>,
	pub revision_id: String,
	pub series: Option<Series>,
	pub title: String,
	pub work_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContributorRole {
	pub name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DisplayPrice {
	pub currency_code: String,
	pub total_amount: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LoveDisplayPrice {
	pub total_amount: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DownloadUrl {
	pub drm_type: String,
	pub format: Format,
	pub size: u64,
	pub platform: String,
	pub url: String,
}

#[derive(Serialize, Deserialize)]
pub enum Format {
	EPUB3FL,
	EPUB,
	EPUB3,
	KEPUB,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Publisher {
	pub imprint: String,
	pub name: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Series {
	pub id: String,
	pub name: String,
	pub number: String,
	pub number_float: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReadingState {
	pub created: DateTime<Utc>,
	pub current_bookmark: CurrentBookmark,
	pub entitlement_id: String,
	pub last_modified: DateTime<Utc>,
	pub priority_timestamp: DateTime<Utc>,
	pub statistics: Statistics,
	pub status_info: StatusInfo,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct CurrentBookmark {
	pub last_modified: DateTime<Utc>,
	pub progress_percent: Option<f32>,
	pub content_source_progress_percent: Option<f32>,
	pub location: Option<Location>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct Location {
	pub value: Option<String>,
	pub type_: Option<String>,
	pub source: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Statistics {
	pub last_modified: DateTime<Utc>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct StatusInfo {
	pub last_modified: DateTime<Utc>,
	pub status: Status,
	pub times_started_reading: u32,
}

// TODO: support dnf?
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Status {
	ReadyToRead,
	Finished,
	Reading,
}

#[derive(Serialize)]
pub struct Empty {}
/// The payload accepted by `PUT /v1/library/{book_id}/state`.
///
/// Kobo sends a deliberately loose object here: each of the three sections
/// can be omitted and firmware versions add fields over time. The adapter
/// keeps the original JSON separately and only projects the fields understood
/// by Stump.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct ReadingStateUpdateRequest {
	#[serde(default)]
	pub reading_states: Vec<ReadingStateUpdate>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct ReadingStateUpdate {
	#[serde(default)]
	pub current_bookmark: Option<CurrentBookmarkUpdate>,
	#[serde(default)]
	pub statistics: Option<StatisticsUpdate>,
	#[serde(default)]
	pub status_info: Option<StatusInfoUpdate>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct CurrentBookmarkUpdate {
	#[serde(default)]
	pub last_modified: Option<String>,
	#[serde(default)]
	pub progress_percent: Option<f32>,
	#[serde(default)]
	pub content_source_progress_percent: Option<f32>,
	#[serde(default)]
	pub location: Option<LocationUpdate>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct LocationUpdate {
	#[serde(default)]
	pub value: Option<String>,
	#[serde(rename = "Type", default)]
	pub type_: Option<String>,
	#[serde(default)]
	pub source: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct StatisticsUpdate {
	#[serde(default)]
	pub last_modified: Option<String>,
	#[serde(default)]
	pub spent_reading_minutes: Option<i64>,
	#[serde(default)]
	pub remaining_time_minutes: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct StatusInfoUpdate {
	#[serde(default)]
	pub last_modified: Option<String>,
	#[serde(default)]
	pub status: Option<String>,
	#[serde(default)]
	pub times_started_reading: Option<u32>,
	#[serde(default)]
	pub last_time_started_reading: Option<String>,
}

/// The one-element response returned by Kobo's state GET endpoint.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct KoboReadingStateResponse {
	pub entitlement_id: String,
	pub created: String,
	pub last_modified: String,
	pub priority_timestamp: String,
	pub status_info: KoboStatusInfoResponse,
	pub statistics: KoboStatisticsResponse,
	pub current_bookmark: KoboCurrentBookmarkResponse,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct KoboCurrentBookmarkResponse {
	pub last_modified: String,
	#[serde(
		skip_serializing_if = "Option::is_none",
		serialize_with = "serialize_kobo_progress"
	)]
	pub progress_percent: Option<f32>,
	#[serde(
		skip_serializing_if = "Option::is_none",
		serialize_with = "serialize_kobo_progress"
	)]
	pub content_source_progress_percent: Option<f32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub location: Option<KoboLocationResponse>,
}
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct KoboLocationResponse {
	pub value: Option<String>,
	#[serde(rename = "Type")]
	pub type_: Option<String>,
	pub source: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct KoboStatisticsResponse {
	pub last_modified: String,
	#[serde(skip_serializing_if = "is_none_or_zero_i64")]
	pub spent_reading_minutes: Option<i64>,
	#[serde(skip_serializing_if = "is_none_or_zero_i64")]
	pub remaining_time_minutes: Option<i64>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct KoboStatusInfoResponse {
	pub last_modified: String,
	pub status: String,
	pub times_started_reading: u32,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub last_time_started_reading: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct KoboStateResult {
	pub result: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct KoboStateUpdateResult {
	pub entitlement_id: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub current_bookmark_result: Option<KoboStateResult>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub statistics_result: Option<KoboStateResult>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub status_info_result: Option<KoboStateResult>,
	pub last_modified: String,
	pub priority_timestamp: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct KoboStateUpdateResponse {
	pub request_result: String,
	pub update_results: Vec<KoboStateUpdateResult>,
}

#[cfg(test)]
mod tests {
	use super::{
		KoboCurrentBookmarkResponse, KoboLocationResponse, KoboReadingStateResponse,
		KoboStateResult, KoboStateUpdateResponse, KoboStateUpdateResult,
		KoboStatisticsResponse, KoboStatusInfoResponse,
	};

	#[test]
	fn state_update_response_matches_calibre_web_wire_shape() {
		let response = KoboStateUpdateResponse {
			request_result: "Success".to_string(),
			update_results: vec![KoboStateUpdateResult {
				entitlement_id: "book-uuid".to_string(),
				current_bookmark_result: Some(KoboStateResult {
					result: "Success".to_string(),
				}),
				statistics_result: Some(KoboStateResult {
					result: "Success".to_string(),
				}),
				status_info_result: Some(KoboStateResult {
					result: "Success".to_string(),
				}),
				last_modified: "2026-01-02T03:04:05Z".to_string(),
				priority_timestamp: "2026-01-02T03:04:05Z".to_string(),
			}],
		};

		assert_eq!(
			serde_json::to_string(&response).expect("response serializes"),
			r#"{"RequestResult":"Success","UpdateResults":[{"EntitlementId":"book-uuid","CurrentBookmarkResult":{"Result":"Success"},"StatisticsResult":{"Result":"Success"},"StatusInfoResult":{"Result":"Success"},"LastModified":"2026-01-02T03:04:05Z","PriorityTimestamp":"2026-01-02T03:04:05Z"}]}"#
		);
	}

	#[test]
	fn state_get_response_omits_zero_optional_values_like_calibre_web() {
		let response = KoboReadingStateResponse {
			entitlement_id: "book-uuid".to_string(),
			created: "2026-01-01T00:00:00Z".to_string(),
			last_modified: "2026-01-02T03:04:05Z".to_string(),
			priority_timestamp: "2026-01-02T03:04:05Z".to_string(),
			status_info: KoboStatusInfoResponse {
				last_modified: "2026-01-02T03:04:05Z".to_string(),
				status: "Reading".to_string(),
				times_started_reading: 1,
				last_time_started_reading: Some("2026-01-02T03:04:05Z".to_string()),
			},
			statistics: KoboStatisticsResponse {
				last_modified: "2026-01-02T03:04:05Z".to_string(),
				spent_reading_minutes: Some(0),
				remaining_time_minutes: Some(12),
			},
			current_bookmark: KoboCurrentBookmarkResponse {
				last_modified: "2026-01-02T03:04:05Z".to_string(),
				progress_percent: Some(42.0),
				content_source_progress_percent: Some(42.5),
				location: Some(KoboLocationResponse {
					value: Some("kobo.1.2".to_string()),
					type_: Some("KoboSpan".to_string()),
					source: "chapter.xhtml".to_string(),
				}),
			},
		};

		assert_eq!(
			serde_json::to_string(&response).expect("response serializes"),
			r#"{"EntitlementId":"book-uuid","Created":"2026-01-01T00:00:00Z","LastModified":"2026-01-02T03:04:05Z","PriorityTimestamp":"2026-01-02T03:04:05Z","StatusInfo":{"LastModified":"2026-01-02T03:04:05Z","Status":"Reading","TimesStartedReading":1,"LastTimeStartedReading":"2026-01-02T03:04:05Z"},"Statistics":{"LastModified":"2026-01-02T03:04:05Z","RemainingTimeMinutes":12},"CurrentBookmark":{"LastModified":"2026-01-02T03:04:05Z","ProgressPercent":42,"ContentSourceProgressPercent":42.5,"Location":{"Value":"kobo.1.2","Type":"KoboSpan","Source":"chapter.xhtml"}}}"#
		);
	}
}
