//! Notification routing: `CoreEvent` → `Notification` mapping, the dispatch
//! job, and the per-user channel settings resolution.
//!
//! One listener task subscribes to the core event channel (see
//! [`spawn_listener`], started by `StumpCore::new`). Each routed event is
//! resolved against the persisted `notification_rules` rows and enqueued as a
//! [`StumpJob::NotificationDispatch`]; the job delivers every (user, channel)
//! target with retry/backoff for retryable transport failures. See
//! `crates/notify/README.md`.

use std::{sync::Arc, time::Duration};

use email::EmailerClientConfig;
use stump_jobs::{
	JobContext, JobError, JobExecuteLog, JobLifecycle, JobOutputExt,
	JobProgress, JobTaskOutput, WorkingState,
};
use stump_notify::{
	Audience, Channel, ChannelError, ChannelRegistry, EmailChannel, Notification,
	NotificationKind, NtfyChannel, Recipient, WebhookChannel, resolve_targets,
};

use crate::{
	event::{
		AnalysisJobFailed, CoreEvent, DevicePaired, DeviceSeen, IngestAwaitingReview,
		ProviderMatchDone, QualityFailed,
	},
	job::{output::NotificationDispatchOutput, stump_job::StumpJob, CoreJobOutput, JobServices},
	utils::encryption::{decrypt_string, encrypt_string, fetch_encryption_key},
	Ctx, CoreError, CoreResult,
};

/// The default retry schedule for a failed delivery attempt. Only
/// [`ChannelError::is_retryable`] failures are retried.
pub const RETRY_DELAYS: [Duration; 3] = [
	Duration::from_secs(1),
	Duration::from_secs(2),
	Duration::from_secs(4),
];

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
				format!("A {protocol} device connected to your account for the first time."),
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
					tracing::warn!(count, "Notification listener lagged behind core events");
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
	let mut channels: Vec<Arc<dyn Channel>> =
		vec![Arc::new(NtfyChannel::new()), Arc::new(WebhookChannel::new())];

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
					tracing::warn!(?error, "Emailer config invalid; email channel unavailable")
				},
			},
			Err(error) => {
				tracing::warn!(?error, "Encryption key unavailable; email channel unavailable")
			},
		}
	}

	Ok(ChannelRegistry::from_channels(channels))
}

fn emailer_config(
	emailer: &emailer::Model,
	encryption_key: &str,
) -> Result<EmailerClientConfig, ChannelError> {
	let password = decrypt_string(&emailer.encrypted_password, &encryption_key.to_string())
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
	encryption_key: &str,
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
		if definition.secret {
			if let Some(value) = settings.get(definition.key).and_then(|v| v.as_str()) {
				if !value.is_empty() {
					let decrypted = decrypt_string(value, &encryption_key.to_string())
						.map_err(|error| ChannelError::Rejected(error.to_string()))?;
					settings.insert(definition.key.to_string(), decrypted.into());
				}
			}
		}
	}

	Ok(Recipient::new(user_id, username, settings))
}

/// Encrypt the secret values of `values` (those whose definition is marked
/// secret) with the server's encryption key, for storage.
pub fn encrypt_secret_values(
	channel: &dyn Channel,
	values: &mut serde_json::Map<String, serde_json::Value>,
	encryption_key: &str,
) -> CoreResult<()> {
	for definition in channel.settings() {
		if definition.secret {
			if let Some(value) = values.get(definition.key).and_then(|v| v.as_str()) {
				if !value.is_empty() {
					let encrypted = encrypt_string(value, &encryption_key.to_string())?;
					values.insert(definition.key.to_string(), encrypted.into());
				}
			}
		}
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

/// One (user, channel, notification) delivery persisted in the job payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedDelivery {
	pub user_id: String,
	pub channel_id: String,
	pub notification: Notification,
}

/// The dispatch job: delivers queued notifications with retry/backoff. A
/// delivery that exhausts its retryable attempts is logged and counted, never
/// fails the whole job — one misconfigured channel must not block the others.
pub struct NotificationDispatchJob {
	pub deliveries: Vec<QueuedDelivery>,
	pub retry_delays: Vec<Duration>,
	pub(crate) registry: Option<Arc<ChannelRegistry>>,
}

impl NotificationDispatchJob {
	pub fn new(deliveries: Vec<QueuedDelivery>) -> Self {
		Self {
			deliveries,
			retry_delays: RETRY_DELAYS.to_vec(),
			registry: None,
		}
	}

	async fn deliver(
		&self,
		ctx: &JobContext<JobServices>,
		delivery: &QueuedDelivery,
	) -> Result<(), ChannelError> {
		let registry = self
			.registry
			.as_ref()
			.ok_or_else(|| ChannelError::Transport("registry missing".to_string()))?;
		let channel = registry.get(&delivery.channel_id).ok_or_else(|| {
			ChannelError::Rejected(format!("unknown channel `{}`", delivery.channel_id))
		})?;

		let encryption_key = fetch_encryption_key(ctx.conn())
			.await
			.map_err(|error| ChannelError::Transport(error.to_string()))?;
		let recipient =
			recipient_for(ctx.conn(), &encryption_key, channel.as_ref(), &delivery.user_id)
				.await?;

		let attempts = self.retry_delays.len() + 1;
		for attempt in 0..attempts {
			if attempt > 0 {
				tokio::time::sleep(self.retry_delays[attempt - 1]).await;
			}
			match channel.send(&recipient, &delivery.notification).await {
				Ok(()) => return Ok(()),
				// A rejected delivery is permanent; retrying cannot fix it.
				Err(error) if !error.is_retryable() => return Err(error),
				Err(error) => {
					tracing::debug!(
						attempt = attempt + 1,
						channel_id = %delivery.channel_id,
						?error,
						"Retryable notification delivery failure"
					);
				},
			}
		}
		Err(ChannelError::Transport(
			"all retry attempts exhausted".to_string(),
		))
	}
}

#[async_trait::async_trait]
impl JobLifecycle for NotificationDispatchJob {
	const NAME: &'static str = "notification_dispatch";

	type Context = JobServices;
	type Output = NotificationDispatchOutput;
	type Task = QueuedDelivery;

	fn description(&self) -> Option<String> {
		Some(format!("Deliver {} notification(s)", self.deliveries.len()))
	}

	async fn init(
		&mut self,
		ctx: &JobContext<Self::Context>,
	) -> Result<WorkingState<Self::Output, Self::Task>, JobError> {
		ctx.report_progress(JobProgress::msg("Resolving channels"));
		let registry = channel_registry(ctx.conn())
			.await
			.map_err(|error| JobError::InitFailed(error.to_string()))?;
		self.registry = Some(Arc::new(registry));

		Ok(WorkingState {
			output: Some(Self::Output::default()),
			tasks: self.deliveries.clone().into(),
			logs: vec![],
		})
	}

	async fn execute_task(
		&self,
		_ctx: &JobContext<Self::Context>,
		task: Self::Task,
	) -> Result<JobTaskOutput<Self>, JobError> {
		let mut output = Self::Output::default();
		let mut logs = Vec::new();

		match self.deliver(_ctx, &task).await {
			Ok(()) => output.sent += 1,
			Err(error) => {
				output.failed += 1;
				tracing::warn!(
					user_id = %task.user_id,
					channel_id = %task.channel_id,
					?error,
					"Notification delivery failed"
				);
				logs.push(
					JobExecuteLog::warn(format!(
						"{} for {}: {error}",
						task.channel_id, task.user_id
					))
					.with_ctx(task.notification.kind.to_string()),
				);
			},
		}

		Ok(JobTaskOutput {
			output,
			subtasks: vec![],
			logs,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn routed(event: &CoreEvent) -> RoutedNotification {
		route_event(event).expect("event should route")
	}

	#[test]
	fn device_seen_only_routes_first_sighting() {
		let first = CoreEvent::DeviceSeen(DeviceSeen {
			device_id: "d1".into(),
			user_id: "u1".into(),
			protocol: stump_devices::Protocol::Kobo,
			first_seen: true,
		});
		let routed = routed(&first);
		assert_eq!(routed.kind, NotificationKind::DeviceFirstSeen);
		assert!(matches!(&routed.audience, Audience::Users(users) if users == &["u1".to_string()]));

		let repeat = CoreEvent::DeviceSeen(DeviceSeen {
			first_seen: false,
			..first_payload()
		});
		assert!(route_event(&repeat).is_none());
	}

	fn first_payload() -> DeviceSeen {
		DeviceSeen {
			device_id: "d1".into(),
			user_id: "u1".into(),
			protocol: stump_devices::Protocol::Kobo,
			first_seen: true,
		}
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
		let routed = routed(&failed);
		assert_eq!(routed.kind, NotificationKind::AnalysisJobFailed);
		assert!(matches!(routed.audience, Audience::Subscribers));

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
			output: CoreJobOutput::LibraryScan(crate::filesystem::scanner::LibraryScanOutput {
				library_id: "lib".into(),
				created_media: 2,
				updated_media: 5,
				skipped_files: 9,
				..Default::default()
			}),
		});
		let routed = routed(&output);
		assert_eq!(routed.kind, NotificationKind::ScanFinished);
		assert_eq!(routed.notification.body, "2 media added, 5 updated, 9 skipped.");
	}
}
