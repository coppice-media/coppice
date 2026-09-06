//! The e-mail lane: one book, one device, one `kindle_deliveries` row.

use std::{future::Future, path::Path};

use chrono::Utc;
use email::AttachmentPayload;
use models::entity::{kindle_delivery, media, user::AuthUser};
use sea_orm::{prelude::*, DatabaseConnection, QueryOrder, QuerySelect, Set};
use stump_devices::{DeviceService, KindleSendSummary};

use crate::{
	convert::KindleConverter,
	error::{KindleError, KindleResult},
	file::{prepare_file, FormatPolicy, KindleFile, AMAZON_MAX_ATTACHMENT_BYTES},
};

/// Mails one attachment.
///
/// The lane owns no transport on purpose: the server already has exactly one
/// SMTP client, one forbidden-recipient list and one send history
/// (`stump_notify::EmailChannel` over the primary emailer), and a second one
/// would drift from it. The implementor also gets to refuse a recipient
/// *before* any conversion work through [`KindleMailer::check_recipient`].
pub trait KindleMailer {
	/// Recipient policy, checked before the book is read or converted. The
	/// default accepts every address.
	fn check_recipient(
		&self,
		recipient: &str,
	) -> impl Future<Output = KindleResult<()>> + Send {
		let _ = recipient;
		std::future::ready(Ok(()))
	}

	fn send(
		&self,
		recipient: &str,
		subject: &str,
		payload: AttachmentPayload,
	) -> impl Future<Output = KindleResult<()>> + Send;
}

/// One completed delivery: the stored row plus the device's name, which the
/// caller would otherwise have to look up again to say "sent to Paperwhite".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
	pub row: kindle_delivery::Model,
	pub device_name: String,
}

/// The send-to-Kindle service: everything one delivery needs that is not the
/// book or the device.
pub struct KindleSend<'a, M, C> {
	pub conn: &'a DatabaseConnection,
	pub devices: &'a DeviceService,
	pub mailer: &'a M,
	pub converter: &'a C,
	/// Where a conversion may write. The caller owns it and removes it; the
	/// library file is never touched.
	pub work_dir: &'a Path,
	/// The emailer's own attachment cap, when it has one. Amazon's
	/// [`AMAZON_MAX_ATTACHMENT_BYTES`] applies regardless.
	pub max_attachment_size_bytes: Option<i32>,
}

impl<M, C> KindleSend<'_, M, C>
where
	M: KindleMailer,
	C: KindleConverter,
{
	/// Mails `media_id` to the Kindle address registered on `device_id` and
	/// records the delivery.
	///
	/// A refusal by the transport is recorded too, with its message in
	/// `error`: a delivery log that only kept successes could not answer the
	/// one question an operator asks it.
	pub async fn send_to_kindle(
		&self,
		user: &AuthUser,
		device_id: &str,
		media_id: &str,
	) -> KindleResult<Delivery> {
		let device = self.devices.get(user, device_id).await?;
		if device.is_revoked() {
			return Err(KindleError::Revoked(device.name));
		}
		let recipient = device
			.kindle_email
			.clone()
			.ok_or_else(|| KindleError::NoAddress(device.name.clone()))?;
		// A new lane must not reach an address the operator marked forbidden.
		self.mailer.check_recipient(&recipient).await?;

		let book = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(media_id))
			.one(self.conn)
			.await?
			.ok_or(KindleError::BookNotFound)?;

		let file = prepare_file(
			Path::new(&book.path),
			FormatPolicy::PreferKindle,
			self.converter,
			self.work_dir,
		)
		.await?;
		self.check_size(&file)?;

		let bytes = file.bytes();
		let format = file.format.clone();
		let note = file.note.clone();
		let converted = file.converted;
		let payload = AttachmentPayload {
			name: file.filename,
			content: file.content,
			content_type: file.content_type.parse().map_err(|error| {
				tracing::warn!(?error, "Failed to parse the attachment content type");
				KindleError::Attachment("the book has no usable content type".to_string())
			})?,
		};

		let outcome = self
			.mailer
			.send(&recipient, &subject(&book.name), payload)
			.await;
		let error = outcome.as_ref().err().map(ToString::to_string);
		// The row is written either way; only a delivery that went out moves
		// the device's sync state.
		let row = self
			.record(kindle_delivery::ActiveModel {
				id: Set(Uuid::new_v4().to_string()),
				media_id: Set(book.id.clone()),
				device_id: Set(device.id.clone()),
				recipient: Set(recipient),
				format: Set(format.clone()),
				bytes: Set(bytes as i64),
				converted: Set(converted),
				note: Set(note),
				error: Set(error),
				sent_at: Set(Utc::now().into()),
			})
			.await?;
		outcome?;

		self.devices
			.record_delivery(&device.id, &KindleSendSummary::new(bytes, format))
			.await?;

		Ok(Delivery {
			row,
			device_name: device.name,
		})
	}

	/// Both caps, in the order that produces the most useful message: the
	/// operator can raise their own emailer limit, but never Amazon's.
	fn check_size(&self, file: &KindleFile) -> KindleResult<()> {
		let bytes = file.bytes();
		if bytes > AMAZON_MAX_ATTACHMENT_BYTES {
			return Err(KindleError::TooLarge(format!(
				"{} is {bytes} bytes, over Amazon's {AMAZON_MAX_ATTACHMENT_BYTES} \
				 byte Send to Kindle limit; copy it over USB instead",
				file.filename
			)));
		}
		if let Some(max) = self.max_attachment_size_bytes {
			if bytes as i64 > i64::from(max) {
				return Err(KindleError::TooLarge(format!(
					"{} is {bytes} bytes, over the emailer's {max} byte limit",
					file.filename
				)));
			}
		}
		Ok(())
	}

	async fn record(
		&self,
		delivery: kindle_delivery::ActiveModel,
	) -> KindleResult<kindle_delivery::Model> {
		Ok(delivery.insert(self.conn).await?)
	}
}

/// The deliveries of `device_ids`, newest first, optionally for one book.
///
/// The caller scopes the device ids (a delivery is only visible to someone who
/// can see the device it went to), which is why this takes ids rather than a
/// user.
pub async fn deliveries(
	conn: &DatabaseConnection,
	device_ids: &[String],
	media_id: Option<&str>,
	limit: u64,
) -> KindleResult<Vec<kindle_delivery::Model>> {
	if device_ids.is_empty() {
		return Ok(Vec::new());
	}
	let mut query = kindle_delivery::Entity::find()
		.filter(kindle_delivery::Column::DeviceId.is_in(device_ids.to_vec()));
	if let Some(media_id) = media_id {
		query = query.filter(kindle_delivery::Column::MediaId.eq(media_id));
	}
	Ok(query
		.order_by_desc(kindle_delivery::Column::SentAt)
		.limit(limit)
		.all(conn)
		.await?)
}

/// The subject Amazon sees.
///
/// `Convert` is a magic word for the service — it makes Amazon reflow a PDF
/// into Kindle format — and Stump never asks for that on the operator's
/// behalf, so a book that happens to be called "Convert" is qualified.
/// Everything else is ignored by Amazon and is there for the operator's own
/// mail log.
pub fn subject(book_name: &str) -> String {
	if book_name.trim().eq_ignore_ascii_case("convert") {
		format!("{book_name} (sent by Stump)")
	} else {
		book_name.to_string()
	}
}
