use crate::common::PatchValue;
use crate::library::KomgaLibraryId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct KomgaUserId(pub String);

impl KomgaUserId {
	pub fn new(value: impl Into<String>) -> Self {
		Self(value.into())
	}
}

impl fmt::Display for KomgaUserId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.0.fmt(f)
	}
}

impl From<String> for KomgaUserId {
	fn from(value: String) -> Self {
		Self(value)
	}
}

impl From<&str> for KomgaUserId {
	fn from(value: &str) -> Self {
		Self(value.to_owned())
	}
}

pub const ROLE_USER: &str = "USER";
pub const ROLE_ADMIN: &str = "ADMIN";
pub const ROLE_FILE_DOWNLOAD: &str = "FILE_DOWNLOAD";
pub const ROLE_PAGE_STREAMING: &str = "PAGE_STREAMING";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaUser {
	pub id: KomgaUserId,
	pub email: String,
	pub roles: BTreeSet<String>,
	pub shared_all_libraries: bool,
	pub shared_libraries_ids: BTreeSet<KomgaLibraryId>,
	pub labels_allow: BTreeSet<String>,
	pub labels_exclude: BTreeSet<String>,
	#[serde(default)]
	pub age_restriction: Option<KomgaAgeRestriction>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaAuthenticationActivity {
	pub user_id: Option<KomgaUserId>,
	pub email: Option<String>,
	pub ip: Option<String>,
	pub user_agent: Option<String>,
	pub success: bool,
	pub error: Option<String>,
	pub date_time: chrono::DateTime<chrono::Utc>,
	pub source: String,
}

#[cfg(test)]
mod authentication_activity_tests {
	use super::*;

	#[test]
	fn authentication_activity_round_trips_pinned_wire_names() {
		let activity = KomgaAuthenticationActivity {
			user_id: Some(KomgaUserId::from("user-id")),
			email: Some("reader".to_owned()),
			ip: Some("127.0.0.1".to_owned()),
			user_agent: Some("Komelia/0.19.0".to_owned()),
			success: true,
			error: None,
			date_time: "2026-09-03T12:34:56Z".parse().expect("valid timestamp"),
			source: "PASSWORD".to_owned(),
		};

		let encoded = serde_json::to_value(&activity).expect("activity serialize");
		assert_eq!(
			encoded,
			serde_json::json!({
				"userId": "user-id",
				"email": "reader",
				"ip": "127.0.0.1",
				"userAgent": "Komelia/0.19.0",
				"success": true,
				"error": null,
				"dateTime": "2026-09-03T12:34:56Z",
				"source": "PASSWORD",
			})
		);
		let parsed: KomgaAuthenticationActivity =
			serde_json::from_value(encoded).expect("activity deserialize");
		assert_eq!(parsed, activity);
	}
}

impl KomgaUser {
	pub fn role_admin(&self) -> bool {
		self.roles.contains(ROLE_ADMIN)
	}

	pub fn role_file_download(&self) -> bool {
		self.roles.contains(ROLE_FILE_DOWNLOAD)
	}

	pub fn role_page_streaming(&self) -> bool {
		self.roles.contains(ROLE_PAGE_STREAMING)
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaUserCreateRequest {
	pub email: String,
	pub password: String,
	pub roles: BTreeSet<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaPasswordUpdateRequest {
	pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaUserUpdateRequest {
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub age_restriction: PatchValue<KomgaAgeRestriction>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub labels_allow: PatchValue<BTreeSet<String>>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub labels_exclude: PatchValue<BTreeSet<String>>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub roles: PatchValue<BTreeSet<String>>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub shared_libraries: PatchValue<KomgaSharedLibrariesUpdate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaSharedLibrariesUpdate {
	pub all: bool,
	pub library_ids: BTreeSet<KomgaLibraryId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaAgeRestriction {
	pub age: i32,
	pub restriction: AllowExclude,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AllowExclude {
	AllowOnly,
	Exclude,
	None,
}
