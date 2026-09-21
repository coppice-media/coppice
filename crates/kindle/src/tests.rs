//! The delivery lane end to end, with a real `lettre` transport standing in
//! for the relay.
//!
//! The mailer under test is not a hand-rolled struct that remembers a
//! `Vec<u8>`: it builds the message with the *production* builder
//! (`EmailerClient::build_message`) and hands it to `lettre`'s
//! [`StubTransport`], so every assertion below is made against the MIME
//! document that would have gone on the wire — which is the only place where
//! "the EPUB went out unchanged" can actually be observed.

use std::{
	path::{Path, PathBuf},
	sync::Mutex,
};

use ::tests::{db::test_database, fake_data};
use email::{AttachmentPayload, EmailerClient, EmailerClientConfig};
use lettre::{transport::stub::StubTransport, Transport};
use models::entity::{device, kindle_delivery, media, user::AuthUser};
use models::shared::enums::DeviceKind;
use pretty_assertions::assert_eq;
use sea_orm::{prelude::*, ActiveModelTrait, DatabaseConnection, Set};
use std::sync::Arc;
use stump_devices::{DeviceService, KINDLE_EMAIL_PROTOCOL};
use tempfile::TempDir;

use crate::{
	convert::{Conversion, KindleConverter},
	error::KindleError,
	file::{prepare_file, FormatPolicy, KINDLE_FORMAT},
	send::{deliveries, subject, Delivery, KindleMailer, KindleSend},
	KindleResult,
};

fn fixture_epub() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("../../core/integration-tests/data/book.epub")
}

/// A mailer that formats the message exactly as the SMTP path would and sends
/// it through `lettre`'s stub transport.
struct StubMailer {
	client: EmailerClient,
	transport: Mutex<StubTransport>,
	forbidden: Vec<String>,
	refuse: bool,
}

impl StubMailer {
	fn new() -> Self {
		Self {
			client: EmailerClient::new(EmailerClientConfig {
				sender_email: "stump@example.test".to_string(),
				sender_display_name: "Stump".to_string(),
				username: "stump".to_string(),
				password: Some("unused-by-the-stub".to_string()),
				host: "smtp.example.test".to_string(),
				port: 587,
				tls_enabled: true,
				max_attachment_size_bytes: None,
				max_num_attachments: None,
			}),
			transport: Mutex::new(StubTransport::new_ok()),
			forbidden: Vec::new(),
			refuse: false,
		}
	}

	fn refusing() -> Self {
		Self {
			transport: Mutex::new(StubTransport::new_error()),
			refuse: true,
			..Self::new()
		}
	}

	fn forbidding(recipient: &str) -> Self {
		Self {
			forbidden: vec![recipient.to_string()],
			..Self::new()
		}
	}

	/// The messages the transport accepted, as the formatted MIME documents
	/// that would have been transmitted.
	fn sent(&self) -> Vec<String> {
		self.transport
			.lock()
			.expect("transport")
			.messages()
			.into_iter()
			.map(|(_, formatted)| formatted)
			.collect()
	}

	fn envelopes(&self) -> Vec<lettre::address::Envelope> {
		self.transport
			.lock()
			.expect("transport")
			.messages()
			.into_iter()
			.map(|(envelope, _)| envelope)
			.collect()
	}
}

impl KindleMailer for StubMailer {
	async fn check_recipient(&self, recipient: &str) -> KindleResult<()> {
		if self.forbidden.iter().any(|entry| entry == recipient) {
			return Err(KindleError::Refused(format!(
				"{recipient} is a forbidden recipient"
			)));
		}
		Ok(())
	}

	async fn send(
		&self,
		recipient: &str,
		subject: &str,
		payload: AttachmentPayload,
	) -> KindleResult<()> {
		let message = self
			.client
			.build_message(
				subject,
				recipient,
				"You have a new attachment from Coppice!".to_string(),
				vec![payload],
			)
			.map_err(|error| KindleError::Refused(error.to_string()))?;
		self.transport
			.lock()
			.expect("transport")
			.send(&message)
			.map_err(|error| {
				KindleError::Refused(format!("Failed to mail the book: {error}"))
			})?;
		// `StubTransport::new_error` reports failure through the send result,
		// which the mapping above already turned into a refusal; this guard
		// only exists for the paranoid case where lettre stops doing that.
		if self.refuse {
			return Err(KindleError::Refused(
				"Failed to mail the book: the relay refused the message".to_string(),
			));
		}
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
	user: AuthUser,
	book: media::Model,
	device: device::Model,
	work_dir: TempDir,
}

impl Fixture {
	/// A user with one book on disk (the repository's fixture EPUB) and one
	/// device carrying a Kindle address.
	async fn setup() -> Self {
		let conn = Arc::new(test_database().await);
		let row = fake_data::User::new("al").insert(conn.as_ref()).await;
		let user = AuthUser {
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
		let book = Self::with_path(
			conn.as_ref(),
			book,
			fixture_epub().to_string_lossy().to_string(),
		)
		.await;

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

	fn lane<'a>(
		&'a self,
		mailer: &'a StubMailer,
		converter: &'a FakeBoko,
		max_attachment_size_bytes: Option<i32>,
	) -> KindleSend<'a, StubMailer, FakeBoko> {
		KindleSend {
			conn: self.conn.as_ref(),
			devices: &self.devices,
			mailer,
			converter,
			work_dir: self.work_dir.path(),
			max_attachment_size_bytes,
		}
	}

	async fn send(
		&self,
		mailer: &StubMailer,
		converter: &FakeBoko,
	) -> KindleResult<Delivery> {
		self.lane(mailer, converter, None)
			.send_to_kindle(&self.user, &self.device.id, &self.book.id)
			.await
	}

	async fn rows(&self) -> Vec<kindle_delivery::Model> {
		kindle_delivery::Entity::find()
			.all(self.conn.as_ref())
			.await
			.expect("deliveries")
	}

	async fn stored_device(&self) -> device::Model {
		self.devices
			.get(&self.user, &self.device.id)
			.await
			.expect("device")
	}
}

/// The base64 body of an attachment, as `lettre` encodes it: the assertion
/// below needs the *transmitted* form of the bytes, not the struct they came
/// from.
fn base64_of(content: &[u8]) -> String {
	use lettre::message::header::ContentType;
	// Round-tripping through a single-part message is how the encoder is
	// reached without depending on a base64 crate here.
	let part = lettre::message::Attachment::new("probe.bin".to_string()).body(
		content.to_vec(),
		ContentType::parse("application/octet-stream").unwrap(),
	);
	let formatted = String::from_utf8(part.formatted()).expect("utf8");
	formatted
		.split("\r\n\r\n")
		.nth(1)
		.expect("a body")
		.trim_end()
		.to_string()
}

/// Amazon accepts an EPUB and converts it itself, so the operator without
/// `boko` gets a successful delivery — and the attachment is the library file
/// byte for byte, which is what the transport's own MIME document proves.
#[tokio::test]
async fn mails_the_epub_unchanged_when_amazon_accepts_it() {
	let fixture = Fixture::setup().await;
	let mailer = StubMailer::new();

	let delivery = fixture
		.send(&mailer, &FakeBoko::Missing)
		.await
		.expect("delivered");

	let source = std::fs::read(fixture_epub()).expect("fixture");
	assert_eq!(delivery.row.format, "epub");
	assert_eq!(delivery.row.converted, false);
	assert_eq!(delivery.row.bytes, source.len() as i64);
	assert_eq!(
		delivery.row.note.as_deref(),
		Some("external tool `boko` is missing")
	);

	let sent = mailer.sent();
	let message = sent.first().expect("one message");
	assert!(
		message.contains("Content-Disposition: attachment; filename=\"book.epub\""),
		"{message}"
	);
	// The bytes on the wire are the library file, unchanged.
	assert!(
		message.contains(&base64_of(&source)),
		"the attachment is not the fixture EPUB"
	);
	assert_eq!(
		mailer.envelopes()[0]
			.to()
			.iter()
			.map(ToString::to_string)
			.collect::<Vec<_>>(),
		vec!["al@kindle.com".to_string()]
	);
}

/// With boko installed the Kindle gets KF8: the attachment is the converted
/// AZW3, named after the book and typed as a Kindle file.
#[tokio::test]
async fn converts_to_azw3_when_amazon_would_have_to() {
	let fixture = Fixture::setup().await;
	let mailer = StubMailer::new();

	let delivery = fixture
		.send(&mailer, &FakeBoko::Installed)
		.await
		.expect("delivered");

	assert_eq!(delivery.row.format, "azw3");
	assert_eq!(delivery.row.converted, true);
	assert_eq!(delivery.row.note, None);
	assert_eq!(delivery.row.bytes, b"BOOKMOBI kf8 payload".len() as i64);
	assert_eq!(delivery.device_name, "Paperwhite");
	assert_eq!(delivery.row.recipient, "al@kindle.com");

	let sent = mailer.sent();
	let message = sent.first().expect("one message");
	assert!(
		message.contains("Content-Disposition: attachment; filename=\"book.azw3\""),
		"{message}"
	);
	assert!(
		message.contains("application/vnd.amazon.mobi8-ebook"),
		"the attachment is not typed as a Kindle file: {message}"
	);
	// The EPUB must not be the thing that went out.
	let source = std::fs::read(fixture_epub()).expect("fixture");
	assert!(!message.contains(&base64_of(&source)));
}

/// The delivery row is the history: one row per send, carrying what left and
/// where it went, plus the device's own sync summary.
#[tokio::test]
async fn records_the_delivery_and_the_device_sync_summary() {
	let fixture = Fixture::setup().await;
	let mailer = StubMailer::new();

	let delivery = fixture
		.send(&mailer, &FakeBoko::Installed)
		.await
		.expect("delivered");

	let rows = fixture.rows().await;
	assert_eq!(rows.len(), 1);
	assert_eq!(rows[0].id, delivery.row.id);
	assert_eq!(rows[0].media_id, fixture.book.id);
	assert_eq!(rows[0].device_id, Some(fixture.device.id.clone()));
	assert_eq!(rows[0].recipient, "al@kindle.com");
	assert_eq!(rows[0].format, "azw3");
	assert_eq!(rows[0].error, None);

	let device = fixture.stored_device().await;
	assert!(device.last_sync_at.is_some());
	assert_eq!(
		device.last_sync_summary,
		Some(serde_json::json!({
			"protocol": KINDLE_EMAIL_PROTOCOL,
			"bytes": rows[0].bytes,
			"format": "azw3",
		}))
	);
	// Nothing authenticated: a delivery is not a sighting.
	assert_eq!(device.last_seen_at, None);
}

/// A refused delivery is still a delivery *attempt*: the row keeps the
/// relay's message, and the device's sync state must not claim a book that
/// never left.
#[tokio::test]
async fn records_the_failure_when_the_relay_refuses() {
	let fixture = Fixture::setup().await;
	let mailer = StubMailer::refusing();

	let error = fixture
		.send(&mailer, &FakeBoko::Installed)
		.await
		.expect_err("refused");
	assert!(
		error.to_string().starts_with("Failed to mail the book"),
		"{error}"
	);

	let rows = fixture.rows().await;
	assert_eq!(rows.len(), 1);
	assert!(rows[0]
		.error
		.as_deref()
		.expect("an error")
		.starts_with("Failed to mail the book"));

	let device = fixture.stored_device().await;
	assert_eq!(device.last_sync_at, None);
	assert_eq!(device.last_sync_summary, None);
}

/// Amazon discards anything over 50 MB without telling the sender, so the cap
/// is enforced here — before an SMTP connection is opened, and before the
/// operator's own emailer limit, which they can raise.
#[tokio::test]
async fn refuses_a_book_over_amazons_fifty_megabyte_limit() {
	let fixture = Fixture::setup().await;
	let big = fixture.work_dir.path().join("huge.azw3");
	// One byte over the ceiling: the refusal is about the limit, not the size.
	let mut content = b"BOOKMOBI".to_vec();
	content.resize(crate::AMAZON_MAX_ATTACHMENT_BYTES as usize + 1, 0);
	std::fs::write(&big, &content).expect("write");
	Fixture::with_path(
		fixture.conn.as_ref(),
		fixture.book.clone(),
		big.to_string_lossy().to_string(),
	)
	.await;

	let mailer = StubMailer::new();
	let error = fixture
		.send(&mailer, &FakeBoko::Missing)
		.await
		.expect_err("refused");
	assert!(
		error.to_string().contains("over Amazon's 52428800 byte"),
		"{error}"
	);

	assert!(mailer.sent().is_empty(), "nothing may be transmitted");
	assert!(fixture.rows().await.is_empty(), "nothing may be recorded");
}

/// The operator's own emailer cap still applies below Amazon's.
#[tokio::test]
async fn refuses_an_attachment_over_the_emailer_limit() {
	let fixture = Fixture::setup().await;
	let mailer = StubMailer::new();

	let error = fixture
		.lane(&mailer, &FakeBoko::Missing, Some(1_024))
		.send_to_kindle(&fixture.user, &fixture.device.id, &fixture.book.id)
		.await
		.expect_err("refused");
	assert!(
		error
			.to_string()
			.contains("over the emailer's 1024 byte limit"),
		"{error}"
	);
	assert!(mailer.sent().is_empty());
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

	let mailer = StubMailer::new();
	let error = fixture
		.send(&mailer, &FakeBoko::Missing)
		.await
		.expect_err("refused");
	assert_eq!(
		error.to_string(),
		"Paperwhite has no Kindle address; set one before sending to it"
	);
	assert!(fixture.rows().await.is_empty());
}

/// A forbidden recipient is refused before the book is read or converted: the
/// lane asks its mailer first, which is where the server's address policy
/// lives.
#[tokio::test]
async fn refuses_a_forbidden_recipient_before_converting() {
	let fixture = Fixture::setup().await;
	let mailer = StubMailer::forbidding("al@kindle.com");

	let error = fixture
		.send(&mailer, &FakeBoko::Installed)
		.await
		.expect_err("refused");
	assert_eq!(error.to_string(), "al@kindle.com is a forbidden recipient");
	// Nothing was converted into the working directory.
	assert_eq!(
		std::fs::read_dir(fixture.work_dir.path())
			.expect("work dir")
			.count(),
		0
	);
}

/// A comic is refused locally: a Kindle cannot open a CBZ and Amazon would
/// silently drop it, which would look like a successful send.
#[tokio::test]
async fn refuses_a_format_amazon_does_not_accept() {
	let fixture = Fixture::setup().await;
	let comic = fixture.work_dir.path().join("book.cbz");
	std::fs::write(&comic, b"PK\x03\x04 not a book for a kindle").expect("write");
	Fixture::with_path(
		fixture.conn.as_ref(),
		fixture.book.clone(),
		comic.to_string_lossy().to_string(),
	)
	.await;

	let mailer = StubMailer::new();
	let error = fixture
		.send(&mailer, &FakeBoko::Missing)
		.await
		.expect_err("refused");
	assert!(
		error.to_string().contains("a Kindle cannot read .cbz"),
		"{error}"
	);
	assert!(mailer.sent().is_empty());
}

/// The USB lane has no gateway behind it: an EPUB the server cannot convert is
/// refused rather than downloaded onto a device that will not open it.
#[tokio::test]
async fn the_usb_lane_requires_a_kindle_format() {
	let work_dir = tempfile::tempdir().expect("work dir");
	let error = prepare_file(
		&fixture_epub(),
		FormatPolicy::RequireKindle,
		&FakeBoko::Missing,
		work_dir.path(),
	)
	.await
	.expect_err("refused");
	assert!(
		error
			.to_string()
			.starts_with("a Kindle opens no .epub over USB"),
		"{error}"
	);

	let converted = prepare_file(
		&fixture_epub(),
		FormatPolicy::RequireKindle,
		&FakeBoko::Installed,
		work_dir.path(),
	)
	.await
	.expect("converted");
	assert_eq!(converted.filename, "book.azw3");
	assert_eq!(converted.format, "azw3");
	assert!(converted.converted);
	assert_eq!(converted.content, b"BOOKMOBI kf8 payload");
}

/// A format a Kindle already reads is handed over untouched by both lanes.
#[tokio::test]
async fn the_usb_lane_passes_a_kindle_format_through() {
	let work_dir = tempfile::tempdir().expect("work dir");
	let source = work_dir.path().join("sideload.azw3");
	std::fs::write(&source, b"BOOKMOBI already a kindle file").expect("write");

	let file = prepare_file(
		&source,
		FormatPolicy::RequireKindle,
		&FakeBoko::Missing,
		work_dir.path(),
	)
	.await
	.expect("passed through");
	assert_eq!(file.filename, "sideload.azw3");
	assert!(!file.converted);
	assert_eq!(file.content_type, "application/vnd.amazon.mobi8-ebook");
	assert_eq!(file.note.as_deref(), Some(".azw3 needs no conversion"));
}

/// The history is per device, newest first, and never leaks another device's
/// deliveries.
#[tokio::test]
async fn deliveries_are_listed_newest_first_per_device() {
	let fixture = Fixture::setup().await;
	let mailer = StubMailer::new();
	fixture
		.send(&mailer, &FakeBoko::Missing)
		.await
		.expect("first");
	fixture
		.send(&mailer, &FakeBoko::Installed)
		.await
		.expect("second");

	let listed = deliveries(
		fixture.conn.as_ref(),
		std::slice::from_ref(&fixture.device.id),
		None,
		10,
	)
	.await
	.expect("listed");
	assert_eq!(listed.len(), 2);
	assert!(listed[0].sent_at >= listed[1].sent_at);

	let for_book = deliveries(
		fixture.conn.as_ref(),
		std::slice::from_ref(&fixture.device.id),
		Some(&fixture.book.id),
		1,
	)
	.await
	.expect("listed");
	assert_eq!(for_book.len(), 1);

	let other_device = deliveries(
		fixture.conn.as_ref(),
		&["not-a-device".to_string()],
		None,
		10,
	)
	.await
	.expect("listed");
	assert!(other_device.is_empty());
}

/// `Convert` is Amazon's magic subject: Coppice never asks the service to
/// reflow a PDF on the operator's behalf, so a book called that is qualified.
#[test]
fn the_subject_never_reads_as_the_convert_keyword() {
	assert_eq!(subject("Dune"), "Dune");
	assert_eq!(subject(" convert "), " convert  (sent by Coppice)");
}
