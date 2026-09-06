//! The Madara WordPress theme (`lib-multisrc/madara` and
//! `lib-multisrc/madaralegacy`), 281 extensions at `064c1a0e`.
//!
//! # Knobs and where they come from
//!
//! | knob | Kotlin member | default |
//! |---|---|---|
//! | `manga_sub_string` | `MadaraBase.mangaSubString` | `manga` |
//! | `genre_directory` | `MadaraBase.genreDirectory` | `manga-genre` |
//! | `filter_non_manga_items` | `MadaraBase.filterNonMangaItems` | `true` |
//! | `browse_mode` | `Madara` vs `MadaraNoAjax` base class | `ajax` |
//! | `use_load_more_request` | `madaralegacy Madara.useLoadMoreRequest` | `auto_detect` |
//! | `chapter_mode` | `MadaraBase.ChapterMode` | `manga_ajax` |
//! | `use_new_chapter_endpoint` | `madaralegacy Madara.useNewChapterEndpoint` | — (legacy alias for `chapter_mode`) |
//! | `chapter_url_suffix` | `madaralegacy Madara.chapterUrlSuffix` | `?style=list` |
//! | `chapter_date_format` / `date_format` | `MadaraBase.chapterDateFormat` | `MMMM dd, yyyy` |
//! | `archive_selector` | `MadaraBase.archiveSelector()` | `div.page-item-detail, .manga__item, .c-tabs-item__content` |
//! | `popular_manga_selector` / `latest_updates_selector` / `search_manga_selector` | `madaralegacy` per-mode selectors | `archive_selector` |
//! | `archive_url_selector` / `popular_manga_url_selector` / `search_manga_url_selector` | `MadaraBase.archiveUrlSelector` | `.post-title a` |
//! | `manga_details_selector_*` | `MadaraBase.mangaDetailsSelector*` | see below |
//! | `chapter_list_selector` | `MadaraBase.chapterListSelector()` | `li.wp-manga-chapter` |
//! | `chapter_url_selector` | `MadaraBase.chapterUrlSelector` | `a` |
//! | `chapter_date_selector` | `MadaraBase.chapterDateSelector` | `span.chapter-release-date` |
//! | `page_list_parse_selector` | `MadaraBase.pageListParseSelector` | `div.page-break, li.blocks-gallery-item, .reading-content .text-left:not(:has(.blocks-gallery-item)) img` |
//!
//! # Remote ids
//!
//! Series ids are the WordPress slug (`/{manga_sub_string}/{slug}/`) and chapter
//! ids are `{series_slug}/{chapter_slug}`. Upstream keys series on the numeric
//! post id, which changes when a site is migrated and cannot be turned back
//! into a URL without a request; the slug is stable, reversible, and readable in
//! a `provider://` path.
//!
//! # Not supported
//!
//! `#chapter-protector-data` (AES-encrypted page lists) needs a per-site key
//! schedule and is refused with `Unsupported` rather than returning an empty
//! chapter, so the failure is visible.

use std::sync::Arc;

use async_trait::async_trait;
use models::entity::provider_source;
use stump_provider::{
	definition::SourceDefinition, RemoteChapter, RemotePage, RemoteSeries, SearchFilter,
	Source, SourceCapabilities, SourceError, SourceHttp, SourceInfo, SourcePage,
	SourceResult,
};

use crate::{
	date::DateParser,
	dom::{Document, NodeId},
	selector::Selector,
	theme::{self, ImageAttrs, ThemeContext, ThemeError, DEFAULT_REQUESTS_PER_SECOND},
};

pub const THEMES: [&str; 2] = ["madara", "madaralegacy"];

/// `MadaraBase.imageFromElement` attribute order.
const IMAGE_ATTRS: ImageAttrs = ImageAttrs(&[
	"data-src",
	"data-lazy-src",
	"data-cfsrc",
	"data-manga-src",
	"srcset",
	"src",
]);

/// `Madara.kt` / `MadaraNoAjax.kt` page size.
const PAGE_SIZE: usize = 25;

/// How chapters are listed. `MadaraBase.ChapterMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChapterMode {
	/// Parsed from the series page itself.
	MangaPage,
	/// `POST /wp-admin/admin-ajax.php` with `action=manga_get_chapters`.
	AdminAjax,
	/// `POST {series}/ajax/chapters/`.
	MangaAjax,
	/// `MangaAjax` with `?t=N` pagination.
	MangaAjaxPaginated,
	/// `POST /index.php` with `maction=get_chapters`.
	MangaAjaxQuery,
}

impl ChapterMode {
	fn parse(value: &str) -> Option<Self> {
		match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
			"manga_page" | "mangapage" => Some(ChapterMode::MangaPage),
			"admin_ajax" | "adminajax" => Some(ChapterMode::AdminAjax),
			"manga_ajax" | "mangaajax" => Some(ChapterMode::MangaAjax),
			"manga_ajax_paginated" | "mangaajaxpaginated" => {
				Some(ChapterMode::MangaAjaxPaginated)
			},
			"manga_ajax_query" | "mangaajaxquery" => Some(ChapterMode::MangaAjaxQuery),
			_ => None,
		}
	}
}

/// How the browse lists are paged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseMode {
	/// `Madara`: `POST admin-ajax.php` with `action=madara_load_more`.
	Ajax,
	/// `MadaraNoAjax`: `GET /{manga_sub_string}/page/N/?m_orderby=`.
	NoAjax,
	/// `madaralegacy` `LoadMoreStrategy.AutoDetect`: try the archive page, and
	/// use the load-more endpoint when `nav.navigation-ajax` is present.
	AutoDetect,
}

impl BrowseMode {
	fn parse(value: &str) -> Option<Self> {
		match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
			"ajax" | "always" | "load_more" => Some(BrowseMode::Ajax),
			"no_ajax" | "never" | "noajax" => Some(BrowseMode::NoAjax),
			"auto_detect" | "autodetect" | "auto" => Some(BrowseMode::AutoDetect),
			_ => None,
		}
	}
}

#[derive(Debug)]
struct Selectors {
	popular: Selector,
	latest: Selector,
	search: Selector,
	popular_url: Selector,
	search_url: Selector,
	next_page: Selector,
	title: Selector,
	author: Selector,
	artist: Selector,
	status: Selector,
	description: Selector,
	thumbnail: Selector,
	genre: Selector,
	tag: Selector,
	series_type: Selector,
	chapter_list: Selector,
	chapter_url: Selector,
	chapter_date: Selector,
	pages: Selector,
	chapters_holder: Selector,
	protector: Selector,
	load_more_marker: Selector,
	single_pager: Selector,
}

/// A Madara-themed site driven by a definition.
#[derive(Debug)]
pub struct MadaraSource {
	context: ThemeContext,
	selectors: Selectors,
	dates: DateParser,
	manga_sub_string: String,
	filter_non_manga_items: bool,
	chapter_mode: ChapterMode,
	browse_mode: BrowseMode,
	chapter_url_suffix: String,
	/// `popularMangaRequest`/`latestUpdatesRequest`/`searchMangaRequest`
	/// overridden with a complete URL. Present means the whole request is
	/// replaced: no `admin-ajax.php`, no `page/N/` + `?m_orderby=`
	/// construction, because upstream's override returns one fixed `GET`.
	popular_url: Option<String>,
	latest_url: Option<String>,
	search_url: Option<String>,
	nsfw: bool,
}

impl MadaraSource {
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
		let archive = theme::selector(
			definition,
			"archive_selector",
			&["manga_entry_selector"],
			"div.page-item-detail, .manga__item, .c-tabs-item__content",
		)?;
		let archive_source = archive.source().to_string();
		let url_default = ".post-title a";
		let selectors = Selectors {
			popular: theme::selector(
				definition,
				"popular_manga_selector",
				&[],
				&archive_source,
			)?,
			latest: theme::selector(
				definition,
				"latest_updates_selector",
				&[],
				&archive_source,
			)?,
			search: theme::selector(
				definition,
				"search_manga_selector",
				&["search_card_selector"],
				&archive_source,
			)?,
			popular_url: theme::selector(
				definition,
				"popular_manga_url_selector",
				&["archive_url_selector", "manga_url_selector"],
				url_default,
			)?,
			search_url: theme::selector(
				definition,
				"search_manga_url_selector",
				&["archive_url_selector", "manga_url_selector"],
				url_default,
			)?,
			next_page: theme::selector(
				definition,
				"popular_manga_next_page_selector",
				&["search_manga_next_page_selector"],
				"div.nav-previous, a.nextpostslink, nav.navigation-ajax",
			)?,
			title: theme::selector(
				definition,
				"manga_details_selector_title",
				&["details_title_selector"],
				"div.post-title h3, div.post-title h1, #manga-title > h1",
			)?,
			author: theme::selector(
				definition,
				"manga_details_selector_author",
				&["details_author_selector"],
				"div.author-content > a, div.manga-authors > a",
			)?,
			artist: theme::selector(
				definition,
				"manga_details_selector_artist",
				&["details_artist_selector"],
				"div.artist-content > a",
			)?,
			status: theme::selector(
				definition,
				"manga_details_selector_status",
				&["details_status_selector"],
				"div.summary-content, div.summary-heading:contains(Status) + div",
			)?,
			description: theme::selector(
				definition,
				"manga_details_selector_description",
				&["details_description_selector"],
				"div.description-summary div.summary__content, div.summary_content div.post-content_item > h5 + div, div.summary_content div.manga-excerpt",
			)?,
			thumbnail: theme::selector(
				definition,
				"manga_details_selector_thumbnail",
				&["details_thumbnail_selector"],
				"div.summary_image img",
			)?,
			genre: theme::selector(
				definition,
				"manga_details_selector_genre",
				&["details_genre_selector"],
				"div.genres-content a",
			)?,
			tag: theme::selector(
				definition,
				"manga_details_selector_tag",
				&[],
				"div.tags-content a",
			)?,
			series_type: theme::selector(
				definition,
				"series_type_selector",
				&[],
				".post-content_item:contains(Type) .summary-content",
			)?,
			chapter_list: theme::selector(
				definition,
				"chapter_list_selector",
				&[],
				"li.wp-manga-chapter",
			)?,
			chapter_url: theme::selector(
				definition,
				"chapter_url_selector",
				&[],
				"a",
			)?,
			chapter_date: theme::selector(
				definition,
				"chapter_date_selector",
				&[],
				"span.chapter-release-date",
			)?,
			pages: theme::selector(
				definition,
				"page_list_parse_selector",
				&["page_list_selector", "page_selector"],
				"div.page-break, li.blocks-gallery-item, .reading-content .text-left:not(:has(.blocks-gallery-item)) img",
			)?,
			chapters_holder: theme::selector(
				definition,
				"chapters_holder_selector",
				&[],
				"[id^=manga-chapters-holder]",
			)?,
			protector: theme::selector(
				definition,
				"chapter_protector_selector",
				&[],
				"#chapter-protector-data",
			)?,
			load_more_marker: theme::selector(
				definition,
				"load_more_marker_selector",
				&[],
				"nav.navigation-ajax",
			)?,
			single_pager: theme::selector(
				definition,
				"single_pager_selector",
				&[],
				"#single-pager",
			)?,
		};
		Ok(Self {
			context,
			selectors,
			dates: chapter_dates(definition),
			manga_sub_string: definition
				.text_knob(&["manga_sub_string"])
				.unwrap_or("manga")
				.trim_matches('/')
				.to_string(),
			filter_non_manga_items: definition
				.bool_knob(&["filter_non_manga_items"])
				.unwrap_or(true),
			chapter_mode: chapter_mode(definition),
			browse_mode: browse_mode(definition),
			chapter_url_suffix: definition
				.text_knob(&["chapter_url_suffix"])
				.unwrap_or("?style=list")
				.to_string(),
			popular_url: definition
				.text_knob(&["popular_manga_url", "popular_url"])
				.map(str::to_string),
			latest_url: definition
				.text_knob(&["latest_updates_url", "latest_url"])
				.map(str::to_string),
			search_url: definition
				.text_knob(&["search_manga_url", "search_url"])
				.map(str::to_string),
			nsfw: definition.nsfw,
		})
	}

	fn archive_path(&self) -> String {
		format!("/{}/", self.manga_sub_string)
	}

	fn series_url(&self, slug: &str) -> String {
		self.context
			.url(&format!("/{}/{slug}/", self.manga_sub_string))
	}

	fn chapter_url(&self, chapter_id: &str) -> String {
		self.context
			.url(&format!("/{}/{chapter_id}/", self.manga_sub_string))
	}

	/// `Madara.ajaxList` / `madaralegacy Madara.loadMoreRequest`.
	fn load_more_form(
		&self,
		page: u32,
		order: Option<&str>,
		query: Option<&str>,
	) -> Vec<(String, String)> {
		let mut form = vec![
			("action".into(), "madara_load_more".into()),
			("page".into(), page.saturating_sub(1).to_string()),
			(
				"template".into(),
				"madara-core/content/content-archive".into(),
			),
			("vars[paged]".into(), "1".into()),
			("vars[template]".into(), "archive".into()),
			("vars[posts_per_page]".into(), PAGE_SIZE.to_string()),
			("vars[post_type]".into(), "wp-manga".into()),
			("vars[post_status]".into(), "publish".into()),
			("vars[sidebar]".into(), "right".into()),
			(
				"vars[manga_archives_item_layout]".into(),
				"big_thumbnail".into(),
			),
		];
		if self.filter_non_manga_items {
			form.push((
				"vars[meta_query][0][key]".into(),
				"_wp_manga_chapter_type".into(),
			));
			form.push(("vars[meta_query][0][value]".into(), "manga".into()));
		}
		if let Some(order) = order {
			form.push(("vars[orderby]".into(), "meta_value_num".into()));
			form.push(("vars[meta_key]".into(), order.into()));
			form.push(("vars[order]".into(), "DESC".into()));
		}
		if let Some(query) = query.filter(|query| !query.trim().is_empty()) {
			form.push(("vars[s]".into(), query.into()));
		}
		form
	}

	async fn load_more(
		&self,
		page: u32,
		order: Option<&str>,
		query: Option<&str>,
		list: &Selector,
		url_selector: &Selector,
	) -> SourceResult<SourcePage<RemoteSeries>> {
		let url = self.context.url("/wp-admin/admin-ajax.php");
		let form = self.load_more_form(page, order, query);
		let document = self.context.post_form(&url, &form).await?;
		let items = self.parse_archive(&document, list, url_selector);
		Ok(SourcePage {
			has_next: items.len() >= PAGE_SIZE,
			items,
		})
	}

	/// `MadaraNoAjax.archivePage`.
	async fn archive_page(
		&self,
		page: u32,
		order: &str,
		path: &str,
		query: &str,
		list: &Selector,
		url_selector: &Selector,
	) -> SourceResult<(SourcePage<RemoteSeries>, bool)> {
		let mut url = self.context.url(path);
		if page > 1 {
			url = format!(
				"{}page/{page}/",
				url.trim_end_matches('/').to_string() + "/"
			);
		}
		let mut query_parts: Vec<String> = Vec::new();
		if !order.is_empty() {
			query_parts.push(format!("m_orderby={order}"));
		}
		if !query.trim().is_empty() {
			query_parts.push(format!("s={}", urlencode(query)));
		}
		if !query_parts.is_empty() {
			url.push('?');
			url.push_str(&query_parts.join("&"));
		}
		let document = self.context.get(&url).await?;
		let load_more = self
			.selectors
			.load_more_marker
			.select_first(&document, document.root())
			.is_some();
		let items = self.parse_archive(&document, list, url_selector);
		let has_next = self
			.selectors
			.next_page
			.select_first(&document, document.root())
			.is_some();
		Ok((SourcePage { items, has_next }, load_more))
	}

	/// A definition that supplies a complete browse URL replaces the request
	/// wholesale, so only `{page}`/`{query}` are substituted. A template with
	/// no `{page}` is a single fixed page, exactly as upstream's override is.
	async fn fixed_page(
		&self,
		template: &str,
		page: u32,
		query: &str,
		list: &Selector,
		url_selector: &Selector,
	) -> SourceResult<SourcePage<RemoteSeries>> {
		if page > 1 && !template.contains("{page}") {
			return Ok(SourcePage::default());
		}
		let url = self.context.url(
			&template
				.replace("{page}", &page.to_string())
				.replace("{query}", &urlencode(query)),
		);
		let document = self.context.get(&url).await?;
		let items = self.parse_archive(&document, list, url_selector);
		let has_next = self
			.selectors
			.next_page
			.select_first(&document, document.root())
			.is_some();
		Ok(SourcePage { items, has_next })
	}

	/// Browse one list, honouring `browse_mode`. `AutoDetect` mirrors
	/// `madaralegacy`'s `detectLoadMore`: fetch the archive page, and if the
	/// site advertises the load-more nav, re-ask through `admin-ajax.php`.
	/// An `override_url` short-circuits all of that.
	#[allow(clippy::too_many_arguments)]
	async fn browse(
		&self,
		page: u32,
		ajax_order: &str,
		archive_order: &str,
		query: &str,
		list: &Selector,
		url_selector: &Selector,
		override_url: Option<&str>,
	) -> SourceResult<SourcePage<RemoteSeries>> {
		if let Some(template) = override_url {
			return self
				.fixed_page(template, page, query, list, url_selector)
				.await;
		}
		let query = (!query.trim().is_empty()).then_some(query);
		match self.browse_mode {
			BrowseMode::Ajax => {
				let result = self
					.load_more(page, Some(ajax_order), query, list, url_selector)
					.await?;
				if !result.items.is_empty() {
					return Ok(result);
				}
				// A site that answers the load-more endpoint with an empty
				// body is really a no-ajax site; fall back rather than
				// reporting an empty catalog.
				Ok(self
					.archive_page(
						page,
						archive_order,
						&self.archive_path(),
						query.unwrap_or_default(),
						list,
						url_selector,
					)
					.await?
					.0)
			},
			BrowseMode::NoAjax => Ok(self
				.archive_page(
					page,
					archive_order,
					&self.archive_path(),
					query.unwrap_or_default(),
					list,
					url_selector,
				)
				.await?
				.0),
			BrowseMode::AutoDetect => {
				let (result, load_more) = self
					.archive_page(
						page,
						archive_order,
						&self.archive_path(),
						query.unwrap_or_default(),
						list,
						url_selector,
					)
					.await?;
				if !load_more && !result.items.is_empty() {
					return Ok(result);
				}
				let ajax = self
					.load_more(page, Some(ajax_order), query, list, url_selector)
					.await?;
				Ok(if ajax.items.is_empty() { result } else { ajax })
			},
		}
	}

	/// `MadaraBase.parseArchive` — but keyed on the slug, not `data-post-id`.
	pub fn parse_archive(
		&self,
		document: &Document,
		list: &Selector,
		url_selector: &Selector,
	) -> Vec<RemoteSeries> {
		let mut out = Vec::new();
		for item in list.select(document, document.root()) {
			let anchor = url_selector
				.select_first(document, item)
				.or_else(|| (item_is_anchor(document, item)).then_some(item));
			let Some(anchor) = anchor else { continue };
			let Some(href) = document.abs_attr(anchor, "href") else {
				continue;
			};
			let Some(slug) = theme::slug_of(&href) else {
				continue;
			};
			let title = document.text(anchor);
			let title = if title.is_empty() {
				document
					.attr(anchor, "title")
					.map(str::to_string)
					.unwrap_or_else(|| slug.clone())
			} else {
				title
			};
			let thumbnail = first_image(document, item);
			out.push(theme::series(slug, title, Some(href), thumbnail, self.nsfw));
		}
		out
	}

	/// `MadaraBase.parseDetails`.
	pub fn parse_details(&self, document: &Document, slug: &str) -> RemoteSeries {
		let root = document.root();
		let selectors = &self.selectors;
		let title = selectors
			.title
			.select_first(document, root)
			.map(|node| document.own_text(node))
			.filter(|title| !title.is_empty())
			.unwrap_or_else(|| slug.to_string());
		let authors = texts(document, &selectors.author, root);
		let artists = texts(document, &selectors.artist, root);
		let description = selectors
			.description
			.select_first(document, root)
			.map(|node| document.text_with_newlines(node))
			.filter(|value| !value.is_empty());
		let thumbnail_url = selectors
			.thumbnail
			.select_first(document, root)
			.and_then(|node| IMAGE_ATTRS.resolve(document, node));
		let status = selectors
			.status
			.select(document, root)
			.last()
			.map(|node| theme::parse_status(&document.text(*node)))
			.unwrap_or_default();
		let mut genres = texts(document, &selectors.genre, root);
		genres.extend(texts(document, &selectors.tag, root));
		if let Some(series_type) = selectors
			.series_type
			.select_first(document, root)
			.map(|node| document.text(node))
			.filter(|value| !value.is_empty())
		{
			genres.push(series_type);
		}
		dedupe_ignoring_case(&mut genres);
		RemoteSeries {
			remote_id: slug.to_string(),
			title,
			url: Some(document.url().to_string()),
			thumbnail_url,
			description,
			authors,
			artists,
			genres,
			status,
			nsfw: self.nsfw,
			..Default::default()
		}
	}

	/// `MadaraBase.parseChapterList` / `chapterFromElement`, keyed on
	/// `{series_slug}/{chapter_slug}`.
	pub fn parse_chapter_list(
		&self,
		document: &Document,
		series_slug: &str,
	) -> Vec<RemoteChapter> {
		let mut out = Vec::new();
		for item in self
			.selectors
			.chapter_list
			.select(document, document.root())
		{
			let anchor = self
				.selectors
				.chapter_url
				.select_first(document, item)
				.or_else(|| item_is_anchor(document, item).then_some(item));
			let Some(anchor) = anchor else { continue };
			let Some(href) = document.abs_attr(anchor, "href") else {
				continue;
			};
			let Some(chapter_slug) = theme::slug_of(&href) else {
				continue;
			};
			let name = document.text(anchor);
			// Dates ride on a "new" badge's alt text, a title attribute, or
			// the release-date span, in that order.
			let raw_date = first_attr(document, item, "img:not(.thumb)", "alt")
				.or_else(|| first_attr(document, item, "span a", "title"))
				.or_else(|| {
					self.selectors
						.chapter_date
						.select_first(document, item)
						.map(|node| document.text(node))
				})
				.or_else(|| first_attr(document, item, "time[datetime]", "datetime"));
			let uploaded_at = raw_date
				.as_deref()
				.and_then(|value| self.dates.parse(value));
			out.push(theme::chapter(
				format!("{series_slug}/{chapter_slug}"),
				(!name.is_empty()).then(|| name.clone()),
				theme::chapter_number(&name),
				uploaded_at,
				Some(href),
				&self.context.info.lang,
			));
		}
		out
	}

	/// `MadaraBase.parsePages`.
	pub fn parse_pages(&self, document: &Document) -> SourceResult<Vec<RemotePage>> {
		let root = document.root();
		if self
			.selectors
			.protector
			.select_first(document, root)
			.is_some()
		{
			return Err(SourceError::Unsupported {
				source_id: self.context.info.id.clone(),
				operation: "AES-protected chapter pages",
			});
		}
		let urls: Vec<String> = self
			.selectors
			.pages
			.select(document, root)
			.into_iter()
			.filter_map(|node| {
				let image = if is_image(document, node) {
					node
				} else {
					first_descendant_image(document, node)?
				};
				IMAGE_ATTRS.resolve(document, image)
			})
			.collect();
		Ok(theme::pages_with_referer(urls, document.url()))
	}

	/// The WordPress post id, needed only by `ChapterMode::AdminAjax`.
	/// `MadaraBase.Document.mangaId()`.
	pub fn manga_id(&self, document: &Document) -> Option<String> {
		let root = document.root();
		if let Some(id) = self
			.selectors
			.chapters_holder
			.select_first(document, root)
			.and_then(|node| document.attr(node, "data-id"))
			.filter(|value| !value.trim().is_empty())
		{
			return Some(id.to_string());
		}
		for (selector, attribute) in [
			("input.rating-post-id", "value"),
			("a[data-post]", "data-post"),
		] {
			if let Some(id) = first_attr(document, root, selector, attribute)
				.filter(|value| !value.trim().is_empty())
			{
				return Some(id);
			}
		}
		first_attr(document, root, "link[rel=shortlink]", "href")
			.filter(|href| href.contains("?p="))
			.and_then(|href| {
				href.split("?p=")
					.nth(1)
					.and_then(|rest| rest.split('&').next())
					.map(str::to_string)
			})
			.filter(|value| !value.is_empty())
	}

	async fn fetch_chapters(
		&self,
		slug: &str,
		details: &Document,
	) -> SourceResult<Vec<RemoteChapter>> {
		let from_page = self.parse_chapter_list(details, slug);
		match self.chapter_mode {
			ChapterMode::MangaPage => return Ok(from_page),
			_ if !from_page.is_empty() => return Ok(from_page),
			_ => {},
		}
		let ajax = self.chapter_ajax(slug, details).await?;
		if !ajax.is_empty() {
			return Ok(ajax);
		}
		Ok(from_page)
	}

	async fn chapter_ajax(
		&self,
		slug: &str,
		details: &Document,
	) -> SourceResult<Vec<RemoteChapter>> {
		match self.chapter_mode {
			ChapterMode::MangaPage => Ok(Vec::new()),
			ChapterMode::AdminAjax => {
				let Some(id) = self.manga_id(details) else {
					// No post id: the new endpoint is the only option left.
					return self.manga_ajax(slug, None).await;
				};
				let url = self.context.url("/wp-admin/admin-ajax.php");
				let form = vec![
					("action".to_string(), "manga_get_chapters".to_string()),
					("manga".to_string(), id),
				];
				let document = self.context.post_form(&url, &form).await?;
				let chapters = self.parse_chapter_list(&document, slug);
				if chapters.is_empty() {
					// Newer Madara answers the old endpoint with HTTP 400 or an
					// empty body; upstream then pins the new one.
					return self.manga_ajax(slug, None).await;
				}
				Ok(chapters)
			},
			ChapterMode::MangaAjax => self.manga_ajax(slug, None).await,
			ChapterMode::MangaAjaxPaginated => {
				let mut out: Vec<RemoteChapter> = Vec::new();
				let mut last: Option<String> = None;
				for page in 1..=MAX_CHAPTER_PAGES {
					let chapters = match self.manga_ajax(slug, Some(page)).await {
						Ok(chapters) => chapters,
						Err(SourceError::NotFound(_)) => break,
						Err(error) => return Err(error),
					};
					let Some(tail) = chapters.last().map(|c| c.remote_id.clone()) else {
						break;
					};
					if last.as_deref() == Some(tail.as_str()) {
						break;
					}
					last = Some(tail);
					out.extend(chapters);
				}
				Ok(out)
			},
			ChapterMode::MangaAjaxQuery => {
				let url = self.context.url("/index.php");
				let form = vec![
					("manga-core".to_string(), slug.to_string()),
					("manga_ajax".to_string(), "1".to_string()),
					("maction".to_string(), "get_chapters".to_string()),
				];
				let document = self.context.post_form(&url, &form).await?;
				Ok(self.parse_chapter_list(&document, slug))
			},
		}
	}

	async fn manga_ajax(
		&self,
		slug: &str,
		page: Option<u32>,
	) -> SourceResult<Vec<RemoteChapter>> {
		let mut url = format!(
			"{}/{}/{slug}/ajax/chapters/",
			self.context.base_url, self.manga_sub_string
		);
		if let Some(page) = page {
			url.push_str(&format!("?t={page}"));
		}
		let document = self.context.post_form(&url, &[]).await?;
		Ok(self.parse_chapter_list(&document, slug))
	}
}

/// Guard for `ChapterMode::MangaAjaxPaginated`, which upstream loops without a
/// bound. A site that always answers 200 with the same body would spin forever.
const MAX_CHAPTER_PAGES: u32 = 200;

fn chapter_dates(definition: &SourceDefinition) -> DateParser {
	let pattern = definition
		.text_knob(&["chapter_date_format", "date_format"])
		.unwrap_or("MMMM dd, yyyy");
	let locale = definition
		.text_knob(&["chapter_date_locale", "date_locale"])
		.unwrap_or("en-US");
	DateParser::new(pattern, locale)
}

fn chapter_mode(definition: &SourceDefinition) -> ChapterMode {
	if let Some(mode) = definition
		.text_knob(&["chapter_mode"])
		.and_then(ChapterMode::parse)
	{
		return mode;
	}
	// `madaralegacy`'s boolean: true selected `{manga}/ajax/chapters`, false the
	// `admin-ajax.php` `manga_get_chapters` action.
	match definition.bool_knob(&["use_new_chapter_endpoint"]) {
		Some(true) => ChapterMode::MangaAjax,
		Some(false) => ChapterMode::AdminAjax,
		None => ChapterMode::MangaAjax,
	}
}

fn browse_mode(definition: &SourceDefinition) -> BrowseMode {
	if let Some(mode) = definition
		.text_knob(&["browse_mode"])
		.and_then(BrowseMode::parse)
	{
		return mode;
	}
	if let Some(mode) = definition
		.text_knob(&["use_load_more_request"])
		.and_then(BrowseMode::parse)
	{
		return mode;
	}
	match definition.bool_knob(&["use_load_more_request"]) {
		Some(true) => BrowseMode::Ajax,
		Some(false) => BrowseMode::NoAjax,
		// `madaralegacy` defaults to AutoDetect; `madara` is always ajax. A
		// definition that names neither gets the safe probe.
		None => match definition.theme.as_str() {
			"madara" => BrowseMode::Ajax,
			_ => BrowseMode::AutoDetect,
		},
	}
}

fn texts(document: &Document, selector: &Selector, root: NodeId) -> Vec<String> {
	selector
		.select(document, root)
		.into_iter()
		.map(|node| document.text(node))
		.filter(|value| !value.is_empty() && !is_updating(value))
		.collect()
}

/// `MadaraBase.updatingRegex`: a placeholder author/artist value.
fn is_updating(value: &str) -> bool {
	let lower = value.to_lowercase();
	lower.contains("updating") || lower.contains("atualizando")
}

fn dedupe_ignoring_case(values: &mut Vec<String>) {
	let mut seen: Vec<String> = Vec::with_capacity(values.len());
	values.retain(|value| {
		let key = value.to_lowercase();
		if seen.contains(&key) {
			return false;
		}
		seen.push(key);
		true
	});
}

fn item_is_anchor(document: &Document, node: NodeId) -> bool {
	document
		.node(node)
		.element()
		.is_some_and(|element| element.name == "a")
}

fn is_image(document: &Document, node: NodeId) -> bool {
	document
		.node(node)
		.element()
		.is_some_and(|element| element.name == "img")
}

fn first_descendant_image(document: &Document, node: NodeId) -> Option<NodeId> {
	document
		.descendants(node)
		.into_iter()
		.find(|child| is_image(document, *child))
}

fn first_image(document: &Document, node: NodeId) -> Option<String> {
	first_descendant_image(document, node)
		.and_then(|image| IMAGE_ATTRS.resolve(document, image))
}

fn first_attr(
	document: &Document,
	root: NodeId,
	selector: &str,
	attribute: &str,
) -> Option<String> {
	let selector = Selector::parse(selector).ok()?;
	selector
		.select_first(document, root)
		.and_then(|node| document.attr(node, attribute))
		.map(str::to_string)
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
impl Source for MadaraSource {
	fn info(&self) -> &SourceInfo {
		&self.context.info
	}

	fn http(&self) -> &SourceHttp {
		&self.context.http
	}

	async fn popular(&self, page: u32) -> SourceResult<SourcePage<RemoteSeries>> {
		self.browse(
			page,
			"_wp_manga_views",
			"views",
			"",
			&self.selectors.popular,
			&self.selectors.popular_url,
			self.popular_url.as_deref(),
		)
		.await
	}

	async fn latest(&self, page: u32) -> SourceResult<SourcePage<RemoteSeries>> {
		if !self.context.info.capabilities.latest {
			return Err(theme::unsupported(&self.context.info, "latest"));
		}
		self.browse(
			page,
			"_latest_update",
			"latest",
			"",
			&self.selectors.latest,
			&self.selectors.popular_url,
			self.latest_url.as_deref(),
		)
		.await
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
		if let Some(template) = self.search_url.as_deref() {
			return self
				.fixed_page(
					template,
					page,
					query,
					&self.selectors.search,
					&self.selectors.search_url,
				)
				.await;
		}
		match self.browse_mode {
			BrowseMode::NoAjax | BrowseMode::AutoDetect => {
				// `MadaraNoAjax.htmlSearch`: WordPress' own search, which is
				// not paged for `wp-manga`.
				if page > 1 {
					return Ok(SourcePage::default());
				}
				let url = format!(
					"{}/?s={}&post_type=wp-manga",
					self.context.base_url,
					urlencode(query)
				);
				let document = self.context.get(&url).await?;
				let items = self.parse_archive(
					&document,
					&self.selectors.search,
					&self.selectors.search_url,
				);
				Ok(SourcePage {
					items,
					has_next: false,
				})
			},
			BrowseMode::Ajax => {
				self.load_more(
					page,
					None,
					Some(query),
					&self.selectors.search,
					&self.selectors.search_url,
				)
				.await
			},
		}
	}

	async fn details(&self, remote_id: &str) -> SourceResult<RemoteSeries> {
		let document = self.context.get(&self.series_url(remote_id)).await?;
		Ok(self.parse_details(&document, remote_id))
	}

	async fn chapters(&self, remote_id: &str) -> SourceResult<Vec<RemoteChapter>> {
		let document = self.context.get(&self.series_url(remote_id)).await?;
		self.fetch_chapters(remote_id, &document).await
	}

	async fn pages(&self, chapter_id: &str) -> SourceResult<Vec<RemotePage>> {
		let url = self.chapter_url(chapter_id);
		let document = self.context.get(&url).await?;
		// `#single-pager` means the chapter paginates one image per request;
		// `?style=list` asks for all of them at once.
		let document = if self
			.selectors
			.single_pager
			.select_first(&document, document.root())
			.is_some()
			&& !self.chapter_url_suffix.is_empty()
		{
			let separator = if url.contains('?') { "&" } else { "" };
			let suffix = self.chapter_url_suffix.trim_start_matches('?');
			self.context
				.get(&format!(
					"{url}{separator}{}{suffix}",
					if separator.is_empty() { "?" } else { "" }
				))
				.await?
		} else {
			document
		};
		self.parse_pages(&document)
	}
}

/// The rate limit a definition without a `rate_limit_*` knob runs at.
pub const fn default_requests_per_second() -> u32 {
	DEFAULT_REQUESTS_PER_SECOND
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::BTreeMap;
	use stump_provider::KnobValue;

	fn definition(knobs: &[(&str, KnobValue)]) -> SourceDefinition {
		SourceDefinition {
			schema: 1,
			id: "en.example".into(),
			name: "Example".into(),
			lang: "en".into(),
			base_url: "https://madara.test".into(),
			theme: "madara".into(),
			version: 1,
			nsfw: false,
			knobs: knobs
				.iter()
				.map(|(key, value)| (key.to_string(), value.clone()))
				.collect::<BTreeMap<_, _>>(),
			upstream: None,
		}
	}

	fn engine(knobs: &[(&str, KnobValue)]) -> MadaraSource {
		MadaraSource::new(&definition(knobs), "en.example", None).expect("engine builds")
	}

	const ARCHIVE: &str = r#"<html><body>
		<div class="page-item-detail" data-post-id="12">
			<div class="post-title"><h3><a href="/manga/first-series/">First Series</a></h3></div>
			<img src="/ph.gif" data-src="/covers/first.jpg">
		</div>
		<div class="page-item-detail">
			<div class="post-title"><h3><a href="https://madara.test/manga/second-series/">Second</a></h3></div>
			<img srcset="/covers/second-small.jpg 320w, /covers/second-big.jpg 1024w">
		</div>
		<div class="nav-previous"><a href="/manga/page/2/">Older</a></div>
	</body></html>"#;

	const DETAILS: &str = r#"<html><body>
		<div class="post-title"><h1>First Series</h1></div>
		<div class="author-content"><a href="/author/one/">Author One</a><a href="/author/x/">Updating</a></div>
		<div class="artist-content"><a href="/artist/one/">Artist One</a></div>
		<div class="description-summary"><div class="summary__content"><p>Line one.</p><p>Line two.</p></div></div>
		<div class="summary_image"><img data-src="/covers/first.jpg"></div>
		<div class="genres-content"><a href="/manga-genre/action/">Action</a><a href="/manga-genre/drama/">Drama</a></div>
		<div class="tags-content"><a href="/manga-tag/action/">action</a></div>
		<div class="post-content">
			<div class="post-content_item"><h5>Type</h5><div class="summary-content">Manhwa</div></div>
			<div class="post-content_item"><h5>Status</h5><div class="summary-content">Completed</div></div>
		</div>
		<div id="manga-chapters-holder" data-id="4242"></div>
		<link rel="shortlink" href="https://madara.test/?p=4242">
	</body></html>"#;

	const CHAPTERS: &str = r#"<html><body><ul>
		<li class="wp-manga-chapter">
			<a href="https://madara.test/manga/first-series/chapter-2/">Chapter 2</a>
			<span class="chapter-release-date">March 3, 2024</span>
		</li>
		<li class="wp-manga-chapter">
			<a href="/manga/first-series/chapter-1-5/">Chapter 1.5</a>
			<span class="chapter-release-date">2 days ago</span>
		</li>
		<li class="wp-manga-chapter">
			<a href="/manga/first-series/chapter-1/">Chapter 1</a>
			<img class="thumb" src="/new.png" alt="skip me">
			<img src="/new.png" alt="January 1, 2024">
		</li>
	</ul></body></html>"#;

	fn document(html: &str, url: &str) -> Document {
		Document::parse(html, url)
	}

	#[test]
	fn archive_items_key_on_the_slug_and_prefer_lazy_covers() {
		let source = engine(&[]);
		let document = document(ARCHIVE, "https://madara.test/manga/");
		let items = source.parse_archive(
			&document,
			&source.selectors.popular,
			&source.selectors.popular_url,
		);
		assert_eq!(items.len(), 2);
		assert_eq!(items[0].remote_id, "first-series");
		assert_eq!(items[0].title, "First Series");
		assert_eq!(
			items[0].url.as_deref(),
			Some("https://madara.test/manga/first-series/")
		);
		assert_eq!(
			items[0].thumbnail_url.as_deref(),
			Some("https://madara.test/covers/first.jpg")
		);
		assert_eq!(items[1].remote_id, "second-series");
		assert_eq!(
			items[1].thumbnail_url.as_deref(),
			Some("https://madara.test/covers/second-big.jpg")
		);
	}

	#[test]
	fn details_use_the_base_class_selectors() {
		let source = engine(&[]);
		let document = document(DETAILS, "https://madara.test/manga/first-series/");
		let series = source.parse_details(&document, "first-series");
		assert_eq!(series.remote_id, "first-series");
		assert_eq!(series.title, "First Series");
		// The "Updating" placeholder is dropped, as upstream does.
		assert_eq!(series.authors, ["Author One"]);
		assert_eq!(series.artists, ["Artist One"]);
		assert_eq!(series.status, stump_provider::SeriesStatus::Completed);
		assert_eq!(series.description.as_deref(), Some("Line one.\nLine two."));
		assert_eq!(
			series.thumbnail_url.as_deref(),
			Some("https://madara.test/covers/first.jpg")
		);
		// Genres, tags and the series type merge, deduped case-insensitively.
		assert_eq!(series.genres, ["Action", "Drama", "Manhwa"]);
	}

	/// The status selector's second alternative,
	/// `div.summary-heading:contains(Status) + div`, is the layout older Madara
	/// templates use; both must resolve or half the theme reports Unknown.
	#[test]
	fn status_also_reads_the_heading_sibling_layout() {
		let source = engine(&[]);
		let document = document(
			r#"<html><body>
				<div class="post-title"><h1>Alt Layout</h1></div>
				<div class="summary-heading">Status</div><div>On Hold</div>
			</body></html>"#,
			"https://madara.test/manga/alt/",
		);
		assert_eq!(
			source.parse_details(&document, "alt").status,
			stump_provider::SeriesStatus::Hiatus
		);
	}

	#[test]
	fn details_selector_knobs_replace_the_defaults() {
		let source = engine(&[(
			"manga_details_selector_title",
			KnobValue::Text("h2.custom-title".into()),
		)]);
		let document = document(
			r#"<html><body><div class="post-title"><h1>Ignored</h1></div>
				<h2 class="custom-title">Knobbed Title</h2></body></html>"#,
			"https://madara.test/manga/x/",
		);
		assert_eq!(source.parse_details(&document, "x").title, "Knobbed Title");
	}

	#[test]
	fn chapters_key_on_series_and_chapter_slug_with_dates() {
		let source = engine(&[]);
		let document = document(CHAPTERS, "https://madara.test/manga/first-series/");
		let chapters = source.parse_chapter_list(&document, "first-series");
		assert_eq!(chapters.len(), 3);
		assert_eq!(chapters[0].remote_id, "first-series/chapter-2");
		assert_eq!(chapters[0].number, Some(2.0));
		assert_eq!(
			chapters[0].uploaded_at.unwrap().date_naive(),
			chrono::NaiveDate::from_ymd_opt(2024, 3, 3).unwrap()
		);
		assert_eq!(chapters[1].remote_id, "first-series/chapter-1-5");
		assert_eq!(chapters[1].number, Some(1.5));
		// "2 days ago" resolves relative to now, so only the offset is checked.
		let relative = chapters[1].uploaded_at.unwrap();
		let expected = chrono::Utc::now() - chrono::Duration::days(2);
		assert!((expected - relative).num_seconds().abs() < 60);
		// The `.thumb` badge is skipped and the real badge's alt is the date.
		assert_eq!(
			chapters[2].uploaded_at.unwrap().date_naive(),
			chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
		);
	}

	#[test]
	fn pages_apply_lazy_precedence_and_a_chapter_referer() {
		let source = engine(&[]);
		let html = r#"<html><body><div class="reading-content">
			<div class="page-break"><img src="/ph.gif" data-src="/pages/1.jpg"></div>
			<div class="page-break"><img data-lazy-src="/pages/2.jpg"></div>
			<div class="page-break"><img src="/pages/3.jpg"></div>
		</div></body></html>"#;
		let url = "https://madara.test/manga/first-series/chapter-1/";
		let pages = source.parse_pages(&document(html, url)).unwrap();
		assert_eq!(pages.len(), 3);
		assert_eq!(pages[0].url, "https://madara.test/pages/1.jpg");
		assert_eq!(pages[1].url, "https://madara.test/pages/2.jpg");
		assert_eq!(pages[2].url, "https://madara.test/pages/3.jpg");
		assert_eq!(pages[0].index, 0);
		assert!(pages.iter().all(|page| page
			.headers
			.iter()
			.any(|(name, value)| name == "Referer" && value == url)));
	}

	#[test]
	fn encrypted_chapters_are_refused_instead_of_returning_nothing() {
		let source = engine(&[]);
		let html = r#"<html><body><div id="chapter-protector-data"
			src="data:text/javascript;base64,AAAA"></div></body></html>"#;
		let error = source
			.parse_pages(&document(html, "https://madara.test/x/"))
			.expect_err("protected chapters are refused");
		assert!(matches!(error, SourceError::Unsupported { .. }), "{error}");
	}

	#[test]
	fn manga_id_falls_back_through_every_upstream_hook() {
		let source = engine(&[]);
		let holder = document(DETAILS, "https://madara.test/manga/x/");
		assert_eq!(source.manga_id(&holder).as_deref(), Some("4242"));

		let rating = document(
			r#"<html><body><input class="rating-post-id" value="77"></body></html>"#,
			"https://madara.test/manga/x/",
		);
		assert_eq!(source.manga_id(&rating).as_deref(), Some("77"));

		let shortlink = document(
			r#"<html><head><link rel="shortlink" href="https://madara.test/?p=91&x=1"></head><body></body></html>"#,
			"https://madara.test/manga/x/",
		);
		assert_eq!(source.manga_id(&shortlink).as_deref(), Some("91"));

		let nothing = document("<html><body></body></html>", "https://madara.test/");
		assert_eq!(source.manga_id(&nothing), None);
	}

	#[test]
	fn chapter_mode_reads_the_modern_knob_and_the_legacy_boolean() {
		assert_eq!(
			engine(&[("chapter_mode", KnobValue::Text("admin_ajax".into()))])
				.chapter_mode,
			ChapterMode::AdminAjax
		);
		assert_eq!(
			engine(&[(
				"chapter_mode",
				KnobValue::Text("manga_ajax_paginated".into())
			)])
			.chapter_mode,
			ChapterMode::MangaAjaxPaginated
		);
		assert_eq!(
			engine(&[("use_new_chapter_endpoint", KnobValue::Bool(false))]).chapter_mode,
			ChapterMode::AdminAjax
		);
		assert_eq!(
			engine(&[("use_new_chapter_endpoint", KnobValue::Bool(true))]).chapter_mode,
			ChapterMode::MangaAjax
		);
		// Unknown values must not silently disable chapters.
		assert_eq!(
			engine(&[("chapter_mode", KnobValue::Text("carrier_pigeon".into()))])
				.chapter_mode,
			ChapterMode::MangaAjax
		);
	}

	/// `override fun popularMangaRequest` replaces the whole request, so the
	/// generated `popular_manga_url`/`latest_updates_url` must bypass both
	/// `admin-ajax.php` and the `/{manga_sub_string}/page/N/?m_orderby=`
	/// construction (`id.pornhwa18` browses `/series/`, not `/manga/`).
	#[test]
	fn generated_request_urls_replace_the_constructed_browse_url() {
		let bare = engine(&[]);
		assert!(bare.popular_url.is_none());
		assert!(bare.latest_url.is_none());
		assert_eq!(bare.archive_path(), "/manga/");
		let overridden = engine(&[
			(
				"popular_manga_url",
				KnobValue::Text(
					"https://pornhwa18.com/series/page/{page}/?m_orderby=views".into(),
				),
			),
			(
				"latest_updates_url",
				KnobValue::Text(
					"https://pornhwa18.com/series/page/{page}/?m_orderby=latest".into(),
				),
			),
		]);
		assert_eq!(
			overridden.popular_url.as_deref(),
			Some("https://pornhwa18.com/series/page/{page}/?m_orderby=views")
		);
		assert_eq!(
			overridden.latest_url.as_deref(),
			Some("https://pornhwa18.com/series/page/{page}/?m_orderby=latest")
		);
	}

	#[test]
	fn browse_mode_defaults_per_theme_and_reads_both_knobs() {
		assert_eq!(engine(&[]).browse_mode, BrowseMode::Ajax);
		let mut legacy = definition(&[]);
		legacy.theme = "madaralegacy".into();
		assert_eq!(
			MadaraSource::new(&legacy, "x", None).unwrap().browse_mode,
			BrowseMode::AutoDetect
		);
		assert_eq!(
			engine(&[("browse_mode", KnobValue::Text("no_ajax".into()))]).browse_mode,
			BrowseMode::NoAjax
		);
		assert_eq!(
			engine(&[("use_load_more_request", KnobValue::Text("Never".into()))])
				.browse_mode,
			BrowseMode::NoAjax
		);
		assert_eq!(
			engine(&[("use_load_more_request", KnobValue::Bool(true))]).browse_mode,
			BrowseMode::Ajax
		);
	}

	#[test]
	fn manga_sub_string_knob_rewrites_every_url() {
		let source = engine(&[("manga_sub_string", KnobValue::Text("series".into()))]);
		assert_eq!(source.series_url("foo"), "https://madara.test/series/foo/");
		assert_eq!(
			source.chapter_url("foo/chapter-1"),
			"https://madara.test/series/foo/chapter-1/"
		);
		assert_eq!(source.archive_path(), "/series/");
	}

	#[test]
	fn load_more_form_matches_the_upstream_body() {
		let source = engine(&[]);
		let form = source.load_more_form(2, Some("_wp_manga_views"), None);
		let get = |key: &str| {
			form.iter()
				.find(|(name, _)| name == key)
				.map(|(_, value)| value.as_str())
		};
		assert_eq!(get("action"), Some("madara_load_more"));
		// Upstream sends the zero-based page.
		assert_eq!(get("page"), Some("1"));
		assert_eq!(get("template"), Some("madara-core/content/content-archive"));
		assert_eq!(get("vars[post_type]"), Some("wp-manga"));
		assert_eq!(get("vars[posts_per_page]"), Some("25"));
		assert_eq!(get("vars[meta_key]"), Some("_wp_manga_views"));
		assert_eq!(
			get("vars[meta_query][0][key]"),
			Some("_wp_manga_chapter_type")
		);

		let unfiltered = engine(&[("filter_non_manga_items", KnobValue::Bool(false))]);
		let form = unfiltered.load_more_form(1, Some("_latest_update"), Some("boku"));
		assert!(form
			.iter()
			.all(|(name, _)| name != "vars[meta_query][0][key]"));
		assert!(form
			.iter()
			.any(|(name, value)| name == "vars[s]" && value == "boku"));
	}

	#[test]
	fn a_bad_selector_knob_fails_at_construction() {
		let error = MadaraSource::new(
			&definition(&[(
				"chapter_list_selector",
				KnobValue::Text("li:nth-of-type(2)".into()),
			)]),
			"en.example",
			None,
		)
		.expect_err("invalid selector is rejected");
		assert!(
			matches!(&error, ThemeError::Selector { knob, .. } if *knob == "chapter_list_selector"),
			"{error}"
		);
	}

	#[test]
	fn base_url_override_from_the_row_wins() {
		let definition = definition(&[]);
		let source =
			MadaraSource::new(&definition, "en.example", Some("https://mirror.test/"))
				.unwrap();
		assert_eq!(source.context.base_url, "https://mirror.test");
		assert_eq!(source.series_url("x"), "https://mirror.test/manga/x/");
	}
}
