//! `SearchController`: the grouped free-text search Kamigura and Turnleaf use.
//!
//! Kavita normalizes both the query and the stored names (lower-cased,
//! non-alphanumerics stripped) and matches a normalized substring per group.
//! Stump stores no normalized columns, so each group runs a case-insensitive
//! `LIKE '%query%'` over the columns Kavita searches — same substring
//! semantics, without the punctuation folding.
//!
//! Every group is present even when empty, as `kavita-ref` renders it, and
//! `includeChapterAndFiles=false` empties exactly `files` and `chapters`.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::{
	extract::Query,
	http::StatusCode,
	response::{IntoResponse, Response},
	routing::get,
	Extension, Json, Router,
};
use models::entity::{
	media, media_annotation, media_metadata, series_metadata, user::AuthUser,
};
use sea_orm::{prelude::*, QueryOrder, QuerySelect};
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::{
	dto::{
		AnnotationDto, BookmarkSearchResultDto, ChapterDto, GenreTagDto, LibraryDto,
		MangaFileDto, PersonDto, SearchResultDto, SearchResultGroupDto, TagDto,
	},
	errors::{APIError, APIResult},
	filter::{
		FilterComparison, SeriesFilterField, SeriesFilterStatementDto, SeriesFilterV2Dto,
	},
	mapper::{
		map_bookmark_search_result, map_chapter, map_file, map_search_result, map_series,
		name_id,
	},
};

use super::{
	annotation::load_annotations,
	library::{map_libraries, visible_libraries},
	metadata::{distinct_values, people_sources, visible_tags},
	query::{
		book_library_ids, find_media, group_by_media, load_by_keys, select_series_keys,
	},
	reader::load_bookmarks,
	reading_list::list_reading_lists,
	route_ci,
	series::UserParams,
	series_filter::plan,
	KavitaBackend,
};

/// Rows Stump returns per group. Kavita's `SearchService` caps every group as
/// well; its constant is not observable from the reference fixture (three
/// series, seven files), so this is Stump's own bound and is documented as a
/// deviation.
const MAX_RESULTS: usize = 30;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchQuery {
	#[serde(default)]
	query_string: Option<String>,
	#[serde(default)]
	include_chapter_and_files: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterIdQuery {
	#[serde(default)]
	chapter_id: Option<i32>,
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(router, "/api/Search/search", get(search));
	route_ci(
		router,
		"/api/Search/series-for-chapter",
		get(series_for_chapter),
	)
}

/// `SearchController.Search`: one grouped answer over every entity kind.
///
/// A blank `queryString` is `400`, as it is on `kavita-ref` (ASP.NET rejects
/// the missing required parameter); the envelope is Stump's `ApiException`
/// shape rather than ASP.NET's `ProblemDetails`.
async fn search(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SearchQuery>,
) -> APIResult<Json<SearchResultGroupDto>> {
	let term = query
		.query_string
		.as_deref()
		.map(str::trim)
		.filter(|term| !term.is_empty())
		.ok_or_else(|| {
			APIError::BadRequest("The queryString field is required.".to_owned())
		})?
		.to_owned();
	let user = auth.user();
	let include_files = query.include_chapter_and_files.unwrap_or(true);
	let ctx = ctx.as_ref();

	let (files, chapters) = if include_files {
		media_hits(ctx, &user, &term).await?
	} else {
		(Vec::new(), Vec::new())
	};

	Ok(Json(SearchResultGroupDto {
		libraries: matching_libraries(ctx, &user, &term).await?,
		series: matching_series(ctx, &user, &term).await?,
		collections: matching_collections(ctx, &user, &term).await?,
		reading_lists: matching_reading_lists(ctx, &user, &term).await?,
		persons: matching_people(ctx, &user, &term).await?,
		genres: matching_genres(ctx, &user, &term).await?,
		tags: matching_tags(ctx, &user, &term).await?,
		files,
		chapters,
		bookmarks: matching_bookmarks(ctx, &user, &term).await?,
		annotations: matching_annotations(ctx, &user, &term).await?,
	}))
}

/// `GET /api/Search/series-for-chapter?chapterId`: the series a chapter
/// belongs to. Kavita answers `204 No Content` for an unknown chapter.
async fn series_for_chapter(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ChapterIdQuery>,
) -> APIResult<Response> {
	let user = auth.user();
	let Some((input, _)) =
		find_media(ctx.as_ref(), &user, query.chapter_id.unwrap_or_default()).await?
	else {
		return Ok(StatusCode::NO_CONTENT.into_response());
	};
	Ok(Json(map_series(&input)).into_response())
}

fn like(term: &str) -> String {
	format!("%{term}%")
}

/// Libraries whose name matches; `kavita-ref` searches the name only, not the
/// folders.
async fn matching_libraries(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	term: &str,
) -> APIResult<Vec<LibraryDto>> {
	let lowered = term.to_lowercase();
	let rows = visible_libraries(ctx, user)
		.await?
		.into_iter()
		.filter(|(library, _)| library.name.to_lowercase().contains(&lowered))
		.take(MAX_RESULTS)
		.collect::<Vec<_>>();
	map_libraries(ctx, &rows).await
}

/// Series whose name matches, through the same filter pipeline the listings
/// use, so books and grouped series are searched in one pass.
async fn matching_series(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	term: &str,
) -> APIResult<Vec<SearchResultDto>> {
	let filter = SeriesFilterV2Dto {
		statements: vec![SeriesFilterStatementDto::new(
			FilterComparison::Matches,
			SeriesFilterField::SeriesName,
			term,
		)],
		..SeriesFilterV2Dto::default()
	};
	let plan = plan(ctx.conn(), user, &filter).await?;
	let book_libraries = book_library_ids(ctx).await?;
	let (keys, _) = select_series_keys(
		ctx,
		user,
		&plan,
		&plan.order.clone(),
		&book_libraries,
		Some((0, MAX_RESULTS as u64)),
	)
	.await?;
	Ok(load_by_keys(ctx, user, &keys)
		.await?
		.iter()
		.map(map_search_result)
		.collect())
}

async fn matching_collections(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	term: &str,
) -> APIResult<Vec<crate::dto::AppUserCollectionDto>> {
	let lowered = term.to_lowercase();
	let collections = super::collection::visible_collections(ctx, user, false)
		.await?
		.into_iter()
		.filter(|(row, _, _)| row.name.to_lowercase().contains(&lowered))
		.take(MAX_RESULTS)
		.collect::<Vec<_>>();
	super::collection::map_collections(ctx, collections).await
}

async fn matching_reading_lists(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	term: &str,
) -> APIResult<Vec<crate::dto::ReadingListDto>> {
	let lowered = term.to_lowercase();
	let (lists, _) = list_reading_lists(ctx, user, false, UserParams::parse("")).await?;
	Ok(lists
		.into_iter()
		.filter(|list| list.title.to_lowercase().contains(&lowered))
		.take(MAX_RESULTS)
		.collect())
}

/// People whose name matches, across every role Stump records.
async fn matching_people(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	term: &str,
) -> APIResult<Vec<PersonDto>> {
	let lowered = term.to_lowercase();
	let mut people: BTreeMap<String, PersonDto> = BTreeMap::new();
	for (series_column, media_column, role) in people_sources() {
		for name in distinct_values(ctx, user, None, series_column, media_column).await? {
			if !name.to_lowercase().contains(&lowered) {
				continue;
			}
			let entry = people
				.entry(name.to_lowercase())
				.or_insert_with(|| PersonDto::new(name_id(&name), name.clone(), vec![]));
			if !entry.roles.contains(&role) {
				entry.roles.push(role);
			}
		}
	}
	Ok(people.into_values().take(MAX_RESULTS).collect())
}

async fn matching_genres(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	term: &str,
) -> APIResult<Vec<GenreTagDto>> {
	let lowered = term.to_lowercase();
	Ok(distinct_values(
		ctx,
		user,
		None,
		Some(series_metadata::Column::Genres),
		Some(media_metadata::Column::Genres),
	)
	.await?
	.into_iter()
	.filter(|title| title.to_lowercase().contains(&lowered))
	.take(MAX_RESULTS)
	.map(|title| GenreTagDto {
		id: name_id(&title),
		title,
	})
	.collect())
}

async fn matching_tags(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	term: &str,
) -> APIResult<Vec<TagDto>> {
	let lowered = term.to_lowercase();
	Ok(visible_tags(ctx, user, None)
		.await?
		.into_iter()
		.filter(|tag| tag.title.to_lowercase().contains(&lowered))
		.take(MAX_RESULTS)
		.collect())
}

/// Files match on their path (`kavita-ref` matches the file path) and
/// chapters on their title; a Stump media item is both, so one query feeds
/// both groups with the columns Kavita searches for each.
async fn media_hits(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	term: &str,
) -> APIResult<(Vec<MangaFileDto>, Vec<ChapterDto>)> {
	let by_path = media::Entity::find_for_user(user)
		.filter(media::Column::DeletedAt.is_null())
		.filter(media::Column::Path.like(like(term)))
		.order_by_asc(media::Column::Name)
		.limit(MAX_RESULTS as u64)
		.all(ctx.conn())
		.await?;
	let by_name = media::Entity::find_for_user(user)
		.filter(media::Column::DeletedAt.is_null())
		.filter(media::Column::Name.like(like(term)))
		.order_by_asc(media::Column::Name)
		.limit(MAX_RESULTS as u64)
		.all(ctx.conn())
		.await?;
	let mut ids = by_path
		.iter()
		.chain(by_name.iter())
		.map(|row| row.id.clone())
		.collect::<Vec<_>>();
	ids.sort();
	ids.dedup();
	let grouped = group_by_media(ctx, user, ids, |id| id.as_str()).await?;
	let mut files = Vec::new();
	let mut chapters = Vec::new();
	for (media_id, series_index, media_index) in &grouped.rows {
		let input = &grouped.inputs[*series_index].media[*media_index];
		if by_path.iter().any(|row| &row.id == media_id) {
			files.push(map_file(input));
		}
		if by_name.iter().any(|row| &row.id == media_id) {
			chapters.push(map_chapter(input));
		}
	}
	Ok((files, chapters))
}

/// Bookmarks whose series name matches, as `kavita-ref` searches them.
async fn matching_bookmarks(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	term: &str,
) -> APIResult<Vec<BookmarkSearchResultDto>> {
	let lowered = term.to_lowercase();
	let loaded = load_bookmarks(ctx, user, None).await?;
	let mut results = Vec::new();
	for (_, _, series_index, media_index) in &loaded.rows {
		let input = &loaded.inputs[*series_index];
		if !input.name().to_lowercase().contains(&lowered) {
			continue;
		}
		let result = map_bookmark_search_result(input, &input.media[*media_index]);
		if !results.contains(&result) {
			results.push(result);
		}
		if results.len() >= MAX_RESULTS {
			break;
		}
	}
	Ok(results)
}

/// Annotations whose note matches. Stump keeps the highlighted text inside
/// the Readium locator JSON, which is not a searchable column, so only the
/// note is matched here.
async fn matching_annotations(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	term: &str,
) -> APIResult<Vec<AnnotationDto>> {
	let media_ids = media_annotation::Entity::find()
		.select_only()
		.column(media_annotation::Column::MediaId)
		.filter(media_annotation::Column::UserId.eq(user.id.clone()))
		.filter(media_annotation::Column::AnnotationText.like(like(term)))
		.into_tuple::<String>()
		.all(ctx.conn())
		.await?;
	if media_ids.is_empty() {
		return Ok(Vec::new());
	}
	// `load_annotations` maps every annotation of the matched media; keep only
	// the ones whose note actually matched.
	let lowered = term.to_lowercase();
	Ok(load_annotations(ctx, user, Some(&media_ids))
		.await?
		.into_iter()
		.filter(|annotation| {
			annotation
				.comment
				.as_deref()
				.is_some_and(|comment| comment.to_lowercase().contains(&lowered))
		})
		.take(MAX_RESULTS)
		.collect())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ids::{IdKind, KavitaIds};
	use crate::test_support::{
		auth_user, db, library_of_type, request, series_with_files, TestBackend,
	};
	use ::tests::fake_data;
	use models::shared::enums::LibraryType as StumpLibraryType;

	const GROUPS: [&str; 11] = [
		"libraries",
		"series",
		"collections",
		"readingLists",
		"persons",
		"genres",
		"tags",
		"files",
		"chapters",
		"bookmarks",
		"annotations",
	];

	/// One Comic library "Test Library" holding series "science comics" with
	/// one file, mirroring `kavita-ref`'s fixture.
	async fn backend() -> (std::sync::Arc<TestBackend>, AuthUser, i32) {
		let conn = db().await;
		let user_row = fake_data::User::new("finder").insert(&conn).await;
		let user = auth_user(&user_row);
		let library = library_of_type(&conn, StumpLibraryType::Comic).await;
		models::entity::library::ActiveModel {
			name: sea_orm::ActiveValue::Set("Test Library".to_owned()),
			..library.clone().into()
		}
		.update(&conn)
		.await
		.unwrap();
		let (_, files) = series_with_files(
			&conn,
			&library.id,
			"science comics",
			&[("science_comics_001", "cbz", 36)],
		)
		.await;
		let backend = std::sync::Arc::new(TestBackend::new(conn));
		let chapter_id = KavitaIds::resolve(backend.conn(), IdKind::Media, &files[0].id)
			.await
			.unwrap();
		(backend, user, chapter_id)
	}

	#[tokio::test]
	async fn search_groups_every_entity_kind_and_never_omits_one() {
		let (backend, user, _) = backend().await;

		let (status, body) = request(
			backend.clone(),
			&user,
			"GET",
			"/api/Search/search?queryString=comics&includeChapterAndFiles=true",
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK, "{body}");
		for group in GROUPS {
			assert!(body.get(group).is_some_and(|v| v.is_array()), "{group}");
		}
		assert_eq!(body["series"][0]["name"], "science comics");
		assert_eq!(body["series"][0]["volumeCount"], 1);
		assert_eq!(body["series"][0]["chapterCount"], 1);
		assert_eq!(body["series"][0]["libraryName"], "Test Library");
		assert_eq!(body["chapters"].as_array().unwrap().len(), 1);
		assert_eq!(body["files"].as_array().unwrap().len(), 1);

		// The library name matches a different term than the series does.
		let (_, by_library) = request(
			backend.clone(),
			&user,
			"GET",
			"/api/search/search?queryString=Test%20Library",
			None,
		)
		.await;
		assert_eq!(by_library["libraries"].as_array().unwrap().len(), 1);
		assert!(by_library["series"].as_array().unwrap().is_empty());

		// `includeChapterAndFiles=false` empties exactly those two groups.
		let (_, without_files) = request(
			backend.clone(),
			&user,
			"GET",
			"/api/Search/search?queryString=comics&includeChapterAndFiles=false",
			None,
		)
		.await;
		assert!(without_files["files"].as_array().unwrap().is_empty());
		assert!(without_files["chapters"].as_array().unwrap().is_empty());
		assert_eq!(without_files["series"].as_array().unwrap().len(), 1);

		// A term nothing matches is every group empty, not a 404.
		let (status, empty) = request(
			backend.clone(),
			&user,
			"GET",
			"/api/Search/search?queryString=zzzznope",
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		for group in GROUPS {
			assert!(empty[group].as_array().unwrap().is_empty(), "{group}");
		}

		// A blank query is rejected, like `kavita-ref`.
		let (status, _) = request(
			backend,
			&user,
			"GET",
			"/api/Search/search?queryString=",
			None,
		)
		.await;
		assert_eq!(status, StatusCode::BAD_REQUEST);
	}

	#[tokio::test]
	async fn series_for_chapter_resolves_the_owning_series_and_204s_otherwise() {
		let (backend, user, chapter_id) = backend().await;
		let (status, body) = request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Search/series-for-chapter?chapterId={chapter_id}"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(body["name"], "science comics");
		assert_eq!(body["pages"], 36);

		let (status, _) = request(
			backend,
			&user,
			"GET",
			"/api/Search/series-for-chapter?chapterId=999999",
			None,
		)
		.await;
		assert_eq!(status, StatusCode::NO_CONTENT);
	}
}
