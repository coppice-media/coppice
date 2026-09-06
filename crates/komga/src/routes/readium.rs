use std::{
	path::{Component, PathBuf},
	sync::Arc,
};

use crate::{
	sse::KomgaEvent, R2Device, R2Location, R2Locator, R2LocatorText, R2Progression,
};
use axum::{
	body::Body,
	extract::Path,
	http::HeaderMap,
	response::{IntoResponse, Response},
	routing::get,
	Extension, Json, Router,
};
use axum_extra::extract::Host;
use chrono::Utc;
use models::txn::begin_write;
use models::{
	domain::reading_state::{Position, ProtocolUpdate, Publication, SourceProtocol},
	entity::{device, media, series, user::AuthUser},
	services::{
		reading_progress::{upsert_reading_session, NormalizedProgression},
		reading_state,
	},
	shared::{
		enums::DeviceKind,
		readium::{ReadiumLocation, ReadiumLocator, ReadiumText},
	},
};
use sea_orm::{prelude::*, ActiveValue::Set};
use stump_auth::AuthContext;

use super::response::{cached_bytes, cached_json};
use super::{KomgaBackend, KomgaEvents};
use crate::errors::{APIError, APIResult};
/// Readium endpoints used by Komelia's EPUB reader.
///
/// Authentication is applied by the parent Komga router. The `.json` aliases
/// are retained because the Stump Readium manifest generator emits those links,
/// while Komelia starts the reader with the extensionless endpoint.
pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::<S>::new()
		.route("/api/v1/books/{id}/manifest", get(get_manifest))
		.route("/api/v1/books/{id}/manifest.json", get(get_manifest))
		.route("/api/v1/books/{id}/positions", get(get_positions))
		.route("/api/v1/books/{id}/positions.json", get(get_positions))
		.route("/api/v1/books/{id}/resource/{*path}", get(get_resource))
		.route(
			"/api/v1/books/{id}/progression",
			get(get_progression).put(put_progression),
		)
}

async fn find_visible_media(
	conn: &DatabaseConnection,
	user: &AuthUser,
	id: &str,
) -> APIResult<media::Model> {
	media::Entity::find_for_user(user)
		.filter(media::Column::Id.eq(id.to_owned()))
		.one(conn)
		.await?
		.ok_or_else(|| APIError::NotFound("Book not found".to_string()))
}

async fn find_visible_epub(
	conn: &DatabaseConnection,
	user: &AuthUser,
	id: &str,
) -> APIResult<media::Model> {
	let book = find_visible_media(conn, user, id).await?;
	if !book.extension.eq_ignore_ascii_case("epub") {
		return Err(APIError::BadRequest(format!(
			"Book media type '{}' is not compatible with EPUB Readium routes",
			book.extension
		)));
	}
	Ok(book)
}

fn epub_base_url(host: &str, scheme: &str, id: &str) -> String {
	format!("{scheme}://{host}/api/v1/books/{id}")
}

fn epub_failure(error: impl std::fmt::Debug, id: &str, kind: &str) -> APIError {
	tracing::warn!(?error, media_id = id, "Failed to generate EPUB {kind}");
	APIError::NotFound(format!("Book {kind} not found"))
}
async fn get_manifest(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Host(host): Host,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	let book = find_visible_epub(ctx.conn(), &user, &id).await?;
	let scheme = headers
		.get("x-forwarded-proto")
		.and_then(|value| value.to_str().ok())
		.unwrap_or("http");
	let mut manifest = ctx
		.readium_manifest(book.path, epub_base_url(&host, scheme, &id))
		.await
		.map_err(|error| epub_failure(error, &id, "manifest"))?;
	normalize_manifest_for_komga_client(&mut manifest);

	cached_json(&headers, &manifest)
}

/// Shape the Readium manifest the way komga-client 0.11 (Komelia) deserializes
/// `WPPublication`: single-string link `rel`, `metadata.published` as a
/// calendar date, and contributor fields (`publisher`, `author`, …) as lists.
/// Stump's native manifest routes are untouched; only the Komga alias is
/// narrowed to that client contract.
fn normalize_manifest_for_komga_client(manifest: &mut serde_json::Value) {
	flatten_link_rels(manifest);
	let Some(serde_json::Value::Object(metadata)) = manifest.get_mut("metadata") else {
		return;
	};
	truncate_published_to_date(metadata);
	for field in WP_METADATA_LIST_FIELDS {
		if let Some(value) = metadata.get_mut(*field) {
			if value.is_string() {
				*value = serde_json::Value::Array(vec![value.take()]);
			}
		}
	}
}

/// `WPMetadata` fields typed `List<String>` in komga-client 0.11.
const WP_METADATA_LIST_FIELDS: &[&str] = &[
	"author",
	"translator",
	"editor",
	"artist",
	"illustrator",
	"letterer",
	"penciler",
	"colorist",
	"inker",
	"contributor",
	"publisher",
	"subject",
];

/// `metadata.published` is `LocalDate` for the client; an EPUB `dc:date` is
/// often a full timestamp, which it rejects with "unparsed text found at
/// index 10". Keep the calendar date only.
fn truncate_published_to_date(metadata: &mut serde_json::Map<String, serde_json::Value>) {
	let Some(serde_json::Value::String(published)) = metadata.get_mut("published") else {
		return;
	};
	let is_date_prefix = published.len() >= 10
		&& published
			.bytes()
			.take(10)
			.enumerate()
			.all(|(index, byte)| match index {
				4 | 7 => byte == b'-',
				_ => byte.is_ascii_digit(),
			});
	if is_date_prefix {
		published.truncate(10);
	}
}

fn flatten_link_rels(value: &mut serde_json::Value) {
	match value {
		serde_json::Value::Object(map) => {
			if let Some(rel @ serde_json::Value::Array(_)) = map.get_mut("rel") {
				let first = rel
					.as_array()
					.and_then(|rels| rels.first().cloned())
					.unwrap_or(serde_json::Value::Null);
				*rel = first;
			}
			map.values_mut().for_each(flatten_link_rels);
		},
		serde_json::Value::Array(items) => items.iter_mut().for_each(flatten_link_rels),
		_ => {},
	}
}

async fn get_positions(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Host(host): Host,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	let book = find_visible_epub(ctx.conn(), &user, &id).await?;
	let scheme = headers
		.get("x-forwarded-proto")
		.and_then(|value| value.to_str().ok())
		.unwrap_or("http");
	let positions = ctx
		.readium_positions(book.path, epub_base_url(&host, scheme, &id))
		.await
		.map_err(|error| epub_failure(error, &id, "positions"))?;

	cached_json(&headers, &positions)
}

fn normalize_resource_path(raw: &str) -> APIResult<PathBuf> {
	if raw.starts_with('/') {
		return Err(APIError::BadRequest(
			"EPUB resource path must be relative".to_string(),
		));
	}
	let path = raw;
	let path = path.split_once('#').map_or(path, |(path, _)| path);
	if path.is_empty() {
		return Err(APIError::BadRequest(
			"EPUB resource path cannot be empty".to_string(),
		));
	}
	if path.contains('\\') || path.as_bytes().contains(&0) {
		return Err(APIError::BadRequest(
			"Invalid EPUB resource path".to_string(),
		));
	}

	let path_buf = PathBuf::from(path);
	if path_buf
		.components()
		.any(|component| !matches!(component, Component::Normal(_)))
	{
		return Err(APIError::BadRequest(
			"EPUB resource path must stay within the book".to_string(),
		));
	}

	Ok(path_buf)
}

async fn get_resource(
	Path((id, raw_path)): Path<(String, String)>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	let book = find_visible_epub(ctx.conn(), &user, &id).await?;
	let resource_path = normalize_resource_path(&raw_path)?;
	let resource = ctx.readium_resource(book.path, resource_path).await?;

	cached_bytes(&headers, &resource.content_type, resource.data)
}

fn model_locator_from_r2(locator: &R2Locator) -> ReadiumLocator {
	ReadiumLocator {
		chapter_title: String::new(),
		href: locator.href.clone(),
		title: locator.title.clone(),
		locations: locator.locations.as_ref().map(|locations| ReadiumLocation {
			fragments: locations.fragment.clone(),
			progression: locations
				.progression
				.map(|value| Decimal::try_from(value as f64).unwrap_or_default()),
			position: locations.position,
			total_progression: locations
				.total_progression
				.map(|value| Decimal::try_from(value as f64).unwrap_or_default()),
			css_selector: None,
			partial_cfi: None,
		}),
		text: locator.text.as_ref().map(|text| ReadiumText {
			after: text.after.clone(),
			before: text.before.clone(),
			highlight: text.highlight.clone(),
		}),
		kobo_span: locator.kobo_span.clone(),
		r#type: locator.r#type.clone(),
	}
}

fn r2_locator_from_model(locator: &ReadiumLocator) -> R2Locator {
	R2Locator {
		href: locator.href.clone(),
		r#type: locator.r#type.clone(),
		title: locator.title.clone(),
		locations: locator.locations.as_ref().map(|locations| R2Location {
			fragment: locations.fragments.clone(),
			position: locations.position,
			progression: locations
				.progression
				.and_then(|value| value.to_string().parse::<f32>().ok()),
			total_progression: locations
				.total_progression
				.and_then(|value| value.to_string().parse::<f32>().ok()),
		}),
		text: locator.text.as_ref().map(|text| R2LocatorText {
			after: text.after.clone(),
			before: text.before.clone(),
			highlight: text.highlight.clone(),
		}),
		kobo_span: locator.kobo_span.clone(),
	}
}

async fn get_progression(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	find_visible_media(ctx.conn(), &user, &id).await?;
	let Some(head) = reading_state::head(ctx.conn(), &user.id, &id).await? else {
		return Ok(axum::http::StatusCode::NO_CONTENT.into_response());
	};

	let device = match head.source_device_id {
		Some(id) => device::Entity::find_by_id(id.clone())
			.one(ctx.conn())
			.await?
			.map(|device| R2Device {
				id: device.id,
				name: device.name,
			})
			.unwrap_or(R2Device {
				id,
				name: String::new(),
			}),
		None => R2Device {
			id: String::new(),
			name: String::new(),
		},
	};
	let modified = head.updated_at.with_timezone(&Utc);
	let locator = head
		.locator
		.as_ref()
		.map(r2_locator_from_model)
		.unwrap_or_default();

	Ok(Json(R2Progression {
		modified,
		device,
		locator,
	})
	.into_response())
}

fn validate_progression(input: &R2Progression, pages: i32) -> APIResult<()> {
	let locations = input.locator.locations.as_ref();
	if let Some(position) = locations.and_then(|locations| locations.position) {
		if pages > -1 && (position < 1 || position > pages) {
			return Err(APIError::BadRequest(format!(
				"Position {position} is out of bounds (1-{pages})"
			)));
		}
		if pages <= -1 && position < 1 {
			return Err(APIError::BadRequest(
				"Position must be one-based".to_string(),
			));
		}
	}

	for (name, value) in [
		(
			"progression",
			locations.and_then(|locations| locations.progression),
		),
		(
			"totalProgression",
			locations.and_then(|locations| locations.total_progression),
		),
	] {
		if let Some(value) = value {
			if !value.is_finite() || !(0.0..=1.0).contains(&value) {
				return Err(APIError::BadRequest(format!(
					"{name} must be between 0 and 1"
				)));
			}
		}
	}
	Ok(())
}

async fn put_progression(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<KomgaEvents>,
	Json(input): Json<R2Progression>,
) -> APIResult<axum::http::StatusCode> {
	let user = auth.user();
	let conn = ctx.conn();
	let book = find_visible_media(conn, &user, &id).await?;
	let parent_series = if let Some(series_id) = book.series_id.as_deref() {
		series::Entity::find_for_user(&user)
			.filter(series::Column::Id.eq(series_id))
			.one(conn)
			.await?
	} else {
		None
	};
	let event_book_id = book.id.clone();
	let event_series_id = book.series_id.clone().unwrap_or_default();
	let event_library_id = parent_series
		.and_then(|series| series.library_id)
		.unwrap_or_default();
	validate_progression(&input, book.pages)?;

	let page = input
		.locator
		.locations
		.as_ref()
		.and_then(|locations| locations.position);
	let total_progression = input
		.locator
		.locations
		.as_ref()
		.and_then(|locations| locations.total_progression);
	let percentage = total_progression
		.map(|value| Decimal::try_from(value as f64))
		.transpose()
		.map_err(|error| APIError::BadRequest(error.to_string()))?;
	let did_complete = page.is_some_and(|page| book.pages > -1 && page >= book.pages)
		|| total_progression.is_some_and(|progression| progression >= 1.0);
	let device_id = if input.device.id.is_empty() && input.device.name.is_empty() {
		None
	} else {
		Some(input.device.id.clone())
	};
	let locator = model_locator_from_r2(&input.locator);
	let progression = NormalizedProgression {
		page,
		locator: Some(locator.clone()),
		percentage,
		elapsed_seconds_delta: None,
		did_complete,
		device_id: device_id.clone(),
		reset_elapsed_seconds: false,
	};
	let raw_payload = serde_json::to_value(&input)
		.map_err(|error| APIError::InternalServerError(error.to_string()))?;
	let head_update = ProtocolUpdate {
		protocol: SourceProtocol::Komga,
		device_id: device_id.clone(),
		updated_at: Some(input.modified),
		position: Position::Locator(locator),
		progression: total_progression.map(f64::from),
		completed: did_complete.then_some(true),
		raw_payload,
	};

	let txn = begin_write(conn).await?;
	if reading_state::head(&txn, &user.id, &id)
		.await?
		.is_some_and(|head| head.updated_at > input.modified)
	{
		return Err(APIError::Conflict(
			"Progression timestamp is older than existing session".to_string(),
		));
	}

	if let Some(device_id) = device_id {
		let exists = device::Entity::find_by_id(device_id.clone())
			.one(&txn)
			.await?
			.is_some();
		if !exists {
			// R2 progression clients register themselves by the device they report;
			// Komelia is the Komga-profile client that writes progression.
			device::ActiveModel {
				id: Set(device_id),
				user_id: Set(user.id.clone()),
				name: Set(input.device.name.clone()),
				kind: Set(DeviceKind::Komelia),
				last_seen_at: Set(Some(Utc::now().into())),
				..Default::default()
			}
			.insert(&txn)
			.await?;
		}
	}

	upsert_reading_session(&txn, &user, &id, progression).await?;
	reading_state::apply(&txn, &user.id, Publication::from(&book), head_update).await?;
	txn.commit().await?;
	events.send(KomgaEvent::ReadProgressChanged {
		book_id: event_book_id.clone().into(),
		user_id: user.id.clone().into(),
	});
	events.send(KomgaEvent::ReadProgressSeriesChanged {
		series_id: event_series_id.clone().into(),
		user_id: user.id.clone().into(),
	});
	events.send(KomgaEvent::BookChanged {
		book_id: event_book_id.into(),
		series_id: event_series_id.clone().into(),
		library_id: event_library_id.clone().into(),
	});
	events.send(KomgaEvent::SeriesChanged {
		series_id: event_series_id.into(),
		library_id: event_library_id.into(),
	});
	Ok(axum::http::StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
	use super::*;
	use chrono::DateTime;

	#[test]
	fn resource_paths_keep_nested_segments() {
		assert_eq!(
			normalize_resource_path("OEBPS/Images/cover.png").unwrap(),
			PathBuf::from("OEBPS/Images/cover.png")
		);
	}

	#[test]
	fn resource_paths_reject_traversal_and_absolute_paths() {
		for path in [
			"../META-INF/container.xml",
			"OEBPS/../META-INF/container.xml",
			"/etc/passwd",
		] {
			assert!(normalize_resource_path(path).is_err(), "{path}");
		}
	}

	#[test]
	fn manifest_base_uses_request_origin_and_book_route() {
		assert_eq!(
			epub_base_url("reader.example:8443", "https", "book-1"),
			"https://reader.example:8443/api/v1/books/book-1"
		);
	}

	#[test]
	fn locator_round_trip_preserves_readium_fields() {
		let locator = R2Locator {
			href: "OEBPS/chapter.xhtml".to_string(),
			r#type: "application/xhtml+xml".to_string(),
			title: Some("Chapter".to_string()),
			locations: Some(R2Location {
				fragment: Some(vec!["epubcfi(/6/2)".to_string()]),
				position: Some(3),
				progression: Some(0.25),
				total_progression: Some(0.5),
			}),
			text: Some(R2LocatorText {
				before: Some("before".to_string()),
				highlight: Some("highlight".to_string()),
				after: Some("after".to_string()),
			}),
			kobo_span: Some("kobo-span".to_string()),
		};
		let round_tripped = r2_locator_from_model(&model_locator_from_r2(&locator));
		assert_eq!(round_tripped.href, locator.href);
		assert_eq!(round_tripped.r#type, locator.r#type);
		assert_eq!(round_tripped.title, locator.title);
		assert_eq!(round_tripped.locations, locator.locations);
		assert_eq!(round_tripped.text, locator.text);
		assert_eq!(round_tripped.kobo_span, locator.kobo_span);
	}

	#[test]
	fn progression_values_and_positions_are_bounded() {
		let input = R2Progression {
			modified: DateTime::parse_from_rfc3339("2026-09-02T16:17:44Z")
				.unwrap()
				.with_timezone(&Utc),
			device: R2Device {
				id: "device".to_string(),
				name: "Komelia".to_string(),
			},
			locator: R2Locator {
				locations: Some(R2Location {
					fragment: None,
					position: Some(2),
					progression: Some(0.5),
					total_progression: None,
				}),
				..Default::default()
			},
		};
		assert!(validate_progression(&input, 2).is_ok());
		assert!(validate_progression(&input, 1).is_err());
	}
	#[test]
	fn manifest_link_rels_are_single_strings() {
		let mut value = serde_json::json!({
			"links": [{"href": "m", "rel": ["self"]}, {"href": "p"}],
			"readingOrder": [{"href": "c", "rel": ["contents", "alternate"]}],
			"toc": [{"href": "t", "children": [{"href": "u", "rel": ["x"]}]}]
		});
		flatten_link_rels(&mut value);
		assert_eq!(value["links"][0]["rel"], "self");
		assert!(value["links"][1].get("rel").is_none());
		assert_eq!(value["readingOrder"][0]["rel"], "contents");
		assert_eq!(value["toc"][0]["children"][0]["rel"], "x");
	}
	#[test]
	fn manifest_metadata_matches_komga_client_types() {
		let mut value = serde_json::json!({
			"metadata": {
				"published": "1994-07-14T10:00:00+00:00",
				"publisher": "Tor Books",
				"author": ["Orson Scott Card"],
				"subject": "Science-Fiction"
			},
			"links": [{"href": "m", "rel": ["self"]}]
		});
		normalize_manifest_for_komga_client(&mut value);
		assert_eq!(value["metadata"]["published"], "1994-07-14");
		assert_eq!(
			value["metadata"]["publisher"],
			serde_json::json!(["Tor Books"])
		);
		assert_eq!(
			value["metadata"]["author"],
			serde_json::json!(["Orson Scott Card"])
		);
		assert_eq!(
			value["metadata"]["subject"],
			serde_json::json!(["Science-Fiction"])
		);
		assert_eq!(value["links"][0]["rel"], "self");

		let mut odd = serde_json::json!({"metadata": {"published": "circa 1994"}});
		normalize_manifest_for_komga_client(&mut odd);
		assert_eq!(odd["metadata"]["published"], "circa 1994");
		let mut none = serde_json::json!({"metadata": {}});
		normalize_manifest_for_komga_client(&mut none);
		assert!(none["metadata"].get("published").is_none());
	}
}
