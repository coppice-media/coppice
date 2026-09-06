//! GraphQL queries for notification channels and rules.

use async_graphql::{Context, Json, Object, Result};
use sea_orm::prelude::*;

use crate::{
	data::CoreContext,
	object::notification::{
		NotificationChannel, NotificationChannelSettingsValue, NotificationRule,
	},
};

/// Load the recipient (defaults + stored + decrypted) for a user/channel.
async fn recipient_for(
	core: &stump_core::Ctx,
	channel: &dyn stump_notify::Channel,
	user_id: &str,
) -> Result<stump_notify::Recipient> {
	let encryption_key = core.get_encryption_key().await?;
	Ok(stump_core::notification::recipient_for(
		core.conn.as_ref(),
		&encryption_key,
		channel,
		user_id,
	)
	.await?)
}

#[derive(Default)]
pub struct NotificationQuery;

#[Object]
impl NotificationQuery {
	/// The notification channels this server offers, with the settings schema
	/// each user can configure.
	async fn notification_channels(
		&self,
		ctx: &Context<'_>,
	) -> Result<Vec<NotificationChannel>> {
		let core = ctx.data::<CoreContext>()?;
		let registry = stump_core::notification::channel_registry(&core.conn).await?;

		Ok(registry
			.all()
			.iter()
			.map(|channel| NotificationChannel::from_channel(channel.as_ref()))
			.collect())
	}

	/// The current user's routing rules. `eventKind` is a `NotificationKind`
	/// name or `*`, which matches every routable kind.
	async fn notification_rules(
		&self,
		ctx: &Context<'_>,
	) -> Result<Vec<NotificationRule>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		let rules = models::entity::notification_rule::Entity::find()
			.filter(models::entity::notification_rule::Column::UserId.eq(&user.id))
			.all(conn)
			.await?;

		Ok(rules.into_iter().map(NotificationRule::from).collect())
	}

	/// The current user's effective settings for every channel, with secret
	/// values redacted.
	async fn notification_channel_settings(
		&self,
		ctx: &Context<'_>,
	) -> Result<Vec<NotificationChannelSettingsValue>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let registry = stump_core::notification::channel_registry(&core.conn).await?;

		let mut result = Vec::new();
		for channel in registry.all() {
			let recipient = recipient_for(core, channel.as_ref(), &user.id).await?;
			let mut values = serde_json::to_value(&recipient.settings)
				.map_err(|error| async_graphql::Error::new(error.to_string()))?;
			if let serde_json::Value::Object(map) = &mut values {
				stump_core::notification::redact_secret_values(channel.as_ref(), map);
			}
			result.push(NotificationChannelSettingsValue {
				channel_id: channel.id().to_string(),
				values: Json(values),
			});
		}
		Ok(result)
	}
}
