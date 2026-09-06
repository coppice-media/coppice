//! `epub-polish`: the in-process, calibre-free polish pass — an EPUB 2 to
//! EPUB 3.3 upgrade, typographic punctuation, and calibre jacket removal.
//!
//! Behaviour provenance (specifications and published rules, never code —
//! calibre is GPL-3 and is only ever *executed*, by `calibre-polish`):
//! - `version="3.0"` is the only conforming value for an EPUB 3 package
//!   document: <https://www.w3.org/TR/epub-33/#attrdef-package-version>
//! - the metadata section must contain exactly one `dcterms:modified`
//!   property, formatted `YYYY-MM-DDThh:mm:ssZ`:
//!   <https://www.w3.org/TR/epub-33/#last-modified-date>
//! - the EPUB navigation document and its `toc`, `page-list` and `landmarks`
//!   `nav` elements, plus the `nav` manifest property:
//!   <https://www.w3.org/TR/epub-33/#sec-nav>,
//!   <https://www.w3.org/TR/epub-33/#sec-nav-toc>,
//!   <https://www.w3.org/TR/epub-33/#sec-nav-pagelist>,
//!   <https://www.w3.org/TR/epub-33/#sec-nav-landmarks>,
//!   <https://www.w3.org/TR/epub-33/#sec-nav-prop>
//! - the NCX the `toc` is generated from, and the `guide` the `landmarks` are
//!   generated from:
//!   <https://idpf.org/epub/20/spec/OPF_2.0.1_draft.htm#Section2.4.1>,
//!   <https://idpf.org/epub/20/spec/OPF_2.0.1_draft.htm#Section2.6>, both
//!   retained as legacy features by
//!   <https://www.w3.org/TR/epub-33/#sec-opf2-guide>
//! - landmark `epub:type` values are EPUB 3 structural semantics vocabulary
//!   terms: <https://www.w3.org/TR/epub-ssv-11/>
//! - the `cover-image` manifest property:
//!   <https://www.w3.org/TR/epub-33/#sec-cover-image>
//! - EPUB 3 content documents are XHTML5, where OPS 2.0.1 required XHTML 1.1:
//!   <https://www.w3.org/TR/epub-33/#sec-xhtml-req>,
//!   <https://idpf.org/epub/20/spec/OPS_2.0.1_draft.htm#Section2.2>
//! - punctuation education follows SmartyPants' published rules, with its
//!   "old school" dashes (`--` en dash, `---` em dash):
//!   <https://daringfireball.net/projects/smartypants/>
//! - the quote glyphs per language are the CLDR `delimiters`
//!   (`quotationStart`/`quotationEnd`, `alternateQuotation*`) of `en`, `de`
//!   and `fr`:
//!   <https://www.unicode.org/reports/tr35/tr35-general.html#Delimiter_Elements>
//!
//! Only three kinds of member are ever rewritten: the package document, the
//! content documents this run was asked to touch, and the navigation document
//! it generates. Every other entry — images, fonts, style sheets, the NCX,
//! `mimetype` — is raw-copied as its original compressed bytes, in its
//! original position. See `crates/tools/README.md` for the option table.

use std::{
	collections::{BTreeMap, BTreeSet},
	fs::File,
	io::{BufReader, Read, Seek, Write},
	path::Path,
};

use chrono::{SecondsFormat, Utc};
use quick_xml::{
	escape::escape,
	events::{BytesEnd, BytesStart, BytesText, Event},
	Reader, Writer,
};
use serde::{Deserialize, Serialize};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

use crate::{
	epub_check::{
		attribute, collect_epubs, decode, finding, flush, is_whitespace, local_name,
		local_string, parse_container, parse_opf, read_entry_text, resolve_href,
		resolve_reference, set_attribute, zip_parent, ManifestItem, Opf, CONTAINER_ENTRY,
		DEFAULT_COVER_ID, NCX_MEDIA_TYPE,
	},
	util, Action, Plan, ProgressSink, Report, Severity, Tool, ToolError, ToolInput,
	ToolResult, Warning,
};

/// The tool id, also its `stump tools <id>` subcommand name.
pub const ID: &str = "epub-polish";
/// The only [`Action::kind`] this tool plans.
pub const ACTION_POLISH: &str = "polish-epub";

const XHTML_MEDIA_TYPE: &str = "application/xhtml+xml";
const XHTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
/// The EPUB 3 `epub:` (OPS) namespace, needed for `epub:type` in the nav.
const OPS_NAMESPACE: &str = "http://www.idpf.org/2007/ops";
const EPUB3_VERSION: &str = "3.0";
const MODIFIED_PROPERTY: &str = "dcterms:modified";
const NAV_PROPERTY: &str = "nav";
const NAV_FILE: &str = "nav.xhtml";
const COVER_IMAGE_PROPERTY: &str = "cover-image";
const XHTML5_DOCTYPE: &str = "<!DOCTYPE html>";
/// The two markers a calibre-inserted jacket page carries.
const JACKET_CLASS: &str = "calibre_jacket";
const JACKET_ID: &str = "jacket";
/// Elements whose text is authored, not prose: punctuation is never educated
/// inside them.
const VERBATIM_ELEMENTS: [&str; 4] = ["pre", "code", "script", "style"];

const ELLIPSIS: char = '\u{2026}';
const EN_DASH: char = '\u{2013}';
const EM_DASH: char = '\u{2014}';
/// U+2019, the apostrophe as well as the English closing single quote.
const APOSTROPHE: &str = "\u{2019}";

/// Stable finding codes: callers filter on these, never on prose.
pub mod codes {
	pub const UNREADABLE: &str = "unreadable";
	pub const INCOMPLETE_PACKAGE: &str = "incomplete-package";
	pub const ALREADY_EPUB3: &str = "already-epub3";
	pub const NCX_MISSING: &str = "ncx-missing";
	pub const NCX_UNPARSABLE: &str = "ncx-unparsable";
	pub const NCX_EMPTY: &str = "ncx-empty";
	pub const JACKET_MISSING: &str = "jacket-missing";
	pub const JACKET_IN_TOC: &str = "jacket-in-toc";
	pub const LANGUAGE_MISSING: &str = "language-missing";
	pub const LANGUAGE_UNSUPPORTED: &str = "language-unsupported";
	pub const UNPARSABLE_DOCUMENT: &str = "unparsable-document";
	pub const NOTHING_TO_DO: &str = "nothing-to-do";
}

/// Stable change codes, listed in every planned action's `detail.changes` so
/// the dry run says *what* it would change without printing the documents.
pub mod changes {
	pub const UPGRADE: &str = "upgrade-epub3";
	pub const NAV: &str = "generate-nav";
	pub const PROLOG: &str = "xhtml5-prolog";
	pub const COVER: &str = "cover-image-property";
	pub const SMARTEN: &str = "smarten-punctuation";
	pub const JACKET: &str = "remove-jacket";
}

/// `epub-polish` options. Every one is off by default, and a run with none of
/// them selected is refused rather than rewriting books for nothing (the same
/// rule as `calibre-polish`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EpubPolishOptions {
	/// Upgrade an EPUB 2 package to EPUB 3.3.
	pub upgrade: bool,
	/// Educate quotes, dashes and ellipses in the content documents' text.
	pub smarten_punctuation: bool,
	/// Drop a calibre-inserted jacket page and its package entries.
	pub remove_jacket: bool,
}

impl EpubPolishOptions {
	fn any(&self) -> bool {
		self.upgrade || self.smarten_punctuation || self.remove_jacket
	}
}

/// Upgrade EPUB 2 to EPUB 3.3, educate punctuation, remove a calibre jacket.
pub struct EpubPolish;

impl Tool for EpubPolish {
	fn id(&self) -> &'static str {
		ID
	}

	fn describe(&self) -> &'static str {
		"Upgrade EPUB 2 to EPUB 3.3, smarten punctuation, and remove a calibre jacket page"
	}

	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError> {
		let options = input.parse_options::<EpubPolishOptions>()?;
		if !options.any() {
			return Err(ToolError::Invalid(
				"no polish action selected: enable upgrade, smarten_punctuation or remove_jacket"
					.to_string(),
			));
		}
		let targets = collect_epubs(&input.paths)?;

		let mut plan = Plan::new(ID);
		for target in targets {
			// A bulk run over a library must not die on one bad book.
			let inspection = match inspect(&target, &options) {
				Ok(inspection) => inspection,
				Err(error) => {
					plan.warn(
						finding(
							codes::UNREADABLE,
							&target,
							None,
							format!("cannot be polished: {error}"),
						)
						.with_severity(Severity::Error),
					);
					continue;
				},
			};

			for warning in inspection.warnings {
				plan.warn(warning);
			}
			if let Some(detail) = inspection.detail {
				plan.push(
					Action::new(ACTION_POLISH)
						.with_source(&target)
						.with_target(&target)
						.with_detail(serde_json::to_value(&detail)?),
				);
			}
		}

		Ok(plan)
	}

	fn apply(
		&self,
		plan: &Plan,
		sink: &mut dyn ProgressSink,
	) -> Result<Report, ToolError> {
		plan.expect_tool(ID)?;

		let mut report = Report::for_plan(plan);
		let total = plan.actions.len();
		for (index, action) in plan.actions.iter().enumerate() {
			if action.kind != ACTION_POLISH {
				report.skipped(
					action.clone(),
					format!("unsupported action kind {:?}", action.kind),
				);
				continue;
			}
			let Some(target) = action.target.clone() else {
				report.skipped(action.clone(), "action has no target");
				continue;
			};
			if !target.is_file() {
				// A plan can be minutes old; never resurrect a moved book.
				report.skipped(
					action.clone(),
					format!("{} is no longer a file", target.display()),
				);
				continue;
			}
			sink.progress(index, total, &format!("polishing {}", target.display()));

			let detail =
				match serde_json::from_value::<PolishDetail>(action.detail.clone()) {
					Ok(detail) => detail,
					Err(error) => {
						report.skipped(
							action.clone(),
							format!("unreadable polish detail: {error}"),
						);
						continue;
					},
				};

			match polish(&target, &detail) {
				Ok(()) => report.applied(action.clone()),
				Err(error) => report.skipped(action.clone(), error.to_string()),
			}
		}
		sink.progress(total, total, "epub-polish complete");

		Ok(report)
	}
}

// ---------------------------------------------------------------------------
// Plan detail
// ---------------------------------------------------------------------------

/// Everything an apply needs. The generated navigation document is carried
/// here verbatim, so the dry run is the document that will be written; the
/// per-document text transforms are pure functions of the archived bytes and
/// the flags below, so an apply still invents nothing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PolishDetail {
	/// Change codes this action performs, for display.
	pub changes: Vec<String>,
	/// Zip path of the package document.
	pub opf_path: String,
	pub upgrade: Option<Upgrade>,
	pub smarten: Option<Smarten>,
	pub jacket: Option<Jacket>,
}

impl PolishDetail {
	fn touches_opf(&self) -> bool {
		self.upgrade.is_some() || self.jacket.is_some()
	}
}

/// The EPUB 2 to EPUB 3.3 package upgrade.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Upgrade {
	/// The `dcterms:modified` value, minted at plan time.
	pub modified: String,
	/// The navigation document to write and add to the manifest; `None` when
	/// the package already carries a `nav` document.
	pub nav: Option<NavDocument>,
	/// Manifest item id that gains the `cover-image` property.
	pub cover_image_id: Option<String>,
	/// Content documents whose XHTML 1.1 prolog becomes the XHTML5 one.
	pub xhtml5_prologs: Vec<String>,
}

/// The generated EPUB navigation document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavDocument {
	/// Zip path the document is written to.
	pub entry: String,
	/// Manifest `href`, relative to the package document's directory.
	pub href: String,
	/// Manifest item id, guaranteed not to collide.
	pub id: String,
	pub xml: String,
}

/// The punctuation pass.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Smarten {
	pub style: QuoteStyle,
	/// Content documents whose text actually changes.
	pub entries: Vec<String>,
}

/// The calibre jacket page to drop, with everything that points at it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Jacket {
	/// Zip path of the jacket document; the entry itself is dropped.
	pub entry: String,
	/// Manifest item id; its `<item>` and spine `<itemref>` are dropped.
	pub item_id: String,
	/// `guide/reference/@href` values pointing at the jacket, dropped with it.
	pub guide_hrefs: Vec<String>,
}

/// Which typographic quotes a language uses, from the CLDR `delimiters` of
/// `en`, `de` and `fr`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteStyle {
	/// `“…”` / `‘…’` — also the fallback for any other language.
	#[default]
	English,
	/// `„…“` / `‚…‘`
	German,
	/// `«…»`, alternates included.
	French,
}

impl QuoteStyle {
	/// The style of a BCP 47 tag's primary language subtag.
	fn for_language(tag: Option<&str>) -> Self {
		match primary_subtag(tag).as_deref() {
			Some("de") => Self::German,
			Some("fr") => Self::French,
			_ => Self::English,
		}
	}

	fn double(&self) -> (&'static str, &'static str) {
		match self {
			Self::English => ("\u{201c}", "\u{201d}"),
			Self::German => ("\u{201e}", "\u{201c}"),
			Self::French => ("\u{ab}", "\u{bb}"),
		}
	}

	fn single(&self) -> (&'static str, &'static str) {
		match self {
			Self::English => ("\u{2018}", APOSTROPHE),
			Self::German => ("\u{201a}", "\u{2018}"),
			// CLDR `fr` uses the same marks for nested quotations.
			Self::French => ("\u{ab}", "\u{bb}"),
		}
	}
}

fn primary_subtag(tag: Option<&str>) -> Option<String> {
	let tag = tag?.trim();
	let primary = tag.split(['-', '_']).next().unwrap_or(tag);
	(!primary.is_empty()).then(|| primary.to_ascii_lowercase())
}

// ---------------------------------------------------------------------------
// Inspection (plan)
// ---------------------------------------------------------------------------

/// What a plan pass found in one EPUB: warnings, and the work it would do.
struct Inspection {
	warnings: Vec<Warning>,
	detail: Option<PolishDetail>,
}

impl Inspection {
	fn warned(warnings: Vec<Warning>) -> Self {
		Self {
			warnings,
			detail: None,
		}
	}
}

/// One XHTML content document of the book, read once per plan pass.
struct Document {
	id: String,
	entry: String,
	text: String,
}

/// Read the EPUB at `path` and decide what a polish would do. Writes nothing.
fn inspect(path: &Path, options: &EpubPolishOptions) -> ToolResult<Inspection> {
	let mut archive = ZipArchive::new(BufReader::new(File::open(path)?))?;
	let mut names = BTreeSet::new();
	for index in 0..archive.len() {
		names.insert(archive.by_index(index)?.name().to_owned());
	}

	let mut warnings = Vec::new();

	let rootfile = read_entry_text(&mut archive, CONTAINER_ENTRY)?
		.and_then(|container| parse_container(&container).ok().flatten())
		.filter(|opf_path| names.contains(opf_path));
	let Some(opf_path) = rootfile else {
		warnings.push(unreadable(
			path,
			None,
			format!("no usable {CONTAINER_ENTRY} rootfile; run epub-check"),
		));
		return Ok(Inspection::warned(warnings));
	};
	let Some(opf_xml) = read_entry_text(&mut archive, &opf_path)? else {
		warnings.push(unreadable(
			path,
			Some(&opf_path),
			"package document disappeared".to_string(),
		));
		return Ok(Inspection::warned(warnings));
	};
	let (opf, guide) = match (parse_opf(&opf_xml), parse_guide(&opf_xml)) {
		(Ok(opf), Ok(guide)) => (opf, guide),
		(Err(error), _) | (_, Err(error)) => {
			warnings.push(unreadable(
				path,
				Some(&opf_path),
				format!("package document is not well-formed XML: {error}"),
			));
			return Ok(Inspection::warned(warnings));
		},
	};
	let opf_dir = zip_parent(&opf_path).to_string();

	let documents = read_documents(&mut archive, &names, &opf_dir, &opf)?;

	let jacket = match options.remove_jacket {
		false => None,
		true => match documents.iter().find(|document| is_jacket(&document.text)) {
			Some(document) => Some(Jacket {
				entry: document.entry.clone(),
				item_id: document.id.clone(),
				guide_hrefs: guide
					.iter()
					.filter(|reference| {
						resolve_href(&opf_dir, &reference.href).as_deref()
							== Some(document.entry.as_str())
					})
					.map(|reference| reference.href.clone())
					.collect(),
			}),
			None => {
				warnings.push(finding(
					codes::JACKET_MISSING,
					path,
					Some(&opf_path),
					format!(
							"no content document carries the {JACKET_CLASS:?} class or the {JACKET_ID:?} id"
						),
				));
				None
			},
		},
	};

	let upgrade = match options.upgrade {
		false => None,
		true => plan_upgrade(
			path,
			&mut archive,
			&Package {
				opf_path: &opf_path,
				opf_dir: &opf_dir,
				opf: &opf,
				guide: &guide,
				names: &names,
			},
			&documents,
			jacket.as_ref(),
			&mut warnings,
		)?,
	};

	let smarten = match options.smarten_punctuation {
		false => None,
		true => plan_smarten(path, &opf, &documents, jacket.as_ref(), &mut warnings),
	};

	let mut changes = Vec::new();
	if let Some(upgrade) = &upgrade {
		changes.push(changes::UPGRADE.to_string());
		if upgrade.nav.is_some() {
			changes.push(changes::NAV.to_string());
		}
		if !upgrade.xhtml5_prologs.is_empty() {
			changes.push(changes::PROLOG.to_string());
		}
		if upgrade.cover_image_id.is_some() {
			changes.push(changes::COVER.to_string());
		}
	}
	if smarten.is_some() {
		changes.push(changes::SMARTEN.to_string());
	}
	if jacket.is_some() {
		changes.push(changes::JACKET.to_string());
	}

	if changes.is_empty() {
		warnings.push(finding(
			codes::NOTHING_TO_DO,
			path,
			Some(&opf_path),
			"the selected polish would change nothing".to_string(),
		));
		return Ok(Inspection::warned(warnings));
	}

	Ok(Inspection {
		warnings,
		detail: Some(PolishDetail {
			changes,
			opf_path,
			upgrade,
			smarten,
			jacket,
		}),
	})
}

/// The parsed package document and the archive it lives in.
struct Package<'a> {
	opf_path: &'a str,
	opf_dir: &'a str,
	opf: &'a Opf,
	guide: &'a [GuideReference],
	names: &'a BTreeSet<String>,
}

impl Package<'_> {
	fn entry_of(&self, item: &ManifestItem) -> Option<String> {
		resolve_href(self.opf_dir, &item.href).filter(|entry| self.names.contains(entry))
	}
}

/// Every XHTML content document that is really in the archive, in manifest
/// order, read once so the jacket scan, the prolog check and the punctuation
/// pass share one decompression.
fn read_documents<R: Read + Seek>(
	archive: &mut ZipArchive<R>,
	names: &BTreeSet<String>,
	opf_dir: &str,
	opf: &Opf,
) -> ToolResult<Vec<Document>> {
	let mut documents: Vec<Document> = Vec::new();
	for item in &opf.items {
		if item.media_type.as_deref() != Some(XHTML_MEDIA_TYPE) {
			continue;
		}
		let Some(entry) = resolve_href(opf_dir, &item.href) else {
			continue;
		};
		if !names.contains(&entry)
			|| documents.iter().any(|document| document.entry == entry)
		{
			continue;
		}
		let Some(text) = read_entry_text(archive, &entry)? else {
			continue;
		};
		documents.push(Document {
			id: item.id.clone(),
			entry,
			text,
		});
	}
	Ok(documents)
}

/// Plan the package upgrade, or `None` when it cannot be done without leaving
/// an invalid EPUB 3 behind.
fn plan_upgrade<R: Read + Seek>(
	path: &Path,
	archive: &mut ZipArchive<R>,
	package: &Package<'_>,
	documents: &[Document],
	jacket: Option<&Jacket>,
	warnings: &mut Vec<Warning>,
) -> ToolResult<Option<Upgrade>> {
	let opf = package.opf;
	if opf.is_epub3() {
		warnings.push(finding(
			codes::ALREADY_EPUB3,
			path,
			Some(package.opf_path),
			format!(
				"package already declares version {:?}",
				opf.version.as_deref().unwrap_or_default()
			),
		));
		return Ok(None);
	}

	// An upgrade converges only on a package that is already a valid EPUB 2;
	// minting a title, an identifier or a language is `epub-check --fix`'s job.
	if opf.items.is_empty()
		|| opf.identifier_ids.is_empty()
		|| opf.titles.is_empty()
		|| opf.languages.is_empty()
	{
		warnings.push(
			finding(
				codes::INCOMPLETE_PACKAGE,
				path,
				Some(package.opf_path),
				"package lacks the manifest, identifier, title or language an EPUB 3 needs; run epub-check --fix first"
					.to_string(),
			)
			.with_severity(Severity::Error),
		);
		return Ok(None);
	}

	let nav = match navigation(path, archive, package, jacket, warnings)? {
		Navigation::Present => None,
		Navigation::Generated(nav) => Some(nav),
		Navigation::Unavailable => return Ok(None),
	};

	let xhtml5_prologs = documents
		.iter()
		.filter(|document| Some(document.entry.as_str()) != jacket.map(jacket_entry))
		.filter(|document| xhtml5_prolog(&document.text).is_some())
		.map(|document| document.entry.clone())
		.collect();

	Ok(Some(Upgrade {
		modified: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
		nav,
		cover_image_id: cover_image_id(package),
		xhtml5_prologs,
	}))
}

enum Navigation {
	/// The package already carries a `nav` document; only the version and the
	/// modification date change.
	Present,
	Generated(NavDocument),
	/// No nav can be produced, so the upgrade is refused rather than writing
	/// an EPUB 3 with no navigation document.
	Unavailable,
}

fn navigation<R: Read + Seek>(
	path: &Path,
	archive: &mut ZipArchive<R>,
	package: &Package<'_>,
	jacket: Option<&Jacket>,
	warnings: &mut Vec<Warning>,
) -> ToolResult<Navigation> {
	let opf = package.opf;
	if opf
		.items
		.iter()
		.any(|item| item.has_property(NAV_PROPERTY) && package.entry_of(item).is_some())
	{
		return Ok(Navigation::Present);
	}

	let ncx = opf
		.items
		.iter()
		.find(|item| {
			item.media_type.as_deref() == Some(NCX_MEDIA_TYPE)
				&& package.entry_of(item).is_some()
		})
		.or_else(|| {
			opf.spine_toc.as_deref().and_then(|toc| {
				opf.items
					.iter()
					.find(|item| item.id == toc && package.entry_of(item).is_some())
			})
		});
	let Some(ncx_entry) = ncx.and_then(|item| package.entry_of(item)) else {
		warnings.push(
			finding(
				codes::NCX_MISSING,
				path,
				Some(package.opf_path),
				"no NCX to generate a navigation document from; navigation is never invented"
					.to_string(),
			)
			.with_severity(Severity::Error),
		);
		return Ok(Navigation::Unavailable);
	};
	let Some(ncx_xml) = read_entry_text(archive, &ncx_entry)? else {
		warnings.push(unreadable(
			path,
			Some(&ncx_entry),
			"NCX disappeared".to_string(),
		));
		return Ok(Navigation::Unavailable);
	};
	let mut ncx = match parse_ncx(&ncx_xml) {
		Ok(ncx) => ncx,
		Err(error) => {
			warnings.push(
				finding(
					codes::NCX_UNPARSABLE,
					path,
					Some(&ncx_entry),
					format!("NCX is not well-formed XML: {error}"),
				)
				.with_severity(Severity::Error),
			);
			return Ok(Navigation::Unavailable);
		},
	};

	let ncx_dir = zip_parent(&ncx_entry).to_string();
	if let Some(jacket) = jacket {
		if prune_points(&mut ncx.points, &ncx_dir, &jacket.entry) {
			warnings.push(finding(
				codes::JACKET_IN_TOC,
				path,
				Some(&ncx_entry),
				"the NCX points at the removed jacket page; the generated navigation document omits it, the NCX keeps it"
					.to_string(),
			));
		}
	}

	if ncx.points.is_empty() {
		warnings.push(
			finding(
				codes::NCX_EMPTY,
				path,
				Some(&ncx_entry),
				"NCX has no usable navPoint".to_string(),
			)
			.with_severity(Severity::Error),
		);
		return Ok(Navigation::Unavailable);
	}

	let (entry, href) = free_nav_entry(package.opf_dir, package.names);
	// The nav lives in the package document's directory, so `guide` hrefs are
	// already relative to it and only NCX `src`s can need rewriting.
	let request = NavRequest {
		title: opf
			.titles
			.first()
			.map(String::as_str)
			.unwrap_or("Navigation"),
		language: opf.languages.first().map(String::as_str),
		toc: nav_entries(&ncx.points, &ncx_dir, package.opf_dir),
		pages: nav_entries(&ncx.pages, &ncx_dir, package.opf_dir),
		landmarks: landmarks(package, jacket),
	};

	Ok(Navigation::Generated(NavDocument {
		entry,
		href,
		id: free_nav_id(opf),
		xml: build_nav(&request),
	}))
}

fn plan_smarten(
	path: &Path,
	opf: &Opf,
	documents: &[Document],
	jacket: Option<&Jacket>,
	warnings: &mut Vec<Warning>,
) -> Option<Smarten> {
	let language = opf.languages.first().map(String::as_str);
	let style = QuoteStyle::for_language(language);
	match primary_subtag(language) {
		None => warnings.push(finding(
			codes::LANGUAGE_MISSING,
			path,
			None,
			"package declares no dc:language; using the English quote style".to_string(),
		)),
		Some(subtag) if !matches!(subtag.as_str(), "en" | "de" | "fr") => {
			warnings.push(finding(
				codes::LANGUAGE_UNSUPPORTED,
				path,
				None,
				format!("no quote style for language {subtag:?}; using the English one"),
			))
		},
		Some(_) => {},
	}

	let mut entries = Vec::new();
	for document in documents {
		if Some(document.entry.as_str()) == jacket.map(jacket_entry) {
			continue;
		}
		match smarten_document(&document.text, style) {
			Ok(smartened) if smartened != document.text => {
				entries.push(document.entry.clone())
			},
			Ok(_) => {},
			Err(error) => warnings.push(finding(
				codes::UNPARSABLE_DOCUMENT,
				path,
				Some(&document.entry),
				format!("punctuation left alone: {error}"),
			)),
		}
	}

	(!entries.is_empty()).then_some(Smarten { style, entries })
}

/// The manifest item that should gain `cover-image`, using the same heuristic
/// as `epub-check` (`<meta name="cover">`, else the `cover` id) so the two
/// tools cannot disagree about which resource is the cover.
fn cover_image_id(package: &Package<'_>) -> Option<String> {
	let opf = package.opf;
	if opf
		.items
		.iter()
		.any(|item| item.has_property(COVER_IMAGE_PROPERTY))
	{
		return None;
	}
	let by_id = |id: &str| {
		opf.items
			.iter()
			.find(|item| item.id == id && package.entry_of(item).is_some())
			.filter(|item| item.is_image())
	};
	opf.meta_cover
		.as_deref()
		.and_then(by_id)
		.or_else(|| by_id(DEFAULT_COVER_ID))
		.map(|item| item.id.clone())
}

fn jacket_entry(jacket: &Jacket) -> &str {
	jacket.entry.as_str()
}

fn unreadable(path: &Path, entry: Option<&str>, message: String) -> Warning {
	finding(codes::UNREADABLE, path, entry, message).with_severity(Severity::Error)
}

/// True when a document carries calibre's jacket markers. Lenient: a document
/// this cannot parse is simply not a jacket.
fn is_jacket(document: &str) -> bool {
	let mut reader = Reader::from_str(document);
	loop {
		let element = match reader.read_event() {
			Ok(Event::Start(element) | Event::Empty(element)) => element,
			Ok(Event::Eof) | Err(_) => return false,
			Ok(_) => continue,
		};
		let is_jacket = |key: &str, matches: fn(&str) -> bool| {
			attribute(&element, key)
				.ok()
				.flatten()
				.is_some_and(|value| matches(&value))
		};
		if is_jacket("id", |value| value.trim() == JACKET_ID)
			|| is_jacket("class", |value| {
				value.split_whitespace().any(|token| token == JACKET_CLASS)
			}) {
			return true;
		}
	}
}

// ---------------------------------------------------------------------------
// Navigation document
// ---------------------------------------------------------------------------

/// One `guide/reference`, in document order.
#[derive(Debug, Clone, PartialEq)]
struct GuideReference {
	kind: String,
	title: Option<String>,
	href: String,
}

#[derive(Debug, Default, Clone, PartialEq)]
struct Ncx {
	points: Vec<NcxPoint>,
	pages: Vec<NcxPoint>,
}

/// A `navPoint` or `pageTarget`: its label, its target, and its children.
#[derive(Debug, Default, Clone, PartialEq)]
struct NcxPoint {
	label: String,
	src: String,
	children: Vec<NcxPoint>,
}

/// The `guide` of a package document, which becomes the `landmarks` nav.
fn parse_guide(xml: &str) -> Result<Vec<GuideReference>, quick_xml::Error> {
	let mut reader = Reader::from_str(xml);
	let mut guide = Vec::new();
	let mut stack: Vec<Vec<u8>> = Vec::new();

	loop {
		let (element, empty) = match reader.read_event()? {
			Event::Start(element) => (element, false),
			Event::Empty(element) => (element, true),
			Event::End(_) => {
				stack.pop();
				continue;
			},
			Event::Eof => break,
			_ => continue,
		};

		let name = local_name(element.name().as_ref()).to_vec();
		if name == b"reference" && stack.last().map(Vec::as_slice) == Some(b"guide") {
			if let Some(href) =
				attribute(&element, "href")?.filter(|href| !href.trim().is_empty())
			{
				guide.push(GuideReference {
					kind: attribute(&element, "type")?.unwrap_or_default(),
					title: attribute(&element, "title")?
						.filter(|title| !title.trim().is_empty()),
					href,
				});
			}
		}
		if !empty {
			stack.push(name);
		}
	}

	Ok(guide)
}

/// The `navMap` and `pageList` of an NCX, in document order.
fn parse_ncx(xml: &str) -> Result<Ncx, quick_xml::Error> {
	let mut reader = Reader::from_str(xml);
	let mut ncx = Ncx::default();
	// The open `navPoint`s, innermost last.
	let mut open: Vec<NcxPoint> = Vec::new();
	let mut page: Option<NcxPoint> = None;
	let mut label: Option<String> = None;

	loop {
		match reader.read_event()? {
			Event::Start(element) => match local_name(element.name().as_ref()) {
				b"navPoint" => open.push(NcxPoint::default()),
				b"pageTarget" => page = Some(NcxPoint::default()),
				b"text" => label = Some(String::new()),
				b"content" => set_src(&element, &mut open, &mut page)?,
				_ => {},
			},
			Event::Empty(element) => {
				if local_name(element.name().as_ref()) == b"content" {
					set_src(&element, &mut open, &mut page)?;
				}
			},
			Event::Text(text) => {
				if let Some(buffer) = label.as_mut() {
					buffer.push_str(&decode(text.as_ref()));
				}
			},
			Event::CData(data) => {
				if let Some(buffer) = label.as_mut() {
					buffer.push_str(&String::from_utf8_lossy(data.as_ref()));
				}
			},
			// quick-xml reports `&amp;`/`&#8212;` as their own event, so a
			// label around one is three events, not one.
			Event::GeneralRef(reference) => {
				if let Some(buffer) = label.as_mut() {
					buffer.push_str(&resolve_reference(reference.as_ref()));
				}
			},
			Event::End(end) => match local_name(end.name().as_ref()) {
				b"text" => {
					// `docTitle`'s own text has no target and is dropped here.
					if let Some(text) = label.take() {
						if let Some(point) = page.as_mut().or_else(|| open.last_mut()) {
							if point.label.is_empty() {
								point.label = text.trim().to_string();
							}
						}
					}
				},
				b"navPoint" => {
					if let Some(point) = open.pop() {
						match open.last_mut() {
							Some(parent) => parent.children.push(point),
							None => ncx.points.push(point),
						}
					}
				},
				b"pageTarget" => {
					if let Some(point) = page.take() {
						ncx.pages.push(point);
					}
				},
				_ => {},
			},
			Event::Eof => break,
			_ => {},
		}
	}

	Ok(ncx)
}

fn set_src(
	element: &BytesStart<'_>,
	open: &mut [NcxPoint],
	page: &mut Option<NcxPoint>,
) -> Result<(), quick_xml::Error> {
	let Some(src) = attribute(element, "src")?.filter(|src| !src.trim().is_empty())
	else {
		return Ok(());
	};
	if let Some(point) = page.as_mut().or_else(|| open.last_mut()) {
		if point.src.is_empty() {
			point.src = src;
		}
	}
	Ok(())
}

/// Drop every point whose target is `dropped`; a point with children keeps its
/// label and loses only its link. Returns whether anything matched.
fn prune_points(points: &mut Vec<NcxPoint>, ncx_dir: &str, dropped: &str) -> bool {
	let mut found = false;
	points.retain_mut(|point| {
		if prune_points(&mut point.children, ncx_dir, dropped) {
			found = true;
		}
		if resolve_href(ncx_dir, &point.src).as_deref() != Some(dropped) {
			return true;
		}
		found = true;
		if point.children.is_empty() {
			return false;
		}
		point.src.clear();
		true
	});
	found
}

/// One entry of a generated `nav`: `href` is `None` for a heading-only entry,
/// which the content model allows as a `span`.
#[derive(Debug, Default, Clone, PartialEq)]
struct NavEntry {
	label: String,
	href: Option<String>,
	children: Vec<NavEntry>,
}

/// One `landmarks` entry: an SSV `epub:type`, its label, and its target.
#[derive(Debug, Clone, PartialEq)]
struct Landmark {
	epub_type: String,
	label: String,
	href: String,
}

struct NavRequest<'a> {
	title: &'a str,
	language: Option<&'a str>,
	toc: Vec<NavEntry>,
	pages: Vec<NavEntry>,
	landmarks: Vec<Landmark>,
}

fn nav_entries(points: &[NcxPoint], ncx_dir: &str, nav_dir: &str) -> Vec<NavEntry> {
	points
		.iter()
		.filter(|point| {
			!point.label.is_empty() || !point.src.is_empty() || !point.children.is_empty()
		})
		.map(|point| NavEntry {
			// A labelless point still has to render as something: its own
			// target is the only honest text available.
			label: match point.label.is_empty() {
				true => point.src.clone(),
				false => point.label.clone(),
			},
			href: rewrite_href(&point.src, ncx_dir, nav_dir),
			children: nav_entries(&point.children, ncx_dir, nav_dir),
		})
		.filter(|entry| !entry.label.is_empty() || !entry.children.is_empty())
		.collect()
}

/// The `guide` mapped onto `landmarks` entries, minus anything pointing at a
/// removed jacket page.
fn landmarks(package: &Package<'_>, jacket: Option<&Jacket>) -> Vec<Landmark> {
	package
		.guide
		.iter()
		.filter(|reference| match jacket {
			Some(jacket) => {
				resolve_href(package.opf_dir, &reference.href).as_deref()
					!= Some(jacket.entry.as_str())
			},
			None => true,
		})
		.map(|reference| {
			let epub_type = landmark_type(&reference.kind);
			Landmark {
				// A reference with no title has only its own semantic to show.
				label: reference.title.clone().unwrap_or_else(|| epub_type.clone()),
				epub_type,
				href: reference.href.clone(),
			}
		})
		.collect()
}

/// OPF 2 `guide/@type` to an EPUB 3 structural semantics vocabulary term.
/// Every other guide type is already an SSV term and passes through.
fn landmark_type(kind: &str) -> String {
	let kind = kind.trim().to_ascii_lowercase();
	match kind.as_str() {
		"text" => "bodymatter".to_string(),
		"title-page" | "titlepage" => "titlepage".to_string(),
		"acknowledgements" => "acknowledgments".to_string(),
		"notes" => "endnotes".to_string(),
		_ => kind,
	}
}

/// An NCX `src` as an href relative to the navigation document. Identical
/// directories keep the original string, so nothing is re-encoded.
fn rewrite_href(src: &str, from_dir: &str, to_dir: &str) -> Option<String> {
	let src = src.trim();
	if src.is_empty() {
		return None;
	}
	if from_dir == to_dir {
		return Some(src.to_string());
	}
	let (path, fragment) = match src.split_once('#') {
		Some((path, fragment)) => (path, Some(fragment)),
		None => (src, None),
	};
	let target = resolve_href(from_dir, path)?;
	let mut href = relative_href(to_dir, &target);
	if let Some(fragment) = fragment {
		href.push('#');
		href.push_str(fragment);
	}
	Some(href)
}

/// `target` (an archive path) relative to `from_dir`, percent-encoded per
/// segment because an href is an IRI, not a file name.
fn relative_href(from_dir: &str, target: &str) -> String {
	let from: Vec<&str> = from_dir
		.split('/')
		.filter(|part| !part.is_empty())
		.collect();
	let to: Vec<&str> = target.split('/').filter(|part| !part.is_empty()).collect();
	let shared = from
		.iter()
		.zip(&to)
		.take_while(|(here, there)| here == there)
		.count();

	let mut parts: Vec<String> = vec!["..".to_string(); from.len() - shared];
	parts.extend(
		to[shared..]
			.iter()
			.map(|part| urlencoding::encode(part).into_owned()),
	);
	parts.join("/")
}

/// A `nav.xhtml` name no archive entry uses.
fn free_nav_entry(opf_dir: &str, names: &BTreeSet<String>) -> (String, String) {
	let entry_of = |file: &str| match opf_dir.is_empty() {
		true => file.to_string(),
		false => format!("{opf_dir}/{file}"),
	};
	if !names.contains(&entry_of(NAV_FILE)) {
		return (entry_of(NAV_FILE), NAV_FILE.to_string());
	}
	(2..)
		.map(|suffix| format!("nav-{suffix}.xhtml"))
		.find(|file| !names.contains(&entry_of(file)))
		.map(|file| (entry_of(&file), file))
		.expect("an unbounded suffix search finds a free name")
}

/// A manifest id no element in this package uses.
fn free_nav_id(opf: &Opf) -> String {
	if !opf.all_ids.contains(NAV_PROPERTY) {
		return NAV_PROPERTY.to_string();
	}
	(2..)
		.map(|suffix| format!("nav-{suffix}"))
		.find(|candidate| !opf.all_ids.contains(candidate))
		.expect("an unbounded suffix search finds a free id")
}

/// Serialise the navigation document: an XHTML5 `toc` nav, plus `page-list`
/// and `landmarks` navs when the book has the data for them.
fn build_nav(request: &NavRequest<'_>) -> String {
	let mut xml = String::with_capacity(1024);
	xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
	xml.push_str(XHTML5_DOCTYPE);
	xml.push_str("\n<html xmlns=\"");
	xml.push_str(XHTML_NAMESPACE);
	xml.push_str("\" xmlns:epub=\"");
	xml.push_str(OPS_NAMESPACE);
	xml.push('"');
	if let Some(language) = request.language {
		let language = escape(language);
		xml.push_str(" xml:lang=\"");
		xml.push_str(&language);
		xml.push_str("\" lang=\"");
		xml.push_str(&language);
		xml.push('"');
	}
	xml.push_str(">\n\t<head>\n\t\t<meta charset=\"utf-8\"/>\n\t\t<title>");
	xml.push_str(&escape(request.title));
	xml.push_str("</title>\n\t</head>\n\t<body>\n");

	push_nav(&mut xml, "toc", Some("toc"), false, &request.toc);
	if !request.pages.is_empty() {
		push_nav(&mut xml, "page-list", None, true, &request.pages);
	}
	if !request.landmarks.is_empty() {
		push_landmarks(&mut xml, &request.landmarks);
	}

	xml.push_str("\t</body>\n</html>\n");
	xml
}

fn push_nav(
	xml: &mut String,
	epub_type: &str,
	id: Option<&str>,
	hidden: bool,
	entries: &[NavEntry],
) {
	xml.push_str("\t\t<nav epub:type=\"");
	xml.push_str(epub_type);
	xml.push('"');
	if let Some(id) = id {
		xml.push_str(" id=\"");
		xml.push_str(id);
		xml.push('"');
	}
	if hidden {
		// XML syntax has no bare boolean attributes.
		xml.push_str(" hidden=\"hidden\"");
	}
	xml.push_str(">\n");
	push_entries(xml, entries, 3);
	xml.push_str("\t\t</nav>\n");
}

fn push_entries(xml: &mut String, entries: &[NavEntry], depth: usize) {
	let indent = "\t".repeat(depth);
	xml.push_str(&indent);
	xml.push_str("<ol>\n");
	for entry in entries {
		xml.push_str(&indent);
		xml.push_str("\t<li>");
		push_link(xml, entry);
		if !entry.children.is_empty() {
			xml.push('\n');
			push_entries(xml, &entry.children, depth + 2);
			xml.push_str(&indent);
			xml.push('\t');
		}
		xml.push_str("</li>\n");
	}
	xml.push_str(&indent);
	xml.push_str("</ol>\n");
}

/// A `toc`/`page-list` entry: an `a` when it has a target, otherwise the
/// `span` the content model allows for a heading-only entry.
fn push_link(xml: &mut String, entry: &NavEntry) {
	match &entry.href {
		Some(href) => {
			xml.push_str("<a href=\"");
			xml.push_str(&escape(href));
			xml.push_str("\">");
			xml.push_str(&escape(&entry.label));
			xml.push_str("</a>");
		},
		None => {
			xml.push_str("<span>");
			xml.push_str(&escape(&entry.label));
			xml.push_str("</span>");
		},
	}
}

fn push_landmarks(xml: &mut String, landmarks: &[Landmark]) {
	xml.push_str("\t\t<nav epub:type=\"landmarks\" hidden=\"hidden\">\n\t\t\t<ol>\n");
	for landmark in landmarks {
		xml.push_str("\t\t\t\t<li><a epub:type=\"");
		xml.push_str(&escape(&landmark.epub_type));
		xml.push_str("\" href=\"");
		xml.push_str(&escape(&landmark.href));
		xml.push_str("\">");
		xml.push_str(&escape(&landmark.label));
		xml.push_str("</a></li>\n");
	}
	xml.push_str("\t\t\t</ol>\n\t\t</nav>\n");
}

// ---------------------------------------------------------------------------
// Apply
// ---------------------------------------------------------------------------

/// Which text transforms one content document needs.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Transform {
	prolog: bool,
	smarten: bool,
}

fn content_transforms(detail: &PolishDetail) -> BTreeMap<&str, Transform> {
	let mut transforms: BTreeMap<&str, Transform> = BTreeMap::new();
	if let Some(upgrade) = &detail.upgrade {
		for entry in &upgrade.xhtml5_prologs {
			transforms.entry(entry.as_str()).or_default().prolog = true;
		}
	}
	if let Some(smarten) = &detail.smarten {
		for entry in &smarten.entries {
			transforms.entry(entry.as_str()).or_default().smarten = true;
		}
	}
	transforms
}

/// Apply `detail` to the EPUB at `path`, in place, atomically.
fn polish(path: &Path, detail: &PolishDetail) -> ToolResult<()> {
	let style = detail
		.smarten
		.as_ref()
		.map(|smarten| smarten.style)
		.unwrap_or_default();

	let mut appended: Vec<(String, Vec<u8>)> = Vec::new();
	let mut replacements: BTreeMap<String, Vec<u8>> = BTreeMap::new();
	{
		let mut archive = ZipArchive::new(BufReader::new(File::open(path)?))?;
		if detail.touches_opf() {
			let xml = entry_text(&mut archive, path, &detail.opf_path)?;
			replacements.insert(detail.opf_path.clone(), rewrite_opf(&xml, detail)?);
		}
		for (entry, transform) in content_transforms(detail) {
			let mut document = entry_text(&mut archive, path, entry)?;
			if transform.prolog {
				if let Some(fixed) = xhtml5_prolog(&document) {
					document = fixed;
				}
			}
			if transform.smarten {
				document = smarten_document(&document, style)?;
			}
			replacements.insert(entry.to_string(), document.into_bytes());
		}
		if let Some(nav) = detail.upgrade.as_ref().and_then(|up| up.nav.as_ref()) {
			// The name was free when the plan was made; if some other writer
			// has taken it since, replace that entry instead of duplicating
			// the name inside the archive.
			let bytes = || nav.xml.clone().into_bytes();
			match archive.index_for_name(&nav.entry).is_some() {
				true => {
					replacements.insert(nav.entry.clone(), bytes());
				},
				false => appended.push((nav.entry.clone(), bytes())),
			}
		}
	}

	let dropped = detail.jacket.as_ref().map(jacket_entry);

	util::write_atomic(path, |sink| {
		let mut archive = ZipArchive::new(BufReader::new(File::open(path)?))?;
		let mut writer = ZipWriter::new(sink);

		for index in 0..archive.len() {
			// `_raw` so no entry is ever decompressed just to be copied back.
			let entry = archive.by_index_raw(index)?;
			let name = entry.name().to_owned();
			if Some(name.as_str()) == dropped {
				continue;
			}

			let mut options = SimpleFileOptions::default();
			if let Some(mode) = entry.unix_mode() {
				options = options.unix_permissions(mode);
			}
			if entry.is_dir() {
				writer.add_directory(name, options)?;
				continue;
			}

			match replacements.get(&name) {
				Some(bytes) => {
					writer.start_file(
						name,
						options.compression_method(CompressionMethod::Deflated),
					)?;
					writer.write_all(bytes)?;
				},
				// Raw copy: same compressed bytes, same method, same order, so
				// `mimetype` keeps its position and no resource can change.
				None => writer.raw_copy_file(entry)?,
			}
		}

		for (name, bytes) in &appended {
			writer.start_file(
				name.as_str(),
				SimpleFileOptions::default()
					.compression_method(CompressionMethod::Deflated),
			)?;
			writer.write_all(bytes)?;
		}

		writer.finish()?;
		Ok(())
	})
}

fn entry_text<R: Read + Seek>(
	archive: &mut ZipArchive<R>,
	path: &Path,
	entry: &str,
) -> ToolResult<String> {
	read_entry_text(archive, entry)?.ok_or_else(|| {
		ToolError::Invalid(format!("{}: {entry:?} disappeared", path.display()))
	})
}

/// Rewrite the package document, preserving every byte this tool does not
/// explicitly change (declaration, comments, attribute order, indentation, and
/// so `prefix`, `unique-identifier`, `spine/@toc` and the NCX item survive).
fn rewrite_opf(xml: &str, detail: &PolishDetail) -> ToolResult<Vec<u8>> {
	let drop_id = detail.jacket.as_ref().map(|jacket| jacket.item_id.as_str());
	let drop_hrefs: BTreeSet<&str> = detail
		.jacket
		.iter()
		.flat_map(|jacket| jacket.guide_hrefs.iter().map(String::as_str))
		.collect();

	let mut reader = Reader::from_str(xml);
	let mut writer = Writer::new(Vec::with_capacity(xml.len() + 512));

	let mut stack: Vec<String> = Vec::new();
	let mut pending_ws: Option<BytesText<'static>> = None;
	let mut metadata_indent: Option<BytesText<'static>> = None;
	let mut manifest_indent: Option<BytesText<'static>> = None;
	// The document's own qualified names, so an inserted element is
	// namespace-correct even in a prefixed package document.
	let mut meta_name: Option<String> = None;
	let mut item_name: Option<String> = None;
	let mut modified_written = false;
	let mut drop_text = false;
	let mut skip_depth: Option<usize> = None;

	loop {
		let event = reader.read_event()?;

		// Inside a dropped `<item>`/`<itemref>`/`<reference>` with children.
		if let Some(depth) = skip_depth {
			match event {
				Event::Start(_) => skip_depth = Some(depth + 1),
				Event::End(_) => skip_depth = (depth > 1).then_some(depth - 1),
				Event::Eof => break,
				_ => {},
			}
			continue;
		}

		match event {
			// The old `dcterms:modified` value, already replaced.
			Event::Text(_) | Event::CData(_) if drop_text => {},
			Event::End(end) if drop_text => {
				drop_text = false;
				writer.write_event(Event::End(end))?;
				stack.pop();
			},
			Event::Text(text) if is_whitespace(text.as_ref()) => {
				flush(&mut writer, &mut pending_ws)?;
				match stack.last().map(String::as_str) {
					Some("metadata") if metadata_indent.is_none() => {
						metadata_indent = Some(text.clone().into_owned());
					},
					Some("manifest") if manifest_indent.is_none() => {
						manifest_indent = Some(text.clone().into_owned());
					},
					_ => {},
				}
				pending_ws = Some(text.into_owned());
			},
			Event::End(end) if local_name(end.name().as_ref()) == b"metadata" => {
				if let Some(upgrade) = &detail.upgrade {
					if !modified_written {
						let indent = metadata_indent
							.clone()
							.unwrap_or_else(|| BytesText::from_escaped("\n\t\t"));
						writer.write_event(Event::Text(indent))?;
						write_modified(
							&mut writer,
							meta_name.as_deref().unwrap_or("meta"),
							&upgrade.modified,
						)?;
						modified_written = true;
					}
				}
				flush(&mut writer, &mut pending_ws)?;
				writer.write_event(Event::End(end))?;
				stack.pop();
			},
			Event::End(end) if local_name(end.name().as_ref()) == b"manifest" => {
				if let Some(nav) = detail
					.upgrade
					.as_ref()
					.and_then(|upgrade| upgrade.nav.as_ref())
				{
					let indent = manifest_indent
						.clone()
						.unwrap_or_else(|| BytesText::from_escaped("\n\t\t"));
					writer.write_event(Event::Text(indent))?;
					write_nav_item(
						&mut writer,
						item_name.as_deref().unwrap_or("item"),
						nav,
					)?;
				}
				flush(&mut writer, &mut pending_ws)?;
				writer.write_event(Event::End(end))?;
				stack.pop();
			},
			Event::Start(start) => {
				let name = local_string(start.name().as_ref());
				if is_dropped(&start, &name, &stack, drop_id, &drop_hrefs)? {
					// Swallow the element's own indentation with it.
					pending_ws = None;
					skip_depth = Some(1);
					continue;
				}
				remember_name(&start, &name, &stack, &mut meta_name, &mut item_name);

				let mut start = start;
				patch_start(&mut start, &name, &stack, detail)?;
				let modified = detail.upgrade.is_some()
					&& !modified_written
					&& is_modified_meta(&start, &name, &stack)?;
				flush(&mut writer, &mut pending_ws)?;
				writer.write_event(Event::Start(start))?;
				stack.push(name);

				if modified {
					// Keep the element, replace only its stale timestamp.
					let value = detail
						.upgrade
						.as_ref()
						.map(|upgrade| upgrade.modified.as_str())
						.unwrap_or_default();
					writer.write_event(Event::Text(BytesText::new(value)))?;
					modified_written = true;
					drop_text = true;
				}
			},
			Event::Empty(empty) => {
				let name = local_string(empty.name().as_ref());
				if is_dropped(&empty, &name, &stack, drop_id, &drop_hrefs)? {
					pending_ws = None;
					continue;
				}
				remember_name(&empty, &name, &stack, &mut meta_name, &mut item_name);

				let mut empty = empty;
				patch_start(&mut empty, &name, &stack, detail)?;
				flush(&mut writer, &mut pending_ws)?;
				if detail.upgrade.is_some()
					&& !modified_written
					&& is_modified_meta(&empty, &name, &stack)?
				{
					// An empty `dcterms:modified` cannot hold a date: give it
					// one by writing the element out in full.
					write_modified(
						&mut writer,
						&String::from_utf8_lossy(empty.name().as_ref()),
						detail
							.upgrade
							.as_ref()
							.map(|upgrade| upgrade.modified.as_str())
							.unwrap_or_default(),
					)?;
					modified_written = true;
					continue;
				}
				writer.write_event(Event::Empty(empty))?;
			},
			Event::End(end) => {
				flush(&mut writer, &mut pending_ws)?;
				writer.write_event(Event::End(end))?;
				stack.pop();
			},
			Event::Eof => {
				flush(&mut writer, &mut pending_ws)?;
				break;
			},
			other => {
				flush(&mut writer, &mut pending_ws)?;
				writer.write_event(other)?;
			},
		}
	}

	Ok(writer.into_inner())
}

fn remember_name(
	element: &BytesStart<'_>,
	name: &str,
	stack: &[String],
	meta_name: &mut Option<String>,
	item_name: &mut Option<String>,
) {
	let qualified = || String::from_utf8_lossy(element.name().as_ref()).into_owned();
	match (name, stack.last().map(String::as_str)) {
		("meta", Some("metadata")) if meta_name.is_none() => {
			*meta_name = Some(qualified());
		},
		("item", Some("manifest")) if item_name.is_none() => {
			*item_name = Some(qualified());
		},
		_ => {},
	}
}

/// `<item>`/`<itemref>`/`<reference>` that point at the jacket page.
fn is_dropped(
	element: &BytesStart<'_>,
	name: &str,
	stack: &[String],
	drop_id: Option<&str>,
	drop_hrefs: &BTreeSet<&str>,
) -> Result<bool, quick_xml::Error> {
	match (name, stack.last().map(String::as_str)) {
		("item", Some("manifest")) => {
			Ok(drop_id.is_some() && attribute(element, "id")?.as_deref() == drop_id)
		},
		("itemref", Some("spine")) => {
			Ok(drop_id.is_some() && attribute(element, "idref")?.as_deref() == drop_id)
		},
		("reference", Some("guide")) => Ok(attribute(element, "href")?
			.is_some_and(|href| drop_hrefs.contains(href.as_str()))),
		_ => Ok(false),
	}
}

fn is_modified_meta(
	element: &BytesStart<'_>,
	name: &str,
	stack: &[String],
) -> Result<bool, quick_xml::Error> {
	if (name, stack.last().map(String::as_str)) != ("meta", Some("metadata")) {
		return Ok(false);
	}
	Ok(attribute(element, "property")?
		.is_some_and(|property| property.trim() == MODIFIED_PROPERTY))
}

fn patch_start(
	element: &mut BytesStart<'_>,
	name: &str,
	stack: &[String],
	detail: &PolishDetail,
) -> Result<(), quick_xml::Error> {
	let Some(upgrade) = &detail.upgrade else {
		return Ok(());
	};
	match (name, stack.last().map(String::as_str)) {
		("package", None) => set_attribute(element, "version", EPUB3_VERSION)?,
		("item", Some("manifest")) => {
			let Some(cover) = &upgrade.cover_image_id else {
				return Ok(());
			};
			if attribute(element, "id")?.as_deref() != Some(cover.as_str()) {
				return Ok(());
			}
			let properties = match attribute(element, "properties")? {
				Some(existing) if !existing.trim().is_empty() => {
					format!("{} {COVER_IMAGE_PROPERTY}", existing.trim())
				},
				_ => COVER_IMAGE_PROPERTY.to_string(),
			};
			set_attribute(element, "properties", &properties)?;
		},
		_ => {},
	}
	Ok(())
}

fn write_modified(
	writer: &mut Writer<Vec<u8>>,
	name: &str,
	modified: &str,
) -> ToolResult<()> {
	let mut start = BytesStart::new(name.to_string());
	start.push_attribute(("property", MODIFIED_PROPERTY));
	writer.write_event(Event::Start(start))?;
	writer.write_event(Event::Text(BytesText::new(modified)))?;
	writer.write_event(Event::End(BytesEnd::new(name.to_string())))?;
	Ok(())
}

fn write_nav_item(
	writer: &mut Writer<Vec<u8>>,
	name: &str,
	nav: &NavDocument,
) -> ToolResult<()> {
	let mut item = BytesStart::new(name.to_string());
	item.push_attribute(("id", nav.id.as_str()));
	item.push_attribute(("href", nav.href.as_str()));
	item.push_attribute(("media-type", XHTML_MEDIA_TYPE));
	item.push_attribute(("properties", NAV_PROPERTY));
	writer.write_event(Event::Empty(item))?;
	Ok(())
}

// ---------------------------------------------------------------------------
// Content documents
// ---------------------------------------------------------------------------

/// `Some(document)` when the prolog declares the OPS 2.0.1 XHTML 1.1 doctype,
/// which is not a legal EPUB 3 content document: it becomes `<!DOCTYPE html>`
/// and the root element gains the XHTML namespace if it lacks one. Only the
/// prolog and that one start tag change; every other byte is preserved.
fn xhtml5_prolog(document: &str) -> Option<String> {
	let start = document.find("<!DOCTYPE")?;
	let end = doctype_end(document, start)?;
	let doctype = &document[start..end];
	if !(doctype.contains("XHTML 1.1") || doctype.contains("xhtml11.dtd")) {
		return None;
	}

	let mut fixed = String::with_capacity(document.len());
	fixed.push_str(&document[..start]);
	fixed.push_str(XHTML5_DOCTYPE);
	let rest = &document[end..];
	match root_namespace(rest) {
		Some(patched) => fixed.push_str(&patched),
		None => fixed.push_str(rest),
	}
	Some(fixed)
}

/// The index just past the `>` closing the doctype at `start`, skipping an
/// internal subset and quoted public/system identifiers.
fn doctype_end(document: &str, start: usize) -> Option<usize> {
	let mut subset = false;
	let mut quote: Option<char> = None;
	for (offset, character) in document[start..].char_indices() {
		match (quote, character) {
			(Some(open), character) if character == open => quote = None,
			(Some(_), _) => {},
			(None, '"' | '\'') => quote = Some(character),
			(None, '[') => subset = true,
			(None, ']') => subset = false,
			(None, '>') if !subset => return Some(start + offset + 1),
			(None, _) => {},
		}
	}
	None
}

/// `Some(rest)` with the XHTML namespace added to the `<html>` start tag when
/// it declares none.
fn root_namespace(rest: &str) -> Option<String> {
	let start = rest.find("<html")?;
	let end = rest[start..].find('>')? + start;
	if rest[start..end].contains("xmlns=") {
		return None;
	}
	let insert = start + "<html".len();
	let mut patched = String::with_capacity(rest.len() + 40);
	patched.push_str(&rest[..insert]);
	patched.push_str(" xmlns=\"");
	patched.push_str(XHTML_NAMESPACE);
	patched.push('"');
	patched.push_str(&rest[insert..]);
	Some(patched)
}

/// Educate the punctuation of a content document's text nodes.
///
/// Attributes, comments, CDATA, the prolog and everything inside `pre`,
/// `code`, `script` and `style` are written back as the bytes they were read
/// as; only text nodes are transformed, and only the characters this tool
/// substitutes differ, so entity references survive untouched.
fn smarten_document(document: &str, style: QuoteStyle) -> ToolResult<String> {
	let mut reader = Reader::from_str(document);
	let mut writer = Writer::new(Vec::with_capacity(document.len() + 64));
	let mut verbatim = 0usize;
	let mut previous: Option<char> = None;

	loop {
		match reader.read_event()? {
			Event::Start(start) => {
				if is_verbatim(start.name().as_ref()) {
					verbatim += 1;
				}
				writer.write_event(Event::Start(start))?;
			},
			Event::End(end) => {
				if is_verbatim(end.name().as_ref()) {
					verbatim = verbatim.saturating_sub(1);
				}
				writer.write_event(Event::End(end))?;
			},
			Event::Text(text) => {
				let smartened = match std::str::from_utf8(text.as_ref()) {
					Ok(raw) if verbatim == 0 => {
						Some(smarten_text(raw, style, &mut previous))
					},
					Ok(raw) => {
						previous = raw.chars().last().or(previous);
						None
					},
					Err(_) => None,
				};
				match smartened {
					Some(smartened) => writer
						.write_event(Event::Text(BytesText::from_escaped(smartened)))?,
					None => writer.write_event(Event::Text(text))?,
				}
			},
			// An entity reference stands for a character, and that character
			// is what decides whether the next quote opens or closes.
			Event::GeneralRef(reference) => {
				previous = resolve_reference(reference.as_ref())
					.chars()
					.next_back()
					.or(previous);
				writer.write_event(Event::GeneralRef(reference))?;
			},
			Event::Eof => break,
			other => writer.write_event(other)?,
		}
	}

	String::from_utf8(writer.into_inner()).map_err(|error| {
		ToolError::Invalid(format!("smartened document is not UTF-8: {error}"))
	})
}

fn is_verbatim(name: &[u8]) -> bool {
	let local = local_name(name);
	VERBATIM_ELEMENTS
		.iter()
		.any(|element| local.eq_ignore_ascii_case(element.as_bytes()))
}

/// One text node's punctuation, SmartyPants' rules with "old school" dashes.
///
/// `previous` is the last source character seen in this document, across
/// markup, which is what decides whether a quote opens or closes.
fn smarten_text(raw: &str, style: QuoteStyle, previous: &mut Option<char>) -> String {
	let (double_open, double_close) = style.double();
	let mut out = String::with_capacity(raw.len());
	let mut rest = raw;

	while let Some(character) = rest.chars().next() {
		let width = match character {
			'.' if rest.starts_with("...") => {
				out.push(ELLIPSIS);
				3
			},
			'-' if rest.starts_with("---") => {
				out.push(EM_DASH);
				3
			},
			'-' if rest.starts_with("--") => {
				out.push(EN_DASH);
				2
			},
			'"' => {
				out.push_str(match opens(*previous) {
					true => double_open,
					false => double_close,
				});
				1
			},
			'\'' => {
				out.push_str(single_quote(*previous, rest, style));
				1
			},
			character => {
				out.push(character);
				character.len_utf8()
			},
		};
		*previous = rest[..width].chars().next_back();
		rest = &rest[width..];
	}

	out
}

/// A quote opens when it starts the text or follows whitespace, an opening
/// bracket, a dash, or another opening quote.
fn opens(previous: Option<char>) -> bool {
	match previous {
		None => true,
		Some(character) => {
			character.is_whitespace()
				|| matches!(
					character,
					'(' | '['
						| '{' | '<' | '-' | EN_DASH
						| EM_DASH | '\u{201c}'
						| '\u{2018}' | '\u{201e}'
						| '\u{201a}' | '\u{ab}'
				)
		},
	}
}

/// U+2019 for an apostrophe — after a word character, or before an elided
/// decade such as `'90s` — and the language's single quotes otherwise.
fn single_quote(previous: Option<char>, rest: &str, style: QuoteStyle) -> &'static str {
	let next = rest[1..].chars().next();
	if previous.is_some_and(char::is_alphanumeric)
		|| next.is_some_and(|character| character.is_ascii_digit())
	{
		return APOSTROPHE;
	}
	let (open, close) = style.single();
	match opens(previous) {
		true => open,
		false => close,
	}
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	use super::*;
	use crate::{epub_check, NoopProgress};
	use pretty_assertions::assert_eq;
	use tempfile::TempDir;

	const MIMETYPE_ENTRY: &str = "mimetype";
	const EPUB_MEDIA_TYPE: &str = "application/epub+zip";
	const OPF_ENTRY: &str = "OEBPS/content.opf";

	const CONTAINER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
	<rootfiles>
		<rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
	</rootfiles>
</container>"#;

	/// An EPUB 2 NCX with a nested `navPoint`, an escaped label, a fragment
	/// target and a `pageList`.
	const NCX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
	<head><meta name="dtb:uid" content="urn:uuid:6b1c4e0a-1d1f-4a5b-9c3e-2a7f1d0c9b11"/></head>
	<docTitle><text>A Polished Book</text></docTitle>
	<navMap>
		<navPoint id="np1" playOrder="1">
			<navLabel><text>Chapter One</text></navLabel>
			<content src="ch1.xhtml"/>
			<navPoint id="np1-1" playOrder="2">
				<navLabel><text>A Section</text></navLabel>
				<content src="ch1.xhtml#s1"/>
			</navPoint>
		</navPoint>
		<navPoint id="np2" playOrder="3">
			<navLabel><text>Chapter Two &amp; Last</text></navLabel>
			<content src="ch2.xhtml"/>
		</navPoint>
	</navMap>
	<pageList>
		<pageTarget id="pt1" type="normal" value="1" playOrder="4">
			<navLabel><text>1</text></navLabel>
			<content src="ch1.xhtml#page1"/>
		</pageTarget>
	</pageList>
</ncx>"#;

	/// The OPS 2.0.1 XHTML 1.1 doctype, prose to educate, and three places
	/// punctuation must never be touched: an attribute, `pre` and `code`.
	const CHAPTER_ONE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.1//EN" "http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd">
<html xmlns="http://www.w3.org/1999/xhtml">
	<head><title>Chapter One</title></head>
	<body>
		<h1 id="s1">Chapter One</h1>
		<p title='he said "so" -- ...'>He said "hello" -- it isn't over... yet---really.</p>
		<pre>"raw" -- kept... exactly</pre>
		<p><code>a--b "c"...</code></p>
		<p>Ein &amp; Zeichen &#8212; bleibt.</p>
	</body>
</html>"#;

	const CHAPTER_TWO: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml">
	<head><title>Chapter Two</title></head>
	<body><p>Sie sagte "ja" -- und ging.</p></body>
</html>"#;

	/// A calibre jacket page: the class calibre writes, and nothing else.
	const JACKET: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
	<head><title>Book Jacket</title></head>
	<body><div class="calibre_jacket"><h1>A Polished Book</h1></div></body>
</html>"#;

	fn package(language: &str, jacket: bool) -> String {
		let (item, itemref, reference) = match jacket {
			true => (
				"\n\t\t<item id=\"jacket-page\" href=\"jacket.xhtml\" media-type=\"application/xhtml+xml\"/>",
				"\n\t\t<itemref idref=\"jacket-page\"/>",
				"\n\t\t<reference type=\"titlepage\" title=\"Jacket\" href=\"jacket.xhtml\"/>",
			),
			false => ("", "", ""),
		};
		format!(
			r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="bookid" prefix="foaf: http://xmlns.com/foaf/spec/">
	<metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
		<dc:identifier id="bookid">urn:uuid:6b1c4e0a-1d1f-4a5b-9c3e-2a7f1d0c9b11</dc:identifier>
		<dc:title>A Polished Book</dc:title>
		<dc:language>{language}</dc:language>
		<meta name="cover" content="cover-image"/>
	</metadata>
	<manifest>
		<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
		<item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
		<item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
		<item id="cover-image" href="cover.jpg" media-type="image/jpeg"/>
		<item id="style" href="style.css" media-type="text/css"/>{item}
	</manifest>
	<spine toc="ncx">{itemref}
		<itemref idref="ch1"/>
		<itemref idref="ch2"/>
	</spine>
	<guide>
		<reference type="cover" title="Cover" href="ch1.xhtml"/>
		<reference type="text" href="ch1.xhtml"/>{reference}
	</guide>
</package>"#
		)
	}

	fn write_epub(path: &Path, entries: &[(&str, &[u8], CompressionMethod)]) {
		let mut writer = ZipWriter::new(File::create(path).unwrap());
		for (name, data, method) in entries {
			writer
				.start_file(
					*name,
					SimpleFileOptions::default().compression_method(*method),
				)
				.unwrap();
			writer.write_all(data).unwrap();
		}
		writer.finish().unwrap();
	}

	/// A valid EPUB 2 book: `epub-check` reports nothing about it, so anything
	/// this tool leaves behind is this tool's own doing.
	fn epub2_book(dir: &Path, language: &str, jacket: bool) -> PathBuf {
		let path = dir.join("book.epub");
		let opf = package(language, jacket);
		let mut entries: Vec<(&str, &[u8], CompressionMethod)> = vec![
			(
				MIMETYPE_ENTRY,
				EPUB_MEDIA_TYPE.as_bytes(),
				CompressionMethod::Stored,
			),
			(
				epub_check::CONTAINER_ENTRY,
				CONTAINER.as_bytes(),
				CompressionMethod::Stored,
			),
			(OPF_ENTRY, opf.as_bytes(), CompressionMethod::Deflated),
			("OEBPS/toc.ncx", NCX.as_bytes(), CompressionMethod::Deflated),
			(
				"OEBPS/ch1.xhtml",
				CHAPTER_ONE.as_bytes(),
				CompressionMethod::Deflated,
			),
			(
				"OEBPS/ch2.xhtml",
				CHAPTER_TWO.as_bytes(),
				CompressionMethod::Deflated,
			),
			(
				"OEBPS/cover.jpg",
				b"\xff\xd8\xff\xe0 not-a-real-jpeg",
				CompressionMethod::Stored,
			),
			(
				"OEBPS/style.css",
				b"body { margin: 0; }",
				CompressionMethod::Deflated,
			),
		];
		if jacket {
			entries.push((
				"OEBPS/jacket.xhtml",
				JACKET.as_bytes(),
				CompressionMethod::Deflated,
			));
		}
		write_epub(&path, &entries);
		assert_eq!(
			epub_check::audit(&path).unwrap().codes(),
			Vec::<&str>::new(),
			"the fixture itself must be a clean EPUB 2"
		);
		path
	}

	fn entry_bytes(path: &Path, name: &str) -> Vec<u8> {
		let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
		let mut bytes = Vec::new();
		archive
			.by_name(name)
			.unwrap()
			.read_to_end(&mut bytes)
			.unwrap();
		bytes
	}

	fn text_of(path: &Path, name: &str) -> String {
		String::from_utf8(entry_bytes(path, name)).unwrap()
	}

	fn entry_names(path: &Path) -> Vec<String> {
		let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
		(0..archive.len())
			.map(|index| archive.by_index(index).unwrap().name().to_owned())
			.collect()
	}

	fn plan_of(path: &Path, options: serde_json::Value) -> Plan {
		EpubPolish
			.plan(&ToolInput::new(vec![path.to_path_buf()]).with_options(options))
			.unwrap()
	}

	fn plan_and_apply(path: &Path, options: serde_json::Value) -> (Plan, Report) {
		let plan = plan_of(path, options);
		let report = EpubPolish.apply(&plan, &mut NoopProgress).unwrap();
		(plan, report)
	}

	fn detail_of(plan: &Plan) -> PolishDetail {
		serde_json::from_value(plan.actions[0].detail.clone()).unwrap()
	}

	fn codes(plan: &Plan) -> Vec<&str> {
		plan.warnings
			.iter()
			.map(|warning| warning.code.as_str())
			.collect()
	}

	#[test]
	fn epub2_becomes_an_epub3_that_epub_check_finds_nothing_wrong_with() {
		let dir = TempDir::new().unwrap();
		let path = epub2_book(dir.path(), "en", false);
		let cover_before = entry_bytes(&path, "OEBPS/cover.jpg");
		let ncx_before = text_of(&path, "OEBPS/toc.ncx");

		let (plan, report) =
			plan_and_apply(&path, serde_json::json!({ "upgrade": true }));
		assert_eq!(report.applied.len(), 1);
		assert!(report.skipped.is_empty(), "{:?}", report.skipped);
		assert_eq!(
			detail_of(&plan).changes,
			vec![
				changes::UPGRADE,
				changes::NAV,
				changes::PROLOG,
				changes::COVER
			],
		);

		// The whole point: a repaired book is a *valid* book.
		assert_eq!(
			epub_check::audit(&path).unwrap().codes(),
			Vec::<&str>::new()
		);
		stump_media::EpubProcessor::open(path.to_str().unwrap())
			.expect("upgraded epub opens with stump_media");

		let opf = text_of(&path, OPF_ENTRY);
		assert!(opf.contains(r#"version="3.0""#), "{opf}");
		assert!(
			opf.contains(r#"unique-identifier="bookid""#)
				&& opf.contains(r#"prefix="foaf: http://xmlns.com/foaf/spec/""#),
			"unique-identifier and prefix survive: {opf}"
		);
		assert!(
			opf.contains(&format!(
				r#"<meta property="dcterms:modified">{}"#,
				detail_of(&plan).upgrade.unwrap().modified
			)),
			"{opf}"
		);
		assert!(
			opf.contains(
				r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#
			),
			"{opf}"
		);
		assert!(
			opf.contains(r#"<item id="cover-image" href="cover.jpg" media-type="image/jpeg" properties="cover-image"/>"#),
			"{opf}"
		);
		assert!(
			opf.contains(r#"<item id="ncx" href="toc.ncx""#)
				&& opf.contains(r#"<spine toc="ncx">"#)
				&& opf.contains(
					r#"<reference type="cover" title="Cover" href="ch1.xhtml"/>"#
				),
			"the EPUB 2 navigation and guide are kept for EPUB 2 readers: {opf}"
		);

		assert_eq!(
			text_of(&path, "OEBPS/toc.ncx"),
			ncx_before,
			"the NCX is copied through untouched"
		);
		assert_eq!(
			entry_bytes(&path, "OEBPS/cover.jpg"),
			cover_before,
			"resources are copied byte-for-byte"
		);
		assert_eq!(
			entry_names(&path).first().map(String::as_str),
			Some(MIMETYPE_ENTRY),
			"the OCF entry order is preserved"
		);

		let chapter = text_of(&path, "OEBPS/ch1.xhtml");
		assert!(
			chapter.starts_with(
				"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\">"
			),
			"only the prolog is rewritten: {chapter}"
		);
		assert!(
			chapter.contains(r#"He said "hello" -- it isn't over... yet---really."#),
			"an upgrade never educates punctuation: {chapter}"
		);
		assert_eq!(
			text_of(&path, "OEBPS/ch2.xhtml"),
			CHAPTER_TWO,
			"a document that already has the XHTML5 doctype is not rewritten"
		);
	}

	#[test]
	fn the_generated_nav_lists_every_ncx_entry() {
		let dir = TempDir::new().unwrap();
		let path = epub2_book(dir.path(), "en", false);

		plan_and_apply(&path, serde_json::json!({ "upgrade": true }));

		assert_eq!(
			text_of(&path, "OEBPS/nav.xhtml"),
			r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="en" lang="en">
	<head>
		<meta charset="utf-8"/>
		<title>A Polished Book</title>
	</head>
	<body>
		<nav epub:type="toc" id="toc">
			<ol>
				<li><a href="ch1.xhtml">Chapter One</a>
					<ol>
						<li><a href="ch1.xhtml#s1">A Section</a></li>
					</ol>
				</li>
				<li><a href="ch2.xhtml">Chapter Two &amp; Last</a></li>
			</ol>
		</nav>
		<nav epub:type="page-list" hidden="hidden">
			<ol>
				<li><a href="ch1.xhtml#page1">1</a></li>
			</ol>
		</nav>
		<nav epub:type="landmarks" hidden="hidden">
			<ol>
				<li><a epub:type="cover" href="ch1.xhtml">Cover</a></li>
				<li><a epub:type="bodymatter" href="ch1.xhtml">bodymatter</a></li>
			</ol>
		</nav>
	</body>
</html>
"#
		);
	}

	#[test]
	fn punctuation_is_educated_in_prose_and_nowhere_else() {
		let dir = TempDir::new().unwrap();
		let path = epub2_book(dir.path(), "en", false);
		let opf_before = entry_bytes(&path, OPF_ENTRY);

		let (plan, report) =
			plan_and_apply(&path, serde_json::json!({ "smarten_punctuation": true }));
		assert!(report.skipped.is_empty(), "{:?}", report.skipped);
		let detail = detail_of(&plan);
		assert_eq!(detail.smarten.as_ref().unwrap().style, QuoteStyle::English);
		assert_eq!(
			detail.smarten.unwrap().entries,
			vec!["OEBPS/ch1.xhtml".to_string(), "OEBPS/ch2.xhtml".to_string()]
		);

		let chapter = text_of(&path, "OEBPS/ch1.xhtml");
		assert!(
			chapter.contains(
				"He said \u{201c}hello\u{201d} \u{2013} it isn\u{2019}t over\u{2026} yet\u{2014}really."
			),
			"quotes, dashes and the ellipsis are educated: {chapter}"
		);
		assert!(
			chapter.contains(r#"<pre>"raw" -- kept... exactly</pre>"#),
			"`pre` is verbatim: {chapter}"
		);
		assert!(
			chapter.contains(r#"<code>a--b "c"...</code>"#),
			"`code` is verbatim: {chapter}"
		);
		assert!(
			chapter.contains(r#"<p title='he said "so" -- ...'>"#),
			"attributes are never touched: {chapter}"
		);
		assert!(
			chapter.contains("Ein &amp; Zeichen &#8212; bleibt."),
			"entity references survive: {chapter}"
		);
		assert!(
			chapter.contains(
				"<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.1//EN\" \"http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd\">"
			),
			"punctuation alone changes no prolog: {chapter}"
		);
		assert_eq!(
			entry_bytes(&path, OPF_ENTRY),
			opf_before,
			"a punctuation-only polish never rewrites the package document"
		);

		stump_media::EpubProcessor::open(path.to_str().unwrap())
			.expect("smartened epub opens with stump_media");
	}

	#[test]
	fn quote_style_follows_dc_language() {
		for (language, style, quoted) in [
			("en", QuoteStyle::English, "\u{201c}ja\u{201d}"),
			("de-DE", QuoteStyle::German, "\u{201e}ja\u{201c}"),
			("fr", QuoteStyle::French, "\u{ab}ja\u{bb}"),
		] {
			let dir = TempDir::new().unwrap();
			let path = epub2_book(dir.path(), language, false);

			let (plan, _) =
				plan_and_apply(&path, serde_json::json!({ "smarten_punctuation": true }));
			assert_eq!(detail_of(&plan).smarten.unwrap().style, style, "{language}");

			let chapter = text_of(&path, "OEBPS/ch2.xhtml");
			assert!(
				chapter.contains(&format!("Sie sagte {quoted} \u{2013} und ging.")),
				"{language}: {chapter}"
			);
		}
	}

	#[test]
	fn an_unsupported_language_is_reported_and_falls_back_to_english() {
		let dir = TempDir::new().unwrap();
		let path = epub2_book(dir.path(), "es", false);

		let plan = plan_of(&path, serde_json::json!({ "smarten_punctuation": true }));

		assert_eq!(codes(&plan), vec![codes::LANGUAGE_UNSUPPORTED]);
		assert_eq!(detail_of(&plan).smarten.unwrap().style, QuoteStyle::English);
	}

	#[test]
	fn a_jacket_page_and_every_reference_to_it_are_removed() {
		let dir = TempDir::new().unwrap();
		let path = epub2_book(dir.path(), "en", true);
		let chapter_before = entry_bytes(&path, "OEBPS/ch1.xhtml");

		let (plan, report) =
			plan_and_apply(&path, serde_json::json!({ "remove_jacket": true }));
		assert!(report.skipped.is_empty(), "{:?}", report.skipped);
		let jacket = detail_of(&plan).jacket.unwrap();
		assert_eq!(jacket.entry, "OEBPS/jacket.xhtml");
		assert_eq!(jacket.item_id, "jacket-page");
		assert_eq!(jacket.guide_hrefs, vec!["jacket.xhtml".to_string()]);

		assert!(
			!entry_names(&path).contains(&"OEBPS/jacket.xhtml".to_string()),
			"{:?}",
			entry_names(&path)
		);
		let opf = text_of(&path, OPF_ENTRY);
		assert!(
			!opf.contains("jacket"),
			"manifest, spine and guide lose the jacket together: {opf}"
		);
		assert!(
			opf.contains(r#"<itemref idref="ch1"/>"#)
				&& opf.contains(r#"<item id="ch1""#),
			"nothing else is dropped: {opf}"
		);
		assert_eq!(
			entry_bytes(&path, "OEBPS/ch1.xhtml"),
			chapter_before,
			"content documents are copied byte-for-byte"
		);

		assert_eq!(
			epub_check::audit(&path).unwrap().codes(),
			Vec::<&str>::new()
		);
		stump_media::EpubProcessor::open(path.to_str().unwrap())
			.expect("de-jacketed epub opens with stump_media");
	}

	#[test]
	fn a_book_with_no_jacket_is_reported_and_left_alone() {
		let dir = TempDir::new().unwrap();
		let path = epub2_book(dir.path(), "en", false);

		let plan = plan_of(&path, serde_json::json!({ "remove_jacket": true }));

		assert_eq!(
			codes(&plan),
			vec![codes::JACKET_MISSING, codes::NOTHING_TO_DO]
		);
		assert!(plan.actions.is_empty());
	}

	#[test]
	fn plan_writes_nothing() {
		let dir = TempDir::new().unwrap();
		let path = epub2_book(dir.path(), "en", true);
		let before = std::fs::read(&path).unwrap();

		let plan = plan_of(
			&path,
			serde_json::json!({
				"upgrade": true,
				"smarten_punctuation": true,
				"remove_jacket": true,
			}),
		);

		assert_eq!(plan.actions.len(), 1, "a dry run still plans the work");
		assert_eq!(std::fs::read(&path).unwrap(), before, "and writes nothing");
	}

	#[test]
	fn an_upgraded_book_is_reported_as_epub3_and_not_upgraded_again() {
		let dir = TempDir::new().unwrap();
		let path = epub2_book(dir.path(), "en", false);
		plan_and_apply(&path, serde_json::json!({ "upgrade": true }));
		let upgraded = std::fs::read(&path).unwrap();

		let plan = plan_of(&path, serde_json::json!({ "upgrade": true }));

		assert_eq!(
			codes(&plan),
			vec![codes::ALREADY_EPUB3, codes::NOTHING_TO_DO]
		);
		assert!(plan.actions.is_empty());
		assert_eq!(std::fs::read(&path).unwrap(), upgraded);
	}

	#[test]
	fn every_option_at_once_produces_one_valid_epub3() {
		let dir = TempDir::new().unwrap();
		let path = epub2_book(dir.path(), "de", true);

		let (plan, report) = plan_and_apply(
			&path,
			serde_json::json!({
				"upgrade": true,
				"smarten_punctuation": true,
				"remove_jacket": true,
			}),
		);
		assert!(report.skipped.is_empty(), "{:?}", report.skipped);
		assert_eq!(
			detail_of(&plan).changes,
			vec![
				changes::UPGRADE,
				changes::NAV,
				changes::PROLOG,
				changes::COVER,
				changes::SMARTEN,
				changes::JACKET,
			],
			"one action does all three jobs"
		);

		assert_eq!(
			epub_check::audit(&path).unwrap().codes(),
			Vec::<&str>::new()
		);
		stump_media::EpubProcessor::open(path.to_str().unwrap())
			.expect("polished epub opens with stump_media");

		let names = entry_names(&path);
		assert!(
			!names.contains(&"OEBPS/jacket.xhtml".to_string()),
			"{names:?}"
		);
		assert!(names.contains(&"OEBPS/nav.xhtml".to_string()), "{names:?}");

		let chapter = text_of(&path, "OEBPS/ch1.xhtml");
		assert!(
			chapter.starts_with(
				"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n"
			),
			"the prolog and the prose are fixed in one pass: {chapter}"
		);
		assert!(
			chapter.contains(
				"He said \u{201e}hello\u{201c} \u{2013} it isn\u{2019}t over\u{2026}"
			),
			"{chapter}"
		);
		assert!(chapter.contains(r#"<code>a--b "c"...</code>"#), "{chapter}");
	}

	#[test]
	fn an_epub2_without_an_ncx_is_never_upgraded() {
		let dir = TempDir::new().unwrap();
		let path = dir.path().join("no-ncx.epub");
		let opf = package("en", false).replace(
			r#"<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>"#,
			"",
		);
		write_epub(
			&path,
			&[
				(
					MIMETYPE_ENTRY,
					EPUB_MEDIA_TYPE.as_bytes(),
					CompressionMethod::Stored,
				),
				(
					epub_check::CONTAINER_ENTRY,
					CONTAINER.as_bytes(),
					CompressionMethod::Stored,
				),
				(OPF_ENTRY, opf.as_bytes(), CompressionMethod::Deflated),
				(
					"OEBPS/ch1.xhtml",
					CHAPTER_ONE.as_bytes(),
					CompressionMethod::Deflated,
				),
				(
					"OEBPS/ch2.xhtml",
					CHAPTER_TWO.as_bytes(),
					CompressionMethod::Deflated,
				),
				(
					"OEBPS/cover.jpg",
					b"\xff\xd8\xff\xe0",
					CompressionMethod::Stored,
				),
				(
					"OEBPS/style.css",
					b"body { margin: 0; }",
					CompressionMethod::Deflated,
				),
			],
		);
		let before = std::fs::read(&path).unwrap();

		let plan = plan_of(&path, serde_json::json!({ "upgrade": true }));

		assert_eq!(codes(&plan), vec![codes::NCX_MISSING, codes::NOTHING_TO_DO]);
		assert!(plan.actions.is_empty());
		assert_eq!(std::fs::read(&path).unwrap(), before);
	}

	#[test]
	fn a_polish_with_no_action_selected_is_refused() {
		let dir = TempDir::new().unwrap();
		let path = epub2_book(dir.path(), "en", false);

		let error = EpubPolish
			.plan(&ToolInput::new(vec![path.clone()]))
			.unwrap_err();

		assert!(matches!(error, ToolError::Invalid(_)), "{error}");
		assert_eq!(
			epub_check::audit(&path).unwrap().codes(),
			Vec::<&str>::new(),
			"a refused plan touches nothing"
		);
	}

	#[test]
	fn an_ncx_in_another_directory_is_relinked_to_the_nav() {
		// The nav lives beside the package document, so a `src` from a deeper
		// NCX has to climb back out or every nav link dangles.
		assert_eq!(
			rewrite_href("text/ch1.xhtml#s1", "OEBPS/nav", "OEBPS"),
			Some("nav/text/ch1.xhtml#s1".to_string())
		);
		assert_eq!(
			rewrite_href("../images/plate.xhtml", "OEBPS/nav", "OEBPS"),
			Some("images/plate.xhtml".to_string())
		);
		assert_eq!(
			rewrite_href("ch1.xhtml", "OEBPS", "OEBPS/deep"),
			Some("../ch1.xhtml".to_string())
		);
		assert_eq!(
			rewrite_href("a b.xhtml", "OEBPS", "text"),
			Some("../OEBPS/a%20b.xhtml".to_string()),
			"an href is an IRI, so a space is percent-encoded"
		);
	}

	#[test]
	fn registered_in_the_tool_registry() {
		let tool = crate::find(ID).expect("epub-polish is registered");
		assert_eq!(tool.id(), ID);
	}
}
