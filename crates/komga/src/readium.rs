use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct R2Device {
	pub id: String,
	pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct R2Location {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub fragment: Option<Vec<String>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub position: Option<i32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub progression: Option<f32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub total_progression: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct R2LocatorText {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub after: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub before: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub highlight: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct R2Locator {
	#[serde(default)]
	pub href: String,
	#[serde(default)]
	pub r#type: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub title: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub locations: Option<R2Location>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub text: Option<R2LocatorText>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub kobo_span: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct R2Progression {
	pub modified: DateTime<Utc>,
	pub device: R2Device,
	pub locator: R2Locator,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct R2Positions {
	pub total: i32,
	pub positions: Vec<R2Locator>,
}

pub type ReadiumDevice = R2Device;
pub type ReadiumLocations = R2Location;
pub type ReadiumLocatorText = R2LocatorText;
pub type ReadiumLocator = R2Locator;
pub type ReadiumProgression = R2Progression;
pub type ReadiumPositions = R2Positions;

#[cfg(test)]
mod tests {
	use super::{R2Device, R2Location, R2Locator, R2LocatorText, R2Progression};
	use chrono::{DateTime, Utc};

	#[test]
	fn progression_round_trips_with_readium_wire_names() {
		let progression = R2Progression {
			modified: DateTime::parse_from_rfc3339("2026-09-02T16:17:44Z")
				.unwrap()
				.with_timezone(&Utc),
			device: R2Device {
				id: "android".into(),
				name: "Komelia".into(),
			},
			locator: R2Locator {
				href: "chapter.xhtml".into(),
				r#type: "application/xhtml+xml".into(),
				title: Some("Chapter".into()),
				locations: Some(R2Location {
					fragment: Some(vec!["epubcfi(/6/2)".into()]),
					position: Some(3),
					progression: Some(0.25),
					total_progression: Some(0.5),
				}),
				text: Some(R2LocatorText {
					before: Some("before".into()),
					highlight: Some("highlight".into()),
					after: Some("after".into()),
				}),
				kobo_span: None,
			},
		};
		let encoded = serde_json::to_string(&progression).unwrap();
		assert!(encoded.contains("totalProgression"));
		assert!(!encoded.contains("total_progression"));
		let decoded: R2Progression = serde_json::from_str(&encoded).unwrap();
		assert_eq!(decoded, progression);
	}
}
