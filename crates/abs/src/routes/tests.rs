//! End-to-end route tests: a request goes through the composed router with a
//! resolved user, against an in-memory database and the stub backend, and the
//! assertions are on the serialised JSON a client would decode.
//!
//! The reference for every expectation is the abs-ref 2.36.0 capture named in
//! the test, or the Lissen model that reads the field.

use axum::http::StatusCode;
use models::shared::enums::LibraryType;
use serde_json::json;

use crate::{
	model::{AbsImage, AbsProgress},
	routes::AbsBackend,
	test_support::{
		auth_user, db, library_of_type, metadata, one_track_audio, request, request_full,
		series_with_files, two_track_audio, TestBackend,
	},
};

/// Registering a literal segment and a `{param}` at the same position is a
/// `matchit` conflict that only surfaces when the router is built, and this
/// profile has exactly that shape: `/items/batch/get` beside
/// `/items/{item_id}`.
#[test]
fn abs_router_composes_without_route_collisions() {
	let router = crate::routes::public_router::<()>()
		.merge(crate::routes::authenticated_router::<()>());
	// `into_make_service` is what forces the match tree to be built; a
	// conflict panics here rather than at request time.
	let _ = router.into_make_service();
}

/// A library holding one single-file audiobook with metadata and audio, plus
/// the user that owns it. The fixture mirrors the abs-ref capture rig:
/// "Analytical Engine" by "Ada Lovelace", 12 seconds long.
struct Fixture {
	backend: std::sync::Arc<TestBackend>,
	user: models::entity::user::AuthUser,
	library_id: String,
	item_id: String,
}

async fn fixture() -> Fixture {
	let conn = db().await;
	let user_row = ::tests::fake_data::User::new("ada").insert(&conn).await;
	let user = auth_user(&user_row);
	let library = library_of_type(&conn, LibraryType::Mixed).await;
	let (_, media_rows) = series_with_files(
		&conn,
		&library.id,
		"Analytical Engine",
		&[("Analytical Engine", "m4b")],
	)
	.await;
	let item = media_rows.into_iter().next().expect("one book");
	metadata(&conn, &item.id, "Analytical Engine", Some("Ada Lovelace")).await;

	let backend = TestBackend::new(conn);
	backend.set_audio(&item.id, one_track_audio(&item.path, 12_000));
	Fixture {
		backend,
		user,
		library_id: library.id,
		item_id: item.id,
	}
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

#[tokio::test]
async fn status_offers_the_local_auth_method_lissen_looks_for() {
	let fixture = fixture().await;
	let (status, body) =
		request(fixture.backend, &fixture.user, "GET", "/status", None).await;

	// `AudiobookshelfAuthService.fetchAuthMethods` refuses to show the
	// username/password form unless "local" is in this list.
	assert_eq!(status, StatusCode::OK);
	assert_eq!(body["authMethods"], json!(["local"]));
	assert_eq!(body["app"], "audiobookshelf");
	assert_eq!(body["serverVersion"], "2.36.0");
	assert_eq!(body["isInit"], true);
	assert!(body["authFormData"]["authLoginCustomMessage"].is_string());
}

#[tokio::test]
async fn ping_answers_the_success_flag() {
	let fixture = fixture().await;
	let (status, body) =
		request(fixture.backend, &fixture.user, "GET", "/ping", None).await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(body["success"], true);
}

#[tokio::test]
async fn login_returns_the_token_trio_only_when_tokens_were_requested() {
	let fixture = fixture().await;
	let body = json!({ "username": fixture.user.username, "password": "correct-horse" });

	let (status, with_tokens, _) = request_full(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		"/login",
		Some(body.clone()),
		&[("x-return-tokens", "true")],
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	let _ = with_tokens;

	let (_, body_with) = request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		"/login",
		Some(body.clone()),
	)
	.await;
	// Without the header abs-ref sends the key as an explicit null, which is
	// how a client tells "no refresh token" from "server does not do them".
	assert!(body_with["user"]["refreshToken"].is_null());
	assert!(body_with["user"].get("refreshToken").is_some());
	assert!(body_with["user"]["accessToken"].is_string());
	assert!(body_with["user"]["token"].is_string());
	// `capture/login.json` carries this flag; the other envelopes do not.
	assert_eq!(body_with["user"]["isOldToken"], true);
	assert_eq!(body_with["Source"], "local");
	assert_eq!(body_with["ereaderDevices"], json!([]));
	assert_eq!(body_with["serverSettings"]["version"], "2.36.0");
	assert_eq!(
		body_with["userDefaultLibraryId"],
		json!(fixture.library_id.clone())
	);
}

#[tokio::test]
async fn login_with_the_wrong_password_is_unauthorized() {
	let fixture = fixture().await;
	let (status, _) = request(
		fixture.backend,
		&fixture.user,
		"POST",
		"/login",
		Some(json!({ "username": fixture.user.username, "password": "nope" })),
	)
	.await;
	// `capture/negatives.txt`: bad password /login: 401.
	assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn refresh_requires_a_refresh_token_and_rejects_an_access_token() {
	let fixture = fixture().await;
	let secret = crate::test_support::SECRET;
	let minted =
		crate::auth::mint_tokens(secret, &fixture.user.id, &fixture.user.username, None)
			.expect("tokens");

	let (status, _, body) = request_full(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		"/auth/refresh",
		None,
		&[
			("x-refresh-token", minted.refresh_token.as_str()),
			("x-return-tokens", "true"),
		],
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert!(body["user"]["accessToken"].is_string());
	assert!(body["user"]["refreshToken"].is_string());
	// Only `/login` flags the legacy token.
	assert!(body["user"].get("isOldToken").is_none());

	// An access token in the refresh header would otherwise buy a 30-day
	// extension of a 1-hour credential.
	let (status, _, _) = request_full(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		"/auth/refresh",
		None,
		&[("x-refresh-token", minted.access_token.as_str())],
	)
	.await;
	assert_eq!(status, StatusCode::UNAUTHORIZED);

	// No header at all is a 401, not a 500.
	let (status, _) = request(
		fixture.backend,
		&fixture.user,
		"POST",
		"/auth/refresh",
		None,
	)
	.await;
	assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn authorize_and_me_carry_the_legacy_token_alone() {
	let fixture = fixture().await;
	for (method, uri) in [("POST", "/api/authorize"), ("GET", "/api/me")] {
		let (status, body) =
			request(fixture.backend.clone(), &fixture.user, method, uri, None).await;
		assert_eq!(status, StatusCode::OK, "{uri}");
		let user = if uri == "/api/me" {
			&body
		} else {
			&body["user"]
		};
		// `capture/{authorize,me}.json` both carry `token` and neither
		// `accessToken` nor `refreshToken`.
		assert!(user["token"].is_string(), "{uri}");
		assert!(user.get("accessToken").is_none(), "{uri}");
		assert!(user.get("refreshToken").is_none(), "{uri}");
		assert_eq!(user["id"], json!(fixture.user.id.clone()), "{uri}");
		assert_eq!(user["type"], "root", "{uri}");
	}
}

#[tokio::test]
async fn me_exposes_the_projections_lissen_decodes_twice() {
	let fixture = fixture().await;
	fixture.backend.set_progress(
		&fixture.user.id,
		&fixture.item_id,
		AbsProgress {
			position_ms: 4_500,
			track_index: Some(0),
			is_finished: false,
			started_at: chrono::Utc::now(),
			last_update: chrono::Utc::now(),
			finished_at: None,
		},
	);
	fixture.backend.upsert_bookmark_for_test(
		&fixture.user.id,
		&fixture.item_id,
		3_000,
		"Mark",
	);

	let (status, body) =
		request(fixture.backend, &fixture.user, "GET", "/api/me", None).await;
	assert_eq!(status, StatusCode::OK);
	// `UserResponse` and `BookmarksResponse` are both decoded from this one
	// object (`AudiobookshelfApiClient.kt:62,77`).
	assert_eq!(body["mediaProgress"][0]["currentTime"], 4.5);
	assert_eq!(
		body["mediaProgress"][0]["libraryItemId"],
		json!(fixture.item_id.clone())
	);
	assert_eq!(body["bookmarks"][0]["time"], 3.0);
	assert_eq!(body["bookmarks"][0]["title"], "Mark");
}

// ---------------------------------------------------------------------------
// Libraries and items
// ---------------------------------------------------------------------------

#[tokio::test]
async fn libraries_hide_the_ones_without_audio() {
	let conn = db().await;
	let user_row = ::tests::fake_data::User::new("ada").insert(&conn).await;
	let user = auth_user(&user_row);

	let audio_library = library_of_type(&conn, LibraryType::Mixed).await;
	let (_, audio_rows) =
		series_with_files(&conn, &audio_library.id, "Audio", &[("Book", "m4b")]).await;
	let comic_library = library_of_type(&conn, LibraryType::Comic).await;
	series_with_files(&conn, &comic_library.id, "Comics", &[("Issue", "cbz")]).await;

	let backend = TestBackend::new(conn);
	backend.set_audio(
		&audio_rows[0].id,
		one_track_audio(&audio_rows[0].path, 12_000),
	);

	let (status, body) = request(backend, &user, "GET", "/api/libraries", None).await;
	assert_eq!(status, StatusCode::OK);
	let libraries = body["libraries"].as_array().expect("libraries");
	// An ABS client cannot render a library with nothing playable in it, and
	// Stump installs are mixed; only the audio one is offered.
	assert_eq!(libraries.len(), 1);
	assert_eq!(libraries[0]["id"], json!(audio_library.id));
	assert_eq!(libraries[0]["mediaType"], "book");
	assert_eq!(libraries[0]["displayOrder"], 1);
	assert_eq!(
		libraries[0]["folders"][0]["libraryId"],
		json!(audio_library.id)
	);
	assert!(libraries[0]["folders"][0]["id"].is_string());
}

#[tokio::test]
async fn library_detail_adds_filterdata_only_when_asked() {
	let fixture = fixture().await;
	let (status, bare) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/libraries/{}", fixture.library_id),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert!(bare.get("filterdata").is_none());
	assert_eq!(bare["id"], json!(fixture.library_id.clone()));

	let (status, with_filter) = request(
		fixture.backend,
		&fixture.user,
		"GET",
		&format!("/api/libraries/{}?include=filterdata", fixture.library_id),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	// Lissen's `LibraryResponse{library, filterdata}`.
	assert_eq!(with_filter["library"]["id"], json!(fixture.library_id));
	assert_eq!(
		with_filter["filterdata"]["authors"][0]["name"],
		"Ada Lovelace"
	);
	assert_eq!(with_filter["filterdata"]["bookCount"], 1);
	assert_eq!(with_filter["issues"], 0);
	assert_eq!(with_filter["numUserPlaylists"], 0);
}

#[tokio::test]
async fn the_item_page_carries_the_minified_envelope_and_shape() {
	let fixture = fixture().await;
	let (status, body) = request(
		fixture.backend,
		&fixture.user,
		"GET",
		&format!(
			"/api/libraries/{}/items?limit=10&page=0&sort=media.metadata.title&desc=0&minified=1&collapseseries=0",
			fixture.library_id
		),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);

	// The envelope keys and their echoed parameters, from
	// `capture/library_items_minified.json`.
	assert_eq!(body["total"], 1);
	assert_eq!(body["limit"], 10);
	assert_eq!(body["page"], 0);
	assert_eq!(body["sortBy"], "media.metadata.title");
	assert_eq!(body["sortDesc"], false);
	assert_eq!(body["mediaType"], "book");
	assert_eq!(body["minified"], true);
	assert_eq!(body["collapseseries"], false);
	assert_eq!(body["offset"], 0);

	let item = &body["results"][0];
	// Minified rows carry `numFiles`/`size` and the minified metadata pair,
	// and no `libraryFiles`/`audioFiles`.
	assert_eq!(item["numFiles"], 1);
	assert!(item["size"].is_i64());
	assert!(item.get("libraryFiles").is_none());
	assert!(item["media"].get("audioFiles").is_none());
	assert_eq!(item["media"]["duration"], 12.0);
	assert_eq!(item["media"]["numTracks"], 1);
	assert_eq!(item["media"]["numChapters"], 1);
	assert_eq!(item["media"]["metadata"]["title"], "Analytical Engine");
	assert_eq!(item["media"]["metadata"]["authorName"], "Ada Lovelace");
	assert_eq!(item["media"]["metadata"]["authorNameLF"], "Lovelace, Ada");
	assert_eq!(item["media"]["metadata"]["seriesName"], "");
	// The full author/series objects belong to the detail shape.
	assert!(item["media"]["metadata"].get("authors").is_none());
	// abs-ref's minified capture omits ebookFile, ebookFormat and numEbooks;
	// ebook metadata is only present on detail/expanded rows.
	assert!(item["media"].get("ebookFile").is_none());
	assert!(item["media"].get("ebookFormat").is_none());
	assert!(item["media"].get("numEbooks").is_none());
}

#[tokio::test]
async fn the_item_page_sorts_and_pages_the_way_the_parameters_ask() {
	let conn = db().await;
	let user_row = ::tests::fake_data::User::new("ada").insert(&conn).await;
	let user = auth_user(&user_row);
	let library = library_of_type(&conn, LibraryType::Mixed).await;
	let (_, rows) = series_with_files(
		&conn,
		&library.id,
		"Shelf",
		&[("Beta", "m4b"), ("Alpha", "m4b"), ("Gamma", "m4b")],
	)
	.await;
	let backend = TestBackend::new(conn);
	for row in &rows {
		backend.set_audio(&row.id, one_track_audio(&row.path, 12_000));
	}
	// Titles deliberately disagree with the file names, so a title sort that
	// silently fell back to `media.name` would show a different order than
	// the titles it renders.
	metadata(backend.conn(), &rows[0].id, "Beta", None).await;
	metadata(backend.conn(), &rows[1].id, "Alpha", None).await;
	metadata(backend.conn(), &rows[2].id, "Gamma", None).await;

	let titles = |body: &serde_json::Value| {
		body["results"]
			.as_array()
			.expect("results")
			.iter()
			.map(|item| {
				item["media"]["metadata"]["title"]
					.as_str()
					.unwrap()
					.to_owned()
			})
			.collect::<Vec<_>>()
	};

	let uri = format!("/api/libraries/{}/items?minified=1", library.id);
	let (_, ascending) = request(backend.clone(), &user, "GET", &uri, None).await;
	assert_eq!(titles(&ascending), ["Alpha", "Beta", "Gamma"]);

	let (_, descending) = request(
		backend.clone(),
		&user,
		"GET",
		&format!("{uri}&desc=1"),
		None,
	)
	.await;
	assert_eq!(titles(&descending), ["Gamma", "Beta", "Alpha"]);

	// `page` is 0-based and `offset` echoes `page * limit`.
	let (_, second_page) = request(
		backend,
		&user,
		"GET",
		&format!("{uri}&limit=1&page=1"),
		None,
	)
	.await;
	assert_eq!(titles(&second_page), ["Beta"]);
	assert_eq!(second_page["total"], 3);
	assert_eq!(second_page["offset"], 1);
}

#[tokio::test]
async fn the_not_finished_filter_hides_finished_books_but_keeps_unstarted_ones() {
	let conn = db().await;
	let user_row = ::tests::fake_data::User::new("ada").insert(&conn).await;
	let user = auth_user(&user_row);
	let library = library_of_type(&conn, LibraryType::Mixed).await;
	let (_, rows) = series_with_files(
		&conn,
		&library.id,
		"Shelf",
		&[("Done", "m4b"), ("Fresh", "m4b")],
	)
	.await;
	let backend = TestBackend::new(conn);
	for row in &rows {
		backend.set_audio(&row.id, one_track_audio(&row.path, 12_000));
	}
	backend.set_progress(
		&user.id,
		&rows[0].id,
		AbsProgress {
			position_ms: 12_000,
			track_index: Some(0),
			is_finished: true,
			started_at: chrono::Utc::now(),
			last_update: chrono::Utc::now(),
			finished_at: Some(chrono::Utc::now()),
		},
	);

	// The literal Lissen ships for "hide completed":
	// `LibraryFilteringRequestConverter.kt:15`.
	let (status, body) = request(
		backend,
		&user,
		"GET",
		&format!(
			"/api/libraries/{}/items?minified=1&filter=progress.bm90LWZpbmlzaGVk",
			library.id
		),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(body["total"], 1);
	assert_eq!(body["results"][0]["id"], json!(rows[1].id.clone()));
}

#[tokio::test]
async fn item_detail_is_the_full_shape_and_expanded_adds_tracks() {
	let fixture = fixture().await;
	let (status, detail) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/items/{}", fixture.item_id),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);

	// `capture/item.json`: the detail shape has libraryFiles/lastScan and the
	// full metadata objects, and no numFiles/size/duration.
	assert!(detail["libraryFiles"].is_array());
	assert_eq!(detail["scanVersion"], "2.36.0");
	assert!(detail.get("numFiles").is_none());
	assert!(detail["media"].get("duration").is_none());
	assert_eq!(
		detail["media"]["libraryItemId"],
		json!(fixture.item_id.clone())
	);
	assert_eq!(
		detail["media"]["metadata"]["authors"][0]["name"],
		"Ada Lovelace"
	);
	assert!(detail["media"]["metadata"]["authors"][0]["id"].is_string());
	assert!(detail["media"]["metadata"].get("authorName").is_none());
	assert_eq!(detail["media"]["chapters"][0]["title"], "Chapter One");
	assert_eq!(detail["media"]["chapters"][0]["id"], 0);
	assert_eq!(detail["media"]["chapters"][0]["start"], 0.0);
	assert_eq!(detail["media"]["chapters"][0]["end"], 12.0);
	// `audioFiles[].index` is 1-based; `ino` is the route key.
	assert_eq!(detail["media"]["audioFiles"][0]["index"], 1);
	assert_eq!(detail["media"]["audioFiles"][0]["ino"], "0");
	assert_eq!(detail["media"]["audioFiles"][0]["duration"], 12.0);
	assert_eq!(detail["media"]["audioFiles"][0]["mimeType"], "audio/mp4");
	assert_eq!(detail["media"]["audioFiles"][0]["metadata"]["ext"], ".m4b");
	// A single-container audiobook is a file, not a folder.
	assert_eq!(detail["isFile"], true);
	assert!(detail["media"].get("tracks").is_none());
	// `item.json` has the explicit null detail field, but no sibling
	// ebookFormat/numEbooks keys.
	assert!(detail["media"].get("ebookFile").is_some());
	assert!(detail["media"]["ebookFile"].is_null());
	assert!(detail["media"].get("ebookFormat").is_none());
	assert!(detail["media"].get("numEbooks").is_none());

	let (status, expanded) = request(
		fixture.backend,
		&fixture.user,
		"GET",
		&format!("/api/items/{}?expanded=1", fixture.item_id),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	let track = &expanded["media"]["tracks"][0];
	assert_eq!(
		track["contentUrl"],
		json!(format!("/api/items/{}/file/0", fixture.item_id))
	);
	assert!(expanded["media"].get("ebookFile").is_some());
	assert!(expanded["media"]["ebookFile"].is_null());
	assert!(expanded["media"].get("ebookFormat").is_none());
	assert!(expanded["media"].get("numEbooks").is_none());
	assert_eq!(track["startOffset"], 0.0);
	assert!(track["title"].is_string());
	// The expanded shape carries both metadata dialects.
	assert!(expanded["media"]["metadata"]["authorName"].is_string());
	assert!(expanded["media"]["metadata"]["authors"].is_array());
	assert!(expanded["numFiles"].is_i64());
}

#[tokio::test]
async fn a_folder_audiobook_reports_its_tracks_in_playback_order() {
	let conn = db().await;
	let user_row = ::tests::fake_data::User::new("ada").insert(&conn).await;
	let user = auth_user(&user_row);
	let library = library_of_type(&conn, LibraryType::Mixed).await;
	let (_, rows) = series_with_files(
		&conn,
		&library.id,
		"Compiler Chronicles",
		&[("First Pass", "mp3")],
	)
	.await;
	let backend = TestBackend::new(conn);
	backend.set_audio(&rows[0].id, two_track_audio(&rows[0].path));

	let (status, body) = request(
		backend,
		&user,
		"GET",
		&format!("/api/items/{}?expanded=1", rows[0].id),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	// Tracks below the item path make it a folder item.
	assert_eq!(body["isFile"], false);
	let tracks = body["media"]["tracks"].as_array().expect("tracks");
	assert_eq!(tracks.len(), 2);
	assert_eq!(tracks[0]["ino"], "0");
	assert_eq!(tracks[0]["startOffset"], 0.0);
	assert_eq!(tracks[1]["ino"], "1");
	// The second track starts where the first ends: a client concatenates by
	// startOffset, so a wrong offset breaks every seek past track one.
	assert_eq!(tracks[1]["startOffset"], 4.0);
	assert_eq!(body["media"]["duration"], 8.0);
	assert_eq!(body["media"]["numTracks"], 2);
	assert_eq!(body["media"]["chapters"][1]["start"], 4.0);
}

#[tokio::test]
async fn unknown_ids_are_not_found() {
	let fixture = fixture().await;
	for uri in [
		"/api/items/nope".to_owned(),
		"/api/libraries/nope".to_owned(),
		"/api/libraries/nope/items".to_owned(),
		"/api/me/progress/nope".to_owned(),
		format!("/api/items/{}/file/9", fixture.item_id),
		"/api/authors/nope".to_owned(),
		format!("/api/authors/{}/image", "nope"),
	] {
		let (status, _) =
			request(fixture.backend.clone(), &fixture.user, "GET", &uri, None).await;
		// `capture/negatives.txt` for the first four.
		assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
	}

	let (status, _) = request(
		fixture.backend,
		&fixture.user,
		"POST",
		"/api/session/nope/sync",
		Some(json!({ "currentTime": 1.0 })),
	)
	.await;
	assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn batch_get_answers_in_the_requested_order_and_drops_unknown_ids() {
	let conn = db().await;
	let user_row = ::tests::fake_data::User::new("ada").insert(&conn).await;
	let user = auth_user(&user_row);
	let library = library_of_type(&conn, LibraryType::Mixed).await;
	let (_, rows) = series_with_files(
		&conn,
		&library.id,
		"Shelf",
		&[("Alpha", "m4b"), ("Beta", "m4b")],
	)
	.await;
	let backend = TestBackend::new(conn);
	for row in &rows {
		backend.set_audio(&row.id, one_track_audio(&row.path, 12_000));
	}

	let (status, body) = request(
		backend.clone(),
		&user,
		"POST",
		"/api/items/batch/get",
		Some(json!({
			"libraryItemIds": [rows[1].id, "nope", rows[0].id]
		})),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	let items = body["libraryItems"].as_array().expect("items");
	assert_eq!(items.len(), 2);
	assert_eq!(items[0]["id"], json!(rows[1].id.clone()));
	assert_eq!(items[1]["id"], json!(rows[0].id.clone()));

	// An empty batch is a client bug, not an empty answer.
	let (status, _) = request(
		backend,
		&user,
		"POST",
		"/api/items/batch/get",
		Some(json!({ "libraryItemIds": [] })),
	)
	.await;
	assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn search_reports_which_field_matched() {
	let fixture = fixture().await;
	let uri = |needle: &str| {
		format!(
			"/api/libraries/{}/search?q={needle}&limit=10",
			fixture.library_id
		)
	};

	let (status, by_title) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&uri("Engine"),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(by_title["book"].as_array().expect("book").len(), 1);
	// abs-ref omits both match keys on a title hit
	// (`capture/library_search.json`).
	assert!(by_title["book"][0].get("matchKey").is_none());
	assert_eq!(
		by_title["book"][0]["libraryItem"]["id"],
		json!(fixture.item_id.clone())
	);

	let (_, by_author) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&uri("Lovelace"),
		None,
	)
	.await;
	assert_eq!(by_author["book"][0]["matchKey"], "authors");
	assert_eq!(by_author["book"][0]["matchText"], "Ada Lovelace");
	assert_eq!(by_author["authors"][0]["name"], "Ada Lovelace");

	let (_, nothing) =
		request(fixture.backend, &fixture.user, "GET", &uri("zzz"), None).await;
	assert_eq!(nothing["book"], json!([]));
	assert_eq!(nothing["authors"], json!([]));
}

#[tokio::test]
async fn authors_are_synthesised_from_the_writers_column() {
	let fixture = fixture().await;
	let (status, page) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!(
			"/api/libraries/{}/authors?limit=10&page=0&sort=name&desc=0",
			fixture.library_id
		),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(page["total"], 1);
	let author = &page["results"][0];
	assert_eq!(author["name"], "Ada Lovelace");
	assert_eq!(author["lastFirst"], "Lovelace, Ada");
	assert_eq!(author["numBooks"], 1);
	assert_eq!(author["libraryId"], json!(fixture.library_id.clone()));
	let author_id = author["id"].as_str().expect("author id").to_owned();

	// The allocated id resolves back to the same author, with its books.
	let (status, detail) = request(
		fixture.backend,
		&fixture.user,
		"GET",
		&format!("/api/authors/{author_id}?include=items"),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(detail["name"], "Ada Lovelace");
	// Lissen decodes these as minified rows (`AuthorItemsResponse`).
	assert_eq!(
		detail["libraryItems"][0]["id"],
		json!(fixture.item_id.clone())
	);
	assert_eq!(
		detail["libraryItems"][0]["media"]["metadata"]["authorName"],
		"Ada Lovelace"
	);
}

#[tokio::test]
async fn the_continue_listening_shelf_appears_only_once_a_book_is_started() {
	let fixture = fixture().await;
	let uri = format!("/api/libraries/{}/personalized", fixture.library_id);

	let (status, cold) =
		request(fixture.backend.clone(), &fixture.user, "GET", &uri, None).await;
	assert_eq!(status, StatusCode::OK);
	let keys = |body: &serde_json::Value| {
		body.as_array()
			.expect("shelves")
			.iter()
			.map(|shelf| shelf["labelStringKey"].as_str().unwrap().to_owned())
			.collect::<Vec<_>>()
	};
	assert!(!keys(&cold).contains(&"LabelContinueListening".to_owned()));

	fixture.backend.set_progress(
		&fixture.user.id,
		&fixture.item_id,
		AbsProgress {
			position_ms: 4_500,
			track_index: Some(0),
			is_finished: false,
			started_at: chrono::Utc::now(),
			last_update: chrono::Utc::now(),
			finished_at: None,
		},
	);
	let (_, warm) = request(fixture.backend, &fixture.user, "GET", &uri, None).await;
	// Lissen's home screen reads exactly this shelf
	// (`RecentListeningResponseConverter.kt`), by `labelStringKey`, and needs
	// `media.metadata.authorName` off the minified entities.
	let shelf = warm
		.as_array()
		.expect("shelves")
		.iter()
		.find(|shelf| shelf["labelStringKey"] == "LabelContinueListening")
		.expect("continue listening shelf");
	assert_eq!(shelf["id"], "continue-listening");
	assert_eq!(shelf["total"], 1);
	assert_eq!(shelf["entities"][0]["id"], json!(fixture.item_id.clone()));
	assert_eq!(
		shelf["entities"][0]["media"]["metadata"]["authorName"],
		"Ada Lovelace"
	);
}

// ---------------------------------------------------------------------------
// Playback, progress and bookmarks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cover_and_track_bytes_are_served_with_range_support() {
	let fixture = fixture().await;
	fixture.backend.set_cover(
		&fixture.item_id,
		AbsImage {
			content_type: "image/jpeg".to_owned(),
			data: vec![1, 2, 3],
		},
	);

	let (status, headers, _) = request_full(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/items/{}/cover?raw=1", fixture.item_id),
		None,
		&[],
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(headers["content-type"], "image/jpeg");

	let (status, headers, _) = request_full(
		fixture.backend,
		&fixture.user,
		"GET",
		&format!("/api/items/{}/file/0", fixture.item_id),
		None,
		&[("range", "bytes=0-99")],
	)
	.await;
	// `capture/file_range_headers.txt`: a ranged request answers 206 with
	// `Accept-Ranges`, which is what makes seeking work in ExoPlayer.
	assert_eq!(status, StatusCode::PARTIAL_CONTENT);
	assert_eq!(headers["accept-ranges"], "bytes");
	assert_eq!(headers["content-type"], "audio/mp4");
}

#[tokio::test]
async fn play_opens_a_session_resuming_the_stored_position() {
	let fixture = fixture().await;
	fixture.backend.set_progress(
		&fixture.user.id,
		&fixture.item_id,
		AbsProgress {
			position_ms: 4_500,
			track_index: Some(0),
			is_finished: false,
			started_at: chrono::Utc::now(),
			last_update: chrono::Utc::now(),
			finished_at: None,
		},
	);

	let (status, session) = request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		&format!("/api/items/{}/play", fixture.item_id),
		Some(json!({
			"deviceInfo": {
				"clientName": "Lissen",
				"deviceId": "test-device",
				"deviceName": "Pixel"
			},
			"supportedMimeTypes": ["audio/mp4"],
			"mediaPlayer": "exo-player",
			"forceDirectPlay": true
		})),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(session["libraryItemId"], json!(fixture.item_id.clone()));
	assert_eq!(session["mediaType"], "book");
	// Stump never transcodes: DirectPlay, always.
	assert_eq!(session["playMethod"], 0);
	// A play request resumes where the unified reading state left off, so a
	// book started in another client continues here.
	assert_eq!(session["currentTime"], 4.5);
	assert_eq!(session["startTime"], 4.5);
	assert_eq!(session["duration"], 12.0);
	assert_eq!(session["timeListening"], 0.0);
	assert_eq!(session["mediaPlayer"], "exo-player");
	assert_eq!(session["deviceInfo"]["deviceId"], "test-device");
	assert_eq!(session["deviceInfo"]["clientName"], "Lissen");
	assert_eq!(session["displayTitle"], "Analytical Engine");
	assert_eq!(session["displayAuthor"], "Ada Lovelace");
	assert_eq!(
		session["audioTracks"][0]["contentUrl"],
		json!(format!("/api/items/{}/file/0", fixture.item_id))
	);
	assert!(session["libraryItem"]["media"]["tracks"].is_array());
	// `capture/play_session.json` has no `userMediaProgress` on the embedded
	// item.
	assert!(session["libraryItem"].get("userMediaProgress").is_none());

	// The session is readable afterwards, which is what a client that
	// reconnects does.
	let session_id = session["id"].as_str().expect("session id");
	let (status, reread) = request(
		fixture.backend,
		&fixture.user,
		"GET",
		&format!("/api/session/{session_id}"),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(reread["id"], json!(session_id));
	assert_eq!(reread["libraryItemId"], json!(fixture.item_id));
}

#[tokio::test]
async fn sync_lands_a_time_position_and_accumulates_listening_time() {
	let fixture = fixture().await;
	let (_, session) = request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		&format!("/api/items/{}/play", fixture.item_id),
		Some(json!({ "mediaPlayer": "exo-player" })),
	)
	.await;
	let session_id = session["id"].as_str().expect("session id").to_owned();

	for (current_time, listened) in [(4.5, 4.5), (9.0, 4.5)] {
		let (status, _, _) = request_full(
			fixture.backend.clone(),
			&fixture.user,
			"POST",
			&format!("/api/session/{session_id}/sync"),
			Some(json!({ "currentTime": current_time, "timeListened": listened })),
			&[],
		)
		.await;
		assert_eq!(status, StatusCode::OK);
	}

	// What reached the reading state: milliseconds, with the track the
	// position falls in, and the elapsed delta per sync.
	let applied = fixture.backend.applied_updates();
	assert_eq!(applied.len(), 2);
	assert_eq!(applied[0].1.position_ms, 4_500);
	assert_eq!(applied[0].1.track_index, Some(0));
	assert_eq!(applied[0].1.duration_ms, 12_000);
	assert_eq!(applied[0].1.elapsed_ms, 4_500);
	assert_eq!(applied[1].1.position_ms, 9_000);

	// The session's own counter is cumulative, not the last delta.
	let (_, reread) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/session/{session_id}"),
		None,
	)
	.await;
	assert_eq!(reread["currentTime"], 9.0);
	assert_eq!(reread["timeListening"], 9.0);

	// And the progress read shows the same position as a fraction.
	let (status, progress) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/me/progress/{}", fixture.item_id),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(progress["currentTime"], 9.0);
	assert_eq!(progress["duration"], 12.0);
	assert_eq!(progress["progress"], 0.75);
	assert_eq!(progress["isFinished"], false);
	assert_eq!(progress["libraryItemId"], json!(fixture.item_id.clone()));
	assert!(progress["mediaItemId"].is_string());
	assert_eq!(progress["mediaItemType"], "book");

	let (status, _) = request(
		fixture.backend,
		&fixture.user,
		"POST",
		&format!("/api/session/{session_id}/close"),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_session_cannot_be_synced_by_another_user() {
	let fixture = fixture().await;
	let (_, session) = request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		&format!("/api/items/{}/play", fixture.item_id),
		None,
	)
	.await;
	let session_id = session["id"].as_str().expect("session id").to_owned();

	let other_row = ::tests::fake_data::User::new("other")
		.insert(fixture.backend.conn())
		.await;
	let other = auth_user(&other_row);

	let (status, _) = request(
		fixture.backend,
		&other,
		"POST",
		&format!("/api/session/{session_id}/sync"),
		Some(json!({ "currentTime": 4.5, "timeListened": 4.5 })),
	)
	.await;
	// A guessed session id must not let one account write progress onto
	// another's book.
	assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn patching_progress_marks_a_book_finished_at_its_end() {
	let fixture = fixture().await;
	let (status, _) = request(
		fixture.backend.clone(),
		&fixture.user,
		"PATCH",
		&format!("/api/me/progress/{}", fixture.item_id),
		Some(json!({ "isFinished": true })),
	)
	.await;
	assert_eq!(status, StatusCode::OK);

	// "Mark as finished" carries no position, so the position is the end.
	let applied = fixture.backend.applied_updates();
	assert_eq!(applied[0].1.position_ms, 12_000);
	assert_eq!(applied[0].1.is_finished, Some(true));
	assert_eq!(applied[0].1.elapsed_ms, 0);

	let (_, progress) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/me/progress/{}", fixture.item_id),
		None,
	)
	.await;
	assert_eq!(progress["isFinished"], true);
	assert_eq!(progress["progress"], 1.0);

	// A bare position patch moves the position without finishing it.
	let (status, _) = request(
		fixture.backend.clone(),
		&fixture.user,
		"PATCH",
		&format!("/api/me/progress/{}", fixture.item_id),
		Some(json!({ "currentTime": 3.0, "isFinished": false })),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	let (_, progress) = request(
		fixture.backend,
		&fixture.user,
		"GET",
		&format!("/api/me/progress/{}", fixture.item_id),
		None,
	)
	.await;
	assert_eq!(progress["currentTime"], 3.0);
	assert_eq!(progress["isFinished"], false);
	assert_eq!(progress["progress"], 0.25);
}

#[tokio::test]
async fn bookmarks_are_created_renamed_and_deleted_by_whole_second() {
	let fixture = fixture().await;
	let base = format!("/api/me/item/{}/bookmark", fixture.item_id);

	let (status, created) = request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		&base,
		Some(json!({ "time": 3, "title": "Captured bookmark" })),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(created["time"], 3.0);
	assert_eq!(created["title"], "Captured bookmark");
	assert_eq!(created["libraryItemId"], json!(fixture.item_id.clone()));
	assert!(created["createdAt"].is_i64());

	let (status, renamed) = request(
		fixture.backend.clone(),
		&fixture.user,
		"PATCH",
		&base,
		Some(json!({ "time": 3, "title": "Captured bookmark renamed" })),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(renamed["title"], "Captured bookmark renamed");
	// A rename keeps the original creation stamp, as abs-ref does
	// (`capture/bookmark_patched.json` repeats `createdAt`).
	assert_eq!(renamed["createdAt"], created["createdAt"]);

	// Lissen deletes by whole second in the path
	// (`AudiobookshelfApiClient.kt:71`).
	let (status, _) = request(
		fixture.backend.clone(),
		&fixture.user,
		"DELETE",
		&format!("{base}/3"),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);

	let (_, me) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		"/api/me",
		None,
	)
	.await;
	assert_eq!(me["bookmarks"], json!([]));

	// Deleting a bookmark that is not there is a 404, not a silent success.
	let (status, _) = request(
		fixture.backend,
		&fixture.user,
		"DELETE",
		&format!("{base}/3"),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_bookmark_needs_a_time() {
	let fixture = fixture().await;
	let (status, _) = request(
		fixture.backend,
		&fixture.user,
		"POST",
		&format!("/api/me/item/{}/bookmark", fixture.item_id),
		Some(json!({ "title": "No time" })),
	)
	.await;
	assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_ebook_row_is_invisible_to_this_profile() {
	let conn = db().await;
	let user_row = ::tests::fake_data::User::new("ada").insert(&conn).await;
	let user = auth_user(&user_row);
	let library = library_of_type(&conn, LibraryType::Book).await;
	let (_, rows) =
		series_with_files(&conn, &library.id, "Shelf", &[("Novel", "epub")]).await;
	let backend = TestBackend::new(conn);

	// Audiobookshelf has no page-based position, so an ebook is a book a
	// client could open and never track: it is not served at all.
	let (status, body) =
		request(backend.clone(), &user, "GET", "/api/libraries", None).await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(body["libraries"], json!([]));

	let (status, _) = request(
		backend,
		&user,
		"GET",
		&format!("/api/items/{}", rows[0].id),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_device_scoped_request_sees_only_its_libraries() {
	let conn = db().await;
	let user_row = ::tests::fake_data::User::new("ada").insert(&conn).await;
	let allowed = library_of_type(&conn, LibraryType::Mixed).await;
	let hidden = library_of_type(&conn, LibraryType::Mixed).await;
	let (_, allowed_rows) =
		series_with_files(&conn, &allowed.id, "Allowed", &[("A", "m4b")]).await;
	let (_, hidden_rows) =
		series_with_files(&conn, &hidden.id, "Hidden", &[("B", "m4b")]).await;

	let backend = TestBackend::new(conn);
	backend.set_audio(
		&allowed_rows[0].id,
		one_track_audio(&allowed_rows[0].path, 12_000),
	);
	backend.set_audio(
		&hidden_rows[0].id,
		one_track_audio(&hidden_rows[0].path, 12_000),
	);

	let mut scoped = auth_user(&user_row);
	scoped.device_library_scope = Some(vec![allowed.id.clone()]);

	let (status, body) =
		request(backend.clone(), &scoped, "GET", "/api/libraries", None).await;
	assert_eq!(status, StatusCode::OK);
	let libraries = body["libraries"].as_array().expect("libraries");
	assert_eq!(libraries.len(), 1);
	assert_eq!(libraries[0]["id"], json!(allowed.id));

	// The scope narrows books too, not just the library list.
	let (status, _) = request(
		backend.clone(),
		&scoped,
		"GET",
		&format!("/api/items/{}", hidden_rows[0].id),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::NOT_FOUND);

	// And a scoped device is honest about it in the permission flags Lissen
	// reads off the user object.
	let (_, me) = request(backend, &scoped, "GET", "/api/me", None).await;
	assert_eq!(me["permissions"]["accessAllLibraries"], false);
	assert_eq!(me["librariesAccessible"], json!([allowed.id]));
}

#[tokio::test]
async fn collapse_series_folds_a_series_into_one_row() {
	let conn = db().await;
	let user_row = ::tests::fake_data::User::new("ada").insert(&conn).await;
	let user = auth_user(&user_row);
	let library = library_of_type(&conn, LibraryType::Mixed).await;
	let (series_row, rows) = series_with_files(
		&conn,
		&library.id,
		"Compiler Chronicles",
		&[("First Pass", "m4b"), ("Second Pass", "m4b")],
	)
	.await;
	let (_, standalone) =
		series_with_files(&conn, &library.id, "Alone", &[("Solo", "m4b")]).await;

	let backend = TestBackend::new(conn);
	for row in rows.iter().chain(standalone.iter()) {
		backend.set_audio(&row.id, one_track_audio(&row.path, 12_000));
	}

	let (status, body) = request(
		backend,
		&user,
		"GET",
		&format!(
			"/api/libraries/{}/items?minified=1&collapseseries=1",
			library.id
		),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(body["collapseseries"], true);
	// Two rows: the folded series and the standalone book.
	assert_eq!(body["total"], 2);
	let collapsed = body["results"]
		.as_array()
		.expect("results")
		.iter()
		.find(|item| !item["collapsedSeries"].is_null())
		.expect("a collapsed row");
	assert_eq!(collapsed["collapsedSeries"]["id"], json!(series_row.id));
	assert_eq!(collapsed["collapsedSeries"]["name"], "Compiler Chronicles");
	assert_eq!(collapsed["collapsedSeries"]["numBooks"], 2);
	assert_eq!(
		collapsed["collapsedSeries"]["libraryItemIds"]
			.as_array()
			.expect("ids")
			.len(),
		2
	);
	// Lissen's `CollapsedSeries` model requires this key to be present and
	// null for a book outside a series.
	assert!(body["results"]
		.as_array()
		.expect("results")
		.iter()
		.any(|item| item["collapsedSeries"].is_null()));
}

// ---------------------------------------------------------------------------
// The official app: offline merge, the launch screens, series browsing
// ---------------------------------------------------------------------------

/// One offline session as `createPartialPlaybackSession` uploads it
/// (`server/ApiHandler.kt:650-668`).
fn local_session(
	id: &str,
	item_id: &str,
	current_time: f64,
	time_listening: f64,
	updated_at: i64,
) -> serde_json::Value {
	json!({
		"id": id,
		"userId": "local",
		"libraryItemId": item_id,
		"episodeId": null,
		"mediaType": "book",
		"displayTitle": "Analytical Engine",
		"displayAuthor": "Ada Lovelace",
		"duration": 12.0,
		"playMethod": 3,
		"startedAt": updated_at - 60_000,
		"updatedAt": updated_at,
		"timeListening": time_listening,
		"currentTime": current_time,
		"mediaPlayer": "exo-player",
		"deviceInfo": {
			"deviceId": "cap-device",
			"clientName": "Abs Android",
			"clientVersion": "0.14.0-beta",
			"manufacturer": "Google",
			"model": "Pixel",
			"sdkVersion": 34
		}
	})
}

/// Seed the head at `position_ms`, dated now, as if another client had just
/// written it.
fn seed_head(fixture: &Fixture, position_ms: i64) {
	fixture.backend.set_progress(
		&fixture.user.id,
		&fixture.item_id,
		AbsProgress {
			position_ms,
			track_index: Some(0),
			is_finished: false,
			started_at: chrono::Utc::now(),
			last_update: chrono::Utc::now(),
			finished_at: None,
		},
	);
}

async fn current_time(fixture: &Fixture) -> f64 {
	let (status, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/me/progress/{}", fixture.item_id),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	body["currentTime"].as_f64().expect("currentTime")
}

#[tokio::test]
async fn a_local_session_newer_than_the_head_advances_it() {
	let fixture = fixture().await;
	seed_head(&fixture, 2_000);
	let now = chrono::Utc::now().timestamp_millis();

	let (status, _) = request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		"/api/session/local",
		Some(local_session("local-1", &fixture.item_id, 9.0, 30.0, now)),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(current_time(&fixture).await, 9.0);

	// The upload is dated by the device clock, not by its arrival: that is
	// what the unified head resolves against.
	let applied = fixture.backend.applied_updates();
	let (media_id, update) = applied.last().expect("an applied update");
	assert_eq!(media_id, &fixture.item_id);
	assert_eq!(update.position_ms, 9_000);
	assert_eq!(update.elapsed_ms, 30_000);
	assert_eq!(update.at.map(|at| at.timestamp_millis()), Some(now));

	// And it is a listening-history row, marked as offline listening.
	let (status, sessions) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		"/api/me/listening-sessions",
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(sessions["total"], 1);
	assert_eq!(sessions["sessions"][0]["id"], "local-1");
	assert_eq!(sessions["sessions"][0]["playMethod"], 3);
	assert_eq!(sessions["sessions"][0]["timeListening"], 30.0);
	// A history row is not a play answer: it carries no tracks and no item.
	assert!(sessions["sessions"][0]["audioTracks"].is_null());
	assert!(sessions["sessions"][0]["libraryItem"].is_null());
}

#[tokio::test]
async fn a_local_session_older_than_the_head_is_provenance_only() {
	let fixture = fixture().await;
	seed_head(&fixture, 9_000);
	let stale = chrono::Utc::now().timestamp_millis() - 3_600_000;

	let (status, _) = request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		"/api/session/local",
		Some(local_session("local-2", &fixture.item_id, 3.0, 12.0, stale)),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	// The head keeps the newer position: an hour-old upload that reports
	// less progress does not rewind a listener.
	assert_eq!(current_time(&fixture).await, 9.0);

	// It reached the reading state as provenance, but did not create or extend
	// listening history because the head rejected the stale position.
	let applied = fixture.backend.applied_updates();
	assert_eq!(
		applied.last().expect("an applied update").1.position_ms,
		3_000
	);
	let (_, sessions) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		"/api/me/listening-sessions",
		None,
	)
	.await;
	assert_eq!(sessions["total"], 0);
}

#[tokio::test]
async fn local_all_reports_per_session_whether_the_head_moved() {
	let fixture = fixture().await;
	seed_head(&fixture, 2_000);
	let now = chrono::Utc::now().timestamp_millis();

	let (status, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		"/api/session/local-all",
		Some(json!({
			"sessions": [
				local_session("fresh", &fixture.item_id, 10.0, 40.0, now),
				local_session("stale", &fixture.item_id, 1.0, 5.0, now - 7_200_000),
				local_session("gone", "00000000-0000-0000-0000-000000000000", 5.0, 5.0, now),
			],
			"deviceInfo": { "deviceId": "cap-device", "manufacturer": "Google", "model": "Pixel" }
		})),
	)
	.await;

	assert_eq!(status, StatusCode::OK);
	let results = body["results"].as_array().expect("results");
	assert_eq!(results.len(), 3);
	// The app matches these back to its stored sessions by id and only
	// deletes the ones that succeeded (`ApiHandler.kt:780-795`).
	assert_eq!(results[0]["id"], "fresh");
	assert_eq!(results[0]["success"], true);
	assert_eq!(results[0]["progressSynced"], true);
	assert_eq!(results[1]["id"], "stale");
	assert_eq!(results[1]["success"], true);
	assert_eq!(results[1]["progressSynced"], false);
	// A book that is gone fails its own session and not the batch.
	assert_eq!(results[2]["id"], "gone");
	assert_eq!(results[2]["success"], false);
	assert!(results[2]["error"].is_string());

	assert_eq!(current_time(&fixture).await, 10.0);
	let (_, sessions) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		"/api/me/listening-sessions",
		None,
	)
	.await;
	assert_eq!(
		sessions["total"], 1,
		"only the accepted merged session is a history row"
	);
}

#[tokio::test]
async fn listening_stats_are_derived_from_the_same_session_rows() {
	let fixture = fixture().await;
	let now = chrono::Utc::now().timestamp_millis();
	for (id, listened) in [("s-1", 30.0), ("s-2", 12.0)] {
		request(
			fixture.backend.clone(),
			&fixture.user,
			"POST",
			"/api/session/local",
			Some(local_session(id, &fixture.item_id, 9.0, listened, now)),
		)
		.await;
	}

	let (status, stats) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		"/api/me/listening-stats",
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(stats["totalTime"], 42.0);
	assert_eq!(stats["items"][&fixture.item_id]["timeListening"], 42.0);
	assert_eq!(
		stats["items"][&fixture.item_id]["mediaMetadata"]["title"],
		"Analytical Engine"
	);
	let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
	assert_eq!(stats["days"][&today], 42.0);
	assert_eq!(stats["today"], 42.0);
	assert_eq!(stats["recentSessions"].as_array().expect("recent").len(), 2);
}

#[tokio::test]
async fn items_in_progress_carries_the_key_the_app_reads_with_getlong() {
	let fixture = fixture().await;
	seed_head(&fixture, 4_000);

	let (status, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		"/api/me/items-in-progress",
		None,
	)
	.await;

	assert_eq!(status, StatusCode::OK);
	let items = body["libraryItems"].as_array().expect("libraryItems");
	assert_eq!(items.len(), 1);
	assert_eq!(items[0]["id"], fixture.item_id);
	// `ItemInProgress.makeFromServerObject` reads this with `getLong`, which
	// throws on a missing key.
	assert!(items[0]["progressLastUpdate"].is_i64());
	// The row is a minified library item, not a bare id.
	assert_eq!(items[0]["media"]["metadata"]["title"], "Analytical Engine");
	assert!(items[0]["media"]["numTracks"].is_i64());

	// A finished book is not "in progress".
	fixture.backend.set_progress(
		&fixture.user.id,
		&fixture.item_id,
		AbsProgress {
			position_ms: 12_000,
			track_index: Some(0),
			is_finished: true,
			started_at: chrono::Utc::now(),
			last_update: chrono::Utc::now(),
			finished_at: Some(chrono::Utc::now()),
		},
	);
	let (_, body) = request(
		fixture.backend,
		&fixture.user,
		"GET",
		"/api/me/items-in-progress",
		None,
	)
	.await;
	assert!(body["libraryItems"]
		.as_array()
		.expect("libraryItems")
		.is_empty());
}

#[tokio::test]
async fn an_episode_progress_path_is_a_404_not_a_missing_route() {
	let fixture = fixture().await;
	for method in ["GET", "PATCH"] {
		let (status, _) = request(
			fixture.backend.clone(),
			&fixture.user,
			method,
			&format!("/api/me/progress/{}/episode-1", fixture.item_id),
			Some(json!({ "isFinished": true })),
		)
		.await;
		assert_eq!(status, StatusCode::NOT_FOUND, "{method}");
	}
}

#[tokio::test]
async fn rss_open_is_a_clean_unsupported_not_found() {
	let response = crate::routes::me::unsupported_rss_feed().await;
	assert_eq!(response.status(), StatusCode::NOT_FOUND);
	assert_eq!(
		response
			.headers()
			.get(axum::http::header::CONTENT_TYPE)
			.and_then(|value| value.to_str().ok()),
		Some("text/plain; charset=utf-8")
	);
	let body = axum::body::to_bytes(response.into_body(), usize::MAX)
		.await
		.expect("body");
	assert_eq!(
		body.as_ref(),
		b"RSS feeds are not supported by the Audiobookshelf profile"
	);
}

#[tokio::test]
async fn the_series_route_lists_multi_book_folders_only() {
	let conn = db().await;
	let user_row = ::tests::fake_data::User::new("ada").insert(&conn).await;
	let user = auth_user(&user_row);
	let library = library_of_type(&conn, LibraryType::Mixed).await;
	let (_, trilogy) = series_with_files(
		&conn,
		&library.id,
		"Compiler Chronicles",
		&[("First Pass", "m4b"), ("Second Pass", "m4b")],
	)
	.await;
	let (_, solo) =
		series_with_files(&conn, &library.id, "Analytical Engine", &[("Solo", "m4b")])
			.await;

	let backend = TestBackend::new(conn);
	for row in trilogy.iter().chain(solo.iter()) {
		backend.set_audio(&row.id, one_track_audio(&row.path, 12_000));
	}

	let (status, body) = request(
		backend.clone(),
		&user,
		"GET",
		&format!(
			"/api/libraries/{}/series?minified=1&sort=name&limit=10000",
			library.id
		),
		None,
	)
	.await;

	assert_eq!(status, StatusCode::OK);
	let results = body["results"].as_array().expect("results");
	// A Stump series holding one audiobook is that book's own folder, not an
	// Audiobookshelf series — the same rule the item list applies.
	assert_eq!(results.len(), 1);
	assert_eq!(results[0]["name"], "Compiler Chronicles");
	assert_eq!(results[0]["libraryId"], library.id);
	assert!(results[0]["nameIgnorePrefix"].is_string());
	assert!(results[0]["addedAt"].is_i64());
	assert_eq!(results[0]["books"].as_array().expect("books").len(), 2);
	assert_eq!(body["total"], 1);
}

#[tokio::test]
async fn the_collections_and_playlists_tabs_answer_an_empty_page() {
	let fixture = fixture().await;
	for path in ["collections?minified=1&sort=name&limit=1000", "playlists"] {
		let (status, body) = request(
			fixture.backend.clone(),
			&fixture.user,
			"GET",
			&format!("/api/libraries/{}/{path}", fixture.library_id),
			None,
		)
		.await;
		// Never a 404: the app opens both tabs on a library, and a Stump
		// shelf is not an Audiobookshelf collection.
		assert_eq!(status, StatusCode::OK, "{path}");
		assert_eq!(body["results"], json!([]), "{path}");
		assert_eq!(body["total"], 0, "{path}");
	}
}

#[tokio::test]
async fn personalized_carries_the_shelves_the_official_app_browses() {
	let conn = db().await;
	let user_row = ::tests::fake_data::User::new("ada").insert(&conn).await;
	let user = auth_user(&user_row);
	let library = library_of_type(&conn, LibraryType::Mixed).await;
	let (_, rows) = series_with_files(
		&conn,
		&library.id,
		"Compiler Chronicles",
		&[("First Pass", "m4b"), ("Second Pass", "m4b")],
	)
	.await;
	for row in &rows {
		metadata(&conn, &row.id, &row.name, Some("Grace Hopper")).await;
	}
	let backend = TestBackend::new(conn);
	for row in &rows {
		backend.set_audio(&row.id, one_track_audio(&row.path, 12_000));
	}

	let (status, body) = request(
		backend,
		&user,
		"GET",
		&format!("/api/libraries/{}/personalized", library.id),
		None,
	)
	.await;

	assert_eq!(status, StatusCode::OK);
	let shelves = body.as_array().expect("shelves");
	let by_id = |id: &str| {
		shelves
			.iter()
			.find(|shelf| shelf["id"] == id)
			.unwrap_or_else(|| panic!("no {id} shelf"))
			.clone()
	};
	// `MediaManager.populatePersonalizedDataForLibrary` keys on these four
	// ids and refuses to decode a `type` outside its subtype list.
	assert_eq!(by_id("recently-added")["type"], "book");
	assert_eq!(by_id("discover")["type"], "book");
	let series = by_id("recent-series");
	assert_eq!(series["type"], "series");
	assert_eq!(series["labelStringKey"], "LabelRecentSeries");
	assert_eq!(series["entities"][0]["name"], "Compiler Chronicles");
	assert_eq!(
		series["entities"][0]["books"]
			.as_array()
			.expect("books")
			.len(),
		2
	);
	let authors = by_id("newest-authors");
	assert_eq!(authors["type"], "authors");
	assert_eq!(authors["entities"][0]["name"], "Grace Hopper");
	assert_eq!(authors["entities"][0]["numBooks"], 2);
}

#[tokio::test]
async fn an_author_filter_narrows_a_page_to_that_authors_books() {
	let conn = db().await;
	let user_row = ::tests::fake_data::User::new("ada").insert(&conn).await;
	let user = auth_user(&user_row);
	let library = library_of_type(&conn, LibraryType::Mixed).await;
	let (_, rows) = series_with_files(
		&conn,
		&library.id,
		"Mixed Shelf",
		&[("Engine", "m4b"), ("Compiler", "m4b")],
	)
	.await;
	metadata(&conn, &rows[0].id, "Engine", Some("Ada Lovelace")).await;
	metadata(&conn, &rows[1].id, "Compiler", Some("Grace Hopper")).await;
	let backend = TestBackend::new(conn);
	for row in &rows {
		backend.set_audio(&row.id, one_track_audio(&row.path, 12_000));
	}

	let (_, authors) = request(
		backend.clone(),
		&user,
		"GET",
		&format!("/api/libraries/{}/authors", library.id),
		None,
	)
	.await;
	let ada = authors["results"]
		.as_array()
		.expect("authors")
		.iter()
		.find(|author| author["name"] == "Ada Lovelace")
		.expect("Ada")["id"]
		.as_str()
		.expect("author id")
		.to_owned();

	use base64::Engine as _;
	let encoded = base64::engine::general_purpose::STANDARD.encode(&ada);
	let (status, body) = request(
		backend,
		&user,
		"GET",
		&format!(
			"/api/libraries/{}/items?limit=1000&minified=1&filter=authors.{encoded}",
			library.id
		),
		None,
	)
	.await;

	assert_eq!(status, StatusCode::OK);
	let results = body["results"].as_array().expect("results");
	assert_eq!(results.len(), 1, "only the filtered author's books");
	assert_eq!(results[0]["media"]["metadata"]["title"], "Engine");
}

#[tokio::test]
async fn logout_answers_the_envelope_the_app_posts_to() {
	let fixture = fixture().await;
	let (status, body) =
		request(fixture.backend, &fixture.user, "POST", "/logout", None).await;
	assert_eq!(status, StatusCode::OK);
	// abs-ref's key is snake_case here, alone among the profile's routes.
	assert_eq!(body, json!({ "redirect_url": null }));
}

#[tokio::test]
async fn the_track_route_hands_the_delivery_layer_its_device() {
	let fixture = fixture().await;
	fixture.backend.set_device("device-phone-opus");

	let (status, _, _) = request_full(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/items/{}/file/0", fixture.item_id),
		None,
		&[("range", "bytes=0-1023")],
	)
	.await;

	assert_eq!(status, StatusCode::PARTIAL_CONTENT);
	// The audio transform preset lives on the device, so a delivery that
	// did not carry it would serve every ABS client the stored encoding.
	let served = fixture.backend.served.lock().clone();
	assert_eq!(
		served,
		vec![(
			fixture.item_id.clone(),
			0,
			Some("bytes=0-1023".to_owned()),
			Some("device-phone-opus".to_owned())
		)]
	);
}

// ---------------------------------------------------------------------------
// The official Audiobookshelf app, v0.14.0-beta (`12025ab5`)
//
// Everything below is pinned to that checkout's own source. The app is a Nuxt
// web view plus a native Kotlin player; the player parses server JSON with
// `jacksonObjectMapper()` (`android/app/src/main/java/com/audiobookshelf/app/
// server/ApiHandler.kt:21,49`), i.e. jackson-module-kotlin, whose rule is:
//
// - a **missing** non-nullable *primitive* (Boolean/Int/Long/Double) takes the
//   JVM default, which is why abs-ref can omit `AudioTrack.isLocal`;
// - a **missing or null** non-nullable *reference* (String, List, object)
//   throws `MissingKotlinParameterException`, and
// - an explicit `null` for a non-nullable primitive throws too.
//
// A throw there is not a caught error: `ApiHandler.playLibraryItem`
// (`ApiHandler.kt:637`) calls `jacksonMapper.readValue<PlaybackSession>` with
// no try/catch.
// ---------------------------------------------------------------------------

/// The non-nullable fields of the app's `PlaybackSession` and everything it
/// contains, as a table taken straight from the Kotlin declarations.
///
/// `(json pointer, must never be null, source)`. A `false` in the middle
/// column means the app declares the field nullable and only the *key* is
/// interesting.
const PLAYBACK_SESSION_FIELDS: &[(&str, bool, &str)] = &[
	// data/PlaybackSession.kt:29-53
	("/id", true, "PlaybackSession.kt:30 id: String"),
	(
		"/mediaType",
		true,
		"PlaybackSession.kt:34 mediaType: String",
	),
	(
		"/mediaMetadata",
		true,
		"PlaybackSession.kt:35 mediaMetadata: MediaTypeMetadata",
	),
	(
		"/deviceInfo",
		true,
		"PlaybackSession.kt:36 deviceInfo: DeviceInfo",
	),
	(
		"/chapters",
		true,
		"PlaybackSession.kt:37 chapters: List<BookChapter>",
	),
	("/duration", true, "PlaybackSession.kt:41 duration: Double"),
	("/playMethod", true, "PlaybackSession.kt:42 playMethod: Int"),
	("/startedAt", true, "PlaybackSession.kt:43 startedAt: Long"),
	("/updatedAt", true, "PlaybackSession.kt:44 updatedAt: Long"),
	(
		"/timeListening",
		true,
		"PlaybackSession.kt:45 timeListening: Long",
	),
	(
		"/audioTracks",
		true,
		"PlaybackSession.kt:46 audioTracks: MutableList<AudioTrack>",
	),
	(
		"/currentTime",
		true,
		"PlaybackSession.kt:47 currentTime: Double",
	),
	// data/DeviceClasses.kt:134-140 — every field non-null
	(
		"/deviceInfo/deviceId",
		true,
		"DeviceClasses.kt:135 deviceId: String",
	),
	(
		"/deviceInfo/manufacturer",
		true,
		"DeviceClasses.kt:136 manufacturer: String",
	),
	(
		"/deviceInfo/model",
		true,
		"DeviceClasses.kt:137 model: String",
	),
	(
		"/deviceInfo/sdkVersion",
		true,
		"DeviceClasses.kt:138 sdkVersion: Int",
	),
	(
		"/deviceInfo/clientVersion",
		true,
		"DeviceClasses.kt:139 clientVersion: String",
	),
	// data/AudioTrack.kt:7-18
	("/audioTracks/0/index", true, "AudioTrack.kt:8 index: Int"),
	(
		"/audioTracks/0/startOffset",
		true,
		"AudioTrack.kt:9 startOffset: Double",
	),
	(
		"/audioTracks/0/duration",
		true,
		"AudioTrack.kt:10 duration: Double",
	),
	(
		"/audioTracks/0/title",
		true,
		"AudioTrack.kt:11 title: String",
	),
	(
		"/audioTracks/0/contentUrl",
		true,
		"AudioTrack.kt:12 contentUrl: String",
	),
	// data/DataClasses.kt:207-237 — BookMetadata, deduced from its key set
	(
		"/mediaMetadata/title",
		true,
		"DataClasses.kt:207 title: String",
	),
	(
		"/mediaMetadata/explicit",
		true,
		"DataClasses.kt:207 explicit: Boolean",
	),
	(
		"/mediaMetadata/genres",
		true,
		"DataClasses.kt:218 genres: MutableList<String>",
	),
	// data/LibraryItem.kt:15-37, nullable on the session but parsed when present
	("/libraryItem/ino", true, "LibraryItem.kt:17 ino: String"),
	(
		"/libraryItem/libraryId",
		true,
		"LibraryItem.kt:18 libraryId: String",
	),
	(
		"/libraryItem/folderId",
		true,
		"LibraryItem.kt:19 folderId: String",
	),
	("/libraryItem/path", true, "LibraryItem.kt:20 path: String"),
	(
		"/libraryItem/relPath",
		true,
		"LibraryItem.kt:21 relPath: String",
	),
	(
		"/libraryItem/mediaType",
		true,
		"LibraryItem.kt:31 mediaType: String",
	),
	(
		"/libraryItem/media",
		true,
		"LibraryItem.kt:32 media: MediaType",
	),
	// data/DataClasses.kt:131-141 — Book
	(
		"/libraryItem/media/metadata",
		true,
		"DataClasses.kt:132 metadata: BookMetadata",
	),
	(
		"/libraryItem/media/tags",
		true,
		"DataClasses.kt:134 tags: List<String>",
	),
	// `ebookFile` is nullable, but the key must exist for the app's
	// `media.ebookFile` read (pages/item/_id/index.vue:461-466).
	(
		"/libraryItem/media/ebookFile",
		false,
		"DataClasses.kt:138 ebookFile: EBookFile?",
	),
];

/// Every field the app's Kotlin data classes require of `POST /api/items/{id}/play`
/// is present and non-null.
///
/// The two audiobooks "did not play" on a real phone because the app looped
/// `POST /api/items/{id}/play` hundreds of times a second; this table is the
/// half of that investigation that has to keep holding.
#[tokio::test]
async fn the_play_session_satisfies_the_apps_kotlin_field_table() {
	let fixture = fixture().await;
	let (status, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		&format!("/api/items/{}/play", fixture.item_id),
		Some(json!({
			"deviceInfo": {
				"deviceId": "official-app",
				"manufacturer": "Google",
				"model": "Pixel 8",
				"sdkVersion": 35,
				"clientVersion": "0.14.0-beta"
			},
			"mediaPlayer": "exo-player",
			"forceDirectPlay": true,
			"forceTranscode": false
		})),
	)
	.await;
	assert_eq!(status, StatusCode::OK);

	for (pointer, non_null, source) in PLAYBACK_SESSION_FIELDS {
		let value = body.pointer(pointer).unwrap_or_else(|| {
			panic!("{pointer} is absent; the app needs it ({source})")
		});
		if *non_null {
			assert!(
				!value.is_null(),
				"{pointer} is null; jackson-module-kotlin throws for {source}"
			);
		}
	}

	// `deviceInfo` is echoed from the request, and the app's own DeviceInfo
	// has no nullable field: a request that named a manufacturer must not
	// come back with `null` there.
	assert_eq!(body["deviceInfo"]["manufacturer"], "Google");
	assert_eq!(body["deviceInfo"]["model"], "Pixel 8");
	assert_eq!(body["deviceInfo"]["clientVersion"], "0.14.0-beta");
	// `audioTracks[].index` is 1-based on the wire; `/public/session/{id}/track/{index}`
	// is addressed with exactly this number.
	assert_eq!(body["audioTracks"][0]["index"], 1);
}

/// `GET /public/session/{id}/track/{index}` serves the track with **no
/// credential**, because that is where the app streams direct play from once
/// the server reports >= 2.22.0
/// (`android/.../data/PlaybackSession.kt:196-204`,
/// `plugins/capacitor/AbsAudioPlayer.js:254-261`).
///
/// Without it ExoPlayer errors, `PlayerNotificationService.kt:617-641` retries
/// by re-POSTing `/play`, and the retry loops forever — the failure a real
/// phone produced as hundreds of `POST /api/items/{id}/play` per second.
#[tokio::test]
async fn the_public_track_route_streams_direct_play_without_a_credential() {
	let fixture = fixture().await;
	let (status, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		&format!("/api/items/{}/play", fixture.item_id),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	let session_id = body["id"].as_str().expect("a session id").to_owned();

	let (status, headers, bytes) = crate::test_support::request_anonymous(
		fixture.backend.clone(),
		"GET",
		&format!("/public/session/{session_id}/track/1"),
		&[],
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(headers["accept-ranges"], "bytes");
	assert!(!bytes.is_empty());

	// The wire index is 1-based and the profile's own track index is 0-based
	// (`mapper::audio_files`); serving track 0 for `track/1` is the bug that
	// would silently play the wrong file on a multi-track book.
	let served = fixture.backend.served.lock().clone();
	assert_eq!(served.len(), 1);
	assert_eq!(served[0].0, fixture.item_id);
	assert_eq!(served[0].1, 0);

	// abs-ref answers 404 for index 0 and for an unknown session.
	let (status, _, _) = crate::test_support::request_anonymous(
		fixture.backend.clone(),
		"GET",
		&format!("/public/session/{session_id}/track/0"),
		&[],
	)
	.await;
	assert_eq!(status, StatusCode::NOT_FOUND);

	let (status, _, _) = crate::test_support::request_anonymous(
		fixture.backend.clone(),
		"GET",
		"/public/session/00000000-0000-0000-0000-000000000000/track/1",
		&[],
	)
	.await;
	assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A ranged public-track request is a `206`, so ExoPlayer can seek.
#[tokio::test]
async fn the_public_track_route_honours_range() {
	let fixture = fixture().await;
	let (_, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		&format!("/api/items/{}/play", fixture.item_id),
		None,
	)
	.await;
	let session_id = body["id"].as_str().unwrap().to_owned();

	let (status, _, _) = crate::test_support::request_anonymous(
		fixture.backend.clone(),
		"GET",
		&format!("/public/session/{session_id}/track/1"),
		&[("range", "bytes=0-99")],
	)
	.await;
	assert_eq!(status, StatusCode::PARTIAL_CONTENT);
}

/// `GET /api/items/{id}/cover` needs no credential.
///
/// `getDoesServerImagesRequireToken` (`store/index.js:95-102`) is *false* for
/// any server >= 2.17, so the webview's `<img src>` carries no token
/// (`store/globals.js:50-57`) and neither does the native art loader
/// (`android/.../data/PlaybackSession.kt:186-190`). Answering `401` is what
/// left the app's shelves blank — 19 of them on the phone run.
#[tokio::test]
async fn the_cover_route_serves_an_anonymous_request() {
	let fixture = fixture().await;
	fixture.backend.set_cover(
		&fixture.item_id,
		AbsImage {
			content_type: "image/jpeg".to_owned(),
			data: vec![1, 2, 3, 4],
		},
	);

	let (status, headers, bytes) = crate::test_support::request_anonymous(
		fixture.backend.clone(),
		"GET",
		&format!("/api/items/{}/cover?ts=1788714442778", fixture.item_id),
		&[],
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(headers["content-type"], "image/jpeg");
	assert_eq!(bytes, vec![1, 2, 3, 4]);

	// It is still a cover lane, not a file oracle: an id that is not an
	// audible book is a 404 even anonymously.
	let (status, _, _) = crate::test_support::request_anonymous(
		fixture.backend.clone(),
		"GET",
		"/api/items/not-a-book/cover",
		&[],
	)
	.await;
	assert_eq!(status, StatusCode::NOT_FOUND);
}

/// The paired EPUB is `media.ebookFile`, the `ebooks.*` filter selects on it,
/// and `/api/items/{id}/ebook` serves it.
#[tokio::test]
async fn a_paired_ebook_edition_is_reported_filtered_and_served() {
	let fixture = fixture().await;
	let ebook = crate::model::AbsEbookFile {
		media_id: "epub-row-id".to_owned(),
		path: "/books/Analytical Engine/Analytical Engine.epub".to_owned(),
		format: "epub".to_owned(),
		byte_size: 4096,
	};

	// Before the pair: the key exists and is null, which is what
	// `Book.ebookFile: EBookFile?` (DataClasses.kt:138) expects, and the
	// item is not in the "has ebook" filter.
	let (status, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/items/{}", fixture.item_id),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert!(body["media"]["ebookFile"].is_null());

	// `ebooks.ZWJvb2s=` is base64("ebook") — the value the app's filter modal
	// builds (`components/modals/FilterModal.vue:237-248,283`).
	let (status, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!(
			"/api/libraries/{}/items?filter=ebooks.ZWJvb2s%3D",
			fixture.library_id
		),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(
		body["total"], 0,
		"an audiobook with no paired ebook must not be in the ebooks filter"
	);

	fixture
		.backend
		.set_ebook(&fixture.user.id, &fixture.item_id, ebook.clone());

	let (_, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/items/{}", fixture.item_id),
		None,
	)
	.await;
	// EBookFile.kt:6-13 declares `ino` and `ebookFormat` non-null.
	assert_eq!(body["media"]["ebookFile"]["ino"], "epub-row-id");
	assert_eq!(body["media"]["ebookFile"]["ebookFormat"], "epub");
	assert_eq!(
		body["media"]["ebookFile"]["metadata"]["filename"],
		"Analytical Engine.epub"
	);
	assert_eq!(body["media"]["ebookFile"]["metadata"]["ext"], ".epub");
	assert_eq!(body["media"]["ebookFile"]["metadata"]["size"], 4096);

	let (_, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!(
			"/api/libraries/{}/items?filter=ebooks.ZWJvb2s%3D",
			fixture.library_id
		),
		None,
	)
	.await;
	assert_eq!(body["total"], 1);
	assert_eq!(body["results"][0]["id"], fixture.item_id);

	// `supplementary` has no Stump equivalent, so it selects nothing rather
	// than falling back to the whole library.
	let (_, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!(
			"/api/libraries/{}/items?filter=ebooks.c3VwcGxlbWVudGFyeQ%3D%3D",
			fixture.library_id
		),
		None,
	)
	.await;
	assert_eq!(body["total"], 0);

	// `/ebook` and `/ebook/{fileId}` are the same bytes
	// (`components/readers/Reader.vue:325-335`).
	for uri in [
		format!("/api/items/{}/ebook", fixture.item_id),
		format!("/api/items/{}/ebook/epub-row-id", fixture.item_id),
	] {
		let (status, headers, _) = request_full(
			fixture.backend.clone(),
			&fixture.user,
			"GET",
			&uri,
			None,
			&[],
		)
		.await;
		assert_eq!(status, StatusCode::OK, "{uri}");
		assert_eq!(headers["content-type"], "application/epub+zip");
	}

	// A file id that is not this item's ebook is a 404, not the ebook.
	let (status, _) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/items/{}/ebook/some-other-row", fixture.item_id),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::NOT_FOUND);

	let served = fixture.backend.served_ebooks.lock().clone();
	assert_eq!(served.len(), 2);
	assert!(served.iter().all(|(id, _)| id == "epub-row-id"));
}

/// `GET /api/items/{id}/download` is a zip, and the per-file download route
/// the Android downloader actually uses names its file.
///
/// The app downloads one part per audio track and one for the ebook, all
/// through `/api/items/{id}/file/{ino}/download`
/// (`android/.../plugins/AbsDownloader.kt:184,201,242`) — the `/download`
/// suffix is what distinguishes it from the playback route.
#[tokio::test]
async fn the_download_routes_answer_a_zip_and_named_files() {
	let fixture = fixture().await;
	fixture.backend.set_ebook(
		&fixture.user.id,
		&fixture.item_id,
		crate::model::AbsEbookFile {
			media_id: "epub-row-id".to_owned(),
			path: "/books/Analytical Engine/Analytical Engine.epub".to_owned(),
			format: "epub".to_owned(),
			byte_size: 4096,
		},
	);

	let (status, headers, _) = request_full(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/items/{}/download", fixture.item_id),
		None,
		&[],
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(headers["content-type"], "application/zip");
	assert!(headers["content-disposition"]
		.to_str()
		.unwrap()
		.starts_with("attachment; filename="));
	// The paired ebook goes in the archive alongside the tracks.
	assert_eq!(
		fixture.backend.downloaded.lock().clone(),
		vec![(fixture.item_id.clone(), Some("epub-row-id".to_owned()))]
	);

	// Per-track download: the same bytes as the playback route, named.
	let (status, headers, _) = request_full(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/items/{}/file/0/download", fixture.item_id),
		None,
		&[],
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert!(headers["content-disposition"]
		.to_str()
		.unwrap()
		.contains("attachment; filename=\""));

	// The ebook part addresses the paired row's id, not a track index.
	let (status, headers, _) = request_full(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/items/{}/file/epub-row-id/download", fixture.item_id),
		None,
		&[],
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(
		headers["content-disposition"],
		"attachment; filename=\"Analytical Engine.epub\""
	);
}

/// Playlists round-trip through the reading-list store, in exactly the
/// request shapes the app sends.
#[tokio::test]
async fn playlists_round_trip_onto_reading_lists() {
	let fixture = fixture().await;

	// `POST /api/playlists` — `{items, libraryId, name}`
	// (`components/modals/playlists/AddCreateModal.vue:180-185`).
	let (status, created) = request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		"/api/playlists",
		Some(json!({
			"items": [{ "libraryItemId": fixture.item_id, "episodeId": null }],
			"libraryId": fixture.library_id,
			"name": "Commute"
		})),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	let playlist_id = created["id"].as_str().expect("a playlist id").to_owned();
	assert_eq!(created["name"], "Commute");
	assert_eq!(created["libraryId"], fixture.library_id);
	assert_eq!(created["userId"], fixture.user.id);
	// `items[].libraryItem` is dereferenced with no null check at
	// `pages/playlist/_id.vue:52`.
	assert_eq!(created["items"][0]["libraryItemId"], fixture.item_id);
	assert_eq!(created["items"][0]["libraryItem"]["mediaType"], "book");
	assert!(created["items"][0]["episodeId"].is_null());
	assert!(created["lastUpdate"].is_number());

	// The linkage is the reading list itself, not a second store.
	let stored = fixture
		.backend
		.playlists
		.lock()
		.get(&fixture.user.id)
		.cloned()
		.unwrap_or_default();
	assert_eq!(stored.len(), 1);
	assert_eq!(stored[0].media_ids, vec![fixture.item_id.clone()]);

	// `GET /api/libraries/{id}/playlists` -> `{results}`
	// (`AddCreateModal.vue:113-115`).
	let (status, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/libraries/{}/playlists", fixture.library_id),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(body["total"], 1);
	assert_eq!(body["results"][0]["id"], playlist_id);

	// `POST /api/playlists/{id}/batch/remove` -> the updated playlist.
	let (status, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		&format!("/api/playlists/{playlist_id}/batch/remove"),
		Some(json!({
			"items": [{ "libraryItemId": fixture.item_id, "episodeId": null }]
		})),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(body["items"].as_array().map(Vec::len), Some(0));

	// `POST /api/playlists/{id}/batch/add` puts it back.
	let (status, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		&format!("/api/playlists/{playlist_id}/batch/add"),
		Some(json!({
			"items": [{ "libraryItemId": fixture.item_id, "episodeId": null }]
		})),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(body["items"].as_array().map(Vec::len), Some(1));

	// `PATCH /api/playlists/{id}`.
	let (status, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"PATCH",
		&format!("/api/playlists/{playlist_id}"),
		Some(json!({ "name": "Renamed", "description": "for the train" })),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(body["name"], "Renamed");
	assert_eq!(body["description"], "for the train");

	// `DELETE /api/playlists/{id}/item/{itemId}`
	// (`components/modals/ItemMoreMenuModal.vue:491-495`).
	let (status, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"DELETE",
		&format!("/api/playlists/{playlist_id}/item/{}", fixture.item_id),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(body["items"].as_array().map(Vec::len), Some(0));

	// `POST /api/playlists/{id}/item` — `{libraryItemId}`.
	let (status, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		&format!("/api/playlists/{playlist_id}/item"),
		Some(json!({ "libraryItemId": fixture.item_id })),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(body["items"].as_array().map(Vec::len), Some(1));

	// `GET /api/playlists/{id}` (`pages/playlist/_id.vue:41`).
	let (status, body) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/playlists/{playlist_id}"),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(body["id"], playlist_id);

	// `DELETE /api/playlists/{id}` — abs-ref answers 200 text/plain.
	let (status, _, _) = request_full(
		fixture.backend.clone(),
		&fixture.user,
		"DELETE",
		&format!("/api/playlists/{playlist_id}"),
		None,
		&[],
	)
	.await;
	assert_eq!(status, StatusCode::OK);

	let (status, _) = request(
		fixture.backend.clone(),
		&fixture.user,
		"GET",
		&format!("/api/playlists/{playlist_id}"),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A playlist entry naming a book the request cannot see is dropped from
/// `items[]` rather than emitted as an id the app would dereference.
#[tokio::test]
async fn a_playlist_never_emits_an_item_without_its_library_item() {
	let fixture = fixture().await;
	let (_, created) = request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		"/api/playlists",
		Some(json!({
			"items": [
				{ "libraryItemId": fixture.item_id },
				{ "libraryItemId": "a-book-that-is-not-visible" }
			],
			"libraryId": fixture.library_id,
			"name": "Mixed"
		})),
	)
	.await;

	let items = created["items"].as_array().expect("items");
	assert_eq!(items.len(), 1);
	assert_eq!(items[0]["libraryItemId"], fixture.item_id);
	assert!(items[0]["libraryItem"].is_object());
}
