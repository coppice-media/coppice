//! The notification dispatch job: delivers the (user, channel) targets an
//! event routed to, with retry/backoff for retryable transport failures.
//! Routing and channel resolution live in `crate::notification`.

use std::{sync::Arc, time::Duration};

use sea_orm::DatabaseConnection;

use serde::{Deserialize, Serialize};
use stump_jobs::{
	JobContext, JobError, JobExecuteLog, JobLifecycle, JobProgress, JobTaskOutput,
	WorkingState,
};
use stump_notify::{Channel, ChannelError, ChannelRegistry, Notification};

use crate::{
	job::{output::NotificationDispatchOutput, JobServices},
	notification::{channel_registry, recipient_for},
	utils::encryption::fetch_encryption_key,
};

/// The default retry schedule for a failed delivery attempt. Only
/// [`ChannelError::is_retryable`] failures are retried.
pub const RETRY_DELAYS: [Duration; 3] = [
	Duration::from_secs(1),
	Duration::from_secs(2),
	Duration::from_secs(4),
];

/// One (user, channel, notification) delivery persisted in the job payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedDelivery {
	pub user_id: String,
	pub channel_id: String,
	pub notification: Notification,
}

/// The dispatch job. A delivery that exhausts its retryable attempts is logged
/// and counted, never fails the whole job — one misconfigured channel must not
/// block the others.
pub struct NotificationDispatchJob {
	pub deliveries: Vec<QueuedDelivery>,
	pub retry_delays: Vec<Duration>,
	registry: Option<Arc<ChannelRegistry>>,
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
		deliver_with_retry(
			ctx.conn(),
			&encryption_key,
			channel.as_ref(),
			delivery,
			&self.retry_delays,
		)
		.await
	}
}

/// Resolve again after every retry delay: account deletion can happen while
/// an earlier transport failure is sleeping, leaving an old recipient snapshot.
async fn deliver_with_retry(
	conn: &DatabaseConnection,
	encryption_key: &String,
	channel: &dyn Channel,
	delivery: &QueuedDelivery,
	retry_delays: &[Duration],
) -> Result<(), ChannelError> {
	let attempts = retry_delays.len() + 1;
	for attempt in 0..attempts {
		if attempt > 0 {
			tokio::time::sleep(retry_delays[attempt - 1]).await;
		}
		let recipient =
			recipient_for(conn, encryption_key, channel, &delivery.user_id).await?;
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
		ctx: &JobContext<Self::Context>,
		task: Self::Task,
	) -> Result<JobTaskOutput<Self>, JobError> {
		let mut output = Self::Output::default();
		let mut logs = Vec::new();

		match self.deliver(ctx, &task).await {
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
					JobExecuteLog::warn(&format!(
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
	use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
	use std::sync::atomic::{AtomicUsize, Ordering};
	use stump_api_types::settings::SettingDefinition;
	use stump_notify::{NotificationKind, Recipient};
	use tokio::sync::Notify;

	struct RetryingChannel {
		attempts: AtomicUsize,
		first_attempt: Notify,
	}

	#[async_trait::async_trait]
	impl Channel for RetryingChannel {
		fn id(&self) -> &'static str {
			"ntfy"
		}
		fn label(&self) -> &'static str {
			"test notification"
		}
		fn settings(&self) -> &[SettingDefinition] {
			&[]
		}
		async fn send(
			&self,
			_: &Recipient,
			_: &Notification,
		) -> Result<(), ChannelError> {
			self.attempts.fetch_add(1, Ordering::SeqCst);
			self.first_attempt.notify_one();
			Err(ChannelError::Transport("retry me".into()))
		}
	}

	#[tokio::test]
	async fn retry_does_not_send_a_queued_notification_after_account_deletion() {
		for hard_delete in [false, true] {
			let dir = tempfile::tempdir().expect("temporary database");
			let url = format!(
				"sqlite://{}?mode=rwc",
				dir.path().join("notify.db").display()
			);
			let db = Arc::new(
				crate::database::connect_at(&url)
					.await
					.expect("migrated database"),
			);
			for sql in [
				"INSERT INTO users (id, username, hashed_password, is_server_owner, created_at, is_locked) VALUES ('owner', 'owner', 'hash', 1, CURRENT_TIMESTAMP, 0)",
				"INSERT INTO users (id, username, hashed_password, is_server_owner, created_at, is_locked) VALUES ('member', 'member', 'hash', 0, CURRENT_TIMESTAMP, 0)",
			] {
				db.execute(Statement::from_string(DatabaseBackend::Sqlite, sql.to_string())).await.expect(sql);
			}
			let channel = Arc::new(RetryingChannel {
				attempts: AtomicUsize::new(0),
				first_attempt: Notify::new(),
			});
			let delivery = QueuedDelivery {
				user_id: "member".into(),
				channel_id: "ntfy".into(),
				notification: Notification::new(
					NotificationKind::DevicePaired,
					"Paired",
					"Device paired",
				),
			};
			let running = {
				let db = Arc::clone(&db);
				let channel = Arc::clone(&channel);
				tokio::spawn(async move {
					deliver_with_retry(
						&db,
						&String::new(),
						channel.as_ref(),
						&delivery,
						&[Duration::from_millis(250)],
					)
					.await
				})
			};
			tokio::time::timeout(
				Duration::from_secs(5),
				channel.first_attempt.notified(),
			)
			.await
			.expect("first transport attempt");
			let deletion = if hard_delete {
				"DELETE FROM users WHERE id = 'member'"
			} else {
				"UPDATE users SET deleted_at = CURRENT_TIMESTAMP WHERE id = 'member'"
			};
			db.execute(Statement::from_string(DatabaseBackend::Sqlite, deletion))
				.await
				.unwrap();
			let result = running.await.expect("delivery task");
			assert!(
				matches!(result, Err(ChannelError::Rejected(_))),
				"{result:?}"
			);
			assert_eq!(
				channel.attempts.load(Ordering::SeqCst),
				1,
				"retry must not call transport"
			);
			assert!(
				matches!(
					recipient_for(&db, &String::new(), channel.as_ref(), "member").await,
					Err(ChannelError::Rejected(_))
				),
				"already queued delivery has no target"
			);
			assert_eq!(
				recipient_for(&db, &String::new(), channel.as_ref(), "owner")
					.await
					.unwrap()
					.username,
				"owner",
				"deleting a user must not suppress another user's notifications"
			);
		}
	}
}
