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
	http::{header, HeaderMap, HeaderValue, StatusCode},
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

/// Where a fragment's resource references point. Kavita rewrites them onto an
/// absolute `//{host}/api/book/{chapterId}/book-resources?apiKey=&file=`
/// (`kavita-ref` 0.9.1.4 `GET /api/Book/3/book-page?page=0` emits
/// `//127.0.0.1:25620/api/book/3/book-resources?apiKey=<key>&file=<name>`),
/// which resolves both in a `WebView` whose base URL is the server — Inkita's
/// is (`ui/reader/screen/ReaderScreen.kt:1304-1305`) — and in a document
/// loaded straight off the server. The key is only known when the request
/// carried one: Stump stores API keys hashed, so a JWT-authenticated request
/// leaves it out and the reader appends its own, which Inkita already does
/// idempotently (`ui/reader/screen/ReaderScreen.kt:1109-1112`).
pub(crate) struct ResourceBase {
	host: Option<String>,
	api_key: Option<String>,
}

impl ResourceBase {
	fn from_request(headers: &HeaderMap, auth: &AuthContext) -> Self {
		Self {
			host: headers
				.get(header::HOST)
				.and_then(|value| value.to_str().ok())
				.filter(|value| !value.is_empty())
				.map(str::to_owned),
			api_key: auth.api_key(),
		}
	}

	/// The `book-resources` URL of one file inside `chapter_id`'s EPUB.
	fn url(&self, chapter_id: i32, file: &str) -> String {
		let authority = match &self.host {
			Some(host) => format!("//{host}"),
			None => String::new(),
		};
		let key = match &self.api_key {
			Some(key) => format!("apiKey={}&", encode_query_value(key)),
			None => String::new(),
		};
		format!(
			"{authority}/api/book/{chapter_id}/book-resources?{key}file={}",
			encode_query_value(file)
		)
	}
}

/// Rewrite the `epub://<package-relative-path>` URIs Stump's EPUB reader
/// emits onto this chapter's `book-resources` route, which is the shape
/// Kavita's own page HTML carries.
pub(crate) fn rewrite_epub_uris(
	html: &str,
	chapter_id: i32,
	base: &ResourceBase,
) -> String {
	const PREFIX: &str = "epub://";
	let mut out = String::with_capacity(html.len());
	let mut rest = html;
	while let Some(at) = rest.find(PREFIX) {
		out.push_str(&rest[..at]);
		let tail = &rest[at + PREFIX.len()..];
		// The URI runs to the end of the quoted attribute value or of the
		// `url()` that holds it; a space or `&` inside a resource path is
		// part of the path, not a terminator.
		let end = tail.find(['"', '\'', '>', ')']).unwrap_or(tail.len());
		out.push_str(&base.url(chapter_id, &tail[..end]));
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

/// The class Kavita's reader wraps a page fragment in, and therefore the
/// scope every rule of the book's own CSS is confined to.
const SCOPE: &str = ".book-content";

/// The pieces of a spine document Kavita's `BookService.GetBookPage` keeps:
/// the body's classes, the body's markup, and the CSS the document carries,
/// either inline or by reference. Everything else — the XML prolog, `<html>`,
/// `<head>`, `<title>`, `<meta>` — is dropped, because the reader injects the
/// fragment into a document it owns.
struct PageParts<'a> {
	body_class: &'a str,
	body: String,
	styles: Vec<&'a str>,
	stylesheets: Vec<&'a str>,
}

impl<'a> PageParts<'a> {
	fn of(document: &'a str) -> Self {
		let (head, body_attrs, body) = split_body(document);
		let mut styles = Vec::new();
		let mut stylesheets = Vec::new();
		for source in [head, body] {
			collect_style_elements(source, &mut styles);
			collect_stylesheet_links(source, &mut stylesheets);
		}
		Self {
			body_class: attribute(body_attrs, "class").unwrap_or_default(),
			body: strip_style_elements(body),
			styles,
			stylesheets,
		}
	}

	/// `<div class="<body classes>">` holding the scoped CSS and the body.
	fn into_fragment(self, css: &str) -> String {
		let scoped = scope_css(css);
		let mut out = String::with_capacity(self.body.len() + scoped.len() + 64);
		out.push_str("<div class=\"");
		out.push_str(self.body_class);
		out.push_str("\">");
		if !scoped.is_empty() {
			out.push_str("<style>");
			out.push_str(&scoped);
			out.push_str("</style>");
		}
		out.push_str(&self.body);
		out.push_str("</div>");
		out
	}
}

/// Everything before `<body`, the body's attribute text, and the body's
/// markup. A document without a body tag is all body, which is what a spine
/// item that is already a fragment amounts to.
fn split_body(document: &str) -> (&str, &str, &str) {
	let Some(open) = find_element(document, "body") else {
		return ("", "", document);
	};
	let head = &document[..open];
	let after_name = &document[open + "<body".len()..];
	let Some(attrs_end) = after_name.find('>') else {
		return (head, after_name, "");
	};
	let attrs = after_name[..attrs_end].trim_end_matches('/');
	let inner = &after_name[attrs_end + 1..];
	let body = match inner.rfind("</body") {
		Some(close) => &inner[..close],
		None => inner,
	};
	(head, attrs, body)
}

/// The byte offset of `<name` as an element start, not as the prefix of a
/// longer tag name.
fn find_element(html: &str, name: &str) -> Option<usize> {
	let mut from = 0;
	while let Some(at) = html[from..].find('<') {
		let at = from + at;
		let rest = &html[at + 1..];
		if rest.len() >= name.len() && rest[..name.len()].eq_ignore_ascii_case(name) {
			let next = rest[name.len()..].chars().next();
			if matches!(next, None | Some('>') | Some('/'))
				|| next.is_some_and(char::is_whitespace)
			{
				return Some(at);
			}
		}
		from = at + 1;
	}
	None
}

/// The value of an unquoted-name attribute in an element's attribute text.
fn attribute<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
	let mut rest = attrs;
	while let Some(at) = rest.to_ascii_lowercase().find(name) {
		let before = rest[..at].chars().next_back();
		let after = rest[at + name.len()..].trim_start();
		if before.is_none_or(char::is_whitespace) && after.starts_with('=') {
			let value = after[1..].trim_start();
			let quote = value.chars().next()?;
			if quote == '"' || quote == '\'' {
				let end = value[1..].find(quote)? + 1;
				return Some(&value[1..end]);
			}
			let end = value.find(char::is_whitespace).unwrap_or(value.len());
			return Some(&value[..end]);
		}
		rest = &rest[at + name.len()..];
	}
	None
}

fn collect_style_elements<'a>(html: &'a str, into: &mut Vec<&'a str>) {
	let mut rest = html;
	while let Some(open) = find_element(rest, "style") {
		let after_name = &rest[open..];
		let Some(content_at) = after_name.find('>') else {
			return;
		};
		let content = &after_name[content_at + 1..];
		let Some(close) = content.to_ascii_lowercase().find("</style") else {
			return;
		};
		into.push(&content[..close]);
		rest = &content[close..];
	}
}

fn collect_stylesheet_links<'a>(html: &'a str, into: &mut Vec<&'a str>) {
	let mut rest = html;
	while let Some(open) = find_element(rest, "link") {
		let after_name = &rest[open + "<link".len()..];
		let Some(attrs_end) = after_name.find('>') else {
			return;
		};
		let attrs = &after_name[..attrs_end];
		let is_sheet = attribute(attrs, "rel")
			.is_some_and(|rel| rel.to_ascii_lowercase().contains("stylesheet"));
		if let (true, Some(href)) = (is_sheet, attribute(attrs, "href")) {
			into.push(href);
		}
		rest = &after_name[attrs_end + 1..];
	}
}

/// The span of the next `<style>…</style>` or void `<link>` element, or
/// `None` once neither is left or the one that is left never closes.
fn next_style_span(html: &str) -> Option<(usize, usize)> {
	let style = find_element(html, "style").map(|at| {
		let end = html[at..]
			.to_ascii_lowercase()
			.find("</style")
			.and_then(|close| html[at + close..].find('>').map(|end| close + end + 1))
			.map(|end| at + end);
		(at, end)
	});
	let link = find_element(html, "link").map(|at| {
		let end = html[at..].find('>').map(|end| at + end + 1);
		(at, end)
	});
	let (at, end) = match (style, link) {
		(Some(style), Some(link)) if link.0 < style.0 => link,
		(Some(style), _) => style,
		(None, Some(link)) => link,
		(None, None) => return None,
	};
	Some((at, end?))
}

/// Drop `<style>` and `<link>` elements from body markup: their CSS is
/// inlined into the fragment's single scoped `<style>` instead, so leaving
/// them would load the unscoped copy as well.
fn strip_style_elements(body: &str) -> String {
	let mut out = String::with_capacity(body.len());
	let mut rest = body;
	while let Some((at, end)) = next_style_span(rest) {
		out.push_str(&rest[..at]);
		rest = &rest[end..];
	}
	out.push_str(rest);
	out
}

/// Confine every rule of a book's stylesheet to the fragment, the way
/// Kavita's `BookService.ScopeStyles` does: each selector is prefixed with
/// `.book-content`, a leading `html`/`body` element selector becomes
/// `.book-content` itself (`body{...}` comes back as
/// `.book-content .book-content{...}` on `kavita-ref`), `@page` is dropped
/// because a book's page box would fight the reader's own layout, and
/// at-rules that nest rules are scoped inside while `@charset`, `@import`,
/// `@font-face` and `@keyframes` pass through untouched.
fn scope_css(css: &str) -> String {
	let mut out = String::with_capacity(css.len());
	scope_rules(css, &mut out);
	out
}

/// At-rules whose body is a list of rules rather than declarations.
const NESTED_AT_RULES: [&str; 5] =
	["media", "supports", "document", "layer", "container"];

fn scope_rules(css: &str, out: &mut String) {
	let mut rest = css;
	loop {
		rest = skip_ignorable(rest);
		if rest.is_empty() {
			return;
		}
		if rest.starts_with('@') {
			let name_end = rest[1..]
				.find(|c: char| !c.is_ascii_alphanumeric() && c != '-')
				.map_or(rest.len(), |at| at + 1);
			let name = rest[1..name_end].to_ascii_lowercase();
			let Some(prelude_end) = rest.find(['{', ';']) else {
				return;
			};
			if rest.as_bytes()[prelude_end] == b';' {
				out.push_str(&rest[..=prelude_end]);
				rest = &rest[prelude_end + 1..];
				continue;
			}
			let Some(body_end) = matching_brace(rest, prelude_end) else {
				return;
			};
			if name != "page" {
				if NESTED_AT_RULES.contains(&name.as_str()) {
					out.push_str(&rest[..=prelude_end]);
					scope_rules(&rest[prelude_end + 1..body_end], out);
					out.push('}');
				} else {
					out.push_str(&rest[..=body_end]);
				}
			}
			rest = &rest[body_end + 1..];
			continue;
		}
		let Some(brace) = rest.find('{') else {
			return;
		};
		let Some(body_end) = matching_brace(rest, brace) else {
			return;
		};
		let selectors = scope_selectors(&rest[..brace]);
		if !selectors.is_empty() {
			out.push_str(&selectors);
			out.push_str(&rest[brace..=body_end]);
		}
		rest = &rest[body_end + 1..];
	}
}

/// Whitespace and comments before the next rule.
fn skip_ignorable(css: &str) -> &str {
	let mut rest = css.trim_start();
	while let Some(tail) = rest.strip_prefix("/*") {
		match tail.find("*/") {
			Some(end) => rest = tail[end + 2..].trim_start(),
			None => return "",
		}
	}
	rest
}

/// The offset of the `}` closing the `{` at `open`, honouring nesting.
fn matching_brace(css: &str, open: usize) -> Option<usize> {
	let mut depth = 0usize;
	for (at, byte) in css.bytes().enumerate().skip(open) {
		match byte {
			b'{' => depth += 1,
			b'}' => {
				depth -= 1;
				if depth == 0 {
					return Some(at);
				}
			},
			_ => {},
		}
	}
	None
}

fn scope_selectors(selectors: &str) -> String {
	let mut scoped: Vec<String> = Vec::new();
	for selector in split_selectors(selectors) {
		let selector = skip_ignorable(selector).trim_end();
		if selector.is_empty() {
			continue;
		}
		scoped.push(format!("{SCOPE} {}", replace_root_element(selector)));
	}
	scoped.join(",")
}

/// `body`/`html` name the document root, which the fragment does not have;
/// the fragment's own wrapper takes their place.
fn replace_root_element(selector: &str) -> String {
	for root in ["html", "body"] {
		if let Some(rest) = selector.strip_prefix(root) {
			if rest.is_empty()
				|| rest.starts_with(['.', '#', ':', '[', '>', '+', '~', ' '])
			{
				return format!("{SCOPE}{rest}");
			}
		}
	}
	selector.to_owned()
}

/// Split a selector list on its top-level commas: a comma inside `(…)`,
/// `[…]` or a string belongs to the selector, not to the list.
fn split_selectors(selectors: &str) -> Vec<&str> {
	let mut parts = Vec::new();
	let mut depth = 0usize;
	let mut quote: Option<char> = None;
	let mut start = 0;
	for (at, ch) in selectors.char_indices() {
		match (quote, ch) {
			(Some(open), ch) if ch == open => quote = None,
			(Some(_), _) => {},
			(None, '"' | '\'') => quote = Some(ch),
			(None, '(' | '[') => depth += 1,
			(None, ')' | ']') => depth = depth.saturating_sub(1),
			(None, ',') if depth == 0 => {
				parts.push(&selectors[start..at]);
				start = at + 1;
			},
			_ => {},
		}
	}
	parts.push(&selectors[start..]);
	parts
}

/// `BookController.GetBookPage`: the spine document covering Stump page
/// `page`, as the body fragment Kavita's reader contract defines.
pub(crate) async fn book_page_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	chapter_id: i32,
	page: i32,
	base: &ResourceBase,
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
	let document = utf8_page(resource.data)?;
	let parts = PageParts::of(&document);
	let mut css = parts.styles.join("\n");
	for href in &parts.stylesheets {
		let Some(path) = in_book_path(href) else {
			continue;
		};
		let Ok(sheet) = ctx.book_resource(user, &media_id, path).await else {
			// A book that names a stylesheet it does not ship still renders;
			// Kavita drops the missing sheet the same way.
			continue;
		};
		let sheet = utf8_page(sheet.data)?;
		if !css.is_empty() {
			css.push('\n');
		}
		css.push_str(&sheet);
	}
	Ok(rewrite_epub_uris(
		&parts.into_fragment(&css),
		chapter_id,
		base,
	))
}

fn utf8_page(data: Vec<u8>) -> APIResult<String> {
	String::from_utf8(data).map_err(|error| {
		APIError::InternalServerError(format!("Book page is not valid UTF-8: {error}"))
	})
}

/// The in-book path a stylesheet `href` names. Stump's EPUB reader rewrites
/// a spine document's internal references to `epub://<package-relative-path>`
/// and `KavitaBackend::book_resource` addresses that path, so the scheme has
/// to come off. An EPUB ships every reflowable resource it references, so an
/// absolute or `data:` URL is not a file this book has and its CSS cannot be
/// scoped into the fragment.
fn in_book_path(href: &str) -> Option<&str> {
	let path = href.strip_prefix("epub://").unwrap_or(href);
	if path.contains("://") || path.starts_with("data:") || path.starts_with("//") {
		return None;
	}
	Some(path.trim_start_matches('/')).filter(|path| !path.is_empty())
}

/// Kavita serves book pages as `text/plain`, which is what its own reader and
/// Inkita's `Response<String>` expect.
async fn book_page(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(chapter_id): Path<i32>,
	Query(query): Query<PageQuery>,
	headers: HeaderMap,
) -> APIResult<Response> {
	let user = auth.user();
	let base = ResourceBase::from_request(&headers, &auth);
	let html = book_page_for(
		ctx.as_ref(),
		&user,
		chapter_id,
		query.page.unwrap_or_default(),
		&base,
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

	fn base(host: Option<&str>, api_key: Option<&str>) -> ResourceBase {
		ResourceBase {
			host: host.map(str::to_owned),
			api_key: api_key.map(str::to_owned),
		}
	}

	/// `epub://` URIs become absolute `book-resources` links on this chapter,
	/// carrying the request's API key the way `kavita-ref` does, with the
	/// path percent-encoded so a space or `&` cannot end the query value. A
	/// `url()` in CSS terminates on its `)`, not only on a quote.
	#[test]
	fn epub_uris_become_absolute_book_resource_links() {
		let html = concat!(
			r#"<img src="epub://OEBPS/Images/cover.jpg"/>"#,
			r#"<a href="epub://OEBPS/ch 1&2.xhtml">next</a>"#,
			r#"<a href="https://example.com/x">out</a>"#,
			r#"<style>a{background:url(epub://OEBPS/bg.png)}</style>"#,
		);
		assert_eq!(
			rewrite_epub_uris(html, 4, &base(Some("h:25600"), Some("stump_a+b"))),
			concat!(
				r#"<img src="//h:25600/api/book/4/book-resources?apiKey=stump_a%2Bb&file=OEBPS/Images/cover.jpg"/>"#,
				r#"<a href="//h:25600/api/book/4/book-resources?apiKey=stump_a%2Bb&file=OEBPS/ch%201%262.xhtml">next</a>"#,
				r#"<a href="https://example.com/x">out</a>"#,
				r#"<style>a{background:url(//h:25600/api/book/4/book-resources?apiKey=stump_a%2Bb&file=OEBPS/bg.png)}</style>"#,
			)
		);
	}

	/// Without a `Host` header or an API key the link stays root-relative and
	/// keyless, which still resolves against a reader's base URL and lets
	/// Inkita append its own key
	/// (`ui/reader/screen/ReaderScreen.kt:1109-1112`).
	#[test]
	fn book_resource_links_degrade_to_a_root_relative_path() {
		assert_eq!(
			rewrite_epub_uris(r#"<img src="epub://a.png"/>"#, 7, &base(None, None)),
			r#"<img src="/api/book/7/book-resources?file=a.png"/>"#
		);
	}

	/// The page is the body fragment `kavita-ref` returns, never the whole
	/// document: the prolog, `<html>`, `<head>`, `<title>` and `<meta>` are
	/// gone, the body's classes land on the wrapping `<div>`, and the CSS the
	/// document carried inline is scoped inside it.
	#[test]
	fn book_page_becomes_a_scoped_body_fragment() {
		let document = concat!(
			"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n",
			"<html xmlns=\"http://www.w3.org/1999/xhtml\"><head>",
			"<title>Cover</title><meta name=\"calibre:cover\" content=\"true\"/>",
			"<style type=\"text/css\">@page {margin:0} body {text-align:center}</style>",
			"</head><body class=\"cover x-cover\"><div>page</div></body></html>",
		);
		let parts = PageParts::of(document);
		assert_eq!(parts.body_class, "cover x-cover");
		assert_eq!(parts.stylesheets, Vec::<&str>::new());
		let css = parts.styles.join("\n");
		assert_eq!(
			PageParts::of(document).into_fragment(&css),
			concat!(
				"<div class=\"cover x-cover\">",
				"<style>.book-content .book-content{text-align:center}</style>",
				"<div>page</div></div>",
			)
		);
	}

	/// A spine item that links its stylesheet names it for the caller to
	/// fetch, and the `<link>` never survives into the fragment: the scoped
	/// copy would otherwise be joined by an unscoped one. The href carries
	/// the `epub://` scheme Stump's EPUB reader writes, which comes off
	/// before `book_resource` sees it; a href pointing outside the book
	/// names no file to inline.
	#[test]
	fn linked_stylesheets_are_named_and_their_elements_dropped() {
		let document = concat!(
			"<html><head>",
			"<link href=\"epub://stylesheet.css\" rel=\"stylesheet\" type=\"text/css\"/>",
			"</head><body><p>text</p>",
			"<link rel=\"stylesheet\" href=\"epub://styles/late.css\"/>",
			"<link rel=\"stylesheet\" href=\"https://cdn.example.com/x.css\"/>",
			"<link rel=\"icon\" href=\"epub://favicon.ico\"/>",
			"</body></html>",
		);
		let parts = PageParts::of(document);
		assert_eq!(
			parts.stylesheets,
			vec![
				"epub://stylesheet.css",
				"epub://styles/late.css",
				"https://cdn.example.com/x.css"
			]
		);
		assert_eq!(
			in_book_path("epub://stylesheet.css"),
			Some("stylesheet.css")
		);
		assert_eq!(
			in_book_path("epub://styles/late.css"),
			Some("styles/late.css")
		);
		assert_eq!(in_book_path("https://cdn.example.com/x.css"), None);
		assert_eq!(in_book_path("epub://"), None);
		assert_eq!(parts.into_fragment(""), "<div class=\"\"><p>text</p></div>");
	}

	/// A spine item that is already a fragment has no body element to split
	/// on and is kept whole.
	#[test]
	fn a_body_less_document_is_all_fragment() {
		assert_eq!(
			PageParts::of("<p>bare</p>").into_fragment(""),
			"<div class=\"\"><p>bare</p></div>"
		);
	}

	/// Every rule is confined to `.book-content`: `body`/`html` become the
	/// scope itself, `@page` is dropped, `@media` is scoped through, and
	/// `@charset`/`@font-face` pass untouched. Commas inside `:not(...)`
	/// do not split a selector list, comments do not leak, and the selector
	/// list comes out minified the way `kavita-ref`'s does.
	#[test]
	fn css_is_scoped_to_the_fragment() {
		let css = concat!(
			"@charset \"utf-8\";\n",
			"@page {margin: 0}\n",
			"/* comment */ body, body.tei {color: #000}\n",
			"div, p {margin-left: 0}\n",
			"@font-face {font-family: 'x'; src: url(x.otf)}\n",
			"@media screen {h1 {page-break-before: always} @page {margin:1pt}}\n",
			"a:not(.x, .y) {color: red}\n",
		);
		assert_eq!(
			scope_css(css),
			concat!(
				"@charset \"utf-8\";",
				".book-content .book-content,.book-content .book-content.tei{color: #000}",
				".book-content div,.book-content p{margin-left: 0}",
				"@font-face {font-family: 'x'; src: url(x.otf)}",
				"@media screen {.book-content h1{page-break-before: always}}",
				".book-content a:not(.x, .y){color: red}",
			)
		);
	}

	/// Unbalanced CSS stops the walk instead of panicking or looping.
	#[test]
	fn unterminated_css_is_dropped() {
		assert_eq!(scope_css("p {color: red"), "");
		assert_eq!(scope_css("@media screen {p {color: red}"), "");
		assert_eq!(scope_css("/* never closed"), "");
	}
}
