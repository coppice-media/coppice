use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
	response::sse::{Event, KeepAlive, Sse},
	routing::get,
	Extension, Router,
};
use futures_util::stream::{self, Stream, StreamExt as _};
use models::entity::{media, series, user::AuthUser};
use sea_orm::{ColumnTrait, DatabaseConnection, QueryFilter};
use stump_auth::AuthContext;
use tokio::sync::broadcast::{self, error::RecvError};

use super::{KomgaBackend, KomgaCoreEvent, KomgaEvents};
use crate::sse::KomgaEvent;

const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// Mounts the Komga-compatible server-sent events endpoint.
///
/// Authentication is applied by the parent Komga router. Each request gets its own
/// broadcast receiver, so a slow subscriber can lag without making event producers wait.
pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::<S>::new().route("/sse/v1/events", get(events))
}

async fn events(
	Extension(backend): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<KomgaEvents>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
	let stream = event_stream(
		backend.core_events(),
		events.subscribe(),
		backend.conn_arc(),
		auth.user(),
	)
	.map(|event| event.map(KomgaSseEvent::into_sse_event));
	Sse::new(stream).keep_alive(KeepAlive::new().interval(KEEP_ALIVE_INTERVAL))
}

/// Events are produced globally; each subscriber only receives events for media
/// and series it may see, using the same visibility rules as the catalog routes.
fn event_stream(
	core_receiver: broadcast::Receiver<KomgaCoreEvent>,
	komga_receiver: broadcast::Receiver<KomgaEvent>,
	conn: Arc<DatabaseConnection>,
	user: AuthUser,
) -> impl Stream<Item = Result<KomgaSseEvent, Infallible>> {
	let core_events = core_event_stream(core_receiver, conn.clone(), user.clone());
	let komga_events = komga_event_stream(komga_receiver, conn, user);
	stream::select(core_events, komga_events)
}

fn core_event_stream(
	receiver: broadcast::Receiver<KomgaCoreEvent>,
	conn: Arc<DatabaseConnection>,
	user: AuthUser,
) -> impl Stream<Item = Result<KomgaSseEvent, Infallible>> {
	stream::unfold(
		(receiver, conn, user),
		|(mut receiver, conn, user)| async move {
			loop {
				match receiver.recv().await {
					Ok(core_event) => {
						if let Some((media_id, event)) = map_core_event(core_event) {
							if is_media_visible(conn.as_ref(), &user, &media_id).await {
								return Some((Ok(event), (receiver, conn, user)));
							}
						}
					},
					Err(RecvError::Lagged(skipped)) => {
						tracing::debug!(
							skipped,
							"Komga SSE subscriber lagged; continuing"
						);
					},
					Err(RecvError::Closed) => return None,
				}
			}
		},
	)
}

fn komga_event_stream(
	receiver: broadcast::Receiver<KomgaEvent>,
	conn: Arc<DatabaseConnection>,
	user: AuthUser,
) -> impl Stream<Item = Result<KomgaSseEvent, Infallible>> {
	stream::unfold(
		(receiver, conn, user),
		|(mut receiver, conn, user)| async move {
			loop {
				match receiver.recv().await {
					Ok(komga_event) => {
						let Some((visibility, event)) = map_komga_event(komga_event)
						else {
							continue;
						};
						if is_event_visible(conn.as_ref(), &user, &visibility).await {
							return Some((Ok(event), (receiver, conn, user)));
						}
					},
					Err(RecvError::Lagged(skipped)) => {
						tracing::debug!(
							skipped,
							"Komga SSE subscriber lagged; continuing"
						);
					},
					Err(RecvError::Closed) => return None,
				}
			}
		},
	)
}

async fn is_media_visible(conn: &DatabaseConnection, user: &AuthUser, id: &str) -> bool {
	match media::Entity::find_for_user(user)
		.filter(media::Column::Id.eq(id))
		.one(conn)
		.await
	{
		Ok(row) => row.is_some(),
		Err(error) => {
			tracing::warn!(?error, "Komga SSE visibility check failed; dropping event");
			false
		},
	}
}

async fn is_series_visible(conn: &DatabaseConnection, user: &AuthUser, id: &str) -> bool {
	match series::Entity::find_for_user(user)
		.filter(series::Column::Id.eq(id))
		.one(conn)
		.await
	{
		Ok(row) => row.is_some(),
		Err(error) => {
			tracing::warn!(
				?error,
				"Komga SSE series visibility check failed; dropping event"
			);
			false
		},
	}
}

async fn any_media_visible(
	conn: &DatabaseConnection,
	user: &AuthUser,
	ids: &[String],
) -> bool {
	for id in ids {
		if is_media_visible(conn, user, id).await {
			return true;
		}
	}
	false
}

async fn any_series_visible(
	conn: &DatabaseConnection,
	user: &AuthUser,
	ids: &[String],
) -> bool {
	for id in ids {
		if is_series_visible(conn, user, id).await {
			return true;
		}
	}
	false
}

#[derive(Debug, PartialEq, Eq)]
enum EventVisibility {
	Media(String),
	Series(String),
	MediaSet(Vec<String>),
	SeriesSet(Vec<String>),
	User(String),
}

async fn is_event_visible(
	conn: &DatabaseConnection,
	user: &AuthUser,
	visibility: &EventVisibility,
) -> bool {
	match visibility {
		EventVisibility::Media(id) => is_media_visible(conn, user, id).await,
		EventVisibility::Series(id) => is_series_visible(conn, user, id).await,
		EventVisibility::MediaSet(ids) => any_media_visible(conn, user, ids).await,
		EventVisibility::SeriesSet(ids) => any_series_visible(conn, user, ids).await,
		EventVisibility::User(event_user_id) => {
			is_user_event_visible(event_user_id, &user.id)
		},
	}
}

fn is_user_event_visible(event_user_id: &str, subscriber_user_id: &str) -> bool {
	event_user_id == subscriber_user_id
}

fn map_komga_event(event: KomgaEvent) -> Option<(EventVisibility, KomgaSseEvent)> {
	let (visibility, event) = match event {
		KomgaEvent::SeriesChanged {
			series_id,
			library_id,
		} => {
			let visibility = EventVisibility::Series(series_id.0.clone());
			let event = KomgaSseEvent::from_payload(
				"SeriesChanged",
				crate::sse::SeriesChanged {
					series_id,
					library_id,
				},
			)?;
			(visibility, event)
		},
		KomgaEvent::BookChanged {
			book_id,
			series_id,
			library_id,
		} => {
			let visibility = EventVisibility::Media(book_id.0.clone());
			let event = KomgaSseEvent::from_payload(
				"BookChanged",
				crate::sse::BookChanged {
					book_id,
					series_id,
					library_id,
				},
			)?;
			(visibility, event)
		},
		KomgaEvent::ReadListChanged {
			read_list_id,
			book_ids,
		} => {
			let visibility = EventVisibility::MediaSet(
				book_ids.iter().map(|id| id.0.clone()).collect(),
			);
			let event = KomgaSseEvent::from_payload(
				"ReadListChanged",
				crate::sse::ReadListChanged {
					read_list_id,
					book_ids,
				},
			)?;
			(visibility, event)
		},
		KomgaEvent::CollectionChanged {
			collection_id,
			series_ids,
		} => {
			let visibility = EventVisibility::SeriesSet(
				series_ids.iter().map(|id| id.0.clone()).collect(),
			);
			let event = KomgaSseEvent::from_payload(
				"CollectionChanged",
				crate::sse::CollectionChanged {
					collection_id,
					series_ids,
				},
			)?;
			(visibility, event)
		},
		KomgaEvent::ReadProgressChanged { book_id, user_id } => {
			let visibility = EventVisibility::User(user_id.0.clone());
			let event = KomgaSseEvent::from_payload(
				"ReadProgressChanged",
				crate::sse::ReadProgressChanged { book_id, user_id },
			)?;
			(visibility, event)
		},
		KomgaEvent::ReadProgressDeleted { book_id, user_id } => {
			let visibility = EventVisibility::User(user_id.0.clone());
			let event = KomgaSseEvent::from_payload(
				"ReadProgressDeleted",
				crate::sse::ReadProgressDeleted { book_id, user_id },
			)?;
			(visibility, event)
		},
		KomgaEvent::ReadProgressSeriesChanged { series_id, user_id } => {
			let visibility = EventVisibility::User(user_id.0.clone());
			let event = KomgaSseEvent::from_payload(
				"ReadProgressSeriesChanged",
				crate::sse::ReadProgressSeriesChanged { series_id, user_id },
			)?;
			(visibility, event)
		},
		KomgaEvent::ReadProgressSeriesDeleted { series_id, user_id } => {
			let visibility = EventVisibility::User(user_id.0.clone());
			let event = KomgaSseEvent::from_payload(
				"ReadProgressSeriesDeleted",
				crate::sse::ReadProgressSeriesDeleted { series_id, user_id },
			)?;
			(visibility, event)
		},
		_ => return None,
	};
	Some((visibility, event))
}

struct KomgaSseEvent {
	name: &'static str,
	data: String,
}

impl KomgaSseEvent {
	fn from_payload<T: serde::Serialize>(name: &'static str, payload: T) -> Option<Self> {
		serde_json::to_string(&payload)
			.map(|data| Self { name, data })
			.map_err(|error| {
				tracing::error!(
					?error,
					event = name,
					"Failed to serialize Komga SSE event"
				)
			})
			.ok()
	}

	fn into_sse_event(self) -> Event {
		Event::default().event(self.name).data(self.data)
	}
}

/// Maps a core event to a Komga event plus the media ID whose visibility gates it.
fn map_core_event(event: KomgaCoreEvent) -> Option<(String, KomgaSseEvent)> {
	match event {
		KomgaCoreEvent::CreatedMedia {
			id,
			series_id,
			library_id,
		} => {
			let media_id = id.clone();
			KomgaSseEvent::from_payload(
				"BookAdded",
				crate::sse::BookAdded {
					book_id: id.into(),
					series_id: series_id.into(),
					library_id: library_id.into(),
				},
			)
			.map(|event| (media_id, event))
		},
		KomgaCoreEvent::Other => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::sse::decode_event;
	use axum::{
		http::{header, StatusCode},
		response::IntoResponse,
	};

	fn core_event(json: &str) -> KomgaCoreEvent {
		serde_json::from_str(json).expect("valid core event")
	}

	#[test]
	fn created_media_is_encoded_with_the_komga_book_added_contract() {
		let event = core_event(
			r#"{
				"__typename": "CreatedMedia",
				"id": "book-1",
				"seriesId": "series-1",
				"libraryId": "library-1"
			}"#,
		);

		let (media_id, mapped) =
			map_core_event(event).expect("created media is mappable");
		assert_eq!(media_id, "book-1");
		assert_eq!(mapped.name, "BookAdded");

		let decoded = decode_event(Some(mapped.name), Some(&mapped.data))
			.expect("valid SSE payload");
		assert_eq!(
			decoded,
			KomgaEvent::BookAdded {
				book_id: "book-1".into(),
				series_id: "series-1".into(),
				library_id: "library-1".into(),
			}
		);
	}

	#[test]
	fn missing_library_is_ignored_without_fabricating_a_deletion() {
		let event = core_event(
			r#"{
				"__typename": "DiscoveredMissingLibrary",
				"id": "library-1"
			}"#,
		);

		assert!(map_core_event(event).is_none());
	}

	#[test]
	fn events_without_a_truthful_komga_equivalent_are_ignored() {
		let event = core_event(
			r#"{
				"__typename": "CreatedManySeries",
				"count": 2,
				"libraryId": "library-1"
			}"#,
		);

		assert!(map_core_event(event).is_none());
	}

	#[test]
	fn changed_events_use_the_komga_event_name_and_payload_shape() {
		let cases = [
			(
				KomgaEvent::SeriesChanged {
					series_id: "series-1".into(),
					library_id: "library-1".into(),
				},
				"SeriesChanged",
				serde_json::json!({
					"seriesId": "series-1",
					"libraryId": "library-1",
				}),
			),
			(
				KomgaEvent::BookChanged {
					book_id: "book-1".into(),
					series_id: "series-1".into(),
					library_id: "library-1".into(),
				},
				"BookChanged",
				serde_json::json!({
					"bookId": "book-1",
					"seriesId": "series-1",
					"libraryId": "library-1",
				}),
			),
			(
				KomgaEvent::ReadProgressChanged {
					book_id: "book-1".into(),
					user_id: "user-1".into(),
				},
				"ReadProgressChanged",
				serde_json::json!({
					"bookId": "book-1",
					"userId": "user-1",
				}),
			),
			(
				KomgaEvent::ReadProgressSeriesChanged {
					series_id: "series-1".into(),
					user_id: "user-1".into(),
				},
				"ReadProgressSeriesChanged",
				serde_json::json!({
					"seriesId": "series-1",
					"userId": "user-1",
				}),
			),
			(
				KomgaEvent::ReadProgressDeleted {
					book_id: "book-1".into(),
					user_id: "user-1".into(),
				},
				"ReadProgressDeleted",
				serde_json::json!({
					"bookId": "book-1",
					"userId": "user-1",
				}),
			),
			(
				KomgaEvent::ReadProgressSeriesDeleted {
					series_id: "series-1".into(),
					user_id: "user-1".into(),
				},
				"ReadProgressSeriesDeleted",
				serde_json::json!({
					"seriesId": "series-1",
					"userId": "user-1",
				}),
			),
			(
				KomgaEvent::ReadListChanged {
					read_list_id: "list-1".into(),
					book_ids: vec!["book-1".into(), "book-2".into()],
				},
				"ReadListChanged",
				serde_json::json!({
					"readListId": "list-1",
					"bookIds": ["book-1", "book-2"],
				}),
			),
			(
				KomgaEvent::CollectionChanged {
					collection_id: "collection-1".into(),
					series_ids: vec!["series-1".into(), "series-2".into()],
				},
				"CollectionChanged",
				serde_json::json!({
					"collectionId": "collection-1",
					"seriesIds": ["series-1", "series-2"],
				}),
			),
		];

		for (event, expected_name, expected_data) in cases {
			let (_, mapped) = map_komga_event(event).expect("event is mappable");
			assert_eq!(mapped.name, expected_name);
			assert_eq!(
				serde_json::from_str::<serde_json::Value>(&mapped.data)
					.expect("valid JSON payload"),
				expected_data
			);
			assert!(!mapped.data.contains("\"type\""));
		}
	}

	#[test]
	fn read_progress_events_are_private_to_the_originating_user() {
		let (visibility, _) = map_komga_event(KomgaEvent::ReadProgressChanged {
			book_id: "book-1".into(),
			user_id: "user-1".into(),
		})
		.expect("event is mappable");
		assert_eq!(visibility, EventVisibility::User("user-1".to_owned()));
		assert!(is_user_event_visible("user-1", "user-1"));
		assert!(!is_user_event_visible("user-1", "user-2"));
	}

	#[tokio::test]
	async fn response_declares_an_event_stream_content_type() {
		let response = Sse::new(stream::empty::<Result<Event, Infallible>>())
			.keep_alive(KeepAlive::new().interval(KEEP_ALIVE_INTERVAL))
			.into_response();

		assert_eq!(response.status(), StatusCode::OK);
		assert_eq!(
			response.headers()[header::CONTENT_TYPE],
			"text/event-stream"
		);
	}
}
