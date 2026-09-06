//! Send-to-Kindle: mail one book to the Kindle address of a registered device.
//!
//! Amazon's *Send to Kindle* service takes an e-mail from an approved sender
//! with the book attached and delivers it to the device that owns the
//! recipient address, so a Kindle needs no protocol, no credential, and no
//! sync session — only an address, kept on the device row as
//! `devices.kindle_email`.
//!
//! The rules this lane implements, and why:
//!
//! - **AZW3 when the operator has boko, the book itself otherwise.** Amazon
//!   accepts EPUB attachments and converts them on its side, so a server
//!   without a converter still delivers a readable book; that fallback is the
//!   normal case, not an error. AZW3 is preferred when
//!   [`stump_tools`](stump_tools::boko)' `boko-convert` is available because
//!   it *is* KF8: nothing depends on Amazon's conversion and the identical
//!   file sideloads over USB. boko is the operator's own GPL install, shelled
//!   out to, never linked.
//! - **Only formats Amazon takes.** See [`KINDLE_FORMATS`]. A CBZ is refused
//!   here rather than bounced by Amazon's gateway minutes later.
//! - **One emailer, one address book.** Delivery goes through the primary
//!   emailer and [`stump_notify::EmailChannel`], i.e. the same SMTP client and
//!   error mapping as `sendAttachmentEmail` and the notification channel, and
//!   it honours the same forbidden-recipient list.
//! - **A delivery is a sync, not a sighting.** The device did not
//!   authenticate, so only `last_sync_at`/`last_sync_summary` move; see
//!   [`stump_devices::DeviceService::record_delivery`].
//!
//! Prose and the operator-facing rules live in
//! `/docs/content/docs/developer/devices.mdx` (`## Send to Kindle`).

use std::path::{Path, PathBuf};

use async_graphql::{Context, Object, Result, SimpleObject, ID};
use email::{AttachmentPayload, EmailContentType};
use models::{
	entity::{
		emailer,
		emailer_send_record::{self, AttachmentMetaModel},
		media,
		user::AuthUser,
	},
	shared::enums::UserPermission,
};
use sea_orm::{prelude::*, DatabaseConnection, NotSet, Set};
use stump_devices::{DeviceService, KindleSendSummary};
use stump_media::{ContentType, FileParts, PathUtils};
use stump_notify::EmailChannel;
use stump_tools::{
	boko::{BokoConvert, KindleExport},
	NoopProgress, Tool,
};

use crate::{data::CoreContext, guard::PermissionGuard, mutation::emailer::sender};

/// The attachment formats Amazon's Send to Kindle service accepts (its own
/// list: DOC, DOCX, HTML, HTM, RTF, TXT, PDF, EPUB) plus the Kindle's native
/// AZW3/MOBI, which need no conversion at all.
///
/// Comic archives are absent on purpose: a Kindle cannot open a CBZ/CBR and
/// Amazon discards it, so mailing one would look like a successful send.
const KINDLE_FORMATS: &[&str] = &[
	"azw3", "doc", "docx", "epub", "htm", "html", "mobi", "pdf", "rtf", "txt",
];

/// The one source format worth converting: everything else in
/// [`KINDLE_FORMATS`] is either already a Kindle format or something boko
/// cannot read.
const CONVERTIBLE_FORMAT: &str = "epub";

/// The extension the conversion produces.
const KINDLE_FORMAT: &str = "azw3";

/// What one send-to-Kindle delivery did.
#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct KindleDelivery {
	pub device_id: String,
	pub device_name: String,
	/// The address the book was mailed to, as registered on the device.
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
		let mailer = EmailChannelMailer {
			channel: EmailChannel::new(config),
		};

		// The converted book is a temporary: it lives in the server's own
		// cache directory (never the library, which is the operator's data)
		// and is removed when this resolver returns.
		let work_dir = tempfile::Builder::new()
			.prefix("send-to-kindle-")
			.tempdir_in(core_ctx.config.get_cache_dir())?;

		deliver(
			conn,
			&core_ctx.devices(),
			user,
			media_id.as_str(),
			device_id.as_str(),
			emailer,
			&mailer,
			&BokoConverter,
			work_dir.path(),
		)
		.await
	}
}

/// Mails one attachment. The SMTP path in production, a recording fake in the
/// tests.
trait KindleMailer {
	async fn send(
		&self,
		recipient: &str,
		subject: &str,
		payload: AttachmentPayload,
	) -> Result<()>;
}

struct EmailChannelMailer {
	channel: EmailChannel,
}

impl KindleMailer for EmailChannelMailer {
	async fn send(
		&self,
		recipient: &str,
		subject: &str,
		payload: AttachmentPayload,
	) -> Result<()> {
		self.channel
			.deliver(recipient, subject, vec![payload])
			.await
			.map_err(|error| format!("Failed to mail the book: {error}").into())
	}
}

/// What the conversion step produced.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Conversion {
	/// The AZW3 boko wrote, inside the caller's working directory.
	Converted(PathBuf),
	/// Nothing was converted; the reason is reported on the delivery and the
	/// book is mailed as it is.
	Skipped(String),
}

/// EPUB to AZW3 conversion. Never fails the send: an unavailable or unhappy
/// converter yields [`Conversion::Skipped`], because Amazon accepts the EPUB.
trait KindleConverter {
	async fn to_azw3(&self, source: &Path, out_dir: &Path) -> Conversion;
}

struct BokoConverter;

impl KindleConverter for BokoConverter {
	async fn to_azw3(&self, source: &Path, out_dir: &Path) -> Conversion {
		let source = source.to_path_buf();
		let out_dir = out_dir.to_path_buf();
		// `Tool::plan` and `Tool::apply` locate and run a child process, so
		// the whole conversion is one blocking unit off the async runtime.
		match tokio::task::spawn_blocking(move || convert_with_boko(&source, &out_dir))
			.await
		{
			Ok(conversion) => conversion,
			Err(error) => Conversion::Skipped(format!(
				"the conversion task did not finish: {error}"
			)),
		}
	}
}

fn convert_with_boko(source: &Path, out_dir: &Path) -> Conversion {
	let input = match KindleExport::input(
		vec![source.to_path_buf()],
		Some(out_dir.to_path_buf()),
	) {
		Ok(input) => input,
		Err(error) => return Conversion::Skipped(error.to_string()),
	};

	let tool = BokoConvert;
	// The expected failure is `ExternalToolMissing`: most servers have no
	// boko, which is exactly what the EPUB fallback exists for.
	let plan = match tool.plan(&input) {
		Ok(plan) => plan,
		Err(error) => return Conversion::Skipped(error.to_string()),
	};
	let Some(target) = plan
		.actions
		.first()
		.and_then(|action| action.target.clone())
	else {
		return Conversion::Skipped(format!(
			"boko planned no conversion: {}",
			describe(plan.warnings.iter().map(|warning| warning.message.clone()))
		));
	};

	match tool.apply(&plan, &mut NoopProgress) {
		Ok(report) if report.applied.len() == 1 && target.is_file() => {
			Conversion::Converted(target)
		},
		Ok(report) => Conversion::Skipped(format!(
			"boko converted nothing: {}",
			describe(report.skipped.into_iter().map(|(_, reason)| reason))
		)),
		Err(error) => Conversion::Skipped(error.to_string()),
	}
}

fn describe(reasons: impl Iterator<Item = String>) -> String {
	let joined = reasons.collect::<Vec<_>>().join("; ");
	if joined.is_empty() {
		"no reason given".to_string()
	} else {
		joined
	}
}

#[allow(clippy::too_many_arguments)]
async fn deliver<M, C>(
	conn: &DatabaseConnection,
	devices: &DeviceService,
	user: &AuthUser,
	media_id: &str,
	device_id: &str,
	emailer: emailer::Model,
	mailer: &M,
	converter: &C,
	work_dir: &Path,
) -> Result<KindleDelivery>
where
	M: KindleMailer,
	C: KindleConverter,
{
	let device = devices.get(user, device_id).await?;
	if device.is_revoked() {
		return Err(format!("{} has been revoked", device.name).into());
	}
	let recipient = device.kindle_email.clone().ok_or_else(|| {
		format!(
			"{} has no Kindle address; set one before sending to it",
			device.name
		)
	})?;
	// A new lane must not reach an address the operator marked forbidden.
	sender::check_forbidden_recipients(user, conn, std::slice::from_ref(&recipient))
		.await?;

	let book = media::Entity::find_for_user(user)
		.filter(media::Column::Id.eq(media_id))
		.one(conn)
		.await?
		.ok_or("Book not found")?;

	let source = PathBuf::from(&book.path);
	let FileParts {
		file_stem,
		extension,
		..
	} = source.file_parts();
	let extension = extension.to_lowercase();
	if !KINDLE_FORMATS.contains(&extension.as_str()) {
		return Err(format!(
			"a Kindle cannot read .{extension}; Amazon accepts {}",
			KINDLE_FORMATS.join(", ")
		)
		.into());
	}

	let conversion = if extension == CONVERTIBLE_FORMAT {
		converter.to_azw3(&source, work_dir).await
	} else {
		Conversion::Skipped(format!(".{extension} needs no conversion"))
	};
	let (path, format, note) = match &conversion {
		Conversion::Converted(path) => (path.as_path(), KINDLE_FORMAT, None),
		Conversion::Skipped(reason) => {
			tracing::debug!(%reason, "Sending the book to the Kindle unconverted");
			(source.as_path(), extension.as_str(), Some(reason.clone()))
		},
	};

	let content = tokio::fs::read(path).await?;
	let payload = attachment(
		&format!("{file_stem}.{format}"),
		format,
		content,
		emailer.max_attachment_size_bytes,
	)?;
	let bytes = payload.content.len() as u64;
	let meta = AttachmentMetaModel::new(
		payload.name.clone(),
		Some(book.id.clone()),
		payload.content.len().try_into()?,
	);

	mailer
		.send(&recipient, &subject(&book.name), payload)
		.await?;

	// Only a delivery that actually went out is recorded, in both places: the
	// emailer's own history (so the Emailer screen shows every send, whichever
	// lane made it) and the device's sync summary.
	let record = emailer_send_record::ActiveModel {
		id: NotSet,
		emailer_id: Set(emailer.id),
		recipient_email: Set(recipient.clone()),
		attachment_meta: Set(Some(AttachmentMetaModel::into_data(&vec![meta])?)),
		sent_at: Set(chrono::Utc::now().into()),
		sent_by_user_id: Set(Some(user.id.clone())),
	};
	let (_, errors) = sender::update_send_records(emailer, conn, vec![record]).await?;
	if !errors.is_empty() {
		tracing::error!(?errors, "Kindle delivery sent but not fully recorded");
	}
	devices
		.record_delivery(&device.id, &KindleSendSummary::new(bytes, format))
		.await?;

	Ok(KindleDelivery {
		device_id: device.id,
		device_name: device.name,
		recipient,
		format: format.to_string(),
		bytes,
		converted: matches!(conversion, Conversion::Converted(_)),
		note,
	})
}

/// The subject Amazon sees.
///
/// `Convert` is a magic word for the service — it makes Amazon reflow a PDF
/// into Kindle format — and Stump never asks for that on the operator's
/// behalf, so a book that happens to be called "Convert" is qualified.
/// Everything else is ignored by Amazon and is there for the operator's own
/// mail log.
fn subject(book_name: &str) -> String {
	if book_name.trim().eq_ignore_ascii_case("convert") {
		format!("{book_name} (sent by Stump)")
	} else {
		book_name.to_string()
	}
}

/// The attachment, with the emailer's size cap enforced before an SMTP
/// connection is opened.
fn attachment(
	name: &str,
	extension: &str,
	content: Vec<u8>,
	max_attachment_size_bytes: Option<i32>,
) -> Result<AttachmentPayload> {
	// Five bytes is what the sniffer below needs, and nothing shorter is a
	// book: the emailer flow refuses the same floor.
	if content.len() < 5 {
		return Err(format!("{name} is too small to be a book").into());
	}
	if let Some(max) = max_attachment_size_bytes {
		if content.len() as i64 > i64::from(max) {
			return Err(format!(
				"{name} is {} bytes, over the emailer's {max} byte limit",
				content.len()
			)
			.into());
		}
	}

	// The bytes decide, except that the extension wins when it names a
	// subtype of what the bytes say: `infer` reports every `BOOKMOBI` Palm
	// database as MOBI, and only `.azw3` distinguishes KF8.
	let mime =
		ContentType::from_bytes_with_fallback(&content[..5], extension).mime_type();
	let content_type = mime.parse::<EmailContentType>().map_err(|error| {
		tracing::warn!(?error, %mime, "Failed to parse the attachment content type");
		format!("{name} has no usable content type")
	})?;

	Ok(AttachmentPayload {
		name: name.to_string(),
		content,
		content_type,
	})
}

#[cfg(test)]
mod tests;
