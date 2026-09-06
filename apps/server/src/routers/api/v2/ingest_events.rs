use std::{convert::Infallible, time::Duration};

use axum::{
	extract::{Query, State},
	http::HeaderMap,
	middleware,
	response::sse::{Event, KeepAlive, Sse},
	routing::get,
	Router,
};
use futures_util::{stream, StreamExt};
use sea_orm::prelude::DateTimeWithTimeZone;
use serde::{Deserialize, Serialize};
use serde_json::json;
use stump_ingest::contract::AnalysisPhase;

use crate::{config::state::AppState, middleware::auth::auth_middleware};

const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);
const MAX_EVENT_BYTES: usize = 256 * 1024;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IngestEventsQuery {
	library_id: Option<String>,
	drop_item_id: Option<String>,
	after: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct IngestSsePayload {
	event_id: String,
	emitted_at: DateTimeWithTimeZone,
	library_id: String,
	drop_item_id: String,
	analysis_job_id: Option<String>,
	status: &'static str,
	phase: &'static str,
	completed: u32,
	total: u32,
	score: Option<u8>,
	message: String,
}

impl IngestSsePayload {
	fn from_stored(event: stump_ingest::progress::StoredProgressEvent) -> Self {
		let status = event.event.status.as_str();
		let phase = match event.event.phase {
			AnalysisPhase::Staging => "STAGING",
			AnalysisPhase::Parsing | AnalysisPhase::Pages => "ANALYSIS",
			AnalysisPhase::Quality => "QUALITY",
			AnalysisPhase::Identify => "IDENTIFY",
			AnalysisPhase::Lookup => "LOOKUP",
			AnalysisPhase::Done => "DONE",
		};
		Self {
			event_id: event.event_id,
			emitted_at: event.emitted_at,
			library_id: event.event.library_id,
			drop_item_id: event.event.drop_item_id,
			analysis_job_id: event.event.analysis_job_id,
			status,
			phase,
			completed: event.event.completed,
			total: event.event.total,
			score: event.event.score,
			message: event.event.message,
		}
	}
}

pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	Router::new()
		.route("/ingest/events", get(events))
		.layer(middleware::from_fn_with_state(app_state, auth_middleware))
}

async fn events(
	State(ctx): State<AppState>,
	Query(query): Query<IngestEventsQuery>,
	headers: HeaderMap,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
	let after = headers
		.get("last-event-id")
		.and_then(|value| value.to_str().ok())
		.map(ToOwned::to_owned)
		.or(query.after);
	let result = ctx
		.ingest()
		.coordinator
		.subscribe_stored(
			query.library_id.as_deref(),
			query.drop_item_id.as_deref(),
			None,
			after.as_deref(),
		)
		.await;
	let stream = match result {
		Ok(events) => events
			.filter_map(|result| async move {
				match result {
					Ok(event) => {
						let payload = IngestSsePayload::from_stored(event);
						let event_id = payload.event_id.clone();
						let data = match serde_json::to_string(&payload) {
							Ok(data) => data,
							Err(error) => {
								tracing::warn!(%error, "failed to serialize ingest progress event");
								return None;
							},
						};
						if data.len() > MAX_EVENT_BYTES {
							tracing::warn!(
								bytes = data.len(),
								"dropping oversized ingest progress event"
							);
							return None;
						}
						Some(Ok(Event::default()
							.id(event_id)
							.event("ingest.progress")
							.data(data)))
					},
					Err(error) => {
						let data = json!({ "code": "CURSOR_EXPIRED", "message": error.to_string() })
							.to_string();
						Some(Ok(Event::default().event("error").data(data)))
					},
				}
			})
			.boxed(),
		Err(error) => stream::once(async move {
			Ok(Event::default().event("error").data(
				json!({ "code": "CURSOR_EXPIRED", "message": error.to_string() })
					.to_string(),
			))
		})
		.boxed(),
	};
	Sse::new(stream).keep_alive(KeepAlive::new().interval(KEEP_ALIVE_INTERVAL))
}
