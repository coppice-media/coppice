//! The notification dispatch job: delivers the (user, channel) targets an
//! event routed to, with retry/backoff for retryable transport failures.
//! Routing and channel resolution live in `crate::notification`.

use std::{sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use stump_jobs::{
	JobContext, JobError, JobExecuteLog, JobLifecycle, JobProgress, JobTaskOutput,
	WorkingState,
};
use stump_notify::{ChannelError, ChannelRegistry, Notification};

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
		let recipient = recipient_for(
			ctx.conn(),
			&encryption_key,
			channel.as_ref(),
			&delivery.user_id,
		)
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
