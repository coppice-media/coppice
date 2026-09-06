//! The email channel: a thin adapter over the `email` crate's
//! [`EmailerClient`](email::EmailerClient).
//!
//! SMTP credentials are server-wide (the `emailers` table); only the
//! recipient address is a per-user setting. The host constructs the channel
//! with the decrypted server config, so [`Channel::send`] maps a
//! [`Notification`] onto an attachment email and the GraphQL
//! `sendAttachmentEmail` flow can reuse the exact same delivery path through
//! [`EmailChannel::deliver`].

use email::EmailError;
use email::{AttachmentPayload, EmailerClient, EmailerClientConfig};
use serde_json::json;
use std::sync::LazyLock;
use stump_api_types::settings::{SettingDefinition, SettingKind};

use crate::{Attachment, Channel, ChannelError, Notification, Recipient};

const CHANNEL_ID: &str = "email";

static SETTINGS: LazyLock<Vec<SettingDefinition>> = LazyLock::new(|| {
	vec![SettingDefinition {
		key: "recipient_email",
		label: "Recipient email",
		description: "The address notifications are sent to",
		kind: SettingKind::String,
		default: json!(""),
		required: true,
		secret: false,
		help_url: None,
	}]
});

/// The SMTP channel wrapping one server-wide [`EmailerClientConfig`].
pub struct EmailChannel {
	client: EmailerClient,
}

impl EmailChannel {
	pub fn new(config: EmailerClientConfig) -> Self {
		Self {
			client: EmailerClient::new(config),
		}
	}

	/// Attachment delivery with the standard Stump attachment notice as the
	/// body; GraphQL `sendAttachmentEmail` funnels through here so both paths
	/// share one SMTP client and error mapping.
	pub async fn deliver(
		&self,
		recipient: &str,
		subject: &str,
		payloads: Vec<AttachmentPayload>,
	) -> Result<(), ChannelError> {
		self.client
			.send_attachments(subject, recipient, payloads)
			.await
			.map_err(map_email_error)
	}

	/// Delivery with an explicit plain-text body; used by [`Channel::send`].
	pub async fn deliver_with_body(
		&self,
		recipient: &str,
		subject: &str,
		body: String,
		payloads: Vec<AttachmentPayload>,
	) -> Result<(), ChannelError> {
		self.client
			.send_message(subject, recipient, body, payloads)
			.await
			.map_err(map_email_error)
	}
}

fn map_email_error(error: EmailError) -> ChannelError {
	match error {
		// Config/address problems are permanent; retrying cannot fix them.
		EmailError::InvalidEmail(message) => ChannelError::Rejected(message),
		EmailError::EmailBuildFailed(error) => ChannelError::Rejected(error.to_string()),
		EmailError::NoPassword => {
			ChannelError::Rejected("the emailer config is missing a password".to_string())
		},
		// SMTP failures can be transient (connection, 4xx).
		EmailError::SendFailed(error) => ChannelError::Transport(error.to_string()),
	}
}

fn body(notification: &Notification) -> String {
	match &notification.link {
		Some(link) => format!("{}\n\n{}", notification.body, link),
		None => notification.body.clone(),
	}
}

#[async_trait::async_trait]
impl Channel for EmailChannel {
	fn id(&self) -> &'static str {
		CHANNEL_ID
	}

	fn label(&self) -> &'static str {
		"Email"
	}

	fn settings(&self) -> &[SettingDefinition] {
		SETTINGS.as_slice()
	}

	async fn send(
		&self,
		recipient: &Recipient,
		notification: &Notification,
	) -> Result<(), ChannelError> {
		let to = recipient.required_str(CHANNEL_ID, "recipient_email")?;

		let payloads = match &notification.attachment {
			Some(attachment) => vec![attachment_payload(attachment)?],
			None => Vec::new(),
		};

		self.deliver_with_body(to, &notification.title, body(notification), payloads)
			.await
	}
}

fn attachment_payload(
	attachment: &Attachment,
) -> Result<AttachmentPayload, ChannelError> {
	let bytes = attachment.load()?;
	let content_type: email::EmailContentType = attachment
		.mime
		.parse()
		.unwrap_or(email::EmailContentType::TEXT_PLAIN);
	Ok(AttachmentPayload {
		name: attachment.filename.clone(),
		content: bytes.into_owned(),
		content_type,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::NotificationKind;
	use stump_api_types::settings::SettingValues;

	fn channel_without_password() -> EmailChannel {
		EmailChannel::new(EmailerClientConfig {
			sender_email: "stump@example.com".to_string(),
			sender_display_name: "Stump".to_string(),
			username: "stump@example.com".to_string(),
			password: None,
			host: "smtp.example.com".to_string(),
			port: 587,
			tls_enabled: true,
			max_attachment_size_bytes: None,
			max_num_attachments: None,
		})
	}

	fn recipient() -> Recipient {
		let mut settings = SettingValues::new();
		settings.insert("recipient_email".to_string(), json!("reader@example.com"));
		Recipient::new("user-1", "reader", settings)
	}

	#[tokio::test]
	async fn missing_password_is_rejected_not_retried() {
		let error = channel_without_password()
			.send(
				&recipient(),
				&Notification::new(NotificationKind::Test, "Test", "body"),
			)
			.await
			.unwrap_err();
		assert!(matches!(error, ChannelError::Rejected(_)));
		assert!(!error.is_retryable());
	}

	#[tokio::test]
	async fn missing_recipient_is_a_setting_error() {
		let error = channel_without_password()
			.send(
				&Recipient::new("user-1", "reader", SettingValues::new()),
				&Notification::new(NotificationKind::Test, "Test", "body"),
			)
			.await
			.unwrap_err();
		assert!(matches!(
			error,
			ChannelError::MissingSetting {
				channel: "email",
				key: "recipient_email"
			}
		));
	}

	#[test]
	fn body_appends_link() {
		let notification = Notification::new(NotificationKind::Test, "Test", "Check it")
			.with_link("https://stump.example/ingest");
		assert_eq!(
			body(&notification),
			"Check it\n\nhttps://stump.example/ingest"
		);
	}
}
