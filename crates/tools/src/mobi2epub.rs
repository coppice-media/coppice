//! `mobi2epub`: convert a Kindle/Mobipocket book (`.mobi`, `.prc`, `.azw`,
//! `.azw3`) into an EPUB 3, in process and with no external converter.
//!
//! The parsing is [`stump_media::MobiBook`]'s: a KF8 part is reassembled from
//! its flow table plus its skeleton and fragment indices, a MOBI 6 part is
//! split on `<mbp:pagebreak>`, resources come out of the record store, the
//! navigation out of the NCX index and the metadata out of EXTH. This module
//! only turns that model into an OCF container.
//!
//! The container follows the same rules [`crate::epub_check`] validates and
//! that its `--fix` writer produces:
//! - `mimetype` first, stored, exact
//!   ([EPUB 3.3 §4.2.6.2](https://www.w3.org/TR/epub-33/#sec-zip-container-mime)),
//! - one `META-INF/container.xml` rootfile
//!   ([§4.2.5.1](https://www.w3.org/TR/epub-33/#sec-container-metainf-container.xml)),
//! - a package document with a resolving `unique-identifier`, `dc:title`,
//!   `dc:language`, a non-empty spine, a `nav` document and a `cover-image`
//!   property ([§5](https://www.w3.org/TR/epub-33/#sec-package-doc)),
//! - plus an EPUB 2 NCX, so a reader that only knows `spine@toc` still gets a
//!   table of contents
//!   ([OPF 2.0.1 §2.4.1](https://idpf.org/epub/20/spec/OPF_2.0.1_draft.htm#Section2.4.1)).
//!
//! Nothing here removes DRM: [`stump_media::MobiBook::open`] refuses a
//! protected file with the DRM detector's own verdict, and Stump's ingest
//! rejects it earlier through the `drm_protected` quality check.

use std::{
	collections::HashSet,
	io::Write,
	path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use stump_media::{ContentType, MobiBook, MobiNavEntry, KINDLE_EXTENSIONS};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

use crate::{
	util, Action, Plan, ProgressSink, Report, Severity, Tool, ToolError, ToolInput,
	ToolResult, Warning,
};

/// The tool id, also its `stump tools <id>` subcommand name.
pub const ID: &str = "mobi2epub";
/// The only [`Action::kind`] this tool plans.
pub const ACTION_CONVERT: &str = "convert";

/// Where the package document lives. Every content document, image and style
/// sheet is a sibling of it, because that is what the rewritten links in
/// [`stump_media::MobiSection`] are relative to.
const ROOT_DIR: &str = "OEBPS";
const OPF_ENTRY: &str = "OEBPS/content.opf";
const NAV_ENTRY: &str = "nav.xhtml";
const NCX_ENTRY: &str = "toc.ncx";
const MIMETYPE_ENTRY: &str = "mimetype";
const EPUB_MEDIA_TYPE: &str = "application/epub+zip";

/// `mobi2epub` options.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct Mobi2EpubOptions {
	/// Where EPUBs are written. Default: next to each source book.
	pub output_dir: Option<PathBuf>,
	/// Descend into subdirectories when a given path is a directory.
	pub recursive: bool,
	/// Replace an existing target EPUB instead of skipping it.
	pub overwrite: bool,
	/// Stem suffix used when the output lands in the source's own directory.
	/// The default is empty because the extension already differs.
	pub suffix: String,
}

impl Default for Mobi2EpubOptions {
	fn default() -> Self {
		Self {
			output_dir: None,
			recursive: false,
			overwrite: false,
			suffix: String::new(),
		}
	}
}

/// Everything `apply` needs to redo a planned conversion without the options
/// blob: a plan is self-contained by contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ConvertDetail {
	/// `true` for a KF8 part, `false` for MOBI 6.
	kf8: bool,
	sections: usize,
	resources: usize,
	flows: usize,
	navigation: usize,
	title: String,
	overwrite: bool,
}

/// Convert Kindle books to EPUB 3 without calibre.
pub struct Mobi2Epub;

impl Tool for Mobi2Epub {
	fn id(&self) -> &'static str {
		ID
	}

	fn describe(&self) -> &'static str {
		"Convert MOBI/AZW/AZW3 books to EPUB 3 in process, with no external converter"
	}

	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError> {
		let options = input.parse_options::<Mobi2EpubOptions>()?;
		if input.paths.is_empty() {
			return Err(ToolError::Invalid("no input paths given".to_string()));
		}

		let mut plan = Plan::new(ID);
		let sources = collect_books(&input.paths, options.recursive)?;
		if sources.is_empty() {
			plan.warn(
				Warning::new(
					"no-books",
					format!(
						"no {} files found in the given paths",
						KINDLE_EXTENSIONS.join("/.")
					),
				)
				.with_severity(Severity::Error),
			);
			return Ok(plan);
		}

		let mut claimed: HashSet<PathBuf> = HashSet::new();
		for source in sources {
			let book = match MobiBook::open(&source) {
				Ok(book) => book,
				Err(error) => {
					// A protected book is a per-file finding, not a reason to
					// abort a whole library run.
					plan.warn(
						Warning::new("unreadable", format!("cannot read book: {error}"))
							.at(&source)
							.with_severity(Severity::Error),
					);
					continue;
				},
			};
			if book.sections().is_empty() {
				plan.warn(
					Warning::new("no-sections", "the book has no readable text")
						.at(&source)
						.with_severity(Severity::Error),
				);
				continue;
			}

			let target = util::derived_target(
				&source,
				options.output_dir.as_deref(),
				&options.suffix,
				"epub",
			)?;
			if target == source {
				plan.warn(
					Warning::new(
						"target-is-source",
						format!("{} would overwrite its own source", target.display()),
					)
					.at(&source)
					.with_severity(Severity::Error),
				);
				continue;
			}
			if !claimed.insert(target.clone()) {
				plan.warn(
					Warning::new(
						"target-collision",
						format!("another source already writes {}", target.display()),
					)
					.at(&source)
					.with_severity(Severity::Error),
				);
				continue;
			}
			if target.exists() && !options.overwrite {
				plan.warn(
					Warning::new(
						"target-exists",
						format!(
							"{} exists; pass overwrite to replace it",
							target.display()
						),
					)
					.at(&source),
				);
				continue;
			}

			let detail = ConvertDetail {
				kf8: book.is_kf8(),
				sections: book.sections().len(),
				resources: book.resources().len(),
				flows: book.flows().len(),
				navigation: book.navigation().len(),
				title: book.title(),
				overwrite: options.overwrite,
			};
			plan.push(
				Action::new(ACTION_CONVERT)
					.with_source(&source)
					.with_target(&target)
					.with_detail(serde_json::to_value(&detail)?),
			);
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
			if action.kind != ACTION_CONVERT {
				report.skipped(
					action.clone(),
					format!("unsupported action kind {:?}", action.kind),
				);
				continue;
			}
			let (Some(source), Some(target)) = (&action.source, &action.target) else {
				report.skipped(action.clone(), "action has no source or target");
				continue;
			};
			let detail =
				match serde_json::from_value::<ConvertDetail>(action.detail.clone()) {
					Ok(detail) => detail,
					Err(error) => {
						report.skipped(
							action.clone(),
							format!("malformed action: {error}"),
						);
						continue;
					},
				};
			if !source.is_file() {
				// A plan can be minutes old; never resurrect a moved book.
				report.skipped(
					action.clone(),
					format!("{} is no longer a file", source.display()),
				);
				continue;
			}
			if target.exists() && !detail.overwrite {
				report.skipped(action.clone(), "target already exists");
				continue;
			}

			sink.progress(index, total, &format!("converting {}", source.display()));
			match convert(source, target) {
				Ok(sections) => {
					if sections != detail.sections {
						report.warn(
							Warning::new(
								"section-count-drift",
								format!(
									"planned {} sections, wrote {sections}; the \
									 book changed since planning",
									detail.sections
								),
							)
							.at(source),
						);
					}
					report.applied(action.clone());
				},
				Err(error) => report.skipped(action.clone(), error.to_string()),
			}
		}
		sink.progress(total, total, "mobi2epub complete");

		Ok(report)
	}
}

/// Expand the caller's paths into the books to convert: a file is taken as-is,
/// a directory is walked for the Kindle extensions.
fn collect_books(paths: &[PathBuf], recursive: bool) -> ToolResult<Vec<PathBuf>> {
	let mut books = Vec::new();
	for path in paths {
		if path.is_file() {
			books.push(path.clone());
			continue;
		}
		if path.is_dir() {
			books.extend(
				util::sorted_files(path, recursive)?
					.into_iter()
					.filter(|candidate| is_kindle_book(candidate)),
			);
		}
	}
	books.dedup();
	Ok(books)
}

fn is_kindle_book(path: &Path) -> bool {
	path.extension()
		.and_then(|extension| extension.to_str())
		.is_some_and(|extension| {
			KINDLE_EXTENSIONS
				.iter()
				.any(|known| known.eq_ignore_ascii_case(extension))
		})
}

// ---------------------------------------------------------------------------
// Conversion
// ---------------------------------------------------------------------------

/// Write the EPUB for `source` at `target`, atomically. Returns the number of
/// content documents written.
fn convert(source: &Path, target: &Path) -> ToolResult<usize> {
	let mut book = MobiBook::open(source)?;
	let package = Package::of(&book);

	util::write_atomic(target, |sink| {
		let mut writer = ZipWriter::new(sink);

		// The OCF rule is about this entry's position *and* its compression:
		// first, stored, no extra field.
		writer.start_file(
			MIMETYPE_ENTRY,
			SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
		)?;
		writer.write_all(EPUB_MEDIA_TYPE.as_bytes())?;

		let deflated =
			SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
		let stored =
			SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

		write_entry(
			&mut writer,
			"META-INF/container.xml",
			deflated,
			container_xml().as_bytes(),
		)?;
		write_entry(&mut writer, OPF_ENTRY, deflated, package.opf().as_bytes())?;
		write_entry(
			&mut writer,
			&format!("{ROOT_DIR}/{NAV_ENTRY}"),
			deflated,
			package.nav().as_bytes(),
		)?;
		write_entry(
			&mut writer,
			&format!("{ROOT_DIR}/{NCX_ENTRY}"),
			deflated,
			package.ncx().as_bytes(),
		)?;

		for section in book.sections() {
			write_entry(
				&mut writer,
				&format!("{ROOT_DIR}/{}", section.name),
				deflated,
				&section.html,
			)?;
		}
		for flow in book.flows() {
			write_entry(
				&mut writer,
				&format!("{ROOT_DIR}/{}", flow.name),
				deflated,
				&flow.bytes,
			)?;
		}
		// Images are already compressed; storing them keeps the container the
		// same size and the write cheap.
		for index in 0..book.resources().len() {
			let name = book.resources()[index].name.clone();
			let bytes = book.resource_bytes(index)?;
			write_entry(&mut writer, &format!("{ROOT_DIR}/{name}"), stored, &bytes)?;
		}

		writer.finish()?;
		Ok(package.sections.len())
	})
}

fn write_entry<W: Write + std::io::Seek>(
	writer: &mut ZipWriter<W>,
	name: &str,
	options: SimpleFileOptions,
	bytes: &[u8],
) -> ToolResult<()> {
	writer.start_file(name, options)?;
	writer.write_all(bytes)?;
	Ok(())
}

fn container_xml() -> String {
	format!(
		"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<container version=\"1.0\" \
		 xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n  \
		 <rootfiles>\n    <rootfile full-path=\"{OPF_ENTRY}\" \
		 media-type=\"application/oebps-package+xml\"/>\n  </rootfiles>\n</container>\n"
	)
}

/// One manifest item.
#[derive(Debug, Clone)]
struct Item {
	id: String,
	href: String,
	media_type: String,
	properties: Option<&'static str>,
}

/// The package document, navigation and NCX of one converted book, derived
/// once so the three documents cannot disagree.
#[derive(Debug, Clone)]
struct Package {
	identifier: String,
	title: String,
	language: String,
	authors: Vec<String>,
	/// Manifest ids of the content documents, in spine order.
	sections: Vec<Item>,
	resources: Vec<Item>,
	flows: Vec<Item>,
	cover_id: Option<String>,
	navigation: Vec<MobiNavEntry>,
	section_names: Vec<String>,
}

impl Package {
	fn of(book: &MobiBook) -> Self {
		let sections = book
			.sections()
			.iter()
			.enumerate()
			.map(|(index, section)| Item {
				id: format!("s{index:04}"),
				href: section.name.clone(),
				media_type: "application/xhtml+xml".to_string(),
				properties: None,
			})
			.collect::<Vec<_>>();

		let mut cover_id = None;
		let resources = book
			.resources()
			.iter()
			.enumerate()
			.map(|(index, resource)| {
				let id = format!("r{index:04}");
				// The first image is the cover: `MobiBook` orders resources by
				// the `kindle:embed` numbering, and EXTH 201 points at the
				// first image record in every fixture and in every kindlegen
				// build.
				let properties = if cover_id.is_none() && resource.content_type.is_image()
				{
					cover_id = Some(id.clone());
					Some("cover-image")
				} else {
					None
				};
				Item {
					id,
					href: resource.name.clone(),
					media_type: media_type_of(resource.content_type),
					properties,
				}
			})
			.collect::<Vec<_>>();

		let flows = book
			.flows()
			.iter()
			.enumerate()
			.map(|(index, flow)| Item {
				id: format!("f{index:04}"),
				href: flow.name.clone(),
				media_type: if flow.name.ends_with(".css") {
					"text/css".to_string()
				} else {
					media_type_of(flow.content_type)
				},
				properties: None,
			})
			.collect::<Vec<_>>();

		Self {
			// A converted book has no EPUB identifier of its own, so one is
			// minted; `urn:uuid` is what `epub-check --fix` inserts too.
			identifier: format!("urn:uuid:{}", uuid::Uuid::new_v4()),
			title: non_empty(book.title()).unwrap_or_else(|| "Untitled".to_string()),
			// ISO 639-2 "undetermined" is the only honest value for a book
			// whose EXTH carries no language.
			language: book
				.language()
				.and_then(non_empty)
				.unwrap_or_else(|| "und".to_string()),
			authors: book.authors(),
			sections,
			resources,
			flows,
			cover_id,
			navigation: book.navigation().to_vec(),
			section_names: book
				.sections()
				.iter()
				.map(|section| {
					section
						.title
						.clone()
						.and_then(non_empty)
						.unwrap_or_else(|| section.name.clone())
				})
				.collect(),
		}
	}

	fn items(&self) -> impl Iterator<Item = &Item> {
		self.sections
			.iter()
			.chain(self.flows.iter())
			.chain(self.resources.iter())
	}

	fn opf(&self) -> String {
		let mut xml = String::with_capacity(1024);
		xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
		xml.push_str(
			"<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" \
			 unique-identifier=\"bookid\">\n",
		);
		xml.push_str("  <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n");
		xml.push_str(&format!(
			"    <dc:identifier id=\"bookid\">{}</dc:identifier>\n",
			escape(&self.identifier)
		));
		xml.push_str(&format!(
			"    <dc:title>{}</dc:title>\n",
			escape(&self.title)
		));
		xml.push_str(&format!(
			"    <dc:language>{}</dc:language>\n",
			escape(&self.language)
		));
		for author in &self.authors {
			xml.push_str(&format!(
				"    <dc:creator>{}</dc:creator>\n",
				escape(author)
			));
		}
		// EPUB 3 requires a last-modified date in the package metadata.
		xml.push_str(&format!(
			"    <meta property=\"dcterms:modified\">{}</meta>\n",
			chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
		));
		if let Some(cover) = &self.cover_id {
			// The EPUB 2 convention as well as the EPUB 3 property, so a
			// reader that only knows one of them still finds the cover.
			xml.push_str(&format!(
				"    <meta name=\"cover\" content=\"{}\"/>\n",
				escape(cover)
			));
		}
		xml.push_str("  </metadata>\n  <manifest>\n");
		xml.push_str(&format!(
			"    <item id=\"nav\" href=\"{NAV_ENTRY}\" \
			 media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n"
		));
		xml.push_str(&format!(
			"    <item id=\"ncx\" href=\"{NCX_ENTRY}\" \
			 media-type=\"application/x-dtbncx+xml\"/>\n"
		));
		for item in self.items() {
			let properties = item
				.properties
				.map(|properties| format!(" properties=\"{properties}\""))
				.unwrap_or_default();
			xml.push_str(&format!(
				"    <item id=\"{}\" href=\"{}\" media-type=\"{}\"{properties}/>\n",
				escape(&item.id),
				escape(&item.href),
				escape(&item.media_type)
			));
		}
		xml.push_str("  </manifest>\n  <spine toc=\"ncx\">\n");
		for section in &self.sections {
			xml.push_str(&format!(
				"    <itemref idref=\"{}\"/>\n",
				escape(&section.id)
			));
		}
		xml.push_str("  </spine>\n</package>\n");
		xml
	}

	/// The EPUB 3 navigation document. It uses the NCX entries when the book
	/// has them, and the section list otherwise, so the `toc` nav is never
	/// empty.
	fn nav(&self) -> String {
		let mut xml = String::with_capacity(512);
		xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
		xml.push_str(
			"<html xmlns=\"http://www.w3.org/1999/xhtml\" \
			 xmlns:epub=\"http://www.idpf.org/2007/ops\">\n<head><title>",
		);
		xml.push_str(&escape(&self.title));
		xml.push_str("</title></head>\n<body>\n<nav epub:type=\"toc\">\n<h1>");
		xml.push_str(&escape(&self.title));
		xml.push_str("</h1>\n");
		self.push_nav_list(&mut xml, &self.entries());
		xml.push_str("</nav>\n</body>\n</html>\n");
		xml
	}

	fn push_nav_list(&self, xml: &mut String, entries: &[MobiNavEntry]) {
		xml.push_str("<ol>\n");
		for entry in entries {
			xml.push_str(&format!(
				"<li><a href=\"{}\">{}</a>",
				escape(&self.href_of(entry)),
				escape(&entry.title)
			));
			if !entry.children.is_empty() {
				xml.push('\n');
				self.push_nav_list(xml, &entry.children);
			}
			xml.push_str("</li>\n");
		}
		xml.push_str("</ol>\n");
	}

	/// The EPUB 2 NCX, flattened: `navPoint` nesting is legal but a flat list
	/// is what every reader that still needs an NCX handles identically.
	fn ncx(&self) -> String {
		let mut xml = String::with_capacity(512);
		xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
		xml.push_str(
			"<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" \
			 version=\"2005-1\">\n<head>\n",
		);
		xml.push_str(&format!(
			"<meta name=\"dtb:uid\" content=\"{}\"/>\n",
			escape(&self.identifier)
		));
		xml.push_str("</head>\n<docTitle><text>");
		xml.push_str(&escape(&self.title));
		xml.push_str("</text></docTitle>\n<navMap>\n");
		for (order, entry) in flatten(&self.entries()).into_iter().enumerate() {
			xml.push_str(&format!(
				"<navPoint id=\"np{order:04}\" playOrder=\"{}\"><navLabel><text>{}\
				 </text></navLabel><content src=\"{}\"/></navPoint>\n",
				order + 1,
				escape(&entry.title),
				escape(&self.href_of(&entry))
			));
		}
		xml.push_str("</navMap>\n</ncx>\n");
		xml
	}

	/// The navigation to publish: the book's own NCX entries, or one entry per
	/// section when it has none.
	fn entries(&self) -> Vec<MobiNavEntry> {
		if !self.navigation.is_empty() {
			return self.navigation.clone();
		}
		self.section_names
			.iter()
			.enumerate()
			.map(|(section, title)| MobiNavEntry {
				title: title.clone(),
				section,
				fragment: String::new(),
				children: Vec::new(),
			})
			.collect()
	}

	/// An entry's `href`, clamped to a section that exists: a book whose NCX
	/// points past its own text must not produce a dangling link.
	fn href_of(&self, entry: &MobiNavEntry) -> String {
		let section = entry.section.min(self.sections.len().saturating_sub(1));
		let href = self
			.sections
			.get(section)
			.map(|item| item.href.clone())
			.unwrap_or_default();
		if entry.fragment.is_empty() {
			href
		} else {
			format!("{href}#{}", entry.fragment)
		}
	}
}

fn flatten(entries: &[MobiNavEntry]) -> Vec<MobiNavEntry> {
	let mut flat = Vec::new();
	for entry in entries {
		flat.push(MobiNavEntry {
			children: Vec::new(),
			..entry.clone()
		});
		flat.extend(flatten(&entry.children));
	}
	flat
}

fn non_empty(value: String) -> Option<String> {
	let trimmed = value.trim();
	(!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn media_type_of(content_type: ContentType) -> String {
	match content_type {
		ContentType::UNKNOWN => "application/octet-stream".to_string(),
		other => other.mime_type(),
	}
}

/// Escape the five XML predefined entities. Metadata comes out of a binary
/// header, so it can hold anything.
fn escape(value: &str) -> String {
	let mut out = String::with_capacity(value.len());
	for character in value.chars() {
		match character {
			'&' => out.push_str("&amp;"),
			'<' => out.push_str("&lt;"),
			'>' => out.push_str("&gt;"),
			'"' => out.push_str("&quot;"),
			'\'' => out.push_str("&apos;"),
			other => out.push(other),
		}
	}
	out
}

#[cfg(test)]
mod tests;
