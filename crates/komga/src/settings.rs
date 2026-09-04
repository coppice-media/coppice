use crate::common::PatchValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaSettings {
	pub delete_empty_collections: bool,
	pub delete_empty_read_lists: bool,
	pub remember_me_duration_days: i32,
	pub thumbnail_size: KomgaThumbnailSize,
	pub task_pool_size: i32,
	pub server_port: SettingMultiSource<Option<i32>>,
	pub server_context_path: SettingMultiSource<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SettingMultiSource<T> {
	pub configuration_source: T,
	pub database_source: T,
	pub effective_value: T,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KomgaThumbnailSize {
	Default,
	Medium,
	Large,
	Xlarge,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaSettingsUpdateRequest {
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub delete_empty_collections: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub delete_empty_read_lists: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub remember_me_duration_days: PatchValue<i32>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub renew_remember_me_key: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub thumbnail_size: PatchValue<KomgaThumbnailSize>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub task_pool_size: PatchValue<i32>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub server_port: PatchValue<i32>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub server_context_path: PatchValue<String>,
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn settings_round_trip_matches_pinned_wire_names() {
		let settings = KomgaSettings {
			delete_empty_collections: false,
			delete_empty_read_lists: true,
			remember_me_duration_days: 30,
			thumbnail_size: KomgaThumbnailSize::Xlarge,
			task_pool_size: 1,
			server_port: SettingMultiSource {
				configuration_source: Some(10801),
				database_source: None,
				effective_value: Some(10801),
			},
			server_context_path: SettingMultiSource {
				configuration_source: None,
				database_source: None,
				effective_value: None,
			},
		};

		let encoded = serde_json::to_value(&settings).expect("settings serialize");
		assert_eq!(
			encoded,
			json!({
				"deleteEmptyCollections": false,
				"deleteEmptyReadLists": true,
				"rememberMeDurationDays": 30,
				"thumbnailSize": "XLARGE",
				"taskPoolSize": 1,
				"serverPort": {
					"configurationSource": 10801,
					"databaseSource": null,
					"effectiveValue": 10801,
				},
				"serverContextPath": {
					"configurationSource": null,
					"databaseSource": null,
					"effectiveValue": null,
				},
			})
		);
		let parsed: KomgaSettings =
			serde_json::from_value(encoded).expect("settings deserialize");
		assert_eq!(parsed, settings);
	}

	#[test]
	fn settings_update_round_trip_preserves_patch_states() {
		let request = KomgaSettingsUpdateRequest {
			delete_empty_collections: PatchValue::Some(true),
			delete_empty_read_lists: PatchValue::None,
			remember_me_duration_days: PatchValue::Unset,
			renew_remember_me_key: PatchValue::Some(false),
			thumbnail_size: PatchValue::Some(KomgaThumbnailSize::Medium),
			task_pool_size: PatchValue::None,
			server_port: PatchValue::Some(1234),
			server_context_path: PatchValue::None,
		};

		let encoded = serde_json::to_value(&request).expect("settings update serialize");
		assert_eq!(
			encoded,
			json!({
				"deleteEmptyCollections": true,
				"deleteEmptyReadLists": null,
				"renewRememberMeKey": false,
				"thumbnailSize": "MEDIUM",
				"taskPoolSize": null,
				"serverPort": 1234,
				"serverContextPath": null,
			})
		);
		let parsed: KomgaSettingsUpdateRequest =
			serde_json::from_value(encoded).expect("settings update deserialize");
		assert_eq!(parsed, request);
	}
}
