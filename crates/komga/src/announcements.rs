use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KomgaJsonFeed {
	pub version: String,
	pub title: String,
	#[serde(rename = "home_page_url")]
	pub home_page_url: Option<String>,
	pub description: Option<String>,
	#[serde(default)]
	pub items: Vec<KomgaAnnouncement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct KomgaAnnouncementId(pub String);

impl KomgaAnnouncementId {
	pub fn new(value: impl Into<String>) -> Self {
		Self(value.into())
	}
}

impl std::fmt::Display for KomgaAnnouncementId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.0.fmt(f)
	}
}

impl From<String> for KomgaAnnouncementId {
	fn from(value: String) -> Self {
		Self(value)
	}
}

impl From<&str> for KomgaAnnouncementId {
	fn from(value: &str) -> Self {
		Self(value.to_owned())
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KomgaAnnouncement {
	pub id: KomgaAnnouncementId,
	pub url: Option<String>,
	pub title: Option<String>,
	pub summary: Option<String>,
	#[serde(rename = "content_html")]
	pub content_html: Option<String>,
	#[serde(rename = "date_modified")]
	pub date_modified: Option<DateTime<Utc>>,
	pub author: Option<Author>,
	#[serde(default)]
	pub tags: BTreeSet<String>,
	#[serde(rename = "_komga")]
	pub komga_extension: Option<KomgaExtension>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Author {
	pub name: Option<String>,
	pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KomgaExtension {
	pub read: bool,
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn json_feed_round_trip_matches_pinned_wire_names() {
		let feed = KomgaJsonFeed {
			version: "https://jsonfeed.org/version/1.1".to_owned(),
			title: "Stump".to_owned(),
			home_page_url: None,
			description: Some("Stump publishes no feed.".to_owned()),
			items: vec![KomgaAnnouncement {
				id: KomgaAnnouncementId("announcement-1".to_owned()),
				url: Some("https://example.test/announcement-1".to_owned()),
				title: Some("Hello".to_owned()),
				summary: Some("Summary".to_owned()),
				content_html: Some("<p>Content</p>".to_owned()),
				date_modified: Some(
					"2026-09-03T12:34:56Z"
						.parse::<DateTime<Utc>>()
						.expect("valid timestamp"),
				),
				author: Some(Author {
					name: Some("Stump".to_owned()),
					url: None,
				}),
				tags: BTreeSet::from(["news".to_owned(), "stump".to_owned()]),
				komga_extension: Some(KomgaExtension { read: false }),
			}],
		};

		let encoded = serde_json::to_value(&feed).expect("feed serialize");
		assert_eq!(
			encoded,
			json!({
				"version": "https://jsonfeed.org/version/1.1",
				"title": "Stump",
				"home_page_url": null,
				"description": "Stump publishes no feed.",
				"items": [{
					"id": "announcement-1",
					"url": "https://example.test/announcement-1",
					"title": "Hello",
					"summary": "Summary",
					"content_html": "<p>Content</p>",
					"date_modified": "2026-09-03T12:34:56Z",
					"author": {"name": "Stump", "url": null},
					"tags": ["news", "stump"],
					"_komga": {"read": false},
				}],
			})
		);
		let parsed: KomgaJsonFeed =
			serde_json::from_value(encoded).expect("feed deserialize");
		assert_eq!(parsed, feed);
	}
}
