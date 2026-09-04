use std::{pin::Pin, sync::Arc};

use chrono::Utc;
use futures::{stream, Stream, StreamExt};
use models::entity::ingest_progress_event;
use sea_orm::{
	entity::prelude::*, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel,
	QueryFilter, QueryOrder,
};
use thiserror::Error;
use tokio::sync::broadcast;

use super::contract::IngestProgressEvent;
use crate::{error::CoreError, CoreResult};

pub type ProgressStream =
	Pin<Box<dyn Stream<Item = Result<IngestProgressEvent, CursorExpired>> + Send>>;

pub type StoredProgressStream =
	Pin<Box<dyn Stream<Item = Result<StoredProgressEvent, CursorExpired>> + Send>>;

#[derive(Debug, Clone, PartialEq)]
pub struct StoredProgressEvent {
	pub event_id: String,
	pub emitted_at: DateTimeWithTimeZone,
	pub event: IngestProgressEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("ingest progress cursor expired; replay begins at {oldest}")]
pub struct CursorExpired {
	pub oldest: String,
}

#[derive(Clone)]
pub struct ProgressHub {
	conn: Arc<DatabaseConnection>,
	retention: u32,
	sender: broadcast::Sender<StoredProgressEvent>,
}

impl ProgressHub {
	pub fn new(conn: Arc<DatabaseConnection>, retention: u32) -> Self {
		let capacity = usize::try_from(retention.max(1)).unwrap_or(usize::MAX);
		let (sender, _) = broadcast::channel(capacity);
		Self {
			conn,
			retention,
			sender,
		}
	}

	pub fn retention(&self) -> u32 {
		self.retention
	}

	pub async fn emit(
		&self,
		mut event: IngestProgressEvent,
	) -> CoreResult<IngestProgressEvent> {
		// Insert once to obtain the database cursor, then stamp that exact
		// cursor into the persisted JSON envelope before broadcasting it.
		event.cursor.clear();
		let created_at = DateTimeWithTimeZone::from(Utc::now());
		let model = ingest_progress_event::ActiveModel {
			library_id: Set(event.library_id.clone()),
			drop_item_id: Set(event.drop_item_id.clone()),
			payload: Set(serde_json::to_value(&event)?),
			created_at: Set(created_at),
			..Default::default()
		};
		let inserted = model.insert(self.conn.as_ref()).await?;
		event.cursor = format_cursor(inserted.cursor);
		let mut update = inserted.clone().into_active_model();
		update.payload = Set(serde_json::to_value(&event)?);
		update.update(self.conn.as_ref()).await?;
		self.prune(inserted.cursor).await?;
		let record = StoredProgressEvent {
			event_id: event.cursor.clone(),
			emitted_at: inserted.created_at,
			event: event.clone(),
		};
		let _ = self.sender.send(record);
		Ok(event)
	}

	pub async fn subscribe(
		&self,
		library_id: Option<&str>,
		drop_item_id: Option<&str>,
		analysis_job_id: Option<&str>,
		after: Option<&str>,
	) -> Result<ProgressStream, CursorExpired> {
		let stream = self
			.subscribe_stored(library_id, drop_item_id, analysis_job_id, after)
			.await?;
		Ok(Box::pin(
			stream.map(|result| result.map(|record| record.event)),
		))
	}

	pub async fn subscribe_stored(
		&self,
		library_id: Option<&str>,
		drop_item_id: Option<&str>,
		analysis_job_id: Option<&str>,
		after: Option<&str>,
	) -> Result<StoredProgressStream, CursorExpired> {
		let receiver = self.sender.subscribe();
		let oldest = self.oldest_cursor().await.map_err(|_| CursorExpired {
			oldest: format_cursor(1),
		})?;
		let after_cursor = match after {
			Some(after) => Some(parse_cursor(after).map_err(|_| CursorExpired {
				oldest: oldest.clone().unwrap_or_else(|| format_cursor(1)),
			})?),
			None => None,
		};
		if let (Some(after_cursor), Some(oldest_cursor)) =
			(after_cursor, oldest.as_deref())
		{
			let oldest_cursor = parse_cursor(oldest_cursor).unwrap_or(1);
			if after_cursor.saturating_add(1) < oldest_cursor {
				return Err(CursorExpired {
					oldest: format_cursor(oldest_cursor as i64),
				});
			}
		}

		let mut query = ingest_progress_event::Entity::find()
			.order_by_asc(ingest_progress_event::Column::Cursor);
		if let Some(after_cursor) = after_cursor {
			query = query
				.filter(ingest_progress_event::Column::Cursor.gt(after_cursor as i64));
		}
		let rows = query
			.all(self.conn.as_ref())
			.await
			.map_err(|_| CursorExpired {
				oldest: oldest.clone().unwrap_or_else(|| format_cursor(1)),
			})?;
		let mut replay = Vec::with_capacity(rows.len());
		let mut last_cursor = after_cursor.unwrap_or(0);
		for row in rows {
			let mut event = serde_json::from_value::<IngestProgressEvent>(row.payload)
				.map_err(|_| CursorExpired {
					oldest: oldest.clone().unwrap_or_else(|| format_cursor(1)),
				})?;
			let event_id = format_cursor(row.cursor);
			event.cursor = event_id.clone();
			last_cursor = last_cursor.max(row.cursor as u64);
			if event_matches(&event, library_id, drop_item_id, analysis_job_id) {
				replay.push(StoredProgressEvent {
					event_id,
					emitted_at: row.created_at,
					event,
				});
			}
		}

		let filters = ProgressFilters {
			library_id: library_id.map(str::to_owned),
			drop_item_id: drop_item_id.map(str::to_owned),
			analysis_job_id: analysis_job_id.map(str::to_owned),
		};
		let live = stream::unfold(
			SubscriptionState::Live {
				receiver,
				filters,
				last_cursor,
			},
			next_live_item,
		);
		let replay = stream::iter(replay.into_iter().map(Ok));
		Ok(Box::pin(replay.chain(live)))
	}

	async fn prune(&self, cursor: i64) -> CoreResult<()> {
		let delete = if self.retention == 0 {
			ingest_progress_event::Entity::delete_many()
		} else if cursor > i64::from(self.retention) {
			ingest_progress_event::Entity::delete_many().filter(
				ingest_progress_event::Column::Cursor
					.lte(cursor - i64::from(self.retention)),
			)
		} else {
			return Ok(());
		};
		delete.exec(self.conn.as_ref()).await?;
		Ok(())
	}

	async fn oldest_cursor(&self) -> CoreResult<Option<String>> {
		ingest_progress_event::Entity::find()
			.order_by_asc(ingest_progress_event::Column::Cursor)
			.one(self.conn.as_ref())
			.await
			.map(|row| row.map(|row| format_cursor(row.cursor)))
			.map_err(CoreError::from)
	}
}
#[derive(Debug)]
enum SubscriptionState {
	Live {
		receiver: broadcast::Receiver<StoredProgressEvent>,
		filters: ProgressFilters,
		last_cursor: u64,
	},
}

#[derive(Debug, Clone)]
struct ProgressFilters {
	library_id: Option<String>,
	drop_item_id: Option<String>,
	analysis_job_id: Option<String>,
}

async fn next_live_item(
	state: SubscriptionState,
) -> Option<(
	Result<StoredProgressEvent, CursorExpired>,
	SubscriptionState,
)> {
	match state {
		SubscriptionState::Live {
			mut receiver,
			filters,
			mut last_cursor,
		} => loop {
			match receiver.recv().await {
				Ok(record) => {
					let cursor = parse_cursor(&record.event.cursor).unwrap_or(0);
					if cursor <= last_cursor
						|| !event_matches(
							&record.event,
							filters.library_id.as_deref(),
							filters.drop_item_id.as_deref(),
							filters.analysis_job_id.as_deref(),
						) {
						continue;
					}
					last_cursor = cursor;
					return Some((
						Ok(record),
						SubscriptionState::Live {
							receiver,
							filters,
							last_cursor,
						},
					));
				},
				Err(broadcast::error::RecvError::Lagged(_)) => {
					return Some((
						Err(CursorExpired {
							oldest: format_cursor(last_cursor.saturating_add(1) as i64),
						}),
						SubscriptionState::Live {
							receiver,
							filters,
							last_cursor,
						},
					));
				},
				Err(broadcast::error::RecvError::Closed) => return None,
			}
		},
	}
}

fn event_matches(
	event: &IngestProgressEvent,
	library_id: Option<&str>,
	drop_item_id: Option<&str>,
	analysis_job_id: Option<&str>,
) -> bool {
	library_id.is_none_or(|value| value == event.library_id)
		&& drop_item_id.is_none_or(|value| value == event.drop_item_id)
		&& analysis_job_id
			.is_none_or(|value| event.analysis_job_id.as_deref() == Some(value))
}

fn parse_cursor(cursor: &str) -> Result<u64, ()> {
	if cursor.is_empty() {
		return Err(());
	}
	let value = cursor.parse::<u64>().map_err(|_| ())?;
	if value > i64::MAX as u64 {
		return Err(());
	}
	Ok(value)
}

pub fn format_cursor(cursor: i64) -> String {
	format!("{cursor:012}")
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use super::{format_cursor, ProgressHub};
	use crate::ingest::contract::{AnalysisPhase, DropItemStatus, IngestProgressEvent};
	use futures::StreamExt;
	use migrations::MigratorTrait;
	use models::{
		entity::{ingest_drop_item, library, library_config},
		shared::enums::FileStatus,
	};
	use sea_orm::{
		ActiveModelTrait,
		ActiveValue::{NotSet, Set},
		Database,
	};

	#[tokio::test]
	async fn replay_and_cursor_expiry_are_deterministic() {
		let conn = Arc::new(Database::connect("sqlite::memory:").await.unwrap());
		migrations::Migrator::up(conn.as_ref(), None).await.unwrap();
		let config = <library_config::ActiveModel as std::default::Default>::default()
			.insert(conn.as_ref())
			.await
			.unwrap();
		library::ActiveModel {
			id: Set("library".to_string()),
			name: Set("Library".to_string()),
			path: Set("/tmp/library".to_string()),
			status: Set(FileStatus::Ready),
			config_id: Set(config.id),
			..Default::default()
		}
		.insert(conn.as_ref())
		.await
		.unwrap();
		for id in ["one", "two", "three"] {
			ingest_drop_item::ActiveModel {
				id: Set(id.to_string()),
				library_id: Set("library".to_string()),
				source_filename: Set(format!("{id}.cbz")),
				byte_size: Set(1),
				source_sha256: Set(id.to_string()),
				media_kind: Set("COMIC_ARCHIVE".to_string()),
				staging_path: Set(format!("/tmp/{id}.cbz")),
				status: Set(DropItemStatus::Analyzing.as_str().to_string()),
				created_by: Set(None),
				relative_path: Set(None),
				analysis_job_id: Set(None),
				quality_report_id: Set(None),
				media_id: Set(None),
				series_id: Set(None),
				pending_fields: Set(None),
				error: Set(None),
				idempotency_key: Set(None),
				revision: Set(1),
				created_at: NotSet,
				updated_at: NotSet,
			}
			.insert(conn.as_ref())
			.await
			.unwrap();
		}
		let hub = ProgressHub::new(conn, 2);
		for id in ["one", "two", "three"] {
			hub.emit(IngestProgressEvent {
				cursor: String::new(),
				library_id: "library".to_string(),
				drop_item_id: id.to_string(),
				analysis_job_id: None,
				phase: AnalysisPhase::Quality,
				status: DropItemStatus::Analyzing,
				completed: 0,
				total: 1,
				score: None,
				message: id.to_string(),
			})
			.await
			.unwrap();
		}
		let expired = hub
			.subscribe(Some("library"), None, None, Some("000000000000"))
			.await;
		assert!(expired.is_err());
		let mut stream = hub
			.subscribe(Some("library"), None, None, None)
			.await
			.unwrap();
		let first = stream.next().await.unwrap().unwrap();
		assert_eq!(first.message, "two");
	}

	#[test]
	fn cursor_is_zero_padded() {
		assert_eq!(format_cursor(7), "000000000007");
	}
}
