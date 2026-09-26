//! Notification routing: `CoreEvent` → `Notification` mapping, the listener
//! that enqueues dispatch jobs, and the per-user channel settings resolution.
//!
//! One listener task subscribes to the core event channel (see
//! [`spawn_listener`], started by `StumpCore::new`). Each routed event is
//! resolved against the persisted `notification_rules` rows and enqueued as a
//! [`StumpJob::NotificationDispatch`]; the job
//! ([`crate::job::notification::NotificationDispatchJob`]) delivers every
//! (user, channel) target with retry/backoff for retryable transport failures.
//! See `crates/notify/README.md`.

use std::sync::Arc;

use email::EmailerClientConfig;
use models::entity::{emailer, notification_channel_setting, notification_rule, user};
use sea_orm::{prelude::*, DatabaseConnection};
use stump_devices::DeviceSeen;
use stump_notify::{
	resolve_targets, Audience, Channel, ChannelError, ChannelRegistry, EmailChannel,
	Notification, NotificationKind, NtfyChannel, Recipient, WebhookChannel,
};

use crate::{
	event::{
		AnalysisJobFailed, CoreEvent, DevicePaired, IngestAwaitingReview,
		ProviderMatchDone, QualityFailed,
	},
	job::{notification::QueuedDelivery, stump_job::StumpJob, CoreJobOutput},
	utils::encryption::{decrypt_string, encrypt_string, fetch_encryption_key},
	CoreResult, Ctx,
};

/// One event routed to the notification pipeline.
pub struct RoutedNotification {
	pub kind: NotificationKind,
	pub audience: Audience,
	pub notification: Notification,
}

/// Map a `CoreEvent` to its notification routing, or `None` when the event is
/// not routable. Pure function so the mapping is unit-testable.
pub fn route_event(event: &CoreEvent) -> Option<RoutedNotification> {
	let (kind, audience, notification) = match event {
		CoreEvent::DevicePaired(DevicePaired {
			device_id: _,
			user_id,
			device_name,
		}) => (
			NotificationKind::DevicePaired,
			Audience::user(user_id.clone()),
			Notification::new(
				NotificationKind::DevicePaired,
				"Device paired",
				format!("{device_name} was paired with your account."),
			),
		),
		CoreEvent::DeviceSeen(DeviceSeen {
			user_id,
			protocol,
			first_seen: true,
			..
		}) => (
			NotificationKind::DeviceFirstSeen,
			Audience::user(user_id.clone()),
			Notification::new(
				NotificationKind::DeviceFirstSeen,
				"Device first seen",
				format!(
					"A {protocol} device connected to your account for the first time."
				),
			),
		),
		CoreEvent::IngestAwaitingReview(IngestAwaitingReview {
			drop_item_id: _,
			source_filename,
			created_by,
			..
		}) => (
			NotificationKind::IngestAwaitingReview,
			audience_or_subscribers(created_by),
			Notification::new(
				NotificationKind::IngestAwaitingReview,
				"Ingest awaiting review",
				format!("{source_filename} finished analysis and needs a decision."),
			),
		),
		CoreEvent::QualityFailed(QualityFailed {
			score,
			failed_checks,
			created_by,
			..
		}) => (
			NotificationKind::QualityFailed,
			audience_or_subscribers(created_by),
			Notification::new(
				NotificationKind::QualityFailed,
				"Quality check failed",
				format!(
					"Quality score {score}: {} check(s) failed ({}).",
					failed_checks.len(),
					failed_checks.join(", "),
				),
			),
		),
		CoreEvent::AnalysisJobFailed(AnalysisJobFailed { error, .. }) => (
			NotificationKind::AnalysisJobFailed,
			Audience::Subscribers,
			Notification::new(
				NotificationKind::AnalysisJobFailed,
				"Analysis job failed",
				error.clone(),
			),
		),
		CoreEvent::ProviderMatchDone(ProviderMatchDone {
			candidate_count,
			created_by,
			..
		}) if *candidate_count > 0 => (
			NotificationKind::ProviderMatchDone,
			audience_or_subscribers(created_by),
			Notification::new(
				NotificationKind::ProviderMatchDone,
				"Metadata match found",
				format!("{candidate_count} metadata candidate(s) are ready for review."),
			),
		),
		CoreEvent::JobOutput(output) => match &output.output {
			CoreJobOutput::LibraryScan(scan) => (
				NotificationKind::ScanFinished,
				Audience::Subscribers,
				Notification::new(
					NotificationKind::ScanFinished,
					"Library scan finished",
					format!(
						"{} media added, {} updated, {} skipped.",
						scan.created_media, scan.updated_media, scan.skipped_files,
					),
				),
			),
			_ => return None,
		},
		_ => return None,
	};
	Some(RoutedNotification {
		kind,
		audience,
		notification,
	})
}

/// The item owner when known, otherwise every user with an enabled rule.
fn audience_or_subscribers(created_by: &Option<String>) -> Audience {
	match created_by {
		Some(user_id) => Audience::user(user_id.clone()),
		None => Audience::Subscribers,
	}
}

/// Resolve `routed` against the persisted rules and enqueue the dispatch job.
async fn dispatch_routed(ctx: &Ctx, routed: RoutedNotification) -> CoreResult<()> {
	let rules = notification_rule::Entity::find()
		.all(ctx.conn.as_ref())
		.await?
		.into_iter()
		.map(|model| stump_notify::Rule {
			user_id: model.user_id,
			event_kind: model.event_kind,
			channel_id: model.channel_id,
			enabled: model.enabled,
		})
		.collect::<Vec<_>>();

	let targets = resolve_targets(&rules, &routed.audience, routed.kind);
	if targets.is_empty() {
		return Ok(());
	}

	let deliveries = targets
		.into_iter()
		.map(|target| QueuedDelivery {
			user_id: target.user_id,
			channel_id: target.channel_id,
			notification: routed.notification.clone(),
		})
		.collect();

	tracing::debug!(kind = %routed.kind, "Enqueuing notification dispatch");
	ctx.enqueue(StumpJob::NotificationDispatch { deliveries })
		.await
}

/// Enqueue a routable notification for one user using their persisted channel
/// rules.
pub async fn enqueue_user_notification(
	ctx: &Ctx,
	user_id: impl Into<String>,
	notification: Notification,
) -> CoreResult<()> {
	let kind = notification.kind;
	dispatch_routed(
		ctx,
		RoutedNotification {
			kind,
			audience: Audience::user(user_id.into()),
			notification,
		},
	)
	.await
}

/// Spawn the long-lived listener that maps core events to notifications.
/// Called once by `StumpCore::new`.
pub fn spawn_listener(ctx: Ctx) {
	tokio::spawn(async move {
		let mut receiver = ctx.get_client_receiver();
		loop {
			match receiver.recv().await {
				Ok(event) => {
					let Some(routed) = route_event(&event) else {
						continue;
					};
					if let Err(error) = dispatch_routed(&ctx, routed).await {
						tracing::error!(?error, "Failed to queue notification dispatch");
					}
				},
				Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
					tracing::warn!(
						count,
						"Notification listener lagged behind core events"
					);
				},
				Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
			}
		}
	});
}

/// The channels offered by this host: `ntfy` and `webhook` are always
/// available; the email channel is registered when a primary emailer is
/// configured (its password is decrypted with the server's encryption key).
pub async fn channel_registry(conn: &DatabaseConnection) -> CoreResult<ChannelRegistry> {
	let mut channels: Vec<Arc<dyn Channel>> = vec![
		Arc::new(NtfyChannel::new()),
		Arc::new(WebhookChannel::new()),
	];

	let emailer = emailer::Entity::find()
		.filter(emailer::Column::IsPrimary.eq(true))
		.one(conn)
		.await
		.ok()
		.flatten();
	if let Some(emailer) = emailer {
		match fetch_encryption_key(conn).await {
			Ok(key) => match emailer_config(&emailer, &key) {
				Ok(config) => {
					channels.insert(0, Arc::new(EmailChannel::new(config)));
				},
				Err(error) => {
					tracing::warn!(
						?error,
						"Emailer config invalid; email channel unavailable"
					)
				},
			},
			Err(error) => {
				tracing::warn!(
					?error,
					"Encryption key unavailable; email channel unavailable"
				)
			},
		}
	}

	Ok(ChannelRegistry::from_channels(channels))
}

fn emailer_config(
	emailer: &emailer::Model,
	encryption_key: &String,
) -> Result<EmailerClientConfig, ChannelError> {
	let password = decrypt_string(&emailer.encrypted_password, encryption_key)
		.map_err(|error| ChannelError::Rejected(error.to_string()))?;
	Ok(EmailerClientConfig {
		sender_email: emailer.sender_email.clone(),
		sender_display_name: emailer.sender_display_name.clone(),
		username: emailer.username.clone(),
		password: Some(password),
		host: emailer.smtp_host.clone(),
		port: emailer.smtp_port.clamp(0, u16::MAX as i32) as u16,
		tls_enabled: emailer.tls_enabled,
		max_attachment_size_bytes: emailer.max_attachment_size_bytes,
		max_num_attachments: emailer.max_num_attachments,
	})
}

/// Build the recipient for `user_id` on `channel`: the channel defaults,
/// overlaid with the stored (secret values still encrypted) settings, then
/// decrypted.
pub async fn recipient_for(
	conn: &DatabaseConnection,
	encryption_key: &String,
	channel: &dyn Channel,
	user_id: &str,
) -> Result<Recipient, ChannelError> {
	let stored = notification_channel_setting::Entity::find_by_id((
		user_id.to_string(),
		channel.id().to_string(),
	))
	.one(conn)
	.await
	.map_err(|error| ChannelError::Transport(error.to_string()))?;

	let username = user::Entity::find_by_id(user_id)
		.one(conn)
		.await
		.ok()
		.flatten()
		.map(|user| user.username)
		.unwrap_or_else(|| user_id.to_string());

	let mut settings = channel.default_settings(user_id);
	if let Some(stored) = stored {
		if let serde_json::Value::Object(stored) = stored.settings {
			for (key, value) in stored {
				settings.insert(key, value);
			}
		}
	}
	for definition in channel.settings() {
		if !definition.secret {
			continue;
		}
		let Some(value) = settings.get(definition.key).and_then(|v| v.as_str()) else {
			continue;
		};
		if value.is_empty() {
			continue;
		}
		let decrypted = decrypt_string(value, encryption_key)
			.map_err(|error| ChannelError::Rejected(error.to_string()))?;
		settings.insert(definition.key.to_string(), decrypted.into());
	}

	Ok(Recipient::new(user_id, username, settings))
}

/// Encrypt the secret values of `values` (those whose definition is marked
/// secret) with the server's encryption key, for storage.
pub fn encrypt_secret_values(
	channel: &dyn Channel,
	values: &mut serde_json::Map<String, serde_json::Value>,
	encryption_key: &String,
) -> CoreResult<()> {
	for definition in channel.settings() {
		if !definition.secret {
			continue;
		}
		let Some(value) = values.get(definition.key).and_then(|v| v.as_str()) else {
			continue;
		};
		if value.is_empty() {
			continue;
		}
		let encrypted = encrypt_string(value, encryption_key)?;
		values.insert(definition.key.to_string(), encrypted.into());
	}
	Ok(())
}

/// Strip the values of secret definitions so settings can be returned to
/// clients without leaking credentials.
pub fn redact_secret_values(
	channel: &dyn Channel,
	values: &mut serde_json::Map<String, serde_json::Value>,
) {
	for definition in channel.settings() {
		if definition.secret {
			values.remove(definition.key);
		}
	}
}

#[cfg(test)]
mod tests {
	use models::shared::enums::DeviceProtocol;

	use super::*;

	fn routed(event: &CoreEvent) -> RoutedNotification {
		route_event(event).expect("event should route")
	}

	fn first_sighting() -> DeviceSeen {
		DeviceSeen {
			device_id: "d1".into(),
			user_id: "u1".into(),
			protocol: DeviceProtocol::Kobo,
			first_seen: true,
		}
	}

	#[test]
	fn device_seen_only_routes_first_sighting() {
		let first = CoreEvent::DeviceSeen(first_sighting());
		let routed = routed(&first);
		assert_eq!(routed.kind, NotificationKind::DeviceFirstSeen);
		assert!(
			matches!(&routed.audience, Audience::Users(users) if users == &["u1".to_string()])
		);
		assert!(routed.notification.body.contains("kobo"));

		let repeat = CoreEvent::DeviceSeen(DeviceSeen {
			first_seen: false,
			..first_sighting()
		});
		assert!(route_event(&repeat).is_none());
	}

	#[test]
	fn ingest_events_route_to_owner_or_subscribers() {
		let review = CoreEvent::IngestAwaitingReview(IngestAwaitingReview {
			library_id: "lib".into(),
			drop_item_id: "drop".into(),
			source_filename: "a.cbz".into(),
			created_by: Some("owner".into()),
		});
		let routed = routed(&review);
		assert_eq!(routed.kind, NotificationKind::IngestAwaitingReview);
		assert!(
			matches!(&routed.audience, Audience::Users(users) if users == &["owner".to_string()])
		);

		let failed = CoreEvent::AnalysisJobFailed(AnalysisJobFailed {
			analysis_job_id: "job".into(),
			error: "boom".into(),
		});
		let failed = route_event(&failed).expect("event should route");
		assert_eq!(failed.kind, NotificationKind::AnalysisJobFailed);
		assert!(matches!(failed.audience, Audience::Subscribers));

		let matched = CoreEvent::ProviderMatchDone(ProviderMatchDone {
			library_id: "lib".into(),
			drop_item_id: "drop".into(),
			created_by: None,
			candidate_count: 0,
		});
		assert!(route_event(&matched).is_none(), "no candidates, no noise");
	}

	#[test]
	fn scan_finished_summary_counts_media() {
		let output = CoreEvent::JobOutput(crate::event::JobOutput {
			id: "job".into(),
			output: CoreJobOutput::LibraryScan(
				crate::filesystem::scanner::LibraryScanOutput {
					library_id: "lib".into(),
					created_media: 2,
					updated_media: 5,
					skipped_files: 9,
					..Default::default()
				},
			),
		});
		let routed = routed(&output);
		assert_eq!(routed.kind, NotificationKind::ScanFinished);
		assert_eq!(
			routed.notification.body,
			"2 media added, 5 updated, 9 skipped."
		);
	}
}
