//! What this crate adds to the send-to-Kindle lane: the server's recipient
//! policy and the emailer history.
//!
//! The lane itself — format policy, conversion, Amazon's ceiling, the
//! `kindle_deliveries` row — is tested against a real `lettre` transport in
//! `crates/kindle/src/tests.rs`. Repeating it here would only test
//! `stump_kindle` twice.

use std::sync::Arc;

use ::tests::{db::test_database, fake_data};
use email::{AttachmentPayload, EmailContentType};
use models::entity::{emailer, emailer_send_record, registered_email_device};
use pretty_assertions::assert_eq;
use sea_orm::{
	ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait, NotSet,
};
use stump_notify::EmailChannel;

use super::*;

struct Fixture {
	conn: Arc<DatabaseConnection>,
	user: AuthUser,
	emailer: emailer::Model,
}

impl Fixture {
	async fn setup() -> Self {
		let conn = Arc::new(test_database().await);
		let row = fake_data::User::new("al").insert(conn.as_ref()).await;
		let user = AuthUser {
			id: row.id,
			username: row.username,
			is_server_owner: true,
			..Default::default()
		};

		let emailer = emailer::ActiveModel {
			id: NotSet,
			name: Set("primary".to_string()),
			is_primary: Set(true),
			sender_email: Set("stump@example.test".to_string()),
			sender_display_name: Set("Stump".to_string()),
			username: Set("stump".to_string()),
			encrypted_password: Set("unused-without-an-smtp-server".to_string()),
			smtp_host: Set("smtp.example.test".to_string()),
			smtp_port: Set(587),
			tls_enabled: Set(true),
			max_attachment_size_bytes: Set(None),
			max_num_attachments: Set(None),
			last_used_at: NotSet,
		}
		.insert(conn.as_ref())
		.await
		.expect("emailer");

		Self {
			conn,
			user,
			emailer,
		}
	}

	fn mailer(&self) -> EmailChannelMailer<'_> {
		EmailChannelMailer {
			channel: EmailChannel::new(email::EmailerClientConfig {
				sender_email: self.emailer.sender_email.clone(),
				sender_display_name: self.emailer.sender_display_name.clone(),
				username: self.emailer.username.clone(),
				password: Some("unused-without-an-smtp-server".to_string()),
				host: self.emailer.smtp_host.clone(),
				port: self.emailer.smtp_port as u16,
				tls_enabled: self.emailer.tls_enabled,
				max_attachment_size_bytes: None,
				max_num_attachments: None,
			}),
			conn: self.conn.as_ref(),
			user: &self.user,
			emailer: self.emailer.clone(),
			media_id: "book-1".to_string(),
		}
	}

	async fn records(&self) -> Vec<emailer_send_record::Model> {
		emailer_send_record::Entity::find()
			.all(self.conn.as_ref())
			.await
			.expect("records")
	}
}

fn payload() -> AttachmentPayload {
	AttachmentPayload {
		name: "book.azw3".to_string(),
		content: b"BOOKMOBI kf8 payload".to_vec(),
		content_type: EmailContentType::parse("application/vnd.amazon.mobi8-ebook")
			.expect("content type"),
	}
}

/// A Kindle address the operator marked forbidden is refused before the lane
/// reads or converts anything: the send-to-Kindle lane must not become a way
/// around the emailer's address policy.
#[tokio::test]
async fn a_forbidden_recipient_is_refused() {
	let fixture = Fixture::setup().await;
	registered_email_device::ActiveModel {
		id: NotSet,
		name: Set("blocked kindle".to_string()),
		email: Set("al@kindle.com".to_string()),
		forbidden: Set(true),
	}
	.insert(fixture.conn.as_ref())
	.await
	.expect("forbidden device");

	let error = fixture
		.mailer()
		.check_recipient("al@kindle.com")
		.await
		.expect_err("refused");
	assert!(matches!(error, KindleError::Refused(_)), "{error:?}");

	// An address that is not on the list is accepted.
	fixture
		.mailer()
		.check_recipient("someone@kindle.com")
		.await
		.expect("allowed");
}

/// A delivery joins the emailer's own history — the Emailer screen shows every
/// send whichever lane made it — carrying the attachment name, its size, and
/// the book it came from.
#[tokio::test]
async fn a_delivery_is_added_to_the_emailer_history() {
	let fixture = Fixture::setup().await;
	let payload = payload();
	let meta = AttachmentMetaModel::new(
		payload.name.clone(),
		Some("book-1".to_string()),
		payload.content.len() as i32,
	);

	fixture.mailer().record("al@kindle.com", meta).await;

	let records = fixture.records().await;
	assert_eq!(records.len(), 1);
	assert_eq!(records[0].recipient_email, "al@kindle.com");
	assert_eq!(
		records[0].sent_by_user_id.as_deref(),
		Some(fixture.user.id.as_str())
	);
	let stored = records[0].attachment_meta.as_ref().expect("meta");
	let meta = AttachmentMetaModel::vec_from_data(stored).expect("decoded");
	assert_eq!(meta.len(), 1);
	assert_eq!(meta[0].filename, "book.azw3");
	assert_eq!(meta[0].media_id.as_deref(), Some("book-1"));
	assert_eq!(meta[0].size, 20);

	// The emailer is stamped as used, like every other send does.
	let emailer = emailer::Entity::find_by_id(fixture.emailer.id)
		.one(fixture.conn.as_ref())
		.await
		.expect("emailer")
		.expect("row");
	assert!(emailer.last_used_at.is_some());
}
