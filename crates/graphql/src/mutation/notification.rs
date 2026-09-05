//! GraphQL mutations for notification channels and rules.

use async_graphql::{Context, Object, Result};
use sea_orm::prelude::*;
use sea_orm::ActiveValue::Set;
use stump_notify::{ChannelRegistry, NotificationKind};

use crate::{
	data::CoreContext,
	error_message,
	object::notification::{
		NotificationChannelSettingsValue, NotificationRule,
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

async fn registry(core: &stump_core::Ctx) -> Result<ChannelRegistry> {
	Ok(stump_core::notification::channel_registry(&core.conn).await?)
}

#[derive(Default)]
pub struct NotificationMutation;

#[derive(async_graphql::InputObject)]
pub struct SetNotificationChannelSettingsInput {
	/// The channel whose per-user settings are written.
	pub channel_id: String,
	/// Values keyed by the channel's settings definition keys. Stored values
	/// are merged over the channel defaults; omitted secret keys keep their
	/// previously stored (encrypted) value.
	pub settings: serde_json::Value,
}

#[Object]
impl NotificationMutation {
	/// Upsert the current user's settings for one channel. Unknown keys are
	/// rejected; secret values are stored encrypted and never returned.
	async fn set_notification_channel_settings(
		&self,
		ctx: &Context<'_>,
		input: SetNotificationChannelSettingsInput,
	) -> Result<NotificationChannelSettingsValue> {
		let stump_auth::AuthContext { user, .. } = ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let channels = registry(core).await?;
		let channel = channels
			.get(&input.channel_id)
			.ok_or_else(|| {
				async_graphql::Error::new(format!(
					"Unknown notification channel `{}`",
					input.channel_id
				))
			})?;

		let serde_json::Value::Object(incoming) = input.settings else {
			return Err(async_graphql::Error::new(
				"Settings must be a JSON object keyed by setting keys",
			));
		};
		for key in incoming.keys() {
			if !channel.settings().iter().any(|definition| definition.key == key) {
				return Err(async_graphql::Error::new(format!(
					"Unknown setting `{key}` for channel `{}`",
					input.channel_id
				)));
			}
		}

		// Start from previously stored values so omitted secrets survive.
		let mut stored = models::entity::notification_channel_setting::Entity::find_by_id((
			user.id.clone(),
			input.channel_id.clone(),
		))
		.one(core.conn.as_ref())
		.await?
		.map(|model| model.settings)
		.unwrap_or_else(|| {
			serde_json::Value::Object(
				channel.default_settings(&user.id).into_iter().collect(),
			)
		});
		let serde_json::Value::Object(stored) = &mut stored else {
			return Err(async_graphql::Error::new(
				"Stored channel settings are corrupted",
			));
		};
		for (key, value) in incoming {
			stored.insert(key, value);
		}

		let encryption_key = core.get_encryption_key().await?;
		stump_core::notification::encrypt_secret_values(
			channel.as_ref(),
			stored,
			&encryption_key,
		)?;

		let active = models::entity::notification_channel_setting::ActiveModel {
			user_id: Set(user.id.clone()),
			channel_id: Set(input.channel_id.clone()),
			settings: Set(serde_json::Value::Object(stored.clone())),
		};
		models::entity::notification_channel_setting::Entity::insert(active)
			.on_conflict(
				sea_orm::sea_query::OnConflict::columns(vec![
					models::entity::notification_channel_setting::Column::UserId,
					models::entity::notification_channel_setting::Column::ChannelId,
				])
				.update_column(models::entity::notification_channel_setting::Column::Settings)
				.to_owned(),
			)
			.exec(core.conn.as_ref())
			.await?;

		// Return the redacted effective values.
		let recipient = recipient_for(core, channel.as_ref(), &user.id).await?;
		let mut values = serde_json::to_value(&recipient.settings)
			.map_err(|error| async_graphql::Error::new(error.to_string()))?;
		if let serde_json::Value::Object(map) = &mut values {
			stump_core::notification::redact_secret_values(channel.as_ref(), map);
		}
		Ok(NotificationChannelSettingsValue {
			channel_id: input.channel_id,
			values,
		})
	}

	/// Create or update the current user's routing rule for one
	/// (event kind, channel) pair. Only the server owner may subscribe to
	/// administrative kinds (scans, ingest, metadata, analysis).
	async fn set_notification_rule(
		&self,
		ctx: &Context<'_>,
		event_kind: NotificationKind,
		channel_id: String,
		enabled: bool,
	) -> Result<NotificationRule> {
		let stump_auth::AuthContext { user, .. } = ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		if event_kind.is_administrative() && !user.is_server_owner {
			return Err(error_message::FORBIDDEN_ACTION.into());
		}

		let registry = registry(core).await?;
		if registry.get(&channel_id).is_none() {
			return Err(async_graphql::Error::new(format!(
				"Unknown notification channel `{channel_id}`"
			)));
		}

		let event_kind_name = event_kind.to_string();
		let active = models::entity::notification_rule::ActiveModel {
			id: sea_orm::ActiveValue::NotSet,
			user_id: Set(user.id.clone()),
			event_kind: Set(event_kind_name.clone()),
			channel_id: Set(channel_id.clone()),
			enabled: Set(enabled),
		};
		let inserted = models::entity::notification_rule::Entity::insert(active)
			.on_conflict(
				sea_orm::sea_query::OnConflict::columns(vec![
					models::entity::notification_rule::Column::UserId,
					models::entity::notification_rule::Column::EventKind,
					models::entity::notification_rule::Column::ChannelId,
				])
				.update_column(models::entity::notification_rule::Column::Enabled)
				.to_owned(),
			)
			.exec_with_returning(core.conn.as_ref())
			.await?;

		Ok(NotificationRule::from(inserted))
	}

	/// Deliver a test notification through one of the current user's channels.
	/// Returns `false` when the channel is not configured; errors on failure.
	async fn test_notification_channel(
		&self,
		ctx: &Context<'_>,
		channel_id: String,
	) -> Result<bool> {
		let stump_auth::AuthContext { user, .. } = ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let channels = registry(core).await?;
		let channel = channels.get(&channel_id).ok_or_else(|| {
			async_graphql::Error::new(format!(
				"Unknown notification channel `{channel_id}`"
			))
		})?;
		let label = channel.label().to_string();

		let recipient = recipient_for(core, channel.as_ref(), &user.id).await?;
		channel
			.send(&recipient, &stump_notify::Notification::test(&label))
			.await?;
		Ok(true)
	}
}
