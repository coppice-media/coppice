use std::{borrow::Cow, path::PathBuf};

use serde::{Deserialize, Serialize};
use stump_api_types::settings::{SettingDefinition, SettingValues};

/// The events a user can route to a channel. Persisted by name in
/// `notification_rules.event_kind` and in queued dispatch jobs, so variants are
/// append-only.
#[derive(
	Debug,
	Clone,
	Copy,
	PartialEq,
	Eq,
	Hash,
	PartialOrd,
	Ord,
	Serialize,
	Deserialize,
	strum::Display,
	strum::EnumString,
	strum::EnumIter,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(feature = "graphql", derive(async_graphql::Enum))]
pub enum NotificationKind {
	/// A pairing was approved and the device received its credential
	DevicePaired,
	/// A registered device authenticated for the first time
	DeviceFirstSeen,
	/// A staged ingest item finished analysis and needs a human decision
	IngestAwaitingReview,
	/// A quality report contains at least one failed check
	QualityFailed,
	/// A library scan completed
	ScanFinished,
	/// A staged analysis job failed
	AnalysisJobFailed,
	/// Provider lookup finished for a staged item
	ProviderMatchDone,
	/// Sent by `testNotificationChannel`; never routed by rules
	Test,
	/// A social recommendation or explicit share was sent to this user.
	RecommendationReceived,
	/// A recipient accepted a recommendation or share grant.
	RecommendationAccepted,
	/// A recipient declined a recommendation or share grant.
	RecommendationDeclined,
	/// A sender or authorized owner revoked a recommendation or share grant.
	RecommendationRevoked,
	/// A recipient's request handoff changed state.
	RequestStatusUpdated,
}

impl NotificationKind {
	/// Kinds a rule may subscribe to (everything but [`Self::Test`]).
	pub fn routable(self) -> bool {
		self != Self::Test
	}

	/// Whether the kind concerns library administration rather than the
	/// recipient's own devices. Hosts gate rule creation on this.
	pub fn is_administrative(self) -> bool {
		!matches!(
			self,
			Self::DevicePaired
				| Self::DeviceFirstSeen
				| Self::RecommendationReceived
				| Self::RecommendationAccepted
				| Self::RecommendationDeclined
				| Self::RecommendationRevoked
				| Self::RequestStatusUpdated
				| Self::Test
		)
	}

	/// ntfy-style emoji shortcodes describing the kind.
	pub fn tags(self) -> &'static str {
		match self {
			Self::DevicePaired => "link",
			Self::DeviceFirstSeen => "eyes",
			Self::IngestAwaitingReview => "inbox_tray",
			Self::QualityFailed => "warning",
			Self::ScanFinished => "books",
			Self::AnalysisJobFailed => "x",
			Self::ProviderMatchDone => "mag",
			Self::Test => "wave",
			Self::RecommendationReceived => "mail",
			Self::RecommendationAccepted => "white_check_mark",
			Self::RecommendationDeclined => "no_entry_sign",
			Self::RecommendationRevoked => "unlock",
			Self::RequestStatusUpdated => "inbox_tray",
		}
	}
}

/// Where an attachment's bytes live. `Path` defers the read to the channel so a
/// queued dispatch payload never carries a whole book.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "source")]
pub enum AttachmentContent {
	Bytes {
		#[serde(with = "base64_bytes")]
		bytes: Vec<u8>,
	},
	Path {
		path: PathBuf,
	},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
	pub filename: String,
	pub mime: String,
	#[serde(flatten)]
	pub content: AttachmentContent,
}

impl Attachment {
	pub fn bytes(
		filename: impl Into<String>,
		mime: impl Into<String>,
		bytes: Vec<u8>,
	) -> Self {
		Self {
			filename: filename.into(),
			mime: mime.into(),
			content: AttachmentContent::Bytes { bytes },
		}
	}

	pub fn path(
		filename: impl Into<String>,
		mime: impl Into<String>,
		path: PathBuf,
	) -> Self {
		Self {
			filename: filename.into(),
			mime: mime.into(),
			content: AttachmentContent::Path { path },
		}
	}

	/// The attachment bytes, reading `Path` content on demand.
	pub fn load(&self) -> Result<Cow<'_, [u8]>, ChannelError> {
		match &self.content {
			AttachmentContent::Bytes { bytes } => Ok(Cow::Borrowed(bytes)),
			AttachmentContent::Path { path } => {
				std::fs::read(path).map(Cow::Owned).map_err(|error| {
					ChannelError::Attachment(format!("{}: {error}", path.display()))
				})
			},
		}
	}

	/// The byte length without reading `Path` content into memory.
	pub fn size(&self) -> Result<u64, ChannelError> {
		match &self.content {
			AttachmentContent::Bytes { bytes } => Ok(bytes.len() as u64),
			AttachmentContent::Path { path } => std::fs::metadata(path)
				.map(|meta| meta.len())
				.map_err(|error| {
					ChannelError::Attachment(format!("{}: {error}", path.display()))
				}),
		}
	}
}

/// One message to deliver. Channels render `title`/`body`/`link` however their
/// medium allows and attach `attachment` when they can carry files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
	pub kind: NotificationKind,
	pub title: String,
	pub body: String,
	pub link: Option<String>,
	pub attachment: Option<Attachment>,
}

impl Notification {
	pub fn new(
		kind: NotificationKind,
		title: impl Into<String>,
		body: impl Into<String>,
	) -> Self {
		Self {
			kind,
			title: title.into(),
			body: body.into(),
			link: None,
			attachment: None,
		}
	}

	pub fn with_link(mut self, link: impl Into<String>) -> Self {
		self.link = Some(link.into());
		self
	}

	pub fn with_attachment(mut self, attachment: Attachment) -> Self {
		self.attachment = Some(attachment);
		self
	}

	/// The message `testNotificationChannel` delivers.
	pub fn test(channel_label: &str) -> Self {
		Self::new(
			NotificationKind::Test,
			"Coppice test notification",
			format!("Your {channel_label} channel is configured correctly."),
		)
	}
}

/// The user a channel delivers to, with that user's stored values for the
/// channel's [`Channel::settings`] (secrets already decrypted by the host).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipient {
	pub user_id: String,
	pub username: String,
	pub settings: SettingValues,
}

impl Recipient {
	pub fn new(
		user_id: impl Into<String>,
		username: impl Into<String>,
		settings: SettingValues,
	) -> Self {
		Self {
			user_id: user_id.into(),
			username: username.into(),
			settings,
		}
	}

	/// A non-empty string setting, or `MissingSetting`.
	pub fn required_str(
		&self,
		channel: &'static str,
		key: &'static str,
	) -> Result<&str, ChannelError> {
		self.optional_str(key)
			.ok_or(ChannelError::MissingSetting { channel, key })
	}

	/// A non-empty string setting when present.
	pub fn optional_str(&self, key: &str) -> Option<&str> {
		self.settings
			.get(key)
			.and_then(serde_json::Value::as_str)
			.map(str::trim)
			.filter(|value| !value.is_empty())
	}
}

/// A message a channel received from the user (reply, command). No shipped
/// channel implements it yet; Telegram bot polling is the intended first user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inbound {
	pub text: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
	#[error("channel `{channel}` is missing the `{key}` setting")]
	MissingSetting {
		channel: &'static str,
		key: &'static str,
	},
	#[error("invalid `{key}` setting: {reason}")]
	InvalidSetting { key: String, reason: String },
	/// The remote side could not be reached or failed; worth retrying.
	#[error("delivery failed: {0}")]
	Transport(String),
	/// The remote side understood and refused the message; retrying is futile.
	#[error("delivery rejected: {0}")]
	Rejected(String),
	#[error("attachment unreadable: {0}")]
	Attachment(String),
	#[error("the channel does not support this operation")]
	Unsupported,
}

impl ChannelError {
	pub fn is_retryable(&self) -> bool {
		matches!(self, Self::Transport(_))
	}
}

/// A delivery medium. Implementations are stateless per user: everything
/// user-specific arrives through [`Recipient::settings`], described by
/// [`Channel::settings`].
#[async_trait::async_trait]
pub trait Channel: Send + Sync {
	/// Stable identifier persisted in rules and settings rows (`ntfy`, `email`, `webhook`)
	fn id(&self) -> &'static str;

	fn label(&self) -> &'static str;

	/// The per-user settings schema.
	fn settings(&self) -> &[SettingDefinition];

	/// The values a user starts with before configuring the channel. Defaults
	/// to each definition's `default`; channels override to derive per-user
	/// values (ntfy generates a topic).
	fn default_settings(&self, _user_id: &str) -> SettingValues {
		self.settings()
			.iter()
			.map(|definition| (definition.key.to_string(), definition.default.clone()))
			.collect()
	}

	async fn send(
		&self,
		recipient: &Recipient,
		notification: &Notification,
	) -> Result<(), ChannelError>;

	/// Poll the channel for messages the user sent back. Channels without an
	/// inbound side return [`ChannelError::Unsupported`].
	async fn receive(
		&self,
		_recipient: &Recipient,
	) -> Result<Vec<Inbound>, ChannelError> {
		Err(ChannelError::Unsupported)
	}
}

mod base64_bytes {
	use base64::{engine::general_purpose::STANDARD, Engine};
	use serde::{Deserialize, Deserializer, Serializer};

	pub fn serialize<S: Serializer>(
		bytes: &[u8],
		serializer: S,
	) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(&STANDARD.encode(bytes))
	}

	pub fn deserialize<'de, D: Deserializer<'de>>(
		deserializer: D,
	) -> Result<Vec<u8>, D::Error> {
		let encoded = String::deserialize(deserializer)?;
		STANDARD.decode(encoded).map_err(serde::de::Error::custom)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn attachment_bytes_round_trip_through_json_as_base64() {
		let notification = Notification::new(NotificationKind::Test, "t", "b")
			.with_attachment(Attachment::bytes("a.txt", "text/plain", b"hello".to_vec()));
		let json = serde_json::to_value(&notification).unwrap();
		assert_eq!(json["attachment"]["bytes"], "aGVsbG8=");
		assert_eq!(json["attachment"]["source"], "bytes");
		let back: Notification = serde_json::from_value(json).unwrap();
		assert_eq!(back, notification);
	}

	#[test]
	fn path_attachment_loads_lazily() {
		let missing = Attachment::path(
			"x.cbz",
			"application/zip",
			PathBuf::from("/definitely/missing"),
		);
		assert!(matches!(missing.load(), Err(ChannelError::Attachment(_))));
		assert_eq!(missing.size().is_err(), true);
	}

	#[test]
	fn kind_names_are_stable_strings() {
		assert_eq!(
			NotificationKind::IngestAwaitingReview.to_string(),
			"INGEST_AWAITING_REVIEW"
		);
		assert_eq!(
			"SCAN_FINISHED".parse::<NotificationKind>().unwrap(),
			NotificationKind::ScanFinished
		);
		assert!(!NotificationKind::Test.routable());
		assert!(!NotificationKind::DevicePaired.is_administrative());
		assert!(NotificationKind::ScanFinished.is_administrative());
	}

	#[test]
	fn recipient_setting_lookup_trims_and_rejects_empty() {
		let mut settings = SettingValues::new();
		settings.insert("topic".into(), serde_json::json!("  stump  "));
		settings.insert("token".into(), serde_json::json!(""));
		let recipient = Recipient::new("u", "al", settings);
		assert_eq!(recipient.optional_str("topic"), Some("stump"));
		assert_eq!(recipient.optional_str("token"), None);
		assert!(matches!(
			recipient.required_str("ntfy", "token"),
			Err(ChannelError::MissingSetting {
				channel: "ntfy",
				key: "token"
			})
		));
	}
}
