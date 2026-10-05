//! Lissen 1.12.9's genre and narrator browsers on a real server:
//! `GET /api/libraries/{id}/stats` (`genresWithCount`),
//! `GET /api/libraries/{id}/narrators`, and the `genres.`/`narrators.` item
//! filters, all scoped to what the user can see.

use base64::{engine::general_purpose::STANDARD, Engine};
use models::{
	domain::audio::AudioChapterSource,
	entity::{library_exclusion, media_audio, media_audio_track, media_metadata, user},
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde_json::{json, Value};
use tests::fake_data;

use super::mount::app;
use crate::common::TestApp;

struct Book<'a> {
	id: &'a str,
	series_id: &'a str,
	extension: &'a str,
	size: i64,
	title: Option<&'a str>,
	writers: &'a str,
	genres: &'a str,
	narrators: &'a str,
	/// Track durations in milliseconds; empty means no probe row.
	tracks: &'a [i64],
}

async fn insert_book(app: &TestApp, book: Book<'_>) {
	let conn = app.conn();
	fake_data::Media {
		series_id: book.series_id.to_owned(),
		id: Some(book.id.to_owned()),
		name: Some(format!("{} file", book.id)),
		extension: Some(book.extension.to_owned()),
		size: Some(book.size),
		..Default::default()
	}
	.insert(conn)
	.await;
	media_metadata::ActiveModel {
		media_id: Set(Some(book.id.to_owned())),
		title: Set(book.title.map(str::to_owned)),
		writers: Set(Some(book.writers.to_owned())),
		genres: Set(Some(book.genres.to_owned())),
		narrators: Set(Some(book.narrators.to_owned())),
		..Default::default()
	}
	.insert(conn)
	.await
	.expect("metadata");
	if book.tracks.is_empty() {
		return;
	}
	media_audio::ActiveModel {
		media_id: Set(book.id.to_owned()),
		duration_ms: Set(book.tracks.iter().sum()),
		codec: Set("aac".to_owned()),
		sample_rate: Set(None),
		channels: Set(None),
		bitrate: Set(None),
		chapter_source: Set(AudioChapterSource::PerTrack),
	}
	.insert(conn)
	.await
	.expect("audio");
	let mut offset = 0;
	for (index, duration_ms) in book.tracks.iter().enumerate() {
		media_audio_track::ActiveModel {
			id: Set(format!("{}-{index}", book.id)),
			media_id: Set(book.id.to_owned()),
			index: Set(index as i32),
			path: Set(format!("/tmp/{}/{index}.m4b", book.id)),
			duration_ms: Set(*duration_ms),
			start_offset_ms: Set(offset),
			byte_size: Set(1),
			mime: Set("audio/mp4".to_owned()),
		}
		.insert(conn)
		.await
		.expect("track");
		offset += duration_ms;
	}
}

const SFF: &str = "Science Fiction & Fantasy";
const OLGA: &str = "Ольга Ü. Smith";

/// One visible audiobook library and one the user is excluded from. The
/// visible one also holds an EPUB, which is not an ABS library item, and a
/// genre (`History Extra`) whose name contains another (`History`).
async fn fixture() -> TestApp {
	let app = app().await;
	let token = app.create_initial_account().await;
	*app.access_token.write().await = Some(token);
	let conn = app.conn();

	for (library, series) in [("audio", "audio-series"), ("hidden", "hidden-series")] {
		fake_data::Library {
			id: Some(library.to_owned()),
			name: Some(library.to_owned()),
			..Default::default()
		}
		.insert(conn)
		.await;
		fake_data::Series {
			id: Some(series.to_owned()),
			name: Some(series.to_owned()),
			library_id: Some(library.to_owned()),
			..Default::default()
		}
		.insert(conn)
		.await;
	}

	insert_book(
		&app,
		Book {
			id: "a1",
			series_id: "audio-series",
			extension: "m4b",
			size: 300,
			title: Some("Alpha"),
			writers: "Author A",
			genres: &format!("{SFF}, History"),
			narrators: &format!("{OLGA}, Narrator 10"),
			tracks: &[1_800_000, 1_800_000],
		},
	)
	.await;
	insert_book(
		&app,
		Book {
			id: "a2",
			series_id: "audio-series",
			extension: "m4b",
			size: 500,
			title: Some("Beta"),
			writers: "Author A, Author B",
			genres: SFF,
			narrators: "Narrator 2",
			tracks: &[7_200_000],
		},
	)
	.await;
	insert_book(
		&app,
		Book {
			id: "a3",
			series_id: "audio-series",
			extension: "mp3",
			size: 100,
			title: None,
			writers: "Author C",
			genres: "Mystery, History Extra",
			narrators: OLGA,
			tracks: &[],
		},
	)
	.await;
	insert_book(
		&app,
		Book {
			id: "e1",
			series_id: "audio-series",
			extension: "epub",
			size: 9_000,
			title: Some("Ebook"),
			writers: "Author E",
			genres: SFF,
			narrators: "Ebook Voice",
			tracks: &[],
		},
	)
	.await;
	insert_book(
		&app,
		Book {
			id: "h1",
			series_id: "hidden-series",
			extension: "m4b",
			size: 700,
			title: Some("Hidden"),
			writers: "Author A",
			genres: SFF,
			narrators: &format!("{OLGA}, Hidden Voice"),
			tracks: &[60_000],
		},
	)
	.await;

	let admin = user::Entity::find()
		.filter(user::Column::Username.eq("initial-server-admin"))
		.one(conn)
		.await
		.expect("user query")
		.expect("admin");
	library_exclusion::ActiveModel {
		user_id: Set(admin.id),
		library_id: Set("hidden".to_owned()),
		..Default::default()
	}
	.insert(conn)
	.await
	.expect("exclusion");

	app
}

/// The filter exactly as Lissen builds it — `key.` + padded standard base64
/// of the UTF-8 name (`common/api/EncodeLibraryFilter.kt`) — then
/// percent-encoded the way OkHttp encodes a `@Query` value.
fn lissen_filter(key: &str, value: &str) -> String {
	urlencoding::encode(&format!("{key}.{}", STANDARD.encode(value))).into_owned()
}

async fn item_ids(app: &TestApp, library_id: &str, filter: &str) -> (i64, Vec<String>) {
	let response = app
		.get(&format!(
			"/api/libraries/{library_id}/items?minified=1&limit=50&page=0&filter={filter}"
		))
		.await;
	response.assert_status_ok();
	let body: Value = response.json();
	let mut ids = body["results"]
		.as_array()
		.expect("results")
		.iter()
		.map(|item| item["id"].as_str().expect("id").to_owned())
		.collect::<Vec<_>>();
	ids.sort();
	(body["total"].as_i64().expect("total"), ids)
}

#[tokio::test]
async fn abs_library_stats_count_only_visible_audio() {
	let app = fixture().await;

	let response = app.get("/api/libraries/audio/stats").await;
	response.assert_status_ok();
	let stats: Value = response.json();

	assert_eq!(
		stats["genresWithCount"],
		json!([
			{ "genre": SFF, "count": 2 },
			{ "genre": "History", "count": 1 },
			{ "genre": "History Extra", "count": 1 },
			{ "genre": "Mystery", "count": 1 },
		])
	);
	assert_eq!(stats["totalGenres"], 4);
	assert_eq!(stats["totalItems"], 3);
	assert_eq!(stats["totalSize"], 900);
	assert_eq!(stats["totalDuration"], 10_800.0);
	assert_eq!(stats["numAudioTracks"], 3);
	assert_eq!(
		stats["largestItems"],
		json!([
			{ "id": "a2", "title": "Beta", "size": 500 },
			{ "id": "a1", "title": "Alpha", "size": 300 },
			{ "id": "a3", "title": "a3 file", "size": 100 },
		])
	);
	assert_eq!(
		stats["longestItems"],
		json!([
			{ "id": "a2", "title": "Beta", "duration": 7_200.0 },
			{ "id": "a1", "title": "Alpha", "duration": 3_600.0 },
			{ "id": "a3", "title": "a3 file", "duration": 0.0 },
		])
	);
	assert_eq!(stats["totalAuthors"], 3);
	let authors = stats["authorsWithCount"].as_array().expect("authors");
	assert_eq!(
		authors
			.iter()
			.map(|author| (author["name"].clone(), author["count"].clone()))
			.collect::<Vec<_>>(),
		vec![
			(json!("Author A"), json!(2)),
			(json!("Author B"), json!(1)),
			(json!("Author C"), json!(1)),
		]
	);
	assert!(authors
		.iter()
		.all(|author| author["id"].as_str().is_some_and(|id| !id.is_empty())));

	// Every key of abs-ref's book-library branch is present, and no other.
	let mut keys = stats
		.as_object()
		.expect("object")
		.keys()
		.map(String::as_str)
		.collect::<Vec<_>>();
	keys.sort_unstable();
	let mut expected = vec![
		"largestItems",
		"totalAuthors",
		"authorsWithCount",
		"totalGenres",
		"genresWithCount",
		"totalItems",
		"longestItems",
		"totalSize",
		"totalDuration",
		"numAudioTracks",
	];
	expected.sort_unstable();
	assert_eq!(keys, expected);

	// A library the user is excluded from does not exist for them.
	app.get("/api/libraries/hidden/stats")
		.await
		.assert_status_not_found();
}

#[tokio::test]
async fn abs_library_narrators_are_visible_credits_in_natural_order() {
	let app = fixture().await;

	let response = app.get("/api/libraries/audio/narrators").await;
	response.assert_status_ok();
	let body: Value = response.json();
	assert_eq!(
		body,
		json!({
			"narrators": [
				{
					"id": urlencoding::encode(&STANDARD.encode("Narrator 2")),
					"name": "Narrator 2",
					"numBooks": 1,
				},
				{
					"id": urlencoding::encode(&STANDARD.encode("Narrator 10")),
					"name": "Narrator 10",
					"numBooks": 1,
				},
				{
					"id": "0J7Qu9GM0LPQsCDDnC4gU21pdGg%3D",
					"name": OLGA,
					"numBooks": 2,
				},
			]
		})
	);

	app.get("/api/libraries/hidden/narrators")
		.await
		.assert_status_not_found();
}

#[tokio::test]
async fn abs_genre_and_narrator_filters_round_trip_lissens_encoding() {
	let app = fixture().await;

	// A name with spaces and `&`, padded base64 and percent-encoded `=`.
	assert_eq!(
		item_ids(&app, "audio", &lissen_filter("genres", SFF)).await,
		(2, vec!["a1".to_owned(), "a2".to_owned()])
	);
	// Exact match: `History Extra` contains `History` but is not it.
	assert_eq!(
		item_ids(&app, "audio", &lissen_filter("genres", "History")).await,
		(1, vec!["a1".to_owned()])
	);
	// Cyrillic and Latin-1 in the name, and a credit shared with a book in
	// a library the user cannot see.
	assert_eq!(
		item_ids(&app, "audio", &lissen_filter("narrators", OLGA)).await,
		(2, vec!["a1".to_owned(), "a3".to_owned()])
	);
	// The `id` abs-ref hands out is already `encodeURIComponent`ed; a client
	// that filters by it percent-encodes it again, and abs-ref's `decode`
	// undoes both.
	let narrators: Value = app.get("/api/libraries/audio/narrators").await.json();
	let olga_id = narrators["narrators"][2]["id"].as_str().expect("id");
	assert_eq!(
		item_ids(
			&app,
			"audio",
			&urlencoding::encode(&format!("narrators.{olga_id}"))
		)
		.await,
		(2, vec!["a1".to_owned(), "a3".to_owned()])
	);
	// A narrator credited only in the hidden library, or only on an EPUB,
	// selects nothing.
	assert_eq!(
		item_ids(&app, "audio", &lissen_filter("narrators", "Hidden Voice")).await,
		(0, Vec::new())
	);
	assert_eq!(
		item_ids(&app, "audio", &lissen_filter("narrators", "Ebook Voice")).await,
		(0, Vec::new())
	);
	// An unsupported group is still no filter at all.
	assert_eq!(
		item_ids(&app, "audio", &lissen_filter("tags", "anything")).await,
		(3, vec!["a1".to_owned(), "a2".to_owned(), "a3".to_owned()])
	);
	app.get(&format!(
		"/api/libraries/hidden/items?minified=1&filter={}",
		lissen_filter("genres", SFF)
	))
	.await
	.assert_status_not_found();
}
