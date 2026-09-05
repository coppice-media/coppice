use std::{collections::BTreeMap, sync::Mutex};

use serde::{Deserialize, Serialize};

/// Queued plus running jobs bucketed by [`JobPayload::kind`](crate::JobPayload::kind)
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[serde(rename_all = "camelCase")]
pub struct JobQueueStatus {
	pub count: i32,
	pub count_by_type: BTreeMap<String, i32>,
}

#[derive(Debug, Default)]
struct JobQueueCounts {
	queued: BTreeMap<&'static str, i32>,
	running: BTreeMap<&'static str, i32>,
}

impl JobQueueCounts {
	fn snapshot(&self) -> JobQueueStatus {
		let mut count_by_type = BTreeMap::new();
		for map in [&self.queued, &self.running] {
			for (&kind, &count) in map {
				if count > 0 {
					*count_by_type.entry(kind.to_owned()).or_insert(0) += count;
				}
			}
		}
		JobQueueStatus {
			count: count_by_type.values().copied().sum(),
			count_by_type,
		}
	}
}

/// Tracks queued and running jobs without querying persistence. Every transition
/// returns the new snapshot so the caller can publish it without waking the worker.
#[derive(Debug, Default)]
pub(crate) struct JobQueueState {
	counts: Mutex<JobQueueCounts>,
}

impl JobQueueState {
	pub(crate) fn enqueued(&self, kind: &'static str) -> JobQueueStatus {
		self.change(|counts| increment(&mut counts.queued, kind))
	}

	pub(crate) fn enqueue_failed(&self, kind: &'static str) -> JobQueueStatus {
		self.change(|counts| decrement(&mut counts.queued, kind))
	}

	pub(crate) fn started(&self, kind: &'static str) -> JobQueueStatus {
		self.change(|counts| {
			decrement(&mut counts.queued, kind);
			increment(&mut counts.running, kind);
		})
	}

	pub(crate) fn finished(&self, kind: &'static str) -> JobQueueStatus {
		self.change(|counts| decrement(&mut counts.running, kind))
	}

	pub(crate) fn snapshot(&self) -> JobQueueStatus {
		self.counts
			.lock()
			.expect("job queue state mutex poisoned")
			.snapshot()
	}

	fn change(&self, transition: impl FnOnce(&mut JobQueueCounts)) -> JobQueueStatus {
		let mut counts = self.counts.lock().expect("job queue state mutex poisoned");
		transition(&mut counts);
		counts.snapshot()
	}
}

fn increment(map: &mut BTreeMap<&'static str, i32>, kind: &'static str) {
	let count = map.entry(kind).or_insert(0);
	*count = count.saturating_add(1);
}

fn decrement(map: &mut BTreeMap<&'static str, i32>, kind: &'static str) {
	let Some(count) = map.get_mut(kind) else {
		return;
	};
	*count = count.saturating_sub(1);
	if *count == 0 {
		map.remove(kind);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn queue_state_reports_queued_running_and_zero_transitions() {
		let state = JobQueueState::default();

		assert_eq!(
			state.enqueued("SCAN"),
			JobQueueStatus {
				count: 1,
				count_by_type: BTreeMap::from([("SCAN".to_owned(), 1)]),
			}
		);
		assert_eq!(
			state.started("SCAN"),
			JobQueueStatus {
				count: 1,
				count_by_type: BTreeMap::from([("SCAN".to_owned(), 1)]),
			}
		);
		assert_eq!(state.snapshot().count, 1);
		assert_eq!(
			state.finished("SCAN"),
			JobQueueStatus {
				count: 0,
				count_by_type: BTreeMap::new(),
			}
		);
	}

	#[test]
	fn failed_enqueue_and_stray_finish_never_go_negative() {
		let state = JobQueueState::default();

		state.enqueued("THUMBNAIL");
		assert_eq!(state.enqueue_failed("THUMBNAIL").count, 0);
		assert_eq!(state.finished("THUMBNAIL").count, 0);
		assert!(state.snapshot().count_by_type.is_empty());
	}
}
