use serde::{Deserialize, Serialize};

/// Komga's `DirectoryRequestDto`: an empty `path` asks for the root
/// directories, and `show_files` includes plain files in the listing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectoryRequest {
	#[serde(default)]
	pub path: String,
	#[serde(default)]
	pub show_files: bool,
}

/// Komga's `DirectoryListingDto`. `parent` is omitted when unset, matching the
/// upstream `@JsonInclude(NON_NULL)` contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryListing {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub parent: Option<String>,
	pub directories: Vec<Path>,
	pub files: Vec<Path>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Path {
	pub r#type: String,
	pub name: String,
	pub path: String,
}
