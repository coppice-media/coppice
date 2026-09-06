use std::{
	path::Path,
	sync::{Arc, Mutex},
};

use ::tests::{db::test_database, fake_data};
use models::entity::{device, emailer, emailer_send_record, media};
use models::shared::enums::DeviceKind;
use pretty_assertions::assert_eq;
use sea_orm::{
	ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait, NotSet,
};
use serde_json::json;
use stump_devices::{DeviceService, KINDLE_EMAIL_PROTOCOL};
use tempfile::TempDir;

use super::*;
use crate::tests::common::get_test_epub_path;

/// A mailer that records what it was handed instead of opening an SMTP
/// connection.
#[derive(Default)]
struct FakeMailer {
	sent: Mutex<Vec<(String, String, AttachmentPayload)>>,
	refuse: bool,
}

impl KindleMailer for FakeMailer {
	async fn send(
		&self,
		recipient: &str,
		subject: &str,
		payload: AttachmentPayload,
	) -> Result<()> {
		if self.refuse {
			return Err("the relay refused the message".into());
		}
		self.sent.lock().expect("sent").push((
			recipient.to_string(),
			subject.to_string(),
			payload,
		));
		Ok(())
	}
}

/// Either an operator with `boko` on PATH or the far more common one without.
enum FakeBoko {
	Installed,
	Missing,
}

impl KindleConverter for FakeBoko {
	async fn to_azw3(&self, source: &Path, out_dir: &Path) -> Conversion {
		match self {
			// The wording `stump_tools` uses when the binary cannot be found.
			Self::Missing => {
				Conversion::Skipped("external tool `boko` is missing".to_string())
			},
			Self::Installed => {
				let mut target =
					out_dir.join(source.file_stem().expect("the source has a stem"));
				target.set_extension(KINDLE_FORMAT);
				// Stands in for boko's output: what matters downstream is that
				// the file exists and is big enough to type.
				std::fs::write(&target, b"BOOKMOBI kf8 payload").expect("write azw3");
				Conversion::Converted(target)
			},
		}
	}
}

struct Fixture {
	conn: Arc<DatabaseConnection>,
	devices: DeviceService,
	user: models::entity::user::AuthUser,
	emailer: emailer::Model,
	book: media::Model,
	device: device::Model,
	work_dir: TempDir,
}

impl Fixture {
	/// A user with one book on disk (the repository's fixture EPUB), a primary
	/// emailer, and one device carrying a Kindle address.
	async fn setup() -> Self {
		let conn = Arc::new(test_database().await);
		let row = fake_data::User::new("al").insert(conn.as_ref()).await;
		let user = models::entity::user::AuthUser {
			id: row.id,
			username: row.username,
			is_server_owner: true,
			..Default::default()
		};

		let library = fake_data::Library::default().insert(conn.as_ref()).await;
		let series = fake_data::Series {
			library_id: Some(library.id),
			..Default::default()
		}
		.insert(conn.as_ref())
		.await;
		let book = fake_data::Media {
			series_id: series.id,
			..Default::default()
		}
		.insert(conn.as_ref())
		.await;
		let book = Self::with_path(conn.as_ref(), book, get_test_epub_path()).await;

		let emailer = emailer::ActiveModel {
			id: NotSet,
			name: Set("primary".to_string()),
			is_primary: Set(true),
			sender_email: Set("stump@example.test".to_string()),
			sender_display_name: Set("Stump".to_string()),
			username: Set("stump".to_string()),
			encrypted_password: Set("unused-by-the-fake-mailer".to_string()),
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

		let devices = DeviceService::new(conn.clone());
		let (device, _) = devices
			.create_device(&user, DeviceKind::Opds, Some("Paperwhite".to_string()))
			.await
			.expect("device");
		let device = devices
			.set_kindle_email(&user, &device.id, Some("al@kindle.com"))
			.await
			.expect("address");

		Self {
			conn,
			devices,
			user,
			emailer,
			book,
			device,
			work_dir: tempfile::tempdir().expect("work dir"),
		}
	}

	async fn with_path(
		conn: &DatabaseConnection,
		book: media::Model,
		path: String,
	) -> media::Model {
		let mut active: media::ActiveModel = book.into();
		active.path = Set(path);
		active.update(conn).await.expect("book path")
	}

	async fn send(
		&self,
		mailer: &FakeMailer,
		converter: &FakeBoko,
	) -> Result<KindleDelivery> {
		deliver(
			self.conn.as_ref(),
			&self.devices,
			&self.user,
			&self.book.id,
			&self.device.id,
			self.emailer.clone(),
			mailer,
			converter,
			self.work_dir.path(),
		)
		.await
	}

	async fn send_records(&self) -> Vec<emailer_send_record::Model> {
		emailer_send_record::Entity::find()
			.all(self.conn.as_ref())
			.await
			.expect("records")
	}

	async fn stored_device(&self) -> device::Model {
		self.devices
			.get(&self.user, &self.device.id)
			.await
			.expect("device")
	}
}

/// With boko installed the Kindle gets KF8: the attachment is the converted
/// AZW3, named after the book and typed as a Kindle file, and both histories
/// record that size and format.
#[tokio::test]
async fn converts_to_azw3_when_boko_is_installed() {
	let fixture = Fixture::setup().await;
	let mailer = FakeMailer::default();

	let delivery = fixture
		.send(&mailer, &FakeBoko::Installed)
		.await
		.expect("delivered");

	assert!(delivery.converted);
	assert_eq!(delivery.format, "azw3");
	assert_eq!(delivery.note, None);
	assert_eq!(delivery.recipient, "al@kindle.com");
	assert_eq!(delivery.device_name, "Paperwhite");

	let sent = mailer.sent.lock().expect("sent");
	let (recipient, subject, payload) = sent.first().expect("one message");
	assert_eq!(recipient, "al@kindle.com");
	assert_eq!(subject, &fixture.book.name);
	assert_eq!(payload.name, "book.azw3");
	assert_eq!(
		payload.content_type,
		EmailContentType::parse("application/vnd.amazon.mobi8-ebook").expect("mime")
	);
	assert_eq!(payload.content, b"BOOKMOBI kf8 payload");
	assert_eq!(delivery.bytes, payload.content.len() as u64);

	assert_eq!(
		fixture.stored_device().await.last_sync_summary,
		Some(json!({
			"protocol": KINDLE_EMAIL_PROTOCOL,
			"bytes": payload.content.len(),
			"format": "azw3",
		}))
	);
	let records = fixture.send_records().await;
	assert_eq!(records.len(), 1);
	assert_eq!(records[0].recipient_email, "al@kindle.com");
}

/// Without boko the EPUB itself goes out — Amazon converts it — and the
/// delivery says why, because that is information and not a failure.
#[tokio::test]
async fn mails_the_epub_when_boko_is_missing() {
	let fixture = Fixture::setup().await;
	let mailer = FakeMailer::default();

	let delivery = fixture
		.send(&mailer, &FakeBoko::Missing)
		.await
		.expect("delivered");

	assert!(!delivery.converted);
	assert_eq!(delivery.format, "epub");
	assert_eq!(
		delivery.note.as_deref(),
		Some("external tool `boko` is missing")
	);

	let sent = mailer.sent.lock().expect("sent");
	let (_, _, payload) = sent.first().expect("one message");
	assert_eq!(payload.name, "book.epub");
	assert_eq!(
		payload.content_type,
		EmailContentType::parse("application/epub+zip").expect("mime")
	);
	assert_eq!(
		payload.content,
		std::fs::read(get_test_epub_path()).expect("fixture")
	);

	assert_eq!(
		fixture.stored_device().await.last_sync_summary,
		Some(json!({
			"protocol": KINDLE_EMAIL_PROTOCOL,
			"bytes": payload.content.len(),
			"format": "epub",
		}))
	);
}

/// A device with no address is not a Kindle target, and the refusal names it
/// so the operator knows where to put one.
#[tokio::test]
async fn refuses_a_device_without_an_address() {
	let fixture = Fixture::setup().await;
	fixture
		.devices
		.set_kindle_email(&fixture.user, &fixture.device.id, None)
		.await
		.expect("cleared");
	let mailer = FakeMailer::default();

	let error = fixture
		.send(&mailer, &FakeBoko::Installed)
		.await
		.expect_err("refused");

	assert!(
		error.message.contains("Paperwhite has no Kindle address"),
		"{}",
		error.message
	);
	assert!(mailer.sent.lock().expect("sent").is_empty());
}

/// A comic is refused locally: a Kindle cannot open a CBZ and Amazon would
/// silently drop it, which would look like a successful send.
#[tokio::test]
async fn refuses_a_format_amazon_does_not_accept() {
	let fixture = Fixture::setup().await;
	let book = Fixture::with_path(
		fixture.conn.as_ref(),
		fixture.book.clone(),
		"/books/vol-1.cbz".to_string(),
	)
	.await;
	assert_eq!(book.id, fixture.book.id);
	let mailer = FakeMailer::default();

	let error = fixture
		.send(&mailer, &FakeBoko::Installed)
		.await
		.expect_err("refused");

	assert!(
		error.message.contains("a Kindle cannot read .cbz"),
		"{}",
		error.message
	);
	assert!(mailer.sent.lock().expect("sent").is_empty());
}

/// A refused delivery records nothing: neither the emailer history nor the
/// device's sync summary may claim a book that never left.
#[tokio::test]
async fn records_nothing_when_the_mail_is_refused() {
	let fixture = Fixture::setup().await;
	let mailer = FakeMailer {
		refuse: true,
		..Default::default()
	};

	let error = fixture
		.send(&mailer, &FakeBoko::Installed)
		.await
		.expect_err("refused");
	assert!(
		error.message.contains("the relay refused the message"),
		"{}",
		error.message
	);

	assert!(fixture.send_records().await.is_empty());
	let device = fixture.stored_device().await;
	assert_eq!(device.last_sync_summary, None);
	assert!(device.last_sync_at.is_none());
}

/// The emailer's cap is enforced before an SMTP connection is opened.
#[tokio::test]
async fn refuses_an_attachment_over_the_emailer_limit() {
	let fixture = Fixture::setup().await;
	let mut emailer = fixture.emailer.clone();
	emailer.max_attachment_size_bytes = Some(8);
	let mailer = FakeMailer::default();

	let error = deliver(
		fixture.conn.as_ref(),
		&fixture.devices,
		&fixture.user,
		&fixture.book.id,
		&fixture.device.id,
		emailer,
		&mailer,
		&FakeBoko::Installed,
		fixture.work_dir.path(),
	)
	.await
	.expect_err("refused");

	assert!(
		error.message.contains("over the emailer's 8 byte limit"),
		"{}",
		error.message
	);
	assert!(mailer.sent.lock().expect("sent").is_empty());
}

/// `Convert` is Amazon's magic subject: Stump never asks the service to
/// reflow a PDF on the operator's behalf, so a book called that is qualified.
#[test]
fn the_subject_never_reads_as_the_convert_keyword() {
	assert_eq!(subject("Dune"), "Dune");
	assert_eq!(subject("convert"), "convert (sent by Stump)");
	assert_eq!(subject(" Convert "), " Convert  (sent by Stump)");
}
