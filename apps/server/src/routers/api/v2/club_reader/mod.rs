#[cfg(feature = "readium")]
mod admin;
#[cfg(feature = "readium")]
mod events;
#[cfg(feature = "readium")]
mod media;
#[cfg(feature = "readium")]
mod messages;
#[cfg(feature = "readium")]
mod reader;
// The guards are shared; the capability helpers serve only the readium routes.
#[cfg_attr(not(feature = "readium"), allow(dead_code))]
mod security;
#[cfg(feature = "readium")]
mod types;

#[cfg(feature = "readium")]
use std::sync::Arc;

use axum::routing::any;
use axum::{middleware, Router};
#[cfg(feature = "readium")]
use axum::{
	routing::{delete, get, patch, post, put},
	Extension,
};

#[cfg(feature = "readium")]
use crate::middleware::auth::auth_middleware;
use crate::{config::state::AppState, errors::APIError};

pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	// One notification hub per mounted app, shared by guest and management
	// writes and the guest event streams.
	#[cfg(feature = "readium")]
	let reader_events = Arc::new(events::ReaderEvents::default());

	#[cfg(feature = "readium")]
	let guest = Router::new()
		.route("/redeem", post(reader::redeem))
		.route("/sessions/{session_id}", get(reader::snapshot))
		.route("/sessions/{session_id}/me", patch(reader::update_viewer))
		.route("/sessions/{session_id}/events", get(events::stream))
		.route(
			"/sessions/{session_id}/messages",
			get(messages::list).post(messages::create),
		)
		.route(
			"/sessions/{session_id}/messages/{message_id}",
			patch(messages::update).delete(messages::delete),
		)
		.route(
			"/sessions/{session_id}/books/{book_id}/progress",
			put(reader::update_progress),
		)
		.route(
			"/sessions/{session_id}/books/{book_id}/annotations",
			post(reader::create_annotation),
		)
		.route(
			"/sessions/{session_id}/books/{book_id}/annotations/{annotation_id}",
			patch(reader::update_annotation).delete(reader::delete_annotation),
		)
		.route(
			"/sessions/{session_id}/books/{book_id}/manifest.json",
			get(media::manifest),
		)
		.route(
			"/sessions/{session_id}/books/{book_id}/positions.json",
			get(media::positions),
		)
		.route(
			"/sessions/{session_id}/books/{book_id}/resource/{*path}",
			get(media::resource),
		)
		.route(
			"/sessions/{session_id}/books/{book_id}/page/{page}",
			get(media::page),
		)
		.route(
			"/sessions/{session_id}/books/{book_id}/audio/{track_index}",
			get(media::audio_track),
		)
		// A router fallback would be dropped once `/api/v2` is nested into the
		// app router, so unmatched guest paths use an explicit catch-all route.
		.route("/{*rest}", any(not_found))
		.layer(Extension(reader_events.clone()))
		.layer(middleware::from_fn_with_state(
			app_state.clone(),
			security::origin_guard,
		))
		.layer(middleware::from_fn(security::guest_response_guard));

	#[cfg(not(feature = "readium"))]
	let guest = Router::new()
		.route("/{*rest}", any(feature_unavailable))
		.layer(middleware::from_fn_with_state(
			app_state.clone(),
			security::origin_guard,
		))
		.layer(middleware::from_fn(security::guest_response_guard));

	#[cfg(feature = "readium")]
	let management = Router::new()
		.route(
			"/book-clubs/{club_id}/reader-sessions",
			get(admin::list).post(admin::create),
		)
		.route(
			"/book-clubs/{club_id}/reader-sessions/{session_id}",
			delete(admin::close),
		)
		.route(
			"/book-clubs/{club_id}/reader-sessions/{session_id}/publish",
			post(admin::publish),
		)
		.route(
			"/book-clubs/{club_id}/reader-sessions/{session_id}/advance",
			post(admin::advance),
		)
		.route(
			"/book-clubs/{club_id}/reader-sessions/{session_id}/participants",
			post(admin::create_participant),
		)
		.route(
			"/book-clubs/{club_id}/reader-sessions/{session_id}/join",
			post(admin::join),
		)
		.route(
			"/book-clubs/{club_id}/reader-sessions/{session_id}/participants/{participant_id}/rotate",
			post(admin::rotate),
		)
		.route(
			"/book-clubs/{club_id}/reader-sessions/{session_id}/participants/{participant_id}",
			delete(admin::revoke_participant),
		)
		.route(
			"/book-clubs/{club_id}/reader-sessions/{session_id}/messages",
			get(messages::organizer_list),
		)
		.route(
			"/book-clubs/{club_id}/reader-sessions/{session_id}/messages/{message_id}",
			delete(messages::moderate),
		)
		.layer(Extension(reader_events))
		.layer(middleware::from_fn_with_state(
			app_state.clone(),
			security::origin_guard,
		))
		.layer(middleware::from_fn_with_state(
			app_state.clone(),
			auth_middleware,
		));

	// `management` is merged into the whole `/api/v2` router, so it must never
	// carry a fallback; unavailable reader-session paths are routed explicitly.
	#[cfg(not(feature = "readium"))]
	let management = Router::new()
		.route(
			"/book-clubs/{club_id}/reader-sessions",
			any(feature_unavailable),
		)
		.route(
			"/book-clubs/{club_id}/reader-sessions/{*rest}",
			any(feature_unavailable),
		)
		.layer(middleware::from_fn_with_state(
			app_state.clone(),
			security::origin_guard,
		));

	Router::new().nest("/club-reader", guest).merge(management)
}

#[cfg(feature = "readium")]
async fn not_found() -> APIError {
	APIError::NotFound("not found".to_string())
}

#[cfg(not(feature = "readium"))]
async fn feature_unavailable() -> APIError {
	APIError::NotFound("book-club reader is unavailable".to_string())
}
