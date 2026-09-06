//! Send-to-Kindle: mail one book to the Kindle address of a registered device.
//!
//! The lane itself lives in [`stump_kindle`]: format policy, the boko
//! conversion, Amazon's 50 MB ceiling and the `kindle_deliveries` history are
//! shared with the USB download route
//! (`POST /api/v2/media/{id}/kindle-file`), so neither surface can drift from
//! the other. What is *here* is only what the lane deliberately does not own:
//!
//! - **One emailer, one address book.** [`EmailChannelMailer`] resolves the
//!   primary emailer, sends through [`stump_notify::EmailChannel`] — the same
//!   SMTP client and error mapping as `sendAttachmentEmail` — honours the same
//!   forbidden-recipient list, and writes the same `emailer_send_records`
//!   history, so the Emailer screen shows every send whichever lane made it.
//! - **A working directory.** The converted book is a temporary under the
//!   server's own cache directory, removed when the resolver returns; the
//!   library file is never touched.
//!
//! Prose and the operator-facing rules live in
//! `/docs/content/docs/developer/devices.mdx` (`## Send to Kindle`).

use async_graphql::{Context, Object, Result, SimpleObject, ID};
use chrono::{DateTime, FixedOffset};
use email::AttachmentPayload;
use models::{
	entity::{
		emailer,
		emailer_send_record::{self, AttachmentMetaModel},
		user::AuthUser,
	},
	shared::enums::UserPermission,
};
use sea_orm::{DatabaseConnection, NotSet, Set};
use stump_kindle::{
	BokoConverter, Delivery, KindleDeliveryRow, KindleError, KindleMailer, KindleResult,
	KindleSend,
};
use stump_notify::EmailChannel;

use crate::{data::CoreContext, guard::PermissionGuard, mutation::emailer::sender};

/// What one send-to-Kindle delivery did: the stored `kindle_deliveries` row,
/// plus the device's name so the caller can say "sent Dune.azw3 (412 KB) to
/// Paperwhite" without a second query.
#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct KindleDelivery {
	pub id: ID,
	pub media_id: String,
	pub device_id: String,
	pub device_name: String,
	/// The address the book was mailed to, as registered on the device when
	/// the delivery happened.
	pub recipient: String,
	/// Extension of the attachment that went out: `azw3` when it was
	/// converted for the Kindle, otherwise the book's own format.
	pub format: String,
	/// Size of that attachment.
	pub bytes: u64,
	/// Whether the book was converted before sending.
	pub converted: bool,
	/// Why the book was sent unconverted — usually that the operator has no
	/// `boko` installed. `null` when it was converted, and never an error:
	/// Amazon converts an EPUB itself.
	pub note: Option<String>,
	/// Why the delivery failed, when it did. A failed send is recorded, which
	/// is what makes this a delivery history rather than a success list.
	pub error: Option<String>,
	pub sent_at: DateTime<FixedOffset>,
}

impl KindleDelivery {
	pub fn new(row: KindleDeliveryRow, device_name: String) -> Self {
		Self {
			id: ID::from(row.id),
			media_id: row.media_id,
			device_id: row.device_id,
			device_name,
			recipient: row.recipient,
			format: row.format,
			bytes: row.bytes.max(0) as u64,
			converted: row.converted,
			note: row.note,
			error: row.error,
			sent_at: row.sent_at,
		}
	}
}

impl From<Delivery> for KindleDelivery {
	fn from(delivery: Delivery) -> Self {
		Self::new(delivery.row, delivery.device_name)
	}
}

#[derive(Default)]
pub struct KindleMutation;

#[Object]
impl KindleMutation {
	/// Mails `media_id` to the Kindle address registered on `device_id`,
	/// converting an EPUB to AZW3 first when the operator has `boko`
	/// installed.
	///
	/// Requires the same `EMAIL_SEND` permission as `sendAttachmentEmail`,
	/// which is the same SMTP path this uses.
	#[graphql(guard = "PermissionGuard::one(UserPermission::EmailSend)")]
	async fn send_to_kindle(
		&self,
		ctx: &Context<'_>,
		media_id: ID,
		device_id: ID,
	) -> Result<KindleDelivery> {
		let core_ctx = ctx.data::<CoreContext>()?;
		let user = &ctx.data::<stump_auth::AuthContext>()?.user;
		let conn = core_ctx.conn.as_ref();
		let encryption_key = core_ctx.get_encryption_key().await?;

		let emailer = sender::get_emailer(conn).await?;
		let config =
			sender::build_emailer_client_config(encryption_key, emailer.clone())?;
		let max_attachment_size_bytes = emailer.max_attachment_size_bytes;
		let mailer = EmailChannelMailer {
			channel: EmailChannel::new(config),
			conn,
			user,
			emailer,
			media_id: media_id.to_string(),
		};

		// The converted book is a temporary: it lives in the server's own
		// cache directory (never the library, which is the operator's data)
		// and is removed when this resolver returns.
		let work_dir = tempfile::Builder::new()
			.prefix("send-to-kindle-")
			.tempdir_in(core_ctx.config.get_cache_dir())?;

		let devices = core_ctx.devices();
		let delivery = KindleSend {
			conn,
			devices: &devices,
			mailer: &mailer,
			converter: &BokoConverter,
			work_dir: work_dir.path(),
			max_attachment_size_bytes,
		}
		.send_to_kindle(user, device_id.as_str(), media_id.as_str())
		.await?;

		Ok(delivery.into())
	}
}

/// The server's one SMTP path, plus the emailer history and recipient policy
/// that go with it.
struct EmailChannelMailer<'a> {
	channel: EmailChannel,
	conn: &'a DatabaseConnection,
	user: &'a AuthUser,
	emailer: emailer::Model,
	media_id: String,
}

impl KindleMailer for EmailChannelMailer<'_> {
	async fn check_recipient(&self, recipient: &str) -> KindleResult<()> {
		sender::check_forbidden_recipients(
			self.user,
			self.conn,
			std::slice::from_ref(&recipient.to_string()),
		)
		.await
		.map_err(|error| KindleError::Refused(error.message))
	}

	async fn send(
		&self,
		recipient: &str,
		subject: &str,
		payload: AttachmentPayload,
	) -> KindleResult<()> {
		let meta = AttachmentMetaModel::new(
			payload.name.clone(),
			Some(self.media_id.clone()),
			payload.content.len() as i32,
		);

		self.channel
			.deliver(recipient, subject, vec![payload])
			.await
			.map_err(|error| {
				KindleError::Refused(format!("Failed to mail the book: {error}"))
			})?;

		self.record(recipient, meta).await;
		Ok(())
	}
}

impl EmailChannelMailer<'_> {
	/// Adds the delivery to the emailer's own history, so the Emailer screen
	/// shows every send whichever lane made it, and stamps the emailer's
	/// `last_used_at` exactly as `sendAttachmentEmail` does.
	///
	/// Only a delivery that actually went out is recorded here; a failed send
	/// is recorded on the `kindle_deliveries` row by the lane itself. A
	/// history that cannot be written is logged and swallowed: the book has
	/// already left, and reporting failure would tell the operator to send it
	/// again.
	async fn record(&self, recipient: &str, meta: AttachmentMetaModel) {
		let attachment_meta = match AttachmentMetaModel::into_data(&vec![meta]) {
			Ok(data) => Some(data),
			Err(error) => {
				tracing::error!(?error, "Failed to serialise the attachment meta");
				None
			},
		};
		let record = emailer_send_record::ActiveModel {
			id: NotSet,
			emailer_id: Set(self.emailer.id),
			recipient_email: Set(recipient.to_string()),
			attachment_meta: Set(attachment_meta),
			sent_at: Set(chrono::Utc::now().into()),
			sent_by_user_id: Set(Some(self.user.id.clone())),
		};
		match sender::update_send_records(self.emailer.clone(), self.conn, vec![record])
			.await
		{
			Ok((_, errors)) if !errors.is_empty() => {
				tracing::error!(?errors, "Kindle delivery sent but not fully recorded");
			},
			Err(error) => {
				tracing::error!(?error, "Kindle delivery sent but not recorded");
			},
			Ok(_) => (),
		}
	}
}

#[cfg(test)]
mod tests;
