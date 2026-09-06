//! The MangaThemesia WordPress theme (`lib-multisrc/mangathemesia`, formerly
//! WPMangaStream/WPMangaReader), 140 extensions at `064c1a0e`.
//!
//! # Knobs and where they come from
//!
//! | knob | Kotlin member | default |
//! |---|---|---|
//! | `manga_url_directory` | `MangaThemesia.mangaUrlDirectory` | `/manga` |
//! | `project_page_string` | `MangaThemesia.projectPageString` | `/project` |
//! | `has_project_page` | `MangaThemesia.hasProjectPage` | `false` |
//! | `date_format` | `MangaThemesia.dateFormat` | `MMMM dd, yyyy` |
//! | `date_locale` | its `Locale` argument | `en-US` |
//! | `search_manga_selector` | `MangaThemesia.searchMangaSelector()` | `.utao .uta .imgu, .listupd .bs .bsx, .listo .bs .bsx` |
//! | `search_manga_next_page_selector` | `MangaThemesia.searchMangaNextPageSelector()` | `div.pagination .next, div.hpage .r` |
//! | `series_details_selector` | `MangaThemesia.seriesDetailsSelector` | `div.bigcontent, div.animefull, div.main-info, div.postbody` |
//! | `series_title_selector` | `seriesTitleSelector` | `h1.entry-title, .ts-breadcrumb li:last-child span` |
//! | `series_author_selector` / `series_artist_selector` | `seriesAuthorSelector` / `seriesArtistSelector` | multilingual label tables |
//! | `series_description_selector` | `seriesDescriptionSelector` | `.desc, .entry-content[itemprop=description]` |
//! | `series_alt_name_selector` | `seriesAltNameSelector` | see below |
//! | `series_genre_selector` | `seriesGenreSelector` | `div.gnr a, .mgen a, .seriestugenre a, ...` |
//! | `series_type_selector` | `seriesTypeSelector` | multilingual label table |
//! | `series_status_selector` | `seriesStatusSelector` | multilingual label table |
//! | `series_thumbnail_selector` | `seriesThumbnailSelector` | `.infomanga > div[itemprop=image] img, .thumb img` |
//! | `chapter_list_selector` | `chapterListSelector()` | `div.bxcl li, div.cl li, #chapterlist li, ul li:has(div.chbox):has(div.eph-num)` |
//! | `page_selector` | `MangaThemesia.pageSelector` | `div#readerarea img` |
//! | `alt_name_prefix` | `altNamePrefix` | `Alternative Names: ` |
//!
//! # Pages
//!
//! `pageListParse` reads `page_selector` first and, when that yields nothing,
//! falls back to the reader's inline script: `ts_reader.run({...})` embeds a
//! JSON payload whose `"images"` array holds the page URLs.
//! `MangaThemesia.JSON_IMAGE_LIST_REGEX` is `"images"\s*:\s*(\[.*?])`, which is
//! what this engine applies — to the whole document, exactly as upstream does,
//! so a reader that splits the call across script tags still resolves.
//!
//! # Remote ids
//!
//! Series ids are the slug under `manga_url_directory`; chapter ids are the
//! chapter's own slug, which MangaThemesia serves at the site root
//! (`/{chapter-slug}/`) rather than nested under the series.

use std::sync::Arc;

use async_trait::async_trait;
use models::entity::provider_source;
use regex::Regex;
use serde::Deserialize;
use stump_provider::{
	definition::SourceDefinition, RemoteChapter, RemotePage, RemoteSeries, SearchFilter,
	Source, SourceCapabilities, SourceHttp, SourceInfo, SourcePage, SourceResult,
};

use crate::{
	date::{self, DateParser},
	dom::{Document, NodeId},
	selector::Selector,
	theme::{self, ImageAttrs, ThemeContext, ThemeError},
};

pub const THEME: &str = "mangathemesia";

/// `MangaThemesia.Element.imgAttr()` attribute order.
const IMAGE_ATTRS: ImageAttrs =
	ImageAttrs(&["data-lazy-src", "data-src", "data-cfsrc", "srcset", "src"]);

/// Multilingual label tables the base class builds with its `selector()`
/// helper; reproduced so a definition that overrides nothing still works on a
/// Spanish, Turkish or Arabic host.
const AUTHOR_DEFAULT: &str = ".infotable tr:contains(Author) td:last-child, .tsinfo .imptdt:contains(Author) i, .fmed b:contains(Author)+span, span:contains(Author), .infotable tr:contains(autor) td:last-child, .tsinfo .imptdt:contains(autor) i, .fmed b:contains(autor)+span, span:contains(autor), .infotable tr:contains(Auteur) td:last-child, .tsinfo .imptdt:contains(Auteur) i, .fmed b:contains(Auteur)+span, span:contains(Auteur), .infotable tr:contains(المؤلف) td:last-child, .tsinfo .imptdt:contains(المؤلف) i, .fmed b:contains(المؤلف)+span, span:contains(المؤلف), .infotable tr:contains(Mangaka) td:last-child, .tsinfo .imptdt:contains(Mangaka) i, .fmed b:contains(Mangaka)+span, span:contains(Mangaka), .infotable tr:contains(seniman) td:last-child, .tsinfo .imptdt:contains(seniman) i, .fmed b:contains(seniman)+span, span:contains(seniman), .infotable tr:contains(Pengarang) td:last-child, .tsinfo .imptdt:contains(Pengarang) i, .fmed b:contains(Pengarang)+span, span:contains(Pengarang), .infotable tr:contains(Yazar) td:last-child, .tsinfo .imptdt:contains(Yazar) i, .fmed b:contains(Yazar)+span, span:contains(Yazar)";
const ARTIST_DEFAULT: &str = ".infotable tr:contains(artist) td:last-child, .tsinfo .imptdt:contains(artist) i, .fmed b:contains(artist)+span, span:contains(artist), .infotable tr:contains(Artiste) td:last-child, .tsinfo .imptdt:contains(Artiste) i, .fmed b:contains(Artiste)+span, span:contains(Artiste), .infotable tr:contains(Artista) td:last-child, .tsinfo .imptdt:contains(Artista) i, .fmed b:contains(Artista)+span, span:contains(Artista), .infotable tr:contains(الرسام) td:last-child, .tsinfo .imptdt:contains(الرسام) i, .fmed b:contains(الرسام)+span, span:contains(الرسام), .infotable tr:contains(Çizer) td:last-child, .tsinfo .imptdt:contains(Çizer) i, .fmed b:contains(Çizer)+span, span:contains(Çizer)";
const STATUS_DEFAULT: &str = ".infotable tr:contains(status) td:last-child, .tsinfo .imptdt:contains(status) i, .fmed b:contains(status)+span span:contains(status), .infotable tr:contains(Statut) td:last-child, .tsinfo .imptdt:contains(Statut) i, .infotable tr:contains(Durum) td:last-child, .tsinfo .imptdt:contains(Durum) i, .infotable tr:contains(Estado) td:last-child, .tsinfo .imptdt:contains(Estado) i, .infotable tr:contains(الحالة) td:last-child, .tsinfo .imptdt:contains(الحالة) i, .infotable tr:contains(สถานะ) td:last-child, .tsinfo .imptdt:contains(สถานะ) i, .infotable tr:contains(stato) td:last-child, .tsinfo .imptdt:contains(stato) i";
const TYPE_DEFAULT: &str = ".infotable tr:contains(type) td:last-child, .tsinfo .imptdt:contains(type) i, .tsinfo .imptdt:contains(type) a, .fmed b:contains(type)+span, span:contains(type) a, .infotable tr:contains(tipe) td:last-child, .tsinfo .imptdt:contains(tipe) i, .tsinfo .imptdt:contains(tipe) a, .infotable tr:contains(النوع) td:last-child, .tsinfo .imptdt:contains(النوع) i, .tsinfo .imptdt:contains(النوع) a, .infotable tr:contains(Türü) td:last-child, .tsinfo .imptdt:contains(Türü) i, a[href*=type\\=]";
const ALT_NAME_DEFAULT: &str = ".alternative, .wd-full:contains(alt) span, .alter, .seriestualt, .infotable tr:contains(Alternative) td:last-child, .infotable tr:contains(Alternatif) td:last-child, .infotable tr:contains(الأسماء الثانوية) td:last-child";
const GENRE_DEFAULT: &str =
	"div.gnr a, .mgen a, .seriestugenre a, span:contains(genre), span:contains(التصنيف)";

/// Placeholders the base class' `removeEmptyPlaceholder` drops.
const PLACEHOLDERS: [&str; 4] = ["-", "n/a", "unknown", ""];

#[derive(Debug)]
struct Selectors {
	list: Selector,
	next_page: Option<Selector>,
	item_url: Option<Selector>,
	item_title: Option<Selector>,
	details: Selector,
	title: Selector,
	author: Selector,
	artist: Selector,
	description: Selector,
	alt_name: Selector,
	genre: Selector,
	series_type: Selector,
	status: Selector,
	thumbnail: Selector,
	chapter_list: Selector,
	chapter_name: Selector,
	chapter_date: Selector,
	updated_on: Selector,
	pages: Selector,
}

/// A MangaThemesia-themed site driven by a definition.
#[derive(Debug)]
pub struct MangaThemesiaSource {
	context: ThemeContext,
	selectors: Selectors,
	dates: DateParser,
	manga_url_directory: String,
	project_page_string: String,
	has_project_page: bool,
	alt_name_prefix: String,
	nsfw: bool,
	images: Regex,
}

impl MangaThemesiaSource {
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
		let selectors = Selectors {
			list: theme::selector(
				definition,
				"search_manga_selector",
				&["popular_manga_selector", "list_selector"],
				".utao .uta .imgu, .listupd .bs .bsx, .listo .bs .bsx",
			)?,
			next_page: theme::optional_selector(
				definition,
				"search_manga_next_page_selector",
				&["popular_manga_next_page_selector"],
				Some("div.pagination .next, div.hpage .r"),
			)?,
			item_url: theme::optional_selector(
				definition,
				"manga_url_selector",
				&[],
				Some("a"),
			)?,
			item_title: theme::optional_selector(
				definition,
				"manga_title_selector",
				&[],
				None,
			)?,
			details: theme::selector(
				definition,
				"series_details_selector",
				&[],
				"div.bigcontent, div.animefull, div.main-info, div.postbody",
			)?,
			title: theme::selector(
				definition,
				"series_title_selector",
				&["details_title_selector"],
				"h1.entry-title, .ts-breadcrumb li:last-child span",
			)?,
			author: theme::selector(
				definition,
				"series_author_selector",
				&["details_author_selector"],
				AUTHOR_DEFAULT,
			)?,
			artist: theme::selector(
				definition,
				"series_artist_selector",
				&["details_artist_selector"],
				ARTIST_DEFAULT,
			)?,
			description: theme::selector(
				definition,
				"series_description_selector",
				&["details_description_selector"],
				".desc, .entry-content[itemprop=description]",
			)?,
			alt_name: theme::selector(
				definition,
				"series_alt_name_selector",
				&[],
				ALT_NAME_DEFAULT,
			)?,
			genre: theme::selector(
				definition,
				"series_genre_selector",
				&["details_genre_selector"],
				GENRE_DEFAULT,
			)?,
			series_type: theme::selector(
				definition,
				"series_type_selector",
				&[],
				TYPE_DEFAULT,
			)?,
			status: theme::selector(
				definition,
				"series_status_selector",
				&["details_status_selector"],
				STATUS_DEFAULT,
			)?,
			thumbnail: theme::selector(
				definition,
				"series_thumbnail_selector",
				&["details_thumbnail_selector"],
				".infomanga > div[itemprop=image] img, .thumb img",
			)?,
			chapter_list: theme::selector(
				definition,
				"chapter_list_selector",
				&[],
				"div.bxcl li, div.cl li, #chapterlist li, ul li:has(div.chbox):has(div.eph-num)",
			)?,
			chapter_name: theme::selector(
				definition,
				"chapter_name_selector",
				&[],
				".lch a, .chapternum",
			)?,
			chapter_date: theme::selector(
				definition,
				"chapter_date_selector",
				&[],
				".chapterdate",
			)?,
			updated_on: theme::selector(
				definition,
				"updated_on_selector",
				&[],
				".listinfo time[itemprop=dateModified], .fmed:contains(update) time, span:contains(update) time",
			)?,
			pages: theme::selector(
				definition,
				"page_selector",
				&["page_list_selector", "page_list_parse_selector"],
				"div#readerarea img",
			)?,
		};
		Ok(Self {
			context,
			selectors,
			dates: DateParser::new(
				definition
					.text_knob(&["date_format", "chapter_date_format"])
					.unwrap_or("MMMM dd, yyyy"),
				definition
					.text_knob(&["date_locale", "chapter_date_locale"])
					.unwrap_or("en-US"),
			),
			manga_url_directory: normalise_directory(
				definition.text_knob(&["manga_url_directory"]).unwrap_or("/manga"),
			),
			project_page_string: normalise_directory(
				definition
					.text_knob(&["project_page_string"])
					.unwrap_or("/project"),
			),
			has_project_page: definition
				.bool_knob(&["has_project_page"])
				.unwrap_or(false),
			alt_name_prefix: definition
				.text_knob(&["alt_name_prefix"])
				.unwrap_or("Alternative Names:")
				.to_string(),
			nsfw: definition.nsfw,
			// `MangaThemesia.JSON_IMAGE_LIST_REGEX`. The pattern is a constant,
			// so compilation cannot fail.
			// `(?s)` so a pretty-printed reader payload, whose array spans
			// lines, still matches; upstream's single-line payloads are
			// unaffected.
			images: Regex::new(r#"(?s)"images"\s*:\s*(\[.*?\])"#)
				.expect("the image list pattern is valid"),
		})
	}

	/// `MangaThemesia.searchMangaRequest`: one endpoint serves popular, latest
	/// and search, distinguished only by `order`.
	fn list_url(&self, page: u32, query: &str, order: &str, project: bool) -> String {
		let directory = if project {
			&self.project_page_string
		} else {
			&self.manga_url_directory
		};
		let mut url = format!("{}{directory}/?", self.context.base_url);
		url.push_str(&format!("title={}", urlencode(query)));
		url.push_str(&format!("&page={page}"));
		if !order.is_empty() {
			url.push_str(&format!("&order={order}"));
		}
		url
	}

	fn series_url(&self, slug: &str) -> String {
		format!(
			"{}{}/{slug}/",
			self.context.base_url, self.manga_url_directory
		)
	}

	/// Chapters live at the site root in this theme.
	fn chapter_url(&self, slug: &str) -> String {
		format!("{}/{slug}/", self.context.base_url)
	}

	async fn list(
		&self,
		page: u32,
		query: &str,
		order: &str,
	) -> SourceResult<SourcePage<RemoteSeries>> {
		let url = self.list_url(page, query, order, false);
		let document = self.context.get(&url).await?;
		Ok(self.parse_list(&document))
	}

	/// `MangaThemesia.searchMangaParse` + `searchMangaFromElement`.
	pub fn parse_list(&self, document: &Document) -> SourcePage<RemoteSeries> {
		let root = document.root();
		let mut items = Vec::new();
		for item in self.selectors.list.select(document, root) {
			let anchor = match &self.selectors.item_url {
				Some(selector) => selector.select_first(document, item),
				None => Some(item),
			};
			let Some(anchor) = anchor else { continue };
			let Some(href) = document.abs_attr(anchor, "href") else {
				continue;
			};
			let Some(slug) = theme::slug_of(&href) else {
				continue;
			};
			let title = match &self.selectors.item_title {
				Some(selector) => selector
					.select_first(document, item)
					.map(|node| document.text(node)),
				None => document.attr(anchor, "title").map(str::to_string),
			}
			.filter(|title| !title.trim().is_empty())
			.unwrap_or_else(|| {
				let text = document.text(anchor);
				if text.is_empty() {
					slug.clone()
				} else {
					text
				}
			});
			let thumbnail = document
				.descendants(item)
				.into_iter()
				.find(|node| is_image(document, *node))
				.and_then(|image| IMAGE_ATTRS.resolve(document, image));
			items.push(theme::series(
				slug,
				title,
				Some(href),
				thumbnail,
				self.nsfw,
			));
		}
		let has_next = self
			.selectors
			.next_page
			.as_ref()
			.is_some_and(|selector| selector.select_first(document, root).is_some());
		SourcePage { items, has_next }
	}

	/// `MangaThemesia.mangaDetailsParse`.
	pub fn parse_details(&self, document: &Document, slug: &str) -> RemoteSeries {
		// The base class scopes every field to the details container, falling
		// back to the whole document when the container is absent.
		let scope = self
			.selectors
			.details
			.select_first(document, document.root())
			.unwrap_or_else(|| document.root());
		let title = self
			.selectors
			.title
			.select_first(document, scope)
			.map(|node| document.text(node))
			.filter(|title| !title.is_empty())
			.unwrap_or_else(|| slug.to_string());
		let mut description = self
			.selectors
			.description
			.select(document, scope)
			.into_iter()
			.map(|node| document.text(node))
			.collect::<Vec<_>>()
			.join("\n")
			.trim()
			.to_string();
		if let Some(alt) = self
			.selectors
			.alt_name
			.select_first(document, scope)
			.map(|node| document.own_text(node))
			.filter(|value| !value.trim().is_empty())
		{
			if !description.is_empty() {
				description.push_str("\n\n");
			}
			description.push_str(self.alt_name_prefix.trim());
			description.push(' ');
			description.push_str(&alt);
		}
		let mut genres: Vec<String> = self
			.selectors
			.genre
			.select(document, scope)
			.into_iter()
			.map(|node| titlecase(&document.text(node)))
			.filter(|value| !value.is_empty())
			.collect();
		if let Some(series_type) = self
			.selectors
			.series_type
			.select_first(document, scope)
			.map(|node| document.own_text(node))
			.filter(|value| !value.trim().is_empty())
		{
			genres.push(titlecase(&series_type));
		}
		RemoteSeries {
			remote_id: slug.to_string(),
			title,
			url: Some(document.url().to_string()),
			thumbnail_url: self
				.selectors
				.thumbnail
				.select_first(document, scope)
				.and_then(|node| IMAGE_ATTRS.resolve(document, node)),
			description: (!description.is_empty()).then_some(description),
			authors: field(document, &self.selectors.author, scope)
				.into_iter()
				.collect(),
			artists: field(document, &self.selectors.artist, scope)
				.into_iter()
				.collect(),
			genres,
			status: self
				.selectors
				.status
				.select_first(document, scope)
				.map(|node| theme::parse_status(&document.text(node)))
				.unwrap_or_default(),
			nsfw: self.nsfw,
			..Default::default()
		}
	}

	/// `MangaThemesia.chapterListParse`: the newest chapter inherits the
	/// series' "Updated On" timestamp when the list carries no dates at all,
	/// so a source without per-chapter dates still sorts.
	pub fn parse_chapters(&self, document: &Document) -> Vec<RemoteChapter> {
		let root = document.root();
		let mut chapters = Vec::new();
		for item in self.selectors.chapter_list.select(document, root) {
			let anchor = if is_anchor(document, item) {
				Some(item)
			} else {
				document
					.descendants(item)
					.into_iter()
					.find(|node| is_anchor(document, *node))
			};
			let Some(anchor) = anchor else { continue };
			let Some(href) = document.abs_attr(anchor, "href") else {
				continue;
			};
			let Some(slug) = theme::slug_of(&href) else {
				continue;
			};
			let name = self
				.selectors
				.chapter_name
				.select_first(document, item)
				.map(|node| document.text(node))
				.filter(|name| !name.is_empty())
				.unwrap_or_else(|| document.text(anchor));
			let uploaded_at = self
				.selectors
				.chapter_date
				.select_first(document, item)
				.map(|node| document.text(node))
				.and_then(|value| self.dates.parse(&value));
			chapters.push(theme::chapter(
				slug,
				(!name.is_empty()).then(|| name.clone()),
				theme::chapter_number(&name),
				uploaded_at,
				Some(href),
				&self.context.info.lang,
			));
		}
		if let Some(first) = chapters.first_mut() {
			if first.uploaded_at.is_none() {
				if let Some(updated) = self
					.selectors
					.updated_on
					.select_first(document, root)
					.and_then(|node| document.attr(node, "datetime"))
					.and_then(date::parse_iso)
				{
					first.uploaded_at = Some(updated);
				}
			}
		}
		chapters
	}

	/// `MangaThemesia.pageListParse`: markup first, then the `ts_reader.run`
	/// payload's `"images"` array.
	pub fn parse_pages(&self, document: &Document) -> Vec<RemotePage> {
		let root = document.root();
		let from_markup: Vec<String> = self
			.selectors
			.pages
			.select(document, root)
			.into_iter()
			.filter_map(|node| IMAGE_ATTRS.resolve(document, node))
			.collect();
		if !from_markup.is_empty() {
			return theme::pages_with_referer(from_markup, document.url());
		}
		let from_script = self
			.script_images(document)
			.unwrap_or_default();
		theme::pages_with_referer(from_script, document.url())
	}

	/// The `"images": [...]` array of the reader payload, decoded as JSON.
	fn script_images(&self, document: &Document) -> Option<Vec<String>> {
		let mut haystack = String::new();
		for node in document.ids() {
			if document
				.node(node)
				.element()
				.is_some_and(|element| element.name == "script")
			{
				haystack.push_str(&document.data(node));
				haystack.push('\n');
			}
		}
		let captured = self.images.captures(&haystack)?.get(1)?.as_str();
		let decoded: Vec<String> = serde_json::from_str(captured).ok()?;
		Some(
			decoded
				.into_iter()
				.filter_map(|url| document.resolve(unescape_slashes(&url).trim()))
				.collect(),
		)
	}
}

/// `/manga` from `manga`, `/manga/` or `manga/`.
fn normalise_directory(value: &str) -> String {
	let trimmed = value.trim().trim_matches('/');
	if trimmed.is_empty() {
		String::new()
	} else {
		format!("/{trimmed}")
	}
}

/// `MangaThemesia.removeEmptyPlaceholder` applied to one field.
fn field(document: &Document, selector: &Selector, scope: NodeId) -> Option<String> {
	selector
		.select_first(document, scope)
		.map(|node| {
			let own = document.own_text(node);
			if own.is_empty() {
				document.text(node)
			} else {
				own
			}
		})
		.map(|value| value.trim().to_string())
		.filter(|value| !PLACEHOLDERS.contains(&value.to_lowercase().as_str()))
}

/// The base class lowercases a genre then title-cases its first character.
fn titlecase(value: &str) -> String {
	let trimmed = value.trim();
	let mut chars = trimmed.chars();
	match chars.next() {
		Some(first) => {
			let mut out: String = first.to_uppercase().collect();
			out.extend(chars.flat_map(|c| c.to_lowercase()));
			out
		},
		None => String::new(),
	}
}

/// The reader payload escapes forward slashes as `\/`.
fn unescape_slashes(value: &str) -> String {
	value.replace("\\/", "/")
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
impl Source for MangaThemesiaSource {
	fn info(&self) -> &SourceInfo {
		&self.context.info
	}

	fn http(&self) -> &SourceHttp {
		&self.context.http
	}

	async fn popular(&self, page: u32) -> SourceResult<SourcePage<RemoteSeries>> {
		self.list(page, "", "popular").await
	}

	async fn latest(&self, page: u32) -> SourceResult<SourcePage<RemoteSeries>> {
		if !self.context.info.capabilities.latest {
			return Err(theme::unsupported(&self.context.info, "latest"));
		}
		self.list(page, "", "update").await
	}

	async fn search(
		&self,
		query: &str,
		filters: &[SearchFilter],
		page: u32,
	) -> SourceResult<SourcePage<RemoteSeries>> {
		// The only filter the theme exposes without a live genre fetch is the
		// project page, and only when the host has one.
		let project = self.has_project_page
			&& filters.iter().any(|filter| {
				filter.key == "project" && filter.value == "project-filter-on"
			});
		let url = self.list_url(page, query, "", project);
		let document = self.context.get(&url).await?;
		Ok(self.parse_list(&document))
	}

	async fn details(&self, remote_id: &str) -> SourceResult<RemoteSeries> {
		let document = self.context.get(&self.series_url(remote_id)).await?;
		Ok(self.parse_details(&document, remote_id))
	}

	async fn chapters(&self, remote_id: &str) -> SourceResult<Vec<RemoteChapter>> {
		let document = self.context.get(&self.series_url(remote_id)).await?;
		Ok(self.parse_chapters(&document))
	}

	async fn pages(&self, chapter_id: &str) -> SourceResult<Vec<RemotePage>> {
		let document = self.context.get(&self.chapter_url(chapter_id)).await?;
		Ok(self.parse_pages(&document))
	}
}

/// `ts_reader.run` payload shape, kept for documentation of what the regex
/// scrapes out of it.
#[derive(Debug, Deserialize)]
pub struct ReaderPayload {
	pub sources: Vec<ReaderSource>,
}

#[derive(Debug, Deserialize)]
pub struct ReaderSource {
	#[serde(default)]
	pub images: Vec<String>,
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::BTreeMap;
	use stump_provider::KnobValue;

	fn definition(knobs: &[(&str, KnobValue)]) -> SourceDefinition {
		SourceDefinition {
			schema: 1,
			id: "en.themesia".into(),
			name: "Themesia".into(),
			lang: "en".into(),
			base_url: "https://themesia.test".into(),
			theme: THEME.into(),
			version: 1,
			nsfw: false,
			knobs: knobs
				.iter()
				.map(|(key, value)| (key.to_string(), value.clone()))
				.collect::<BTreeMap<_, _>>(),
			upstream: None,
		}
	}

	fn engine(knobs: &[(&str, KnobValue)]) -> MangaThemesiaSource {
		MangaThemesiaSource::new(&definition(knobs), "en.themesia", None)
			.expect("engine builds")
	}

	#[test]
	fn list_urls_carry_title_page_and_order() {
		let source = engine(&[]);
		assert_eq!(
			source.list_url(2, "", "popular", false),
			"https://themesia.test/manga/?title=&page=2&order=popular"
		);
		assert_eq!(
			source.list_url(1, "solo leveling", "", false),
			"https://themesia.test/manga/?title=solo+leveling&page=1"
		);
		let custom =
			engine(&[("manga_url_directory", KnobValue::Text("series".into()))]);
		assert_eq!(
			custom.list_url(1, "", "update", false),
			"https://themesia.test/series/?title=&page=1&order=update"
		);
		assert_eq!(custom.series_url("abc"), "https://themesia.test/series/abc/");
	}

	#[test]
	fn project_page_knob_swaps_the_directory() {
		let source = engine(&[
			("has_project_page", KnobValue::Bool(true)),
			("project_page_string", KnobValue::Text("/proyek".into())),
		]);
		assert_eq!(
			source.list_url(1, "", "", true),
			"https://themesia.test/proyek/?title=&page=1"
		);
		assert!(source.has_project_page);
	}

	#[test]
	fn list_items_read_the_anchor_title_and_lazy_cover() {
		let source = engine(&[]);
		let html = r#"<html><body><div class="listupd">
			<div class="bs"><div class="bsx">
				<a href="/manga/alpha/" title="Alpha Series">
					<img src="/ph.gif" data-lazy-src="/covers/alpha.jpg">
				</a>
			</div></div>
			<div class="bs"><div class="bsx">
				<a href="https://themesia.test/manga/beta/"><img data-src="/covers/beta.jpg">Beta</a>
			</div></div>
			<div class="pagination"><a class="next" href="/manga/?page=2">Next</a></div>
		</div></body></html>"#;
		let page = source.parse_list(&Document::parse(html, "https://themesia.test/manga/"));
		assert_eq!(page.items.len(), 2);
		assert_eq!(page.items[0].remote_id, "alpha");
		assert_eq!(page.items[0].title, "Alpha Series");
		assert_eq!(
			page.items[0].thumbnail_url.as_deref(),
			Some("https://themesia.test/covers/alpha.jpg")
		);
		assert_eq!(page.items[1].remote_id, "beta");
		assert_eq!(page.items[1].title, "Beta");
		assert!(page.has_next);
	}

	#[test]
	fn details_scope_to_the_container_and_drop_placeholders() {
		let source = engine(&[]);
		let html = r#"<html><body>
			<div class="bigcontent">
				<h1 class="entry-title">Alpha Series</h1>
				<div class="thumb"><img data-src="/covers/alpha.jpg"></div>
				<div class="tsinfo">
					<div class="imptdt">Author <i>Kubo T.</i></div>
					<div class="imptdt">artist <i>-</i></div>
					<div class="imptdt">Status <i>Ongoing</i></div>
					<div class="imptdt">Type <i>Manhwa</i></div>
				</div>
				<div class="alternative">Alfa</div>
				<div class="desc">A description.</div>
				<div class="mgen"><a href="/genres/action/">ACTION</a><a href="/genres/drama/">Drama</a></div>
			</div>
			<h1 class="entry-title">Outside the container</h1>
		</body></html>"#;
		let series = source.parse_details(
			&Document::parse(html, "https://themesia.test/manga/alpha/"),
			"alpha",
		);
		assert_eq!(series.title, "Alpha Series");
		assert_eq!(series.authors, ["Kubo T."]);
		// `-` is a placeholder, not an artist.
		assert!(series.artists.is_empty());
		assert_eq!(series.status, stump_provider::SeriesStatus::Ongoing);
		assert_eq!(series.genres, ["Action", "Drama", "Manhwa"]);
		assert_eq!(
			series.description.as_deref(),
			Some("A description.\n\nAlternative Names: Alfa")
		);
		assert_eq!(
			series.thumbnail_url.as_deref(),
			Some("https://themesia.test/covers/alpha.jpg")
		);
	}

	#[test]
	fn chapters_read_names_dates_and_the_updated_on_fallback() {
		let source = engine(&[]);
		let html = r#"<html><body>
			<div class="listinfo"><time itemprop="dateModified" datetime="2024-04-05"></time></div>
			<div id="chapterlist"><ul>
				<li><div class="eph-num"><a href="/alpha-chapter-3/">
					<span class="chapternum">Chapter 3</span>
					<span class="chapterdate">April 5, 2024</span></a></div></li>
				<li><div class="eph-num"><a href="/alpha-chapter-2/">
					<span class="chapternum">Chapter 2</span></a></div></li>
			</ul></div>
		</body></html>"#;
		let chapters = source
			.parse_chapters(&Document::parse(html, "https://themesia.test/manga/alpha/"));
		assert_eq!(chapters.len(), 2);
		assert_eq!(chapters[0].remote_id, "alpha-chapter-3");
		assert_eq!(chapters[0].title.as_deref(), Some("Chapter 3"));
		assert_eq!(chapters[0].number, Some(3.0));
		assert_eq!(
			chapters[0].uploaded_at.unwrap().date_naive(),
			chrono::NaiveDate::from_ymd_opt(2024, 4, 5).unwrap()
		);
		assert_eq!(chapters[1].remote_id, "alpha-chapter-2");
		assert_eq!(chapters[1].uploaded_at, None);
	}

	#[test]
	fn dateless_chapter_lists_inherit_the_series_updated_on() {
		let source = engine(&[]);
		let html = r#"<html><body>
			<div class="listinfo"><time itemprop="dateModified" datetime="2024-04-05"></time></div>
			<div id="chapterlist"><ul>
				<li><a href="/alpha-chapter-3/">Chapter 3</a></li>
				<li><a href="/alpha-chapter-2/">Chapter 2</a></li>
			</ul></div>
		</body></html>"#;
		let chapters = source
			.parse_chapters(&Document::parse(html, "https://themesia.test/manga/alpha/"));
		assert_eq!(
			chapters[0].uploaded_at.unwrap().date_naive(),
			chrono::NaiveDate::from_ymd_opt(2024, 4, 5).unwrap()
		);
		assert_eq!(chapters[1].uploaded_at, None);
	}

	#[test]
	fn pages_prefer_reader_markup() {
		let source = engine(&[]);
		let html = r#"<html><body><div id="readerarea">
			<img src="/ph.gif" data-lazy-src="/pages/1.jpg">
			<img data-src="/pages/2.jpg">
		</div>
		<script>ts_reader.run({"sources":[{"images":["\/pages\/ignored.jpg"]}]});</script>
		</body></html>"#;
		let url = "https://themesia.test/alpha-chapter-1/";
		let pages = source.parse_pages(&Document::parse(html, url));
		assert_eq!(pages.len(), 2);
		assert_eq!(pages[0].url, "https://themesia.test/pages/1.jpg");
		assert_eq!(pages[1].url, "https://themesia.test/pages/2.jpg");
		assert!(pages[0]
			.headers
			.iter()
			.any(|(name, value)| name == "Referer" && value == url));
	}

	#[test]
	fn pages_fall_back_to_the_ts_reader_payload() {
		let source = engine(&[]);
		let html = r#"<html><body><div id="readerarea"></div>
		<script>
			ts_reader.run({"post_id":42,"sources":[{"source":"Main","images":[
				"https:\/\/cdn.test\/1.jpg","https:\/\/cdn.test\/2.jpg","\/relative\/3.jpg"
			]}]});
		</script></body></html>"#;
		let url = "https://themesia.test/alpha-chapter-1/";
		let pages = source.parse_pages(&Document::parse(html, url));
		assert_eq!(pages.len(), 3);
		assert_eq!(pages[0].url, "https://cdn.test/1.jpg");
		assert_eq!(pages[1].url, "https://cdn.test/2.jpg");
		// Relative entries resolve against the chapter URL.
		assert_eq!(pages[2].url, "https://themesia.test/relative/3.jpg");
		assert_eq!(pages[2].index, 2);
	}

	#[test]
	fn a_chapter_with_neither_markup_nor_payload_yields_no_pages() {
		let source = engine(&[]);
		let pages = source.parse_pages(&Document::parse(
			"<html><body><div id=\"readerarea\"></div></body></html>",
			"https://themesia.test/x/",
		));
		assert!(pages.is_empty());
	}

	#[test]
	fn page_selector_knob_overrides_the_reader_area() {
		let source = engine(&[("page_selector", KnobValue::Text("#custom img".into()))]);
		let html = r#"<html><body>
			<div id="readerarea"><img src="/wrong.jpg"></div>
			<div id="custom"><img src="/right.jpg"></div>
		</body></html>"#;
		let pages = source.parse_pages(&Document::parse(html, "https://themesia.test/x/"));
		assert_eq!(pages.len(), 1);
		assert_eq!(pages[0].url, "https://themesia.test/right.jpg");
	}

	#[test]
	fn locale_aware_chapter_dates() {
		let source = engine(&[
			("date_format", KnobValue::Text("MMMM dd, yyyy".into())),
			("date_locale", KnobValue::Text("id".into())),
		]);
		let html = r#"<html><body><div id="chapterlist"><ul>
			<li><a href="/x-chapter-1/">Chapter 1</a>
				<span class="chapterdate">Maret 14, 2024</span></li>
		</ul></div></body></html>"#;
		let chapters =
			source.parse_chapters(&Document::parse(html, "https://themesia.test/manga/x/"));
		assert_eq!(
			chapters[0].uploaded_at.unwrap().date_naive(),
			chrono::NaiveDate::from_ymd_opt(2024, 3, 14).unwrap()
		);
	}
}
