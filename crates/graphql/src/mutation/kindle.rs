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
		kindle_destination,
		user::AuthUser,
	},
	shared::enums::UserPermission,
};

use sea_orm::{
	sea_query::Expr, ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait,
	IntoActiveModel, NotSet, QueryFilter, Set,
};

use crate::{data::CoreContext, guard::PermissionGuard, mutation::emailer::sender};
use stump_kindle::{
	BokoConverter, KindleDeliveryRow, KindleError, KindleMailer, KindleResult, KindleSend,
};
use stump_notify::EmailChannel;

/// What one legacy device send did: the stored row plus the resolved device
/// name. This shape is intentionally unchanged: mobile clients rely on
/// `deviceId: String!` for `kindleDeliveries` and `sendToKindle`.
#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct KindleDelivery {
	pub id: ID,
	pub media_id: String,
	pub device_id: String,
	pub device_name: String,
	pub recipient: String,
	pub format: String,
	pub bytes: u64,
	pub converted: bool,
	pub note: Option<String>,
	pub error: Option<String>,
	pub sent_at: DateTime<FixedOffset>,
}

impl KindleDelivery {
	/// Legacy rows are guaranteed to have a device id by the legacy query and
	/// mutation path. Returning `None` keeps the invariant explicit instead of
	/// manufacturing an empty id for a destination row.
	pub fn from_legacy(row: KindleDeliveryRow, device_name: String) -> Option<Self> {
		Some(Self {
			id: ID::from(row.id),
			media_id: row.media_id,
			device_id: row.device_id?,
			device_name,
			recipient: row.recipient,
			format: row.format,
			bytes: row.bytes.max(0) as u64,
			converted: row.converted,
			note: row.note,
			error: row.error,
			sent_at: row.sent_at,
		})
	}
}

/// Destination-only delivery output. It deliberately does not reuse the
/// legacy `KindleDelivery` shape: destination sends have no device id, while
/// mobile clients must keep seeing `deviceId: String!` on the old type.
#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct KindleDestinationDelivery {
	pub id: ID,
	pub media_id: String,
	pub destination_id: ID,
	pub destination_name: String,
	pub destination_email: String,
	pub recipient: String,
	pub format: String,
	pub bytes: u64,
	pub converted: bool,
	pub note: Option<String>,
	pub error: Option<String>,
	pub sent_at: DateTime<FixedOffset>,
}

impl KindleDestinationDelivery {
	pub fn from_row(row: KindleDeliveryRow) -> Option<Self> {
		Some(Self {
			id: ID::from(row.id),
			media_id: row.media_id,
			destination_id: ID::from(row.destination_id?),
			destination_name: row.destination_name?,
			destination_email: row.recipient.clone(),
			recipient: row.recipient,
			format: row.format,
			bytes: row.bytes.max(0) as u64,
			converted: row.converted,
			note: row.note,
			error: row.error,
			sent_at: row.sent_at,
		})
	}
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct KindleDestination {
	pub id: ID,
	pub name: String,
	pub email: String,
	pub is_default: bool,
	pub created_at: DateTime<FixedOffset>,
	pub updated_at: DateTime<FixedOffset>,
}

impl From<kindle_destination::Model> for KindleDestination {
	fn from(model: kindle_destination::Model) -> Self {
		Self {
			id: ID::from(model.id),
			name: model.name,
			email: model.email,
			is_default: model.is_default,
			created_at: model.created_at,
			updated_at: model.updated_at,
		}
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

		KindleDelivery::from_legacy(delivery.row, delivery.device_name)
			.ok_or_else(|| "legacy Kindle delivery had no device id".into())
	}
	/// Creates or updates one destination owned by the current user. Amazon
	/// only accepts mail from an approved sender configured in the server SMTP
	/// emailer; this mutation validates the recipient syntax but cannot verify
	/// Amazon's allowlist.
	async fn upsert_kindle_destination(
		&self,
		ctx: &Context<'_>,
		id: Option<ID>,
		name: String,
		email: String,
		make_default: bool,
	) -> Result<KindleDestination> {
		let core_ctx = ctx.data::<CoreContext>()?;
		let user = &ctx.data::<stump_auth::AuthContext>()?.user;
		let conn = core_ctx.conn.as_ref();
		let name = name.trim().to_owned();
		if name.is_empty() {
			return Err("Kindle destination name cannot be blank".into());
		}
		let email = stump_devices::normalize_kindle_email(&email)
			.map_err(|error| async_graphql::Error::new(error.to_string()))?;
		let now = chrono::Utc::now().into();

		let creating = id.is_none();
		let mut model = if let Some(id) = id {
			kindle_destination::Entity::find_by_id(id.to_string())
				.filter(kindle_destination::Column::UserId.eq(&user.id))
				.one(conn)
				.await?
				.ok_or("Kindle destination not found")?
				.into_active_model()
		} else {
			kindle_destination::ActiveModel {
				id: Set(uuid::Uuid::new_v4().to_string()),
				user_id: Set(user.id.clone()),
				name: Set(name.clone()),
				email: Set(email.clone()),
				is_default: Set(make_default),
				created_at: Set(now),
				updated_at: Set(chrono::Utc::now().into()),
			}
		};
		model.name = Set(name);
		model.email = Set(email);
		model.updated_at = Set(chrono::Utc::now().into());
		if make_default || creating {
			model.is_default = Set(make_default);
		}
		let destination = if creating {
			model.insert(conn).await?
		} else {
			model.update(conn).await?
		};
		if destination.is_default {
			kindle_destination::Entity::update_many()
				.filter(kindle_destination::Column::UserId.eq(&user.id))
				.filter(kindle_destination::Column::Id.ne(&destination.id))
				.col_expr(kindle_destination::Column::IsDefault, Expr::value(false))
				.exec(conn)
				.await?;
		}
		Ok(destination.into())
	}

	async fn delete_kindle_destination(&self, ctx: &Context<'_>, id: ID) -> Result<bool> {
		let core_ctx = ctx.data::<CoreContext>()?;
		let user = &ctx.data::<stump_auth::AuthContext>()?.user;
		let result = kindle_destination::Entity::delete_many()
			.filter(kindle_destination::Column::Id.eq(id.to_string()))
			.filter(kindle_destination::Column::UserId.eq(&user.id))
			.exec(core_ctx.conn.as_ref())
			.await?;
		Ok(result.rows_affected > 0)
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::EmailSend)")]
	async fn send_to_kindle_destination(
		&self,
		ctx: &Context<'_>,
		media_id: ID,
		destination_id: ID,
	) -> Result<KindleDestinationDelivery> {
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
		.send_to_destination(user, destination_id.as_str(), media_id.as_str())
		.await?;
		KindleDestinationDelivery::from_row(delivery.row).ok_or_else(|| {
			"destination Kindle delivery lacked destination snapshot".into()
		})
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
