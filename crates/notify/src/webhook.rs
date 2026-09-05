//! The generic webhook channel.
//!
//! Each [`Notification`] is POSTed as JSON to the user-configured URL. When a
//! `secret` is configured the request carries
//! `X-Stump-Signature: sha256=<hex>` — an HMAC-SHA256 over the exact request
//! body — so receivers can authenticate the caller and reject replays of
//! tampered payloads.

use std::sync::LazyLock;

use data_encoding::HEXLOWER;
use ring::hmac;
use serde_json::json;
use stump_api_types::settings::{SettingDefinition, SettingKind};

use crate::{Channel, ChannelError, Notification, Recipient};

const CHANNEL_ID: &str = "webhook";
const SIGNATURE_HEADER: &str = "X-Stump-Signature";

static SETTINGS: LazyLock<Vec<SettingDefinition>> = LazyLock::new(|| {
	vec![
		SettingDefinition {
			key: "url",
			label: "Webhook URL",
			description: "The HTTP(S) endpoint notifications are POSTed to",
			kind: SettingKind::String,
			default: json!(""),
			required: true,
			secret: false,
			help_url: None,
		},
		SettingDefinition {
			key: "secret",
			label: "Signing secret",
			description: "Optional HMAC-SHA256 secret used to sign request bodies",
			kind: SettingKind::String,
			default: json!(""),
			required: false,
			secret: true,
			help_url: None,
		},
	]
});

/// The signed JSON webhook channel. Stateless per user.
pub struct WebhookChannel {
	client: reqwest::Client,
}

impl Default for WebhookChannel {
	fn default() -> Self {
		Self::new()
	}
}

impl WebhookChannel {
	pub fn new() -> Self {
		Self {
			client: reqwest::Client::new(),
		}
	}

	/// The JSON body a notification is rendered to.
	pub fn render(notification: &Notification) -> serde_json::Value {
		json!({
			"kind": notification.kind.to_string(),
			"title": notification.title,
			"body": notification.body,
			"link": notification.link,
			"attachment": notification.attachment.as_ref().map(|attachment| {
				json!({
					"filename": attachment.filename,
					"mime": attachment.mime,
				})
			}),
		})
	}

	/// `sha256=<hex>` HMAC over `body`, or `None` without a secret.
	pub fn signature(secret: &str, body: &[u8]) -> Option<String> {
		if secret.is_empty() {
			return None;
		}
		let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
		Some(format!(
			"sha256={}",
			HEXLOWER.encode(hmac::sign(&key, body).as_ref())
		))
	}
}

#[async_trait::async_trait]
impl Channel for WebhookChannel {
	fn id(&self) -> &'static str {
		CHANNEL_ID
	}

	fn label(&self) -> &'static str {
		"Webhook"
	}

	fn settings(&self) -> &[SettingDefinition] {
		SETTINGS.as_slice()
	}

	async fn send(
		&self,
		recipient: &Recipient,
		notification: &Notification,
	) -> Result<(), ChannelError> {
		let url = recipient.required_str(CHANNEL_ID, "url")?.to_string();
		let secret = recipient
			.optional_str("secret")
			.unwrap_or_default()
			.to_string();

		let body = serde_json::to_vec(&Self::render(notification))
			.map_err(|error| ChannelError::Rejected(error.to_string()))?;

		let mut request = self
			.client
			.post(&url)
			.header(reqwest::header::CONTENT_TYPE, "application/json");

		if let Some(signature) = Self::signature(&secret, &body) {
			request = request.header(SIGNATURE_HEADER, signature);
		}

		let response = request
			.body(body)
			.send()
			.await
			.map_err(|error| ChannelError::Transport(error.to_string()))?;

		let status = response.status();
		if status.is_success() {
			Ok(())
		} else if status.is_client_error() {
			Err(ChannelError::Rejected(format!("webhook returned {status}")))
		} else {
			Err(ChannelError::Transport(format!("webhook returned {status}")))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Attachment, NotificationKind};

	#[test]
	fn signature_is_stable_hmac_sha256() {
		// Reference: printf 'hello' | openssl dgst -sha256 -hmac 'topsecret'
		assert_eq!(
			WebhookChannel::signature("topsecret", b"hello").as_deref(),
			Some("ed76fd36523b8becda5a3b36d0e3737e8ae5111f55e26c7c3a455a3ce29636d2")
		);
	}

	#[test]
	fn signature_is_none_without_secret() {
		assert_eq!(WebhookChannel::signature("", b"hello"), None);
	}

	#[test]
	fn render_includes_kind_title_and_attachment_metadata() {
		let notification = Notification::new(NotificationKind::ScanFinished, "Scan", "done")
			.with_attachment(Attachment::bytes("a.cbz", "application/zip", vec![]));
		let rendered = WebhookChannel::render(&notification);
		assert_eq!(rendered["kind"], "SCAN_FINISHED");
		assert_eq!(rendered["title"], "Scan");
		assert_eq!(rendered["attachment"]["filename"], "a.cbz");
		assert_eq!(rendered["attachment"]["mime"], "application/zip");
		assert_eq!(rendered["link"], serde_json::Value::Null);
	}
}
