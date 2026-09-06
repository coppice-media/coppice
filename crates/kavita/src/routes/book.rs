//! `BookController`: the EPUB reader surface Inkita and Kover drive.
//!
//! Kavita addresses an EPUB by spine item and calls that a page. Stump's page
//! count for an EPUB is the Readium synthetic count — a spine item is worth
//! `ceil(compressed_size / 1KiB)` pages, at least one — and that is the count
//! the whole Kavita profile already reports for the chapter and clamps
//! progress against. These routes therefore keep Stump's page space:
//! `book-info.pages` is the chapter's `pages`, `book-page?page=` serves the
//! spine item that covers that page, and a navigation entry sits on the first
//! page of the spine item it points at. Progress written by the book reader
//! then lands in the same space as progress written through
//! `POST /api/Reader/progress` by every other client.

use std::sync::Arc;

use axum::{
	body::Body,
	extract::{Path, Query},
	http::{header, HeaderValue, StatusCode},
	response::{IntoResponse, Response},
	routing::get,
	Extension, Json, Router,
};
use models::entity::user::AuthUser;
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::{
	dto::{BookChapterItemDto, BookInfoDto},
	errors::{APIError, APIResult},
	mapper::map_book_info,
};

use super::{
	query::find_media, route_ci, KavitaBackend, KavitaBookResource, KavitaBookStructure,
	KavitaNavPoint,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageQuery {
	#[serde(default)]
	page: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileQuery {
	#[serde(default)]
	file: Option<String>,
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(router, "/api/Book/{chapterId}/book-info", get(book_info));
	let router = route_ci(router, "/api/Book/{chapterId}/chapters", get(book_chapters));
	let router = route_ci(router, "/api/Book/{chapterId}/book-page", get(book_page));
	route_ci(
		router,
		"/api/Book/{chapterId}/book-resources",
		get(book_resources),
	)
}

/// `BookController.GetBookInfo`. Kavita answers for any chapter, not just an
/// EPUB one: `kavita-ref` returns the same block for a CBZ with an empty
/// `bookTitle`, so the route never inspects the file.
pub(crate) async fn book_info_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	chapter_id: i32,
) -> APIResult<BookInfoDto> {
	let (input, index) = find_media(ctx, user, chapter_id)
		.await?
		.ok_or_else(|| APIError::NotFound("Chapter does not exist".to_owned()))?;
	Ok(map_book_info(&input, &input.media[index]))
}

async fn book_info(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(chapter_id): Path<i32>,
) -> APIResult<Json<BookInfoDto>> {
	let user = auth.user();
	Ok(Json(book_info_for(ctx.as_ref(), &user, chapter_id).await?))
}

fn map_nav(
	points: &[KavitaNavPoint],
	structure: &KavitaBookStructure,
) -> Vec<BookChapterItemDto> {
	points
		.iter()
		.map(|point| BookChapterItemDto {
			title: point.title.clone(),
			part: point.fragment.clone(),
			page: structure.page_for_spine_index(point.spine_index),
			children: map_nav(&point.children, structure),
		})
		.collect()
}

/// `BookController.GetBookChapters`: the EPUB navigation tree, each entry on
/// the page its spine item starts at.
pub(crate) async fn book_chapters_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	chapter_id: i32,
) -> APIResult<Vec<BookChapterItemDto>> {
	let (input, index) = find_media(ctx, user, chapter_id)
		.await?
		.ok_or_else(|| APIError::NotFound("Chapter does not exist".to_owned()))?;
	let structure = ctx
		.book_structure(user, &input.media[index].media.id)
		.await?;
	Ok(map_nav(&structure.navigation, &structure))
}

async fn book_chapters(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(chapter_id): Path<i32>,
) -> APIResult<Json<Vec<BookChapterItemDto>>> {
	let user = auth.user();
	Ok(Json(
		book_chapters_for(ctx.as_ref(), &user, chapter_id).await?,
	))
}

/// Rewrite the `epub://<package-relative-path>` URIs Stump's EPUB reader
/// emits onto this chapter's `book-resources` route, which is the shape
/// Kavita's own page HTML carries and the one Inkita's reader appends its
/// `apiKey` to (`ui/reader/screen/ReaderScreen.kt:1108-1112`).
pub(crate) fn rewrite_epub_uris(html: &str, chapter_id: i32) -> String {
	const PREFIX: &str = "epub://";
	let mut out = String::with_capacity(html.len());
	let mut rest = html;
	while let Some(at) = rest.find(PREFIX) {
		out.push_str(&rest[..at]);
		let tail = &rest[at + PREFIX.len()..];
		// The URI runs to the end of the quoted attribute value; a space or
		// `&` inside a resource path is part of the path, not a terminator.
		let end = tail.find(['"', '\'', '>']).unwrap_or(tail.len());
		out.push_str(&format!(
			"/api/Book/{chapter_id}/book-resources?file={}",
			encode_query_value(&tail[..end])
		));
		rest = &tail[end..];
	}
	out.push_str(rest);
	out
}

/// Percent-encode the characters that would end or reinterpret a query value.
fn encode_query_value(value: &str) -> String {
	let mut encoded = String::with_capacity(value.len());
	for byte in value.bytes() {
		match byte {
			b'A'..=b'Z'
			| b'a'..=b'z'
			| b'0'..=b'9'
			| b'-'
			| b'_'
			| b'.'
			| b'~'
			| b'/' => encoded.push(byte as char),
			_ => encoded.push_str(&format!("%{byte:02X}")),
		}
	}
	encoded
}

/// `BookController.GetBookPage`: the spine document covering Stump page
/// `page`, as scoped HTML.
pub(crate) async fn book_page_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	chapter_id: i32,
	page: i32,
) -> APIResult<String> {
	let (input, index) = find_media(ctx, user, chapter_id)
		.await?
		.ok_or_else(|| APIError::NotFound("Chapter does not exist".to_owned()))?;
	let media_id = input.media[index].media.id.clone();
	let structure = ctx.book_structure(user, &media_id).await?;
	let spine_index = structure.spine_index_for_page(page).ok_or_else(|| {
		APIError::BadRequest(format!("Page {page} is not part of this book"))
	})?;
	let resource = ctx.book_page(user, &media_id, spine_index).await?;
	let html = String::from_utf8(resource.data).map_err(|error| {
		APIError::InternalServerError(format!("Book page is not valid UTF-8: {error}"))
	})?;
	Ok(rewrite_epub_uris(&html, chapter_id))
}

/// Kavita serves book pages as `text/plain`, which is what its own reader and
/// Inkita's `Response<String>` expect.
async fn book_page(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(chapter_id): Path<i32>,
	Query(query): Query<PageQuery>,
) -> APIResult<Response> {
	let user = auth.user();
	let html = book_page_for(
		ctx.as_ref(),
		&user,
		chapter_id,
		query.page.unwrap_or_default(),
	)
	.await?;
	Ok((
		[(
			header::CONTENT_TYPE,
			HeaderValue::from_static("text/plain; charset=utf-8"),
		)],
		html,
	)
		.into_response())
}

/// `BookController.GetBookPageResources`: one resource from inside the EPUB.
/// Kavita answers `400 File was not found in book` for a miss, which is what
/// the reader treats as a broken image rather than a dead session.
pub(crate) async fn book_resource_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	chapter_id: i32,
	file: &str,
) -> APIResult<KavitaBookResource> {
	if file.trim().is_empty() {
		return Err(APIError::BadRequest(
			"File was not found in book".to_owned(),
		));
	}
	let (input, index) = find_media(ctx, user, chapter_id)
		.await?
		.ok_or_else(|| APIError::NotFound("Chapter does not exist".to_owned()))?;
	let media_id = input.media[index].media.id.clone();
	ctx.book_resource(user, &media_id, file)
		.await
		.map_err(|error| match error {
			APIError::NotFound(_) => {
				APIError::BadRequest("File was not found in book".to_owned())
			},
			other => other,
		})
}

async fn book_resources(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(chapter_id): Path<i32>,
	Query(query): Query<FileQuery>,
) -> APIResult<Response> {
	let user = auth.user();
	let resource = book_resource_for(
		ctx.as_ref(),
		&user,
		chapter_id,
		query.file.as_deref().unwrap_or_default(),
	)
	.await?;
	let mut response = Response::new(Body::from(resource.data));
	*response.status_mut() = StatusCode::OK;
	response.headers_mut().insert(
		header::CONTENT_TYPE,
		HeaderValue::from_str(&resource.content_type)
			.unwrap_or(HeaderValue::from_static("application/octet-stream")),
	);
	Ok(response)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::routes::{KavitaBookStructure, KavitaSpineItem};

	fn structure(pages: &[i32]) -> KavitaBookStructure {
		KavitaBookStructure {
			spine: pages
				.iter()
				.map(|pages| KavitaSpineItem { pages: *pages })
				.collect(),
			navigation: Vec::new(),
		}
	}

	/// A page maps to the spine item whose synthetic page budget covers it;
	/// past the end there is no item, and a zero-page (non-linear) item is
	/// never selected.
	#[test]
	fn pages_map_onto_the_spine_budget() {
		let book = structure(&[1, 3, 1]);
		assert_eq!(book.page_offsets(), (vec![0, 1, 4], 5));
		assert_eq!(book.spine_index_for_page(0), Some(0));
		assert_eq!(book.spine_index_for_page(1), Some(1));
		assert_eq!(book.spine_index_for_page(3), Some(1));
		assert_eq!(book.spine_index_for_page(4), Some(2));
		assert_eq!(book.spine_index_for_page(5), None);
		assert_eq!(book.spine_index_for_page(-1), None);
		assert_eq!(book.page_for_spine_index(2), 4);

		let with_non_linear = structure(&[2, 0, 2]);
		assert_eq!(with_non_linear.spine_index_for_page(2), Some(2));
	}

	/// `epub://` URIs become `book-resources` links on this chapter, with the
	/// path percent-encoded so a space or `&` cannot end the query value.
	#[test]
	fn epub_uris_become_book_resource_links() {
		let html = concat!(
			r#"<img src="epub://OEBPS/Images/cover.jpg"/>"#,
			r#"<a href="epub://OEBPS/ch 1&2.xhtml">next</a>"#,
			r#"<a href="https://example.com/x">out</a>"#,
		);
		assert_eq!(
			rewrite_epub_uris(html, 4),
			concat!(
				r#"<img src="/api/Book/4/book-resources?file=OEBPS/Images/cover.jpg"/>"#,
				r#"<a href="/api/Book/4/book-resources?file=OEBPS/ch%201%262.xhtml">next</a>"#,
				r#"<a href="https://example.com/x">out</a>"#,
			)
		);
	}
}
