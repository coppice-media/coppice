use chrono::{DateTime, FixedOffset, Utc};
use models::shared::enums::DeviceProtocol;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A partial device telemetry update. `None` means that this writer did not
/// report the field and therefore must not erase another protocol's value.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTelemetryPatch {
	pub battery_percent: Option<i32>,
	pub charging: Option<bool>,
	pub battery_source: Option<String>,
	pub battery_observed_at: Option<DateTime<FixedOffset>>,
	pub sync_status: Option<String>,
	pub sync_protocol: Option<DeviceProtocol>,
	pub synced_at: Option<DateTime<FixedOffset>>,
	#[serde(default)]
	pub counters: DeviceTelemetryCountersPatch,
}

/// Cumulative counter observations carried by a partial telemetry update.
/// Counters merge by taking the greatest observed value, so retries and a
/// lower/stale report cannot regress the device history.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTelemetryCountersPatch {
	pub progress: Option<i64>,
	pub highlights: Option<i64>,
	pub notes: Option<i64>,
	pub bookmarks: Option<i64>,
	pub sessions: Option<i64>,
	pub items: Option<i64>,
}

impl DeviceTelemetryPatch {
	/// Extracts the typed subset understood by the registry from a legacy
	/// protocol summary. Unknown summary keys remain in `last_sync_summary` but
	/// never affect typed fields.
	pub fn from_summary(
		summary: &Value,
		protocol: DeviceProtocol,
		now: DateTime<FixedOffset>,
	) -> Self {
		let object = summary.as_object();
		let value = |camel: &str, snake: &str| {
			object.and_then(|object| object.get(camel).or_else(|| object.get(snake)))
		};
		let battery_percent = value("batteryPercent", "battery_percent")
			.and_then(Value::as_i64)
			.and_then(|value| i32::try_from(value).ok())
			.filter(|value| (0..=100).contains(value));
		let charging = value("charging", "is_charging").and_then(Value::as_bool);
		let battery_source = value("batterySource", "battery_source")
			.and_then(Value::as_str)
			.map(str::to_owned);
		let battery_observed_at =
			value("batteryObservedAt", "battery_observed_at").and_then(parse_timestamp);
		let sync_status = value("syncStatus", "sync_status")
			.or_else(|| value("status", "status"))
			.and_then(Value::as_str)
			.map(str::to_owned)
			.or_else(|| Some("synced".to_owned()));
		let counter_object = object
			.and_then(|object| object.get("counters"))
			.and_then(Value::as_object);
		let counter_value = |camel: &str, snake: &str| {
			value(camel, snake).or_else(|| {
				counter_object
					.and_then(|object| object.get(camel).or_else(|| object.get(snake)))
			})
		};
		let counters = DeviceTelemetryCountersPatch {
			progress: counter(counter_value("progress", "progress")),
			highlights: counter(counter_value("highlights", "highlights")),
			notes: counter(counter_value("notes", "notes")),
			bookmarks: counter(counter_value("bookmarks", "bookmarks")),
			sessions: counter(counter_value("sessions", "sessions")),
			items: counter(counter_value("items", "items")),
		};
		Self {
			battery_percent,
			charging,
			battery_source,
			battery_observed_at: battery_observed_at.or_else(|| {
				(battery_percent.is_some() || charging.is_some()).then_some(now)
			}),
			sync_status,
			sync_protocol: Some(protocol),
			synced_at: Some(now),
			counters,
		}
	}
}

fn counter(value: Option<&Value>) -> Option<i64> {
	value.and_then(Value::as_i64).filter(|value| *value >= 0)
}

fn parse_timestamp(value: &Value) -> Option<DateTime<FixedOffset>> {
	value
		.as_str()
		.and_then(|value| DateTime::parse_from_rfc3339(value).ok())
}

/// Converts a persisted row into the JSON-free GraphQL/domain snapshot without
/// exposing the raw SeaORM model outside the storage boundary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceTelemetrySnapshot {
	pub battery_percent: Option<i32>,
	pub charging: Option<bool>,
	pub battery_source: Option<String>,
	pub battery_observed_at: Option<DateTime<Utc>>,
	pub sync_status: Option<String>,
	pub sync_protocol: Option<DeviceProtocol>,
	pub synced_at: Option<DateTime<Utc>>,
	pub counters: DeviceTelemetryCounters,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceTelemetryCounters {
	pub progress: Option<i64>,
	pub highlights: Option<i64>,
	pub notes: Option<i64>,
	pub bookmarks: Option<i64>,
	pub sessions: Option<i64>,
	pub items: Option<i64>,
}
