//! The MyMangaReaderCMS theme (`lib-multisrc/mmrcms`).
//!
//! Only five extensions carry this theme at `064c1a0e`, and they have drifted
//! furthest from the base class: Read Comics Online, for instance, was rebuilt
//! on a Tailwind layout and overrides its list, details, chapter and page
//! parsing wholesale. Every extraction point is therefore a knob, defaulting to
//! the base class value — a redesigned host stays expressible as data instead
//! of needing code.
//!
//! # Knobs and where they come from
//!
//! | knob | Kotlin member | default |
//! |---|---|---|
//! | `item_path` (alias `item_url`) | `MMRCMS.itemPath` | `manga` |
//! | `supports_advanced_search` | `MMRCMS.supportsAdvancedSearch` | `true` |
//! | `date_format` / `date_locale` | `MMRCMS.dateFormat` | `d MMM. yyyy`, `en-US` |
//! | `chapter_string` | `MMRCMS.chapterString` | `Chapter`/`Capítulo`/`Chapitre` by `lang` |
//! | `chapter_name_prefix` | `MMRCMS.chapterNamePrefix` | empty |
//! | `popular_manga_url` (alias `popular_url`) | `MMRCMS.popularMangaRequest` | `/filterList?page={page}&sortBy=views&asc=false` |
//! | `latest_updates_url` (alias `latest_url`) | `MMRCMS.latestUpdatesRequest` | `/latest-release?page={page}` |
//! | `search_manga_url` (alias `search_url`) | `MMRCMS.searchMangaRequest` | `/search?query={query}` |
//! | `popular_manga_selector` | `popularMangaSelector()` | `div.media` |
//! | `latest_updates_selector` | `latestUpdatesSelector()` | `div.mangalist div.manga-item` |
//! | `search_manga_selector` | `searchMangaSelector()` | `div.media` |
//! | `popular_manga_next_page_selector` | `popularMangaNextPageSelector()` | `.pagination a[rel=next]` |
//! | `search_manga_next_page_selector` | the search response's own control | `popular_manga_next_page_selector` |
//! | `manga_url_selector` | the anchor inside a card | `.media-heading a, .manga-heading a` |
//! | `details_title_selector` | `MMRCMS.detailsTitleSelector` | `.listmanga-header, .widget-title` |
//! | `details_thumbnail_selector` | `.row img.img-responsive` | same |
//! | `details_description_selector` | `.row .well` | same |
//! | `chapter_list_selector` | `chapterListSelector()` | `ul.chapters > li:not(.btn)` |
//! | `chapter_url_selector` | `.chapter-title-rtl a` | same |
//! | `chapter_name_selector` | `.chapter-title-rtl` | same |
//! | `chapter_date_selector` | `.date-chapter-title-rtl` | same |
//! | `page_list_selector` | `pageListParse` | `#all > img.img-responsive` |
//!
//! An anchor knob (`manga_url_selector`, `search_manga_url_selector`,
//! `chapter_url_selector`) set to `:self` means the matched card *is* the
//! anchor, which is how a Tailwind rebuild wraps a whole cell in one `<a>`.
//! `chapter_string` is the one knob where an explicitly empty string differs
//! from an absent one: `""` strips the repeated series title, absent falls
//! back to the per-language default below.
//!
//! `{page}` and `{query}` in the URL knobs are substituted; `{query}` is
//! percent-encoded. A knob may be a path or a full URL.
//!
//! # Search
//!
//! The base class' text search hits `/search?query=` and gets JSON back
//! (`Dto.kt`: `{"suggestions":[{"value","data"}]}`), then pages that directory
//! locally 24 at a time. When `search_url` is overridden to an HTML endpoint
//! (Read Comics Online uses `/advanced-search?name=`), the response is parsed
//! with `search_manga_selector` instead. Which one applies is decided by the
//! response body, not by configuration, so a host that changes shape does not
//! need a new knob.
//!
//! # Remote ids
//!
//! Series ids are the slug under `item_path`; chapter ids are
//! `{series_slug}/{chapter_segments}`, since MMRCMS chapter URLs carry a
//! volume/chapter pair (`/comic/foo/12` or `/manga/foo/1/3`).

use std::sync::Arc;

use async_trait::async_trait;
use models::entity::provider_source;
use serde::Deserialize;
use stump_provider::{
	definition::{KnobValue, SourceDefinition},
	RemoteChapter, RemotePage, RemoteSeries, SearchFilter, Source, SourceCapabilities,
	SourceHttp, SourceInfo, SourcePage, SourceResult,
};

use crate::{
	date::DateParser,
	dom::{Document, NodeId},
	selector::Selector,
	theme::{self, ImageAttrs, ThemeContext, ThemeError},
};

pub const THEME: &str = "mmrcms";

/// `MMRCMS.Element.imgAttr()` attribute order.
const IMAGE_ATTRS: ImageAttrs = ImageAttrs(&[
	"data-background-image",
	"data-cfsrc",
	"data-lazy-src",
	"data-src",
	"srcset",
	"src",
]);

/// `MMRCMS.parseSearchDirectory` page size.
const DIRECTORY_PAGE_SIZE: usize = 24;

/// Cover placeholder the base class replaces with a guessed path.
const NO_IMAGE: &str = "no-image.png";

#[derive(Debug, Deserialize)]
struct SearchResult {
	#[serde(default)]
	suggestions: Vec<Suggestion>,
}

#[derive(Debug, Deserialize)]
struct Suggestion {
	value: String,
	data: String,
}

#[derive(Debug)]
struct Selectors {
	popular: Selector,
	latest: Selector,
	search: Selector,
	next_page: Option<Selector>,
	search_next_page: Option<Selector>,
	item_url: Option<Selector>,
	item_title: Option<Selector>,
	search_item_url: Option<Selector>,
	search_item_title: Option<Selector>,
	title: Selector,
	thumbnail: Selector,
	description: Selector,
	detail_rows: Selector,
	status: Option<Selector>,
	genre: Option<Selector>,
	author: Option<Selector>,
	chapter_list: Selector,
	chapter_url: Option<Selector>,
	chapter_name: Selector,
	chapter_date: Selector,
	pages: Selector,
}

/// An MMRCMS-themed site driven by a definition.
#[derive(Debug)]
pub struct MmrcmsSource {
	context: ThemeContext,
	selectors: Selectors,
	dates: DateParser,
	item_path: String,
	popular_url: String,
	latest_url: String,
	search_url: String,
	chapter_string: String,
	chapter_name_prefix: String,
	nsfw: bool,
}

impl MmrcmsSource {
	pub fn build(
		definition: &SourceDefinition,
		row: &provider_source::Model,
	) -> Result<Arc<dyn Source>, ThemeError> {
		Self::new(definition, &row.id, Some(row.base_url.as_str()))
			.map(|source| Arc::new(source) as Arc<dyn Source>)
	}

	pub fn new(
		definition: &SourceDefinition,
		instance_id: &str,
		base_url: Option<&str>,
	) -> Result<Self, ThemeError> {
		let supports_latest = definition.bool_knob(&["supports_latest"]).unwrap_or(true);
		let context = ThemeContext::new(
			definition,
			instance_id,
			base_url,
			SourceCapabilities {
				popular: true,
				latest: supports_latest,
				search: true,
			},
		)?;
		let card_default = "div.media";
		let selectors = Selectors {
			popular: theme::selector(
				definition,
				"popular_manga_selector",
				&["search_manga_selector"],
				card_default,
			)?,
			latest: theme::selector(
				definition,
				"latest_updates_selector",
				&["popular_manga_selector"],
				"div.mangalist div.manga-item",
			)?,
			search: theme::selector(
				definition,
				"search_manga_selector",
				&[],
				card_default,
			)?,
			next_page: theme::optional_selector(
				definition,
				"popular_manga_next_page_selector",
				&["search_manga_next_page_selector"],
				Some(".pagination a[rel=next]"),
			)?,
			search_next_page: theme::optional_selector(
				definition,
				"search_manga_next_page_selector",
				&["popular_manga_next_page_selector"],
				Some(".pagination a[rel=next]"),
			)?,
			item_url: theme::optional_selector(
				definition,
				"manga_url_selector",
				&[],
				Some(".media-heading a, .manga-heading a"),
			)?,
			item_title: theme::optional_selector(
				definition,
				"manga_title_selector",
				&[],
				None,
			)?,
			search_item_url: theme::optional_selector(
				definition,
				"search_manga_url_selector",
				&["manga_url_selector"],
				Some(".media-heading a, .manga-heading a"),
			)?,
			search_item_title: theme::optional_selector(
				definition,
				"search_manga_title_selector",
				&["manga_title_selector"],
				None,
			)?,
			title: theme::selector(
				definition,
				"details_title_selector",
				&[],
				".listmanga-header, .widget-title",
			)?,
			thumbnail: theme::selector(
				definition,
				"details_thumbnail_selector",
				&[],
				".row img.img-responsive",
			)?,
			description: theme::selector(
				definition,
				"details_description_selector",
				&[],
				".row .well",
			)?,
			detail_rows: theme::selector(
				definition,
				"details_row_selector",
				&[],
				".row .dl-horizontal dt",
			)?,
			status: theme::optional_selector(
				definition,
				"details_status_selector",
				&[],
				None,
			)?,
			genre: theme::optional_selector(
				definition,
				"details_genre_selector",
				&[],
				None,
			)?,
			author: theme::optional_selector(
				definition,
				"details_author_selector",
				&[],
				None,
			)?,
			chapter_list: theme::selector(
				definition,
				"chapter_list_selector",
				&[],
				"ul.chapters > li:not(.btn)",
			)?,
			chapter_url: theme::optional_selector(
				definition,
				"chapter_url_selector",
				&[],
				Some(".chapter-title-rtl a"),
			)?,
			chapter_name: theme::selector(
				definition,
				"chapter_name_selector",
				&[],
				".chapter-title-rtl",
			)?,
			chapter_date: theme::selector(
				definition,
				"chapter_date_selector",
				&[],
				".date-chapter-title-rtl",
			)?,
			pages: theme::selector(
				definition,
				"page_list_selector",
				&["page_selector", "page_list_parse_selector"],
				"#all > img.img-responsive",
			)?,
		};
		let item_path = definition
			.text_knob(&["item_path", "item_url"])
			.unwrap_or("manga")
			.trim_matches('/')
			.to_string();
		Ok(Self {
			context,
			selectors,
			dates: DateParser::new(
				definition
					.text_knob(&["date_format"])
					.unwrap_or("d MMM. yyyy"),
				definition.text_knob(&["date_locale"]).unwrap_or("en-US"),
			),
			// The generator names these after the Kotlin member
			// (`popularMangaRequest` -> `popular_manga_url`); the shorter
			// names are accepted as aliases.
			popular_url: definition
				.text_knob(&["popular_manga_url", "popular_url"])
				.unwrap_or("/filterList?page={page}&sortBy=views&asc=false")
				.to_string(),
			latest_url: definition
				.text_knob(&["latest_updates_url", "latest_url"])
				.unwrap_or("/latest-release?page={page}")
				.to_string(),
			search_url: definition
				.text_knob(&["search_manga_url", "search_url"])
				.unwrap_or("/search?query={query}")
				.to_string(),
			// `override val chapterString = ""` (Read Comics Online) means
			// "drop the repeated series title", not "fall back to `Chapter`",
			// so an explicitly empty knob must survive: `text_knob` filters
			// empty strings, the raw knob does not.
			chapter_string: definition
				.knob(&["chapter_string"])
				.and_then(KnobValue::as_str)
				.map(str::to_string)
				.unwrap_or_else(|| default_chapter_string(definition.lang()).to_string()),
			chapter_name_prefix: definition
				.text_knob(&["chapter_name_prefix"])
				.unwrap_or_default()
				.to_string(),
			nsfw: definition.nsfw,
			item_path,
		})
	}

	fn expand(&self, template: &str, page: u32, query: &str) -> String {
		let expanded = template
			.replace("{page}", &page.to_string())
			.replace("{query}", &urlencode(query));
		self.context.url(&expanded)
	}

	/// A browse URL knob without a `{page}` placeholder is one fixed page
	/// (`bg.utsukushii` overrides `popularMangaRequest` with a bare
	/// `/manga-list`). Re-requesting it for page 2 would hand the
	/// materialiser the same series forever, so the list ends after page 1.
	fn exhausted(&self, template: &str, page: u32) -> bool {
		page > 1 && !template.contains("{page}")
	}

	fn series_url(&self, slug: &str) -> String {
		self.context.url(&format!("/{}/{slug}", self.item_path))
	}

	fn chapter_url(&self, chapter_id: &str) -> String {
		self.context
			.url(&format!("/{}/{chapter_id}", self.item_path))
	}

	/// `MMRCMS.guessCover`: a missing or placeholder cover is served from the
	/// upload directory under the series slug.
	fn guess_cover(&self, slug: &str, url: Option<String>) -> Option<String> {
		match url {
			Some(url) if !url.ends_with(NO_IMAGE) => Some(url),
			_ => Some(format!(
				"{}/uploads/manga/{slug}/cover/cover_250x350.jpg",
				self.context.base_url
			)),
		}
	}

	/// `searchMangaFromElement` and its popular/latest aliases. `next_page` is
	/// passed in because a redesigned host paginates its search results with a
	/// different control than its directory (Read Comics Online: `nav
	/// a[rel=next]` browsing, `span a[rel=next]` searching).
	pub fn parse_cards(
		&self,
		document: &Document,
		list: &Selector,
		item_url: Option<&Selector>,
		item_title: Option<&Selector>,
		next_page: Option<&Selector>,
	) -> SourcePage<RemoteSeries> {
		let root = document.root();
		let mut items = Vec::new();
		for card in list.select(document, root) {
			let anchor = match item_url {
				Some(selector) => selector.select_first(document, card),
				None => Some(card),
			}
			.or_else(|| (is_anchor(document, card)).then_some(card));
			let Some(anchor) = anchor else { continue };
			let Some(href) = document.abs_attr(anchor, "href") else {
				continue;
			};
			let Some(slug) = theme::slug_of(&href) else {
				continue;
			};
			let title = match item_title {
				Some(selector) => selector
					.select_first(document, card)
					.map(|node| document.text(node)),
				None => None,
			}
			.filter(|title| !title.trim().is_empty())
			.unwrap_or_else(|| {
				let text = document.text(anchor);
				if text.is_empty() {
					document
						.attr(anchor, "title")
						.map(str::to_string)
						.unwrap_or_else(|| slug.clone())
				} else {
					text
				}
			});
			let cover = document
				.descendants(card)
				.into_iter()
				.find(|node| is_image(document, *node))
				.and_then(|image| IMAGE_ATTRS.resolve(document, image));
			items.push(theme::series(
				&slug,
				title,
				Some(href),
				self.guess_cover(&slug, cover),
				self.nsfw,
			));
		}
		let has_next = next_page
			.is_some_and(|selector| selector.select_first(document, root).is_some());
		SourcePage { items, has_next }
	}

	/// One page of the JSON search directory, sliced locally as upstream does.
	fn directory_page(
		&self,
		suggestions: &[Suggestion],
		page: u32,
	) -> SourcePage<RemoteSeries> {
		let start = (page.saturating_sub(1) as usize) * DIRECTORY_PAGE_SIZE;
		let end = (start + DIRECTORY_PAGE_SIZE).min(suggestions.len());
		if start >= suggestions.len() {
			return SourcePage::default();
		}
		let items = suggestions[start..end]
			.iter()
			.map(|suggestion| {
				theme::series(
					&suggestion.data,
					suggestion.value.clone(),
					Some(self.series_url(&suggestion.data)),
					self.guess_cover(&suggestion.data, None),
					self.nsfw,
				)
			})
			.collect();
		SourcePage {
			items,
			has_next: end < suggestions.len(),
		}
	}

	/// `MMRCMS.mangaDetailsParse`.
	pub fn parse_details(&self, document: &Document, slug: &str) -> RemoteSeries {
		let root = document.root();
		let title = self
			.selectors
			.title
			.select_first(document, root)
			.map(|node| document.text(node))
			.filter(|title| !title.is_empty())
			.unwrap_or_else(|| slug.to_string());
		let cover = self
			.selectors
			.thumbnail
			.select_first(document, root)
			.and_then(|node| IMAGE_ATTRS.resolve(document, node));
		let description = self
			.selectors
			.description
			.select_first(document, root)
			.map(|node| description_text(document, node))
			.filter(|value| !value.is_empty());
		let mut series = RemoteSeries {
			remote_id: slug.to_string(),
			title,
			url: Some(document.url().to_string()),
			thumbnail_url: self.guess_cover(slug, cover),
			description,
			nsfw: self.nsfw,
			..Default::default()
		};
		// The base class walks the definition list and switches on the label.
		for row in self.selectors.detail_rows.select(document, root) {
			let label = document
				.text(row)
				.to_lowercase()
				.trim_end_matches(':')
				.trim()
				.to_string();
			let Some(value) = next_element_sibling(document, row) else {
				continue;
			};
			if DETAIL_AUTHOR.contains(&label.as_str()) {
				series.authors = vec![document.text(value)];
			} else if DETAIL_ARTIST.contains(&label.as_str()) {
				series.artists = vec![document.text(value)];
			} else if DETAIL_GENRE.contains(&label.as_str()) {
				series.genres = anchor_texts(document, value);
			} else if DETAIL_STATUS.contains(&label.as_str()) {
				series.status = theme::parse_status(&document.text(value));
			}
		}
		// Explicit selector knobs win over the label walk: a redesigned host
		// has no definition list to walk.
		if let Some(status) = self
			.selectors
			.status
			.as_ref()
			.and_then(|selector| selector.select_first(document, root))
		{
			series.status = theme::parse_status(&document.text(status));
		}
		if let Some(selector) = &self.selectors.genre {
			let genres: Vec<String> = selector
				.select(document, root)
				.into_iter()
				.map(|node| document.text(node))
				.filter(|value| !value.is_empty())
				.collect();
			if !genres.is_empty() {
				series.genres = genres;
			}
		}
		if let Some(selector) = &self.selectors.author {
			let authors: Vec<String> = selector
				.select(document, root)
				.into_iter()
				.map(|node| document.text(node))
				.filter(|value| !value.is_empty())
				.collect();
			if !authors.is_empty() {
				series.authors = authors;
			}
		}
		series
	}

	/// `MMRCMS.chapterListParse` / `chapterFromElement`.
	pub fn parse_chapters(&self, document: &Document, slug: &str) -> Vec<RemoteChapter> {
		let root = document.root();
		let series_title = self
			.selectors
			.title
			.select_first(document, root)
			.map(|node| document.text(node))
			.unwrap_or_default();
		let mut out = Vec::new();
		for item in self.selectors.chapter_list.select(document, root) {
			let anchor = match &self.selectors.chapter_url {
				Some(selector) => selector.select_first(document, item),
				None => Some(item),
			}
			.or_else(|| is_anchor(document, item).then_some(item));
			let Some(anchor) = anchor else { continue };
			let Some(href) = document.abs_attr(anchor, "href") else {
				continue;
			};
			let Some(chapter_id) = self.chapter_id(&href, slug) else {
				continue;
			};
			let raw_name = self
				.selectors
				.chapter_name
				.select_first(document, item)
				.map(|node| document.text(node))
				.filter(|name| !name.is_empty())
				.unwrap_or_else(|| document.text(anchor));
			let name = self.clean_chapter_name(&series_title, &raw_name);
			let uploaded_at = self
				.selectors
				.chapter_date
				.select_first(document, item)
				.map(|node| document.text(node))
				.and_then(|value| self.dates.parse(&value));
			out.push(theme::chapter(
				chapter_id,
				(!name.is_empty()).then(|| name.clone()),
				theme::chapter_number(&name),
				uploaded_at,
				Some(href),
				&self.context.info.lang,
			));
		}
		out
	}

	/// Everything after `/{item_path}/` in a chapter URL, which is
	/// `{series}/{chapter}` or `{series}/{volume}/{chapter}`.
	fn chapter_id(&self, href: &str, slug: &str) -> Option<String> {
		let path = href.split(['?', '#']).next().unwrap_or(href);
		let marker = format!("/{}/", self.item_path);
		let rest = path.split(&marker).nth(1)?.trim_matches('/');
		if rest.is_empty() {
			return None;
		}
		if rest.starts_with(slug) {
			return Some(rest.to_string());
		}
		Some(format!("{slug}/{rest}"))
	}

	/// `MMRCMS.cleanChapterName`: strip the series title a host repeats in
	/// every chapter label, then normalise `"a : b"` to `"a: b"`.
	pub fn clean_chapter_name(&self, series_title: &str, name: &str) -> String {
		let needle = format!("{}{series_title}", self.chapter_name_prefix);
		let initial = if !series_title.is_empty() && name.contains(&needle) {
			name.replacen(&needle, &self.chapter_string, 1)
		} else {
			name.to_string()
		};
		let mut parts = initial.splitn(2, ':').map(str::trim);
		let first = parts.next().unwrap_or_default().to_string();
		match parts.next() {
			Some(second) if !second.is_empty() && second != first => {
				format!("{first}: {second}")
			},
			_ => first,
		}
	}

	/// `MMRCMS.pageListParse`.
	pub fn parse_pages(&self, document: &Document) -> Vec<RemotePage> {
		let urls: Vec<String> = self
			.selectors
			.pages
			.select(document, document.root())
			.into_iter()
			.filter_map(|node| {
				let image = if is_image(document, node) {
					node
				} else {
					document
						.descendants(node)
						.into_iter()
						.find(|child| is_image(document, *child))?
				};
				IMAGE_ATTRS.resolve(document, image)
			})
			.collect();
		theme::pages_with_referer(urls, document.url())
	}
}

/// `MMRCMS.chapterString` defaults, keyed by the instance language.
fn default_chapter_string(lang: &str) -> &'static str {
	match lang {
		"es" => "Capítulo",
		"fr" => "Chapitre",
		_ => "Chapter",
	}
}

// Detail-list labels, from MMRCMS.kt's `detail*` hash sets.
const DETAIL_AUTHOR: [&str; 12] = [
	"author(s)",
	"autor(es)",
	"auteur(s)",
	"著作",
	"yazar(lar)",
	"mangaka(lar)",
	"pengarang/penulis",
	"pengarang",
	"penulis",
	"autor",
	"المؤلف",
	"перевод",
];
const DETAIL_ARTIST: [&str; 9] = [
	"artist(s)",
	"artiste(s)",
	"sanatçi(lar)",
	"artista(s)",
	"artist(s)/ilustrator",
	"الرسام",
	"seniman",
	"rysownik/rysownicy",
	"artista",
];
const DETAIL_GENRE: [&str; 11] = [
	"categories",
	"categorías",
	"catégories",
	"ジャンル",
	"kategoriler",
	"categorias",
	"kategorie",
	"التصنيفات",
	"жанр",
	"kategori",
	"género",
];
const DETAIL_STATUS: [&str; 7] = [
	"status",
	"statut",
	"estado",
	"状態",
	"durum",
	"الحالة",
	"статус",
];

fn description_text(document: &Document, node: NodeId) -> String {
	// The base class removes the `<h5>` heading before reading the text.
	let text = document.text_with_newlines(node);
	text.lines()
		.filter(|line| !line.trim().is_empty())
		.skip_while(|line| {
			let lower = line.trim().to_lowercase();
			lower == "summary" || lower == "sinopsis" || lower == "résumé"
		})
		.collect::<Vec<_>>()
		.join("\n")
		.trim()
		.to_string()
}

fn anchor_texts(document: &Document, node: NodeId) -> Vec<String> {
	document
		.descendants(node)
		.into_iter()
		.filter(|child| is_anchor(document, *child))
		.map(|child| document.text(child))
		.filter(|value| !value.is_empty())
		.collect()
}

fn next_element_sibling(document: &Document, node: NodeId) -> Option<NodeId> {
	let parent = document.parent(node)?;
	let siblings = &document.node(parent).children;
	let position = siblings.iter().position(|child| *child == node)?;
	siblings[position + 1..]
		.iter()
		.copied()
		.find(|child| document.is_element(*child))
}

fn is_image(document: &Document, node: NodeId) -> bool {
	document
		.node(node)
		.element()
		.is_some_and(|element| element.name == "img")
}

fn is_anchor(document: &Document, node: NodeId) -> bool {
	document
		.node(node)
		.element()
		.is_some_and(|element| element.name == "a")
}

fn urlencode(value: &str) -> String {
	let mut out = String::with_capacity(value.len());
	for byte in value.bytes() {
		match byte {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
				out.push(byte as char)
			},
			b' ' => out.push('+'),
			_ => out.push_str(&format!("%{byte:02X}")),
		}
	}
	out
}

#[async_trait]
impl Source for MmrcmsSource {
	fn info(&self) -> &SourceInfo {
		&self.context.info
	}

	fn http(&self) -> &SourceHttp {
		&self.context.http
	}

	async fn popular(&self, page: u32) -> SourceResult<SourcePage<RemoteSeries>> {
		if self.exhausted(&self.popular_url, page) {
			return Ok(SourcePage::default());
		}
		let url = self.expand(&self.popular_url, page, "");
		let document = self.context.get(&url).await?;
		Ok(self.parse_cards(
			&document,
			&self.selectors.popular,
			self.selectors.item_url.as_ref(),
			self.selectors.item_title.as_ref(),
			self.selectors.next_page.as_ref(),
		))
	}

	async fn latest(&self, page: u32) -> SourceResult<SourcePage<RemoteSeries>> {
		if !self.context.info.capabilities.latest {
			return Err(theme::unsupported(&self.context.info, "latest"));
		}
		if self.exhausted(&self.latest_url, page) {
			return Ok(SourcePage::default());
		}
		let url = self.expand(&self.latest_url, page, "");
		let document = self.context.get(&url).await?;
		Ok(self.parse_cards(
			&document,
			&self.selectors.latest,
			self.selectors.item_url.as_ref(),
			self.selectors.item_title.as_ref(),
			self.selectors.next_page.as_ref(),
		))
	}

	async fn search(
		&self,
		query: &str,
		_filters: &[SearchFilter],
		page: u32,
	) -> SourceResult<SourcePage<RemoteSeries>> {
		if query.trim().is_empty() {
			return self.popular(page).await;
		}
		let url = self.expand(&self.search_url, page, query);
		let fetched = self.context.http.get_text(&url, &[]).await?;
		// The classic endpoint answers JSON, a redesigned one HTML. Decide on
		// the body rather than on a knob.
		if let Ok(result) = serde_json::from_str::<SearchResult>(&fetched.body) {
			if !result.suggestions.is_empty() {
				return Ok(self.directory_page(&result.suggestions, page));
			}
		}
		// The JSON directory is unpaged and sliced locally above; an HTML
		// endpoint without `{page}` cannot answer page 2.
		if self.exhausted(&self.search_url, page) {
			return Ok(SourcePage::default());
		}
		let document = Document::parse(&fetched.body, &fetched.url);
		Ok(self.parse_cards(
			&document,
			&self.selectors.search,
			self.selectors.search_item_url.as_ref(),
			self.selectors.search_item_title.as_ref(),
			self.selectors.search_next_page.as_ref(),
		))
	}

	async fn details(&self, remote_id: &str) -> SourceResult<RemoteSeries> {
		let document = self.context.get(&self.series_url(remote_id)).await?;
		Ok(self.parse_details(&document, remote_id))
	}

	async fn chapters(&self, remote_id: &str) -> SourceResult<Vec<RemoteChapter>> {
		let document = self.context.get(&self.series_url(remote_id)).await?;
		Ok(self.parse_chapters(&document, remote_id))
	}

	async fn pages(&self, chapter_id: &str) -> SourceResult<Vec<RemotePage>> {
		let document = self.context.get(&self.chapter_url(chapter_id)).await?;
		Ok(self.parse_pages(&document))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::BTreeMap;
	use stump_provider::KnobValue;

	fn definition(knobs: &[(&str, KnobValue)]) -> SourceDefinition {
		SourceDefinition {
			schema: 1,
			id: "en.readcomicsonline".into(),
			name: "Read Comics Online".into(),
			lang: "en".into(),
			base_url: "https://readcomicsonline.test".into(),
			theme: THEME.into(),
			version: 2,
			nsfw: false,
			knobs: knobs
				.iter()
				.map(|(key, value)| (key.to_string(), value.clone()))
				.collect::<BTreeMap<_, _>>(),
			upstream: None,
		}
	}

	fn engine(knobs: &[(&str, KnobValue)]) -> MmrcmsSource {
		MmrcmsSource::new(&definition(knobs), "en.mmrcms", None).expect("engine builds")
	}

	/// The knob set that expresses Read Comics Online's redesigned layout.
	fn read_comics_online() -> MmrcmsSource {
		engine(&[
			("item_path", KnobValue::Text("comic".into())),
			("chapter_string", KnobValue::Text(String::new())),
			("date_format", KnobValue::Text("d MMM yyyy".into())),
			(
				"popular_url",
				KnobValue::Text("/comic-list?sort=views&page={page}".into()),
			),
			(
				"latest_url",
				KnobValue::Text("/comic-list?sort=latest&page={page}".into()),
			),
			(
				"search_url",
				KnobValue::Text("/advanced-search?name={query}&page={page}".into()),
			),
			(
				"popular_manga_selector",
				KnobValue::Text("div.comic-list-layout .grid > .group".into()),
			),
			(
				"manga_url_selector",
				KnobValue::Text("a.block.text-sm.font-semibold".into()),
			),
			(
				"popular_manga_next_page_selector",
				KnobValue::Text("nav a[rel=next]".into()),
			),
			(
				"search_manga_selector",
				KnobValue::Text("div a:has(img)".into()),
			),
			("search_manga_url_selector", KnobValue::Text(":self".into())),
			("search_manga_title_selector", KnobValue::Text("p".into())),
			(
				"search_manga_next_page_selector",
				KnobValue::Text("span a[rel=next]".into()),
			),
			(
				"details_title_selector",
				KnobValue::Text("h1.text-2xl".into()),
			),
			(
				"details_thumbnail_selector",
				KnobValue::Text("img.w-full.rounded-xl".into()),
			),
			(
				"details_description_selector",
				KnobValue::Text("p.mt-5.text-sm".into()),
			),
			(
				"details_status_selector",
				KnobValue::Text("div.flex.flex-wrap.gap-2 span.rounded-full".into()),
			),
			(
				"details_genre_selector",
				KnobValue::Text("dl div:contains(Genres:) a".into()),
			),
			(
				"details_author_selector",
				KnobValue::Text("div:has(span:contains(Author:)) > a".into()),
			),
			(
				"chapter_list_selector",
				KnobValue::Text(".overflow-hidden.border-ink-600 > a".into()),
			),
			("chapter_url_selector", KnobValue::Text(":self".into())),
			(
				"chapter_name_selector",
				KnobValue::Text(".text-brand-400".into()),
			),
			(
				"chapter_date_selector",
				KnobValue::Text(".text-slate-500".into()),
			),
			(
				"page_list_selector",
				KnobValue::Text("#reader-all img".into()),
			),
		])
	}

	#[test]
	fn base_class_urls_and_item_path() {
		let source = engine(&[]);
		assert_eq!(
			source.expand(&source.popular_url, 3, ""),
			"https://readcomicsonline.test/filterList?page=3&sortBy=views&asc=false"
		);
		assert_eq!(
			source.expand(&source.latest_url, 1, ""),
			"https://readcomicsonline.test/latest-release?page=1"
		);
		assert_eq!(
			source.expand(&source.search_url, 1, "iron man"),
			"https://readcomicsonline.test/search?query=iron+man"
		);
		assert_eq!(
			source.series_url("foo"),
			"https://readcomicsonline.test/manga/foo"
		);
	}

	#[test]
	fn url_knobs_expand_page_and_query_placeholders() {
		let source = read_comics_online();
		assert_eq!(
			source.expand(&source.popular_url, 2, ""),
			"https://readcomicsonline.test/comic-list?sort=views&page=2"
		);
		assert_eq!(
			source.expand(&source.search_url, 1, "iron man"),
			"https://readcomicsonline.test/advanced-search?name=iron+man&page=1"
		);
		assert_eq!(
			source.series_url("iron-man"),
			"https://readcomicsonline.test/comic/iron-man"
		);
	}

	/// `sources-import` names a request override after its Kotlin member
	/// (`popularMangaRequest` -> `popular_manga_url`) and emits an absolute
	/// URL. Reading only the short alias silently ignored every generated
	/// browse override, which is how `bg.utsukushii` browsed the wrong path.
	#[test]
	fn generated_request_knob_names_drive_browsing() {
		let source = engine(&[(
			"popular_manga_url",
			KnobValue::Text("https://utsukushii-bg.com/manga-list".into()),
		)]);
		assert_eq!(
			source.expand(&source.popular_url, 1, ""),
			"https://utsukushii-bg.com/manga-list"
		);
		// No `{page}`: one fixed page, so page 2 is the end of the list
		// rather than a second copy of page 1.
		assert!(source.exhausted(&source.popular_url, 2));
		assert!(!source.exhausted(&source.popular_url, 1));
		let paged = engine(&[(
			"latest_updates_url",
			KnobValue::Text("/latest?p={page}".into()),
		)]);
		assert_eq!(
			paged.expand(&paged.latest_url, 4, ""),
			"https://readcomicsonline.test/latest?p=4"
		);
		assert!(!paged.exhausted(&paged.latest_url, 4));
	}

	#[test]
	fn base_class_cards_read_the_media_heading() {
		let source = engine(&[]);
		let html = r#"<html><body>
			<div class="media">
				<img class="img-responsive" data-src="/uploads/manga/alpha/cover/cover_250x350.jpg">
				<h5 class="media-heading"><a href="/manga/alpha">Alpha</a></h5>
			</div>
			<div class="media">
				<img class="img-responsive" src="/no-image.png">
				<h5 class="media-heading"><a href="/manga/beta">Beta</a></h5>
			</div>
			<ul class="pagination"><li><a rel="next" href="?page=2">2</a></li></ul>
		</body></html>"#;
		let page = source.parse_cards(
			&Document::parse(html, "https://readcomicsonline.test/filterList"),
			&source.selectors.popular,
			source.selectors.item_url.as_ref(),
			source.selectors.item_title.as_ref(),
			source.selectors.next_page.as_ref(),
		);
		assert_eq!(page.items.len(), 2);
		assert_eq!(page.items[0].remote_id, "alpha");
		assert_eq!(page.items[0].title, "Alpha");
		// A `no-image.png` placeholder is replaced by the guessed cover path.
		assert_eq!(
			page.items[1].thumbnail_url.as_deref(),
			Some("https://readcomicsonline.test/uploads/manga/beta/cover/cover_250x350.jpg")
		);
		assert!(page.has_next);
	}

	#[test]
	fn redesigned_cards_are_expressible_with_knobs_alone() {
		let source = read_comics_online();
		let html = r#"<html><body><div class="comic-list-layout"><div class="grid">
			<div class="group">
				<img src="/covers/ironman.jpg">
				<a class="block text-sm font-semibold" href="/comic/iron-man">Iron Man</a>
			</div>
			<div class="group">
				<img src="/covers/thor.jpg">
				<a class="block text-sm font-semibold" href="/comic/thor">Thor</a>
			</div>
		</div></div>
		<nav><a rel="next" href="?page=2">next</a></nav></body></html>"#;
		let page = source.parse_cards(
			&Document::parse(html, "https://readcomicsonline.test/comic-list"),
			&source.selectors.popular,
			source.selectors.item_url.as_ref(),
			source.selectors.item_title.as_ref(),
			source.selectors.next_page.as_ref(),
		);
		assert_eq!(page.items.len(), 2);
		assert_eq!(page.items[0].remote_id, "iron-man");
		assert_eq!(page.items[0].title, "Iron Man");
		assert_eq!(
			page.items[0].thumbnail_url.as_deref(),
			Some("https://readcomicsonline.test/covers/ironman.jpg")
		);
		assert!(page.has_next);
	}

	#[test]
	fn self_selector_lets_the_card_be_its_own_anchor() {
		let source = read_comics_online();
		let html = r#"<html><body><div>
			<a href="/comic/hulk"><img src="/covers/hulk.jpg"><p>Hulk</p></a>
			<a href="/comic/vision"><img src="/covers/vision.jpg"><p>Vision</p></a>
		</div></body></html>"#;
		let page = source.parse_cards(
			&Document::parse(html, "https://readcomicsonline.test/advanced-search"),
			&source.selectors.search,
			source.selectors.search_item_url.as_ref(),
			source.selectors.search_item_title.as_ref(),
			source.selectors.search_next_page.as_ref(),
		);
		assert_eq!(page.items.len(), 2);
		assert_eq!(page.items[0].remote_id, "hulk");
		assert_eq!(page.items[0].title, "Hulk");
		assert_eq!(page.items[1].title, "Vision");
	}

	/// Browsing and searching paginate with different controls on a
	/// redesigned host, so one shared `next_page` selector is wrong: the
	/// search page must not be declared exhausted because the directory's
	/// `nav` control is absent, and vice versa.
	#[test]
	fn search_pagination_is_independent_of_directory_pagination() {
		let source = read_comics_online();
		let search_html = r#"<html><body>
			<div><a href="/comic/hulk"><img src="/c/h.jpg"><p>Hulk</p></a></div>
			<span><a rel="next" href="?name=h&page=2">next</a></span>
		</body></html>"#;
		let searched = source.parse_cards(
			&Document::parse(
				search_html,
				"https://readcomicsonline.test/advanced-search",
			),
			&source.selectors.search,
			source.selectors.search_item_url.as_ref(),
			source.selectors.search_item_title.as_ref(),
			source.selectors.search_next_page.as_ref(),
		);
		assert!(searched.has_next, "`span a[rel=next]` paginates search");
		// The same body browsed: only `nav a[rel=next]` counts there.
		let browsed = source.parse_cards(
			&Document::parse(search_html, "https://readcomicsonline.test/comic-list"),
			&source.selectors.search,
			source.selectors.search_item_url.as_ref(),
			source.selectors.search_item_title.as_ref(),
			source.selectors.next_page.as_ref(),
		);
		assert!(!browsed.has_next, "`nav a[rel=next]` is absent");
	}

	/// `override val chapterString = ""` strips the repeated series title
	/// outright. Treating the empty knob as "unset" would substitute the
	/// `en` default `Chapter` and rename every chapter.
	#[test]
	fn empty_chapter_string_knob_strips_rather_than_substitutes() {
		let source = read_comics_online();
		assert_eq!(source.chapter_string, "");
		assert_eq!(
			source.clean_chapter_name("1776 (2025-)", "1776 (2025-) #4"),
			"#4"
		);
		// Unset, the base-class default for `en` applies instead.
		assert_eq!(
			engine(&[]).clean_chapter_name("1776 (2025-)", "1776 (2025-) #4"),
			"Chapter #4"
		);
	}

	#[test]
	fn details_walk_the_definition_list() {
		let source = engine(&[]);
		let html = r#"<html><body><div class="row">
			<img class="img-responsive" src="/uploads/manga/alpha/cover/cover_250x350.jpg">
			<div class="well"><h5>Summary</h5><p>First line.</p><p>Second line.</p></div>
			<dl class="dl-horizontal">
				<dt>Author(s)</dt><dd>Stan L.</dd>
				<dt>Artist(s)</dt><dd>Jack K.</dd>
				<dt>Categories</dt><dd><a href="/category/action">Action</a><a href="/category/sci-fi">Sci-fi</a></dd>
				<dt>Status</dt><dd>Complete</dd>
			</dl>
		</div>
		<h1 class="listmanga-header">Alpha Comic</h1></body></html>"#;
		let series = source.parse_details(
			&Document::parse(html, "https://readcomicsonline.test/manga/alpha"),
			"alpha",
		);
		assert_eq!(series.title, "Alpha Comic");
		assert_eq!(series.authors, ["Stan L."]);
		assert_eq!(series.artists, ["Jack K."]);
		assert_eq!(series.genres, ["Action", "Sci-fi"]);
		assert_eq!(series.status, stump_provider::SeriesStatus::Completed);
		assert_eq!(
			series.description.as_deref(),
			Some("First line.\nSecond line.")
		);
	}

	#[test]
	fn redesigned_details_use_the_selector_knobs() {
		let source = read_comics_online();
		let html = r#"<html><body>
			<h1 class="text-2xl">Iron Man</h1>
			<img class="w-full rounded-xl" src="/covers/ironman.jpg">
			<p class="mt-5 text-sm">Tony builds a suit.</p>
			<div class="flex flex-wrap gap-2"><span class="rounded-full">Ongoing</span></div>
			<dl><div>Genres: <a href="/genre/action">Action</a><a href="/genre/superhero">Superhero</a></div></dl>
			<div><span>Author:</span> <a href="/author/stan-l">Stan L.</a></div>
		</body></html>"#;
		let series = source.parse_details(
			&Document::parse(html, "https://readcomicsonline.test/comic/iron-man"),
			"iron-man",
		);
		assert_eq!(series.title, "Iron Man");
		assert_eq!(series.status, stump_provider::SeriesStatus::Ongoing);
		assert_eq!(series.genres, ["Action", "Superhero"]);
		assert_eq!(series.authors, ["Stan L."]);
		assert_eq!(series.description.as_deref(), Some("Tony builds a suit."));
		assert_eq!(
			series.thumbnail_url.as_deref(),
			Some("https://readcomicsonline.test/covers/ironman.jpg")
		);
	}

	#[test]
	fn chapters_key_on_the_path_below_item_path() {
		let source = engine(&[]);
		let html = r#"<html><body>
			<h1 class="listmanga-header">One Piece</h1>
			<ul class="chapters">
				<li><div class="chapter-title-rtl"><a href="/manga/one-piece/1105">One Piece 1105 : Chapter 1105</a></div>
					<div class="date-chapter-title-rtl">3 Mar. 2024</div></li>
				<li><div class="chapter-title-rtl"><a href="/manga/one-piece/2/3">Vol 2 Chapter 3</a></div></li>
				<li class="btn"><a href="/manga/one-piece">skip</a></li>
			</ul></body></html>"#;
		let chapters = source.parse_chapters(
			&Document::parse(html, "https://readcomicsonline.test/manga/one-piece"),
			"one-piece",
		);
		assert_eq!(chapters.len(), 2);
		assert_eq!(chapters[0].remote_id, "one-piece/1105");
		// `cleanChapterName` collapses the repeated title.
		assert_eq!(chapters[0].title.as_deref(), Some("Chapter 1105"));
		assert_eq!(chapters[0].number, Some(1105.0));
		assert_eq!(
			chapters[0].uploaded_at.unwrap().date_naive(),
			chrono::NaiveDate::from_ymd_opt(2024, 3, 3).unwrap()
		);
		assert_eq!(chapters[1].remote_id, "one-piece/2/3");
	}

	#[test]
	fn redesigned_chapters_use_self_anchors() {
		let source = read_comics_online();
		let html = r#"<html><body>
			<h1 class="text-2xl">Iron Man</h1>
			<div class="overflow-hidden border-ink-600">
				<a href="/comic/iron-man/12"><span class="text-brand-400">Issue #12</span>
					<span class="text-slate-500">4 Apr 2024</span></a>
				<a href="/comic/iron-man/11"><span class="text-brand-400">Issue #11</span>
					<span class="text-slate-500">1 Apr 2024</span></a>
			</div></body></html>"#;
		let chapters = source.parse_chapters(
			&Document::parse(html, "https://readcomicsonline.test/comic/iron-man"),
			"iron-man",
		);
		assert_eq!(chapters.len(), 2);
		assert_eq!(chapters[0].remote_id, "iron-man/12");
		assert_eq!(chapters[0].title.as_deref(), Some("Issue #12"));
		assert_eq!(chapters[0].number, Some(12.0));
		assert_eq!(
			chapters[0].uploaded_at.unwrap().date_naive(),
			chrono::NaiveDate::from_ymd_opt(2024, 4, 4).unwrap()
		);
		assert_eq!(chapters[1].remote_id, "iron-man/11");
	}

	#[test]
	fn pages_read_the_configured_reader_container() {
		let base = engine(&[]);
		let html = r#"<html><body><div id="all">
			<img class="img-responsive" data-src="/pages/1.jpg">
			<img class="img-responsive" src="/pages/2.jpg">
		</div></body></html>"#;
		let url = "https://readcomicsonline.test/manga/alpha/1";
		let pages = base.parse_pages(&Document::parse(html, url));
		assert_eq!(pages.len(), 2);
		assert_eq!(pages[0].url, "https://readcomicsonline.test/pages/1.jpg");
		assert!(pages[0]
			.headers
			.iter()
			.any(|(name, value)| name == "Referer" && value == url));

		let redesigned = read_comics_online();
		let html = r#"<html><body><div id="reader-all">
			<img data-lazy-src="/pages/a.jpg"><img src="/pages/b.jpg">
		</div><div id="all"><img src="/pages/ignored.jpg"></div></body></html>"#;
		let pages = redesigned.parse_pages(&Document::parse(html, url));
		assert_eq!(pages.len(), 2);
		assert_eq!(pages[0].url, "https://readcomicsonline.test/pages/a.jpg");
		assert_eq!(pages[1].url, "https://readcomicsonline.test/pages/b.jpg");
	}

	#[test]
	fn json_search_directory_pages_locally() {
		let source = engine(&[]);
		let suggestions: Vec<Suggestion> = (0..30)
			.map(|index| Suggestion {
				value: format!("Title {index}"),
				data: format!("slug-{index}"),
			})
			.collect();
		let first = source.directory_page(&suggestions, 1);
		assert_eq!(first.items.len(), 24);
		assert_eq!(first.items[0].remote_id, "slug-0");
		assert_eq!(
			first.items[0].url.as_deref(),
			Some("https://readcomicsonline.test/manga/slug-0")
		);
		assert!(first.has_next);
		let second = source.directory_page(&suggestions, 2);
		assert_eq!(second.items.len(), 6);
		assert!(!second.has_next);
		assert!(source.directory_page(&suggestions, 3).items.is_empty());
	}

	#[test]
	fn chapter_names_collapse_repeated_titles() {
		let source = engine(&[]);
		assert_eq!(
			source.clean_chapter_name("One Piece", "One Piece 1105 : Chapter 1105"),
			"Chapter 1105"
		);
		assert_eq!(
			source.clean_chapter_name("One Piece", "One Piece 1105"),
			"Chapter 1105"
		);
		assert_eq!(source.clean_chapter_name("", "Issue #4"), "Issue #4");
		// Spanish hosts get the localised word.
		let mut spanish = definition(&[]);
		spanish.lang = "es".into();
		let spanish = MmrcmsSource::new(&spanish, "es.x", None).unwrap();
		assert_eq!(
			spanish.clean_chapter_name("Naruto", "Naruto 700"),
			"Capítulo 700"
		);
	}

	#[test]
	fn chapter_ids_survive_absolute_and_nested_urls() {
		let source = read_comics_online();
		assert_eq!(
			source
				.chapter_id(
					"https://readcomicsonline.test/comic/iron-man/12",
					"iron-man"
				)
				.as_deref(),
			Some("iron-man/12")
		);
		assert_eq!(
			source
				.chapter_id("/comic/iron-man/2/3", "iron-man")
				.as_deref(),
			Some("iron-man/2/3")
		);
		assert_eq!(source.chapter_id("/comic/", "iron-man"), None);
	}
}
