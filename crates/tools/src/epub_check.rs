//! `epub-check`: validate an EPUB against the container/package rules a GUI
//! editor such as Sigil enforces, and repair the mechanical subset of them.
//!
//! Behaviour provenance (read, never copied — Sigil is GPL-3.0):
//! - OCF `mimetype` entry rules (first entry, stored, exact media type):
//!   <https://www.w3.org/TR/epub-33/#sec-zip-container-mime>
//! - `META-INF/container.xml` rootfile:
//!   <https://www.w3.org/TR/epub-33/#sec-container-metainf-container.xml>
//! - package document requirements (`unique-identifier` resolving to a
//!   `dc:identifier`, `dc:title`, `dc:language`, manifest/spine integrity, the
//!   `nav` and `cover-image` properties):
//!   <https://www.w3.org/TR/epub-33/#sec-package-doc>
//! - EPUB 2 NCX:
//!   <https://idpf.org/epub/20/spec/OPF_2.0.1_draft.htm#Section2.4.1>
//! - the Sigil "mend on open" behaviour this mirrors:
//!   <https://sigil-ebook.github.io/sigil-gitbook/current_maintenance/>
//!
//! A repair only ever rewrites the OCF envelope and the package document.
//! Content documents, style sheets and resources are raw-copied (same
//! compressed bytes, same method), so no repair can change what a reader
//! renders. See `crates/tools/README.md` for the option table.

use std::{
	borrow::Cow,
	collections::BTreeSet,
	fs::File,
	io::{BufReader, Read, Seek, Write},
	path::{Path, PathBuf},
};

use quick_xml::{
	escape::unescape,
	events::{BytesEnd, BytesStart, BytesText, Event},
	Reader, Writer,
};
use serde::{Deserialize, Serialize};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

use crate::{
	util, Action, Plan, ProgressSink, Report, Severity, Tool, ToolError, ToolInput,
	ToolResult, Warning,
};

/// The tool id, also its `stump tools <id>` subcommand name.
pub const ID: &str = "epub-check";
/// The only [`Action::kind`] this tool plans.
pub const ACTION_REPAIR: &str = "repair-epub";

const MIMETYPE_ENTRY: &str = "mimetype";
const EPUB_MEDIA_TYPE: &str = "application/epub+zip";
pub(crate) const CONTAINER_ENTRY: &str = "META-INF/container.xml";
const OPF_MEDIA_TYPE: &str = "application/oebps-package+xml";
pub(crate) const NCX_MEDIA_TYPE: &str = "application/x-dtbncx+xml";
const DC_NAMESPACE: &str = "http://purl.org/dc/elements/1.1/";
/// ISO 639-2 "undetermined": the only honest value for a missing language.
const UNDETERMINED_LANGUAGE: &str = "und";
/// Fallback `dc:identifier` id when the package declares none.
const FALLBACK_IDENTIFIER_ID: &str = "bookid";
/// The manifest id Stump itself treats as the cover, mirroring
/// `DEFAULT_EPUB_COVER_ID` in `crates/media/src/media/format/epub.rs`.
pub(crate) const DEFAULT_COVER_ID: &str = "cover";

/// Stable finding codes: callers filter on these, never on prose.
pub mod codes {
	pub const MIMETYPE_MISSING: &str = "mimetype-missing";
	pub const MIMETYPE_NOT_FIRST: &str = "mimetype-not-first";
	pub const MIMETYPE_COMPRESSED: &str = "mimetype-compressed";
	pub const MIMETYPE_CONTENT: &str = "mimetype-content";
	pub const CONTAINER_MISSING: &str = "container-missing";
	pub const CONTAINER_UNPARSABLE: &str = "container-unparsable";
	pub const CONTAINER_ROOTFILE_MISSING: &str = "container-rootfile-missing";
	pub const CONTAINER_ROOTFILE_DANGLING: &str = "container-rootfile-dangling";
	pub const OPF_UNPARSABLE: &str = "opf-unparsable";
	pub const OPF_IDENTIFIER_MISSING: &str = "opf-identifier-missing";
	pub const OPF_UNIQUE_IDENTIFIER_MISSING: &str = "opf-unique-identifier-missing";
	pub const OPF_UNIQUE_IDENTIFIER_UNRESOLVED: &str = "opf-unique-identifier-unresolved";
	pub const OPF_TITLE_MISSING: &str = "opf-title-missing";
	pub const OPF_LANGUAGE_MISSING: &str = "opf-language-missing";
	pub const MANIFEST_ITEM_MISSING: &str = "manifest-item-missing";
	pub const SPINE_IDREF_UNRESOLVED: &str = "spine-idref-unresolved";
	pub const SPINE_EMPTY: &str = "spine-empty";
	pub const NAV_DOCUMENT_MISSING: &str = "nav-document-missing";
	pub const NCX_MISSING: &str = "ncx-missing";
	pub const COVER_IMAGE_PROPERTY_MISSING: &str = "cover-image-property-missing";
}

/// `epub-check` options.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EpubCheckOptions {
	/// Plan repairs for the fixable findings. With `false` (the default) the
	/// plan is findings-only and an apply writes nothing.
	pub fix: bool,
}

/// Validate EPUB envelope/package structure and repair the mechanical
/// violations.
pub struct EpubCheck;

impl Tool for EpubCheck {
	fn id(&self) -> &'static str {
		ID
	}

	fn describe(&self) -> &'static str {
		"Validate EPUB container/package structure and repair the mechanical violations"
	}

	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError> {
		let options = input.parse_options::<EpubCheckOptions>()?;
		let targets = collect_epubs(&input.paths)?;

		let mut plan = Plan::new(ID);
		for target in targets {
			let outcome = audit(&target)?;
			for finding in outcome.findings {
				plan.warn(finding);
			}
			if !options.fix {
				continue;
			}
			if let Some(repair) = outcome.repair {
				plan.push(
					Action::new(ACTION_REPAIR)
						.with_source(&target)
						.with_target(&target)
						.with_detail(serde_json::to_value(&repair)?),
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
			if action.kind != ACTION_REPAIR {
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
			sink.progress(index, total, &format!("repairing {}", target.display()));

			let repair = match serde_json::from_value::<RepairPlan>(action.detail.clone())
			{
				Ok(repair) => repair,
				Err(error) => {
					report.skipped(
						action.clone(),
						format!("unreadable repair detail: {error}"),
					);
					continue;
				},
			};

			match repair_epub(&target, &repair) {
				Ok(()) => report.applied(action.clone()),
				Err(error) => report.skipped(action.clone(), error.to_string()),
			}
		}
		sink.progress(total, total, "epub-check complete");

		Ok(report)
	}
}

/// What [`audit`] found in one EPUB, and how much of it this tool can repair.
#[derive(Debug, Clone, PartialEq)]
pub struct Audit {
	pub path: PathBuf,
	/// Every finding, fixable or not, in envelope-then-package order.
	pub findings: Vec<Warning>,
	/// `None` when nothing is repairable.
	pub repair: Option<RepairPlan>,
}

impl Audit {
	/// The codes of every finding, in report order.
	pub fn codes(&self) -> Vec<&str> {
		self.findings
			.iter()
			.map(|finding| finding.code.as_str())
			.collect()
	}
}

/// The exact edits an apply performs. Storing them in the plan keeps the dry
/// run truthful: the apply re-derives nothing, not even the generated UUID.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RepairPlan {
	/// Finding codes this repair addresses, for display.
	pub fixes: Vec<String>,
	/// Rewrite `mimetype` as the first, stored, exact-content entry.
	pub rewrite_mimetype: bool,
	/// Zip path of the package document to rewrite; `None` leaves it untouched.
	pub opf_path: Option<String>,
	/// Insert `<dc:language>und</dc:language>`.
	pub add_language: bool,
	/// Set `unique-identifier` on `<package>`.
	pub set_package_uid: Option<String>,
	/// Set `id` on the first `<dc:identifier>` that has none.
	pub set_first_identifier_id: Option<String>,
	/// Insert a fresh `<dc:identifier>`.
	pub insert_identifier: Option<InsertedIdentifier>,
	/// Manifest item ids to drop because their file is missing, together with
	/// any spine `itemref` pointing at them.
	pub drop_item_ids: Vec<String>,
	/// The dropped items' hrefs, for display.
	pub drop_item_hrefs: Vec<String>,
	/// Remove `spine/@toc` because it names a dropped item.
	pub clear_spine_toc: bool,
	/// Manifest item id that gains the `cover-image` property.
	pub cover_image_id: Option<String>,
}

impl RepairPlan {
	fn touches_opf(&self) -> bool {
		self.add_language
			|| self.set_package_uid.is_some()
			|| self.set_first_identifier_id.is_some()
			|| self.insert_identifier.is_some()
			|| self.cover_image_id.is_some()
			|| self.clear_spine_toc
			|| !self.drop_item_ids.is_empty()
	}

	fn is_empty(&self) -> bool {
		!self.rewrite_mimetype && !self.touches_opf()
	}
}

/// A `dc:identifier` an apply will insert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertedIdentifier {
	pub id: String,
	pub value: String,
}

// ---------------------------------------------------------------------------
// Audit
// ---------------------------------------------------------------------------

/// Read the EPUB at `path` and report every violation. Writes nothing.
pub fn audit(path: &Path) -> ToolResult<Audit> {
	let mut archive = ZipArchive::new(BufReader::new(File::open(path)?))?;

	let mut findings = Vec::new();
	let mut repair = RepairPlan::default();

	let entries = read_entries(&mut archive)?;
	let names: BTreeSet<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();

	check_mimetype(path, &mut archive, &entries, &mut findings, &mut repair)?;

	let Some(container) = read_entry_text(&mut archive, CONTAINER_ENTRY)? else {
		findings.push(
			finding(
				codes::CONTAINER_MISSING,
				path,
				None,
				format!("archive has no {CONTAINER_ENTRY}"),
			)
			.with_severity(Severity::Error),
		);
		return Ok(finish(path, findings, repair));
	};

	let opf_path = match parse_container(&container) {
		Err(error) => {
			findings.push(
				finding(
					codes::CONTAINER_UNPARSABLE,
					path,
					Some(CONTAINER_ENTRY),
					format!("{CONTAINER_ENTRY} is not well-formed XML: {error}"),
				)
				.with_severity(Severity::Error),
			);
			return Ok(finish(path, findings, repair));
		},
		Ok(None) => {
			findings.push(
				finding(
					codes::CONTAINER_ROOTFILE_MISSING,
					path,
					Some(CONTAINER_ENTRY),
					format!("{CONTAINER_ENTRY} declares no usable <rootfile>"),
				)
				.with_severity(Severity::Error),
			);
			return Ok(finish(path, findings, repair));
		},
		Ok(Some(full_path)) => full_path,
	};

	let opf_xml = match names.contains(opf_path.as_str()) {
		true => read_entry_text(&mut archive, &opf_path)?,
		false => None,
	};
	let Some(opf_xml) = opf_xml else {
		findings.push(
			finding(
				codes::CONTAINER_ROOTFILE_DANGLING,
				path,
				Some(CONTAINER_ENTRY),
				format!("rootfile {opf_path:?} is not in the archive"),
			)
			.with_severity(Severity::Error),
		);
		return Ok(finish(path, findings, repair));
	};

	let opf = match parse_opf(&opf_xml) {
		Ok(opf) => opf,
		Err(error) => {
			findings.push(
				finding(
					codes::OPF_UNPARSABLE,
					path,
					Some(&opf_path),
					format!("package document is not well-formed XML: {error}"),
				)
				.with_severity(Severity::Error),
			);
			return Ok(finish(path, findings, repair));
		},
	};

	check_identifiers(path, &opf_path, &opf, &mut findings, &mut repair);
	check_title_and_language(path, &opf_path, &opf, &mut findings, &mut repair);
	let present =
		check_manifest(path, &opf_path, &opf, &names, &mut findings, &mut repair);
	check_spine(path, &opf_path, &opf, &repair, &mut findings);
	check_navigation(path, &opf_path, &opf, &present, &mut findings);
	check_cover(path, &opf_path, &opf, &present, &mut findings, &mut repair);

	// Dropping the NCX item would leave `spine/@toc` dangling.
	repair.clear_spine_toc = opf
		.spine_toc
		.as_ref()
		.is_some_and(|toc| repair.drop_item_ids.contains(toc));

	if repair.touches_opf() {
		repair.opf_path = Some(opf_path);
	}

	Ok(finish(path, findings, repair))
}

fn finish(path: &Path, findings: Vec<Warning>, mut repair: RepairPlan) -> Audit {
	repair.fixes = findings
		.iter()
		.filter(|finding| finding.fixable)
		.map(|finding| finding.code.clone())
		.collect();
	Audit {
		path: path.to_path_buf(),
		findings,
		repair: (!repair.is_empty()).then_some(repair),
	}
}

/// One zip entry's identity, in archive order.
#[derive(Debug, Clone)]
struct Entry {
	name: String,
	compression: CompressionMethod,
}

fn read_entries<R: Read + Seek>(archive: &mut ZipArchive<R>) -> ToolResult<Vec<Entry>> {
	let mut entries = Vec::with_capacity(archive.len());
	for index in 0..archive.len() {
		let file = archive.by_index(index)?;
		entries.push(Entry {
			name: file.name().to_owned(),
			compression: file.compression(),
		});
	}
	Ok(entries)
}

pub(crate) fn read_entry_text<R: Read + Seek>(
	archive: &mut ZipArchive<R>,
	name: &str,
) -> ToolResult<Option<String>> {
	let mut file = match archive.by_name(name) {
		Ok(file) => file,
		Err(zip::result::ZipError::FileNotFound) => return Ok(None),
		Err(error) => return Err(error.into()),
	};
	let mut bytes = Vec::with_capacity(file.size() as usize);
	file.read_to_end(&mut bytes)?;
	Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

fn check_mimetype<R: Read + Seek>(
	path: &Path,
	archive: &mut ZipArchive<R>,
	entries: &[Entry],
	findings: &mut Vec<Warning>,
	repair: &mut RepairPlan,
) -> ToolResult<()> {
	let Some(position) = entries
		.iter()
		.position(|entry| entry.name == MIMETYPE_ENTRY)
	else {
		findings.push(
			finding(
				codes::MIMETYPE_MISSING,
				path,
				None,
				format!("archive has no {MIMETYPE_ENTRY} entry"),
			)
			.with_severity(Severity::Error)
			.fixable(),
		);
		repair.rewrite_mimetype = true;
		return Ok(());
	};

	if position != 0 {
		findings.push(
			finding(
				codes::MIMETYPE_NOT_FIRST,
				path,
				Some(MIMETYPE_ENTRY),
				format!(
					"{MIMETYPE_ENTRY} is entry {position}, it must be the first entry"
				),
			)
			.with_severity(Severity::Error)
			.fixable(),
		);
		repair.rewrite_mimetype = true;
	}

	let compression = entries[position].compression;
	if compression != CompressionMethod::Stored {
		findings.push(
			finding(
				codes::MIMETYPE_COMPRESSED,
				path,
				Some(MIMETYPE_ENTRY),
				format!(
					"{MIMETYPE_ENTRY} is {compression} compressed, it must be stored"
				),
			)
			.with_severity(Severity::Error)
			.fixable(),
		);
		repair.rewrite_mimetype = true;
	}

	let mut bytes = Vec::new();
	archive.by_index(position)?.read_to_end(&mut bytes)?;
	if bytes != EPUB_MEDIA_TYPE.as_bytes() {
		findings.push(
			finding(
				codes::MIMETYPE_CONTENT,
				path,
				Some(MIMETYPE_ENTRY),
				format!(
					"{MIMETYPE_ENTRY} must contain exactly {EPUB_MEDIA_TYPE:?}, found {:?}",
					String::from_utf8_lossy(&bytes)
				),
			)
			.with_severity(Severity::Error)
			.fixable(),
		);
		repair.rewrite_mimetype = true;
	}

	Ok(())
}

fn check_identifiers(
	path: &Path,
	opf_path: &str,
	opf: &Opf,
	findings: &mut Vec<Warning>,
	repair: &mut RepairPlan,
) {
	if opf.identifier_ids.is_empty() {
		let id = opf
			.unique_identifier
			.clone()
			.unwrap_or_else(|| opf.free_id());
		findings.push(
			finding(
				codes::OPF_IDENTIFIER_MISSING,
				path,
				Some(opf_path),
				"package metadata has no <dc:identifier>".to_string(),
			)
			.with_severity(Severity::Error)
			.fixable(),
		);
		if opf.unique_identifier.is_none() {
			repair.set_package_uid = Some(id.clone());
		}
		repair.insert_identifier = Some(InsertedIdentifier {
			id,
			value: new_urn_uuid(),
		});
		return;
	}

	let Some(unique_identifier) = opf.unique_identifier.as_deref() else {
		findings.push(
			finding(
				codes::OPF_UNIQUE_IDENTIFIER_MISSING,
				path,
				Some(opf_path),
				"<package> declares no unique-identifier".to_string(),
			)
			.with_severity(Severity::Error)
			.fixable(),
		);
		match opf.identifier_ids.iter().flatten().next() {
			Some(id) => repair.set_package_uid = Some(id.clone()),
			None => {
				// Every identifier is anonymous: name the first one and point
				// the package at it.
				let id = opf.free_id();
				repair.set_first_identifier_id = Some(id.clone());
				repair.set_package_uid = Some(id);
			},
		}
		return;
	};

	let resolved = opf
		.identifier_ids
		.iter()
		.any(|id| id.as_deref() == Some(unique_identifier));
	if resolved {
		return;
	}

	findings.push(
		finding(
			codes::OPF_UNIQUE_IDENTIFIER_UNRESOLVED,
			path,
			Some(opf_path),
			format!(
				"unique-identifier {unique_identifier:?} matches no <dc:identifier> id"
			),
		)
		.with_severity(Severity::Error)
		.fixable(),
	);

	// Prefer adopting a real identifier value over minting a new one.
	if opf.identifier_ids.iter().any(Option::is_none) {
		repair.set_first_identifier_id = Some(unique_identifier.to_string());
	} else {
		repair.insert_identifier = Some(InsertedIdentifier {
			id: unique_identifier.to_string(),
			value: new_urn_uuid(),
		});
	}
}

fn check_title_and_language(
	path: &Path,
	opf_path: &str,
	opf: &Opf,
	findings: &mut Vec<Warning>,
	repair: &mut RepairPlan,
) {
	if opf.titles.is_empty() {
		// A title cannot be invented without guessing, so this is report-only.
		findings.push(
			finding(
				codes::OPF_TITLE_MISSING,
				path,
				Some(opf_path),
				"package metadata has no non-empty <dc:title>".to_string(),
			)
			.with_severity(Severity::Error),
		);
	}

	if opf.languages.is_empty() {
		findings.push(
			finding(
				codes::OPF_LANGUAGE_MISSING,
				path,
				Some(opf_path),
				format!(
					"package metadata has no non-empty <dc:language>; {UNDETERMINED_LANGUAGE:?} will be used"
				),
			)
			.with_severity(Severity::Error)
			.fixable(),
		);
		repair.add_language = true;
	}
}

/// Reports manifest items whose file is missing and returns the ids of the
/// items that do resolve to an archive entry.
fn check_manifest(
	path: &Path,
	opf_path: &str,
	opf: &Opf,
	names: &BTreeSet<&str>,
	findings: &mut Vec<Warning>,
	repair: &mut RepairPlan,
) -> BTreeSet<String> {
	let base = zip_parent(opf_path);
	let mut present = BTreeSet::new();

	for item in &opf.items {
		let Some(resolved) = resolve_href(base, &item.href) else {
			// Not an archive-local path (absolute URL, or `..` past the root):
			// there is nothing in this container to check.
			continue;
		};
		if names.contains(resolved.as_str()) {
			present.insert(item.id.clone());
			continue;
		}
		findings.push(
			finding(
				codes::MANIFEST_ITEM_MISSING,
				path,
				Some(opf_path),
				format!(
					"manifest item {:?} points at {:?}, which is not in the archive",
					item.id, item.href
				),
			)
			.with_severity(Severity::Error)
			.fixable(),
		);
		repair.drop_item_ids.push(item.id.clone());
		repair.drop_item_hrefs.push(item.href.clone());
	}

	present
}

fn check_spine(
	path: &Path,
	opf_path: &str,
	opf: &Opf,
	repair: &RepairPlan,
	findings: &mut Vec<Warning>,
) {
	let ids: BTreeSet<&str> = opf.items.iter().map(|item| item.id.as_str()).collect();
	for idref in &opf.itemrefs {
		// A dropped item's itemref goes with it, so it is not a second finding.
		if ids.contains(idref.as_str()) || repair.drop_item_ids.contains(idref) {
			continue;
		}
		findings.push(
			finding(
				codes::SPINE_IDREF_UNRESOLVED,
				path,
				Some(opf_path),
				format!("spine itemref {idref:?} matches no manifest item id"),
			)
			.with_severity(Severity::Error),
		);
	}

	// EPUB 3.3 §5.7.1: the spine must list at least one content document, and
	// dropping every itemref would leave a book that opens to nothing. Nothing
	// can be invented here, so this is report-only.
	let surviving = opf
		.itemrefs
		.iter()
		.filter(|idref| !repair.drop_item_ids.contains(idref))
		.count();
	if surviving == 0 {
		findings.push(
			finding(
				codes::SPINE_EMPTY,
				path,
				Some(opf_path),
				"spine lists no readable content document".to_string(),
			)
			.with_severity(Severity::Error),
		);
	}
}

fn check_navigation(
	path: &Path,
	opf_path: &str,
	opf: &Opf,
	present: &BTreeSet<String>,
	findings: &mut Vec<Warning>,
) {
	if opf.is_epub3() {
		let has_nav = opf
			.items
			.iter()
			.any(|item| item.has_property("nav") && present.contains(&item.id));
		if !has_nav {
			findings.push(
				finding(
					codes::NAV_DOCUMENT_MISSING,
					path,
					Some(opf_path),
					"no manifest item carries the \"nav\" property".to_string(),
				)
				.with_severity(Severity::Error),
			);
		}
		return;
	}

	let has_ncx = opf.items.iter().any(|item| {
		item.media_type.as_deref() == Some(NCX_MEDIA_TYPE) && present.contains(&item.id)
	});
	if !has_ncx {
		findings.push(
			finding(
				codes::NCX_MISSING,
				path,
				Some(opf_path),
				format!("no manifest item has media-type {NCX_MEDIA_TYPE}"),
			)
			.with_severity(Severity::Error),
		);
	}
}

fn check_cover(
	path: &Path,
	opf_path: &str,
	opf: &Opf,
	present: &BTreeSet<String>,
	findings: &mut Vec<Warning>,
	repair: &mut RepairPlan,
) {
	if !opf.is_epub3() {
		// `cover-image` is an EPUB 3 manifest property. EPUB 2 uses the
		// `<meta name="cover">` convention, which this tool leaves alone.
		return;
	}
	if opf
		.items
		.iter()
		.any(|item| item.has_property("cover-image"))
	{
		return;
	}

	let by_id = |id: &str| {
		opf.items
			.iter()
			.find(|item| item.id == id && present.contains(&item.id))
			.filter(|item| item.is_image())
	};
	let Some(cover) = opf
		.meta_cover
		.as_deref()
		.and_then(|id| by_id(id))
		.or_else(|| by_id(DEFAULT_COVER_ID))
	else {
		return;
	};

	// EPUB 3.3 §5.6.2.1 makes the property mandatory for a cover image, but
	// *which* resource is the cover is a heuristic here (`<meta name="cover">`,
	// else the `cover` id), so this stays a warning rather than an error.
	findings.push(
		finding(
			codes::COVER_IMAGE_PROPERTY_MISSING,
			path,
			Some(opf_path),
			format!(
				"cover image item {:?} does not declare the \"cover-image\" property",
				cover.id
			),
		)
		.fixable(),
	);
	repair.cover_image_id = Some(cover.id.clone());
}

/// A finding. `entry` is the archive member it is about, so a multi-file run
/// stays unambiguous: `<book.epub>/OEBPS/content.opf`.
pub(crate) fn finding(
	code: &str,
	path: &Path,
	entry: Option<&str>,
	message: String,
) -> Warning {
	let at = match entry {
		Some(entry) => path.join(entry),
		None => path.to_path_buf(),
	};
	Warning::new(code, message).at(at)
}

fn new_urn_uuid() -> String {
	format!("urn:uuid:{}", uuid::Uuid::new_v4())
}

// ---------------------------------------------------------------------------
// Repair
// ---------------------------------------------------------------------------

/// Apply `repair` to the EPUB at `path`, in place, atomically.
///
/// The rewritten archive stores `mimetype` first and raw-copies every other
/// entry, except the package document when the plan rewrites it.
fn repair_epub(path: &Path, repair: &RepairPlan) -> ToolResult<()> {
	let new_opf = match repair.opf_path.as_deref() {
		Some(opf_path) => {
			let mut archive = ZipArchive::new(BufReader::new(File::open(path)?))?;
			let xml = read_entry_text(&mut archive, opf_path)?.ok_or_else(|| {
				ToolError::Invalid(format!(
					"{}: package document {opf_path:?} disappeared",
					path.display()
				))
			})?;
			Some(rewrite_opf(&xml, repair)?)
		},
		None => None,
	};

	util::write_atomic(path, |sink| {
		let mut archive = ZipArchive::new(BufReader::new(File::open(path)?))?;
		let mut writer = ZipWriter::new(sink);

		writer.start_file(
			MIMETYPE_ENTRY,
			SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
		)?;
		writer.write_all(EPUB_MEDIA_TYPE.as_bytes())?;

		for index in 0..archive.len() {
			// `_raw` so no entry is ever decompressed just to be copied back.
			let entry = archive.by_index_raw(index)?;
			let name = entry.name().to_owned();
			if name == MIMETYPE_ENTRY {
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

			match new_opf.as_deref() {
				Some(bytes) if Some(name.as_str()) == repair.opf_path.as_deref() => {
					writer.start_file(
						name,
						options.compression_method(CompressionMethod::Deflated),
					)?;
					writer.write_all(bytes)?;
				},
				// Raw copy: same compressed bytes, same method, so content
				// documents cannot change.
				_ => writer.raw_copy_file(entry)?,
			}
		}

		writer.finish()?;
		Ok(())
	})
}

/// Rewrite the package document, preserving every byte this tool does not
/// explicitly change (declaration, comments, attribute order, indentation).
fn rewrite_opf(xml: &str, repair: &RepairPlan) -> ToolResult<Vec<u8>> {
	let naming = DcNaming::detect(xml)?;
	let drop_ids: BTreeSet<&str> =
		repair.drop_item_ids.iter().map(String::as_str).collect();

	let mut reader = Reader::from_str(xml);
	let mut writer = Writer::new(Vec::with_capacity(xml.len() + 128));

	let mut stack: Vec<String> = Vec::new();
	let mut pending_ws: Option<BytesText<'static>> = None;
	let mut metadata_indent: Option<BytesText<'static>> = None;
	let mut identifier_named = repair.set_first_identifier_id.is_none();
	let mut skip_depth: Option<usize> = None;

	loop {
		let event = reader.read_event()?;

		// Inside a dropped `<item>`/`<itemref>` that had children.
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
			Event::Text(text) if is_whitespace(text.as_ref()) => {
				flush(&mut writer, &mut pending_ws)?;
				if stack.last().map(String::as_str) == Some("metadata")
					&& metadata_indent.is_none()
				{
					metadata_indent = Some(text.clone().into_owned());
				}
				pending_ws = Some(text.into_owned());
			},
			Event::End(end) if local_name(end.name().as_ref()) == b"metadata" => {
				let indent = metadata_indent
					.clone()
					.unwrap_or_else(|| BytesText::from_escaped("\n\t\t"));
				if let Some(inserted) = &repair.insert_identifier {
					writer.write_event(Event::Text(indent.clone()))?;
					write_dc_element(
						&mut writer,
						&naming,
						"identifier",
						&inserted.value,
						&[("id", inserted.id.as_str())],
					)?;
				}
				if repair.add_language {
					writer.write_event(Event::Text(indent))?;
					write_dc_element(
						&mut writer,
						&naming,
						"language",
						UNDETERMINED_LANGUAGE,
						&[],
					)?;
				}
				flush(&mut writer, &mut pending_ws)?;
				writer.write_event(Event::End(end))?;
				stack.pop();
			},
			Event::Start(start) => {
				let name = local_string(start.name().as_ref());
				let parent = stack.last().map(String::as_str);
				if is_dropped(&start, &name, parent, &drop_ids)? {
					// Swallow the element's own indentation with it.
					pending_ws = None;
					skip_depth = Some(1);
					continue;
				}

				let mut start = start;
				patch_start(&mut start, &name, parent, repair, &mut identifier_named)?;
				flush(&mut writer, &mut pending_ws)?;
				writer.write_event(Event::Start(start))?;
				stack.push(name);
			},
			Event::Empty(empty) => {
				let name = local_string(empty.name().as_ref());
				let parent = stack.last().map(String::as_str);
				if is_dropped(&empty, &name, parent, &drop_ids)? {
					pending_ws = None;
					continue;
				}

				let mut empty = empty;
				patch_start(&mut empty, &name, parent, repair, &mut identifier_named)?;
				flush(&mut writer, &mut pending_ws)?;
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

pub(crate) fn flush(
	writer: &mut Writer<Vec<u8>>,
	pending: &mut Option<BytesText<'static>>,
) -> ToolResult<()> {
	if let Some(text) = pending.take() {
		writer.write_event(Event::Text(text))?;
	}
	Ok(())
}

/// `<item id=X>` / `<itemref idref=X>` whose manifest item is being dropped.
fn is_dropped(
	element: &BytesStart<'_>,
	name: &str,
	parent: Option<&str>,
	drop_ids: &BTreeSet<&str>,
) -> Result<bool, quick_xml::Error> {
	if drop_ids.is_empty() {
		return Ok(false);
	}
	let key = match (name, parent) {
		("item", Some("manifest")) => "id",
		("itemref", Some("spine")) => "idref",
		_ => return Ok(false),
	};
	Ok(attribute(element, key)?.is_some_and(|value| drop_ids.contains(value.as_str())))
}

fn patch_start(
	element: &mut BytesStart<'_>,
	name: &str,
	parent: Option<&str>,
	repair: &RepairPlan,
	identifier_named: &mut bool,
) -> Result<(), quick_xml::Error> {
	match (name, parent) {
		("package", None) => {
			if let Some(uid) = &repair.set_package_uid {
				if attribute(element, "unique-identifier")?.is_none() {
					set_attribute(element, "unique-identifier", uid)?;
				}
			}
		},
		("identifier", Some("metadata")) => {
			if let Some(id) = &repair.set_first_identifier_id {
				if !*identifier_named && attribute(element, "id")?.is_none() {
					set_attribute(element, "id", id)?;
					*identifier_named = true;
				}
			}
		},
		("item", Some("manifest")) => {
			let Some(cover_id) = &repair.cover_image_id else {
				return Ok(());
			};
			if attribute(element, "id")?.as_deref() != Some(cover_id.as_str()) {
				return Ok(());
			}
			let properties = match attribute(element, "properties")? {
				Some(existing) if !existing.trim().is_empty() => {
					format!("{} cover-image", existing.trim())
				},
				_ => "cover-image".to_string(),
			};
			set_attribute(element, "properties", &properties)?;
		},
		("spine", Some("package")) => {
			if repair.clear_spine_toc {
				remove_attribute(element, "toc")?;
			}
		},
		_ => {},
	}
	Ok(())
}

/// The `dc` prefix (and whether the namespace still needs declaring) for
/// inserted metadata elements.
#[derive(Debug, Clone, PartialEq)]
struct DcNaming {
	prefix: Option<String>,
	declare_namespace: bool,
}

impl DcNaming {
	const TERMS: [&'static [u8]; 8] = [
		b"identifier",
		b"title",
		b"language",
		b"creator",
		b"contributor",
		b"publisher",
		b"date",
		b"description",
	];

	/// Copy the document's own prefix rather than assuming `dc:`, so an
	/// inserted element is namespace-correct in any package document.
	fn detect(xml: &str) -> Result<Self, quick_xml::Error> {
		let mut reader = Reader::from_str(xml);
		let mut declared: Option<String> = None;
		loop {
			let element = match reader.read_event()? {
				Event::Start(element) | Event::Empty(element) => element,
				Event::Eof => break,
				_ => continue,
			};
			if declared.is_none() {
				declared = dc_namespace_prefix(&element)?;
			}
			let name = element.name();
			if Self::TERMS
				.iter()
				.any(|term| *term == local_name(name.as_ref()))
			{
				let prefix = name
					.prefix()
					.map(|prefix| String::from_utf8_lossy(prefix.as_ref()).into_owned());
				return Ok(Self {
					prefix,
					declare_namespace: false,
				});
			}
		}

		match declared {
			Some(prefix) => Ok(Self {
				prefix: Some(prefix),
				declare_namespace: false,
			}),
			// Nothing to copy: declare the namespace on the inserted element.
			None => Ok(Self {
				prefix: Some("dc".to_string()),
				declare_namespace: true,
			}),
		}
	}

	fn qualify(&self, local: &str) -> String {
		match self.prefix.as_deref() {
			Some(prefix) => format!("{prefix}:{local}"),
			None => local.to_string(),
		}
	}
}

fn dc_namespace_prefix(
	element: &BytesStart<'_>,
) -> Result<Option<String>, quick_xml::Error> {
	for attribute in element.attributes() {
		let attribute = attribute?;
		let key = attribute.key.as_ref();
		if !key.starts_with(b"xmlns:") || decode(&attribute.value) != DC_NAMESPACE {
			continue;
		}
		return Ok(Some(String::from_utf8_lossy(&key[6..]).into_owned()));
	}
	Ok(None)
}

fn write_dc_element(
	writer: &mut Writer<Vec<u8>>,
	naming: &DcNaming,
	local: &str,
	text: &str,
	attributes: &[(&str, &str)],
) -> ToolResult<()> {
	let name = naming.qualify(local);
	let mut start = BytesStart::new(name.clone());
	if naming.declare_namespace {
		start.push_attribute(("xmlns:dc", DC_NAMESPACE));
	}
	for (key, value) in attributes {
		start.push_attribute((*key, *value));
	}
	writer.write_event(Event::Start(start))?;
	writer.write_event(Event::Text(BytesText::new(text)))?;
	writer.write_event(Event::End(BytesEnd::new(name)))?;
	Ok(())
}

// ---------------------------------------------------------------------------
// XML models
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Opf {
	pub(crate) version: Option<String>,
	pub(crate) unique_identifier: Option<String>,
	/// The `id` of every `<dc:identifier>`, in document order.
	pub(crate) identifier_ids: Vec<Option<String>>,
	pub(crate) titles: Vec<String>,
	pub(crate) languages: Vec<String>,
	pub(crate) items: Vec<ManifestItem>,
	pub(crate) itemrefs: Vec<String>,
	pub(crate) meta_cover: Option<String>,
	/// `spine/@toc`, the EPUB 2 pointer at the NCX item.
	pub(crate) spine_toc: Option<String>,
	/// Every `id` in the document, so a minted id cannot collide.
	pub(crate) all_ids: BTreeSet<String>,
}

impl Opf {
	/// EPUB 3 requires a `nav` document; EPUB 2 (and a package with no usable
	/// version) is checked against the NCX rules instead.
	pub(crate) fn is_epub3(&self) -> bool {
		self.version
			.as_deref()
			.is_some_and(|version| version.trim().starts_with('3'))
	}

	/// An `id` no element in this package uses. `bookid` is taken often enough
	/// in the wild (Calibre, Sigil, InDesign) to need the fallback.
	fn free_id(&self) -> String {
		if !self.all_ids.contains(FALLBACK_IDENTIFIER_ID) {
			return FALLBACK_IDENTIFIER_ID.to_string();
		}
		(2..)
			.map(|suffix| format!("{FALLBACK_IDENTIFIER_ID}-{suffix}"))
			.find(|candidate| !self.all_ids.contains(candidate))
			.unwrap_or_else(|| format!("{FALLBACK_IDENTIFIER_ID}-{}", new_urn_uuid()))
	}
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct ManifestItem {
	pub(crate) id: String,
	pub(crate) href: String,
	pub(crate) media_type: Option<String>,
	pub(crate) properties: Option<String>,
}

impl ManifestItem {
	pub(crate) fn has_property(&self, property: &str) -> bool {
		self.properties.as_deref().is_some_and(|properties| {
			properties.split_whitespace().any(|token| token == property)
		})
	}

	pub(crate) fn is_image(&self) -> bool {
		self.media_type
			.as_deref()
			.is_some_and(|media_type| media_type.starts_with("image/"))
	}
}

/// The `full-path` of the first `<rootfile>` with the package media type, or of
/// the first `<rootfile>` at all when none declares it.
pub(crate) fn parse_container(xml: &str) -> Result<Option<String>, quick_xml::Error> {
	let mut reader = Reader::from_str(xml);
	let mut fallback = None;
	loop {
		let element = match reader.read_event()? {
			Event::Start(element) | Event::Empty(element) => element,
			Event::Eof => break,
			_ => continue,
		};
		if local_name(element.name().as_ref()) != b"rootfile" {
			continue;
		}
		let Some(full_path) =
			attribute(&element, "full-path")?.filter(|path| !path.trim().is_empty())
		else {
			continue;
		};
		if attribute(&element, "media-type")?.as_deref() == Some(OPF_MEDIA_TYPE) {
			return Ok(Some(normalize_zip_path(&full_path)));
		}
		fallback.get_or_insert_with(|| normalize_zip_path(&full_path));
	}
	Ok(fallback)
}

pub(crate) fn parse_opf(xml: &str) -> Result<Opf, quick_xml::Error> {
	let mut reader = Reader::from_str(xml);
	let mut opf = Opf::default();
	let mut stack: Vec<Vec<u8>> = Vec::new();
	// The metadata element currently collecting text, and its buffer.
	let mut collecting: Option<(Vec<u8>, String)> = None;

	loop {
		match reader.read_event()? {
			Event::Start(element) => {
				let name = local_name(element.name().as_ref()).to_vec();
				let parent = stack.last().map(Vec::as_slice);
				read_element(&mut opf, &element, &name, parent)?;
				if parent == Some(b"metadata".as_slice())
					&& matches!(name.as_slice(), b"title" | b"language")
				{
					collecting = Some((name.clone(), String::new()));
				}
				stack.push(name);
			},
			Event::Empty(element) => {
				let name = local_name(element.name().as_ref()).to_vec();
				let parent = stack.last().map(Vec::as_slice);
				read_element(&mut opf, &element, &name, parent)?;
			},
			Event::Text(text) => {
				if let Some((_, buffer)) = collecting.as_mut() {
					buffer.push_str(&decode(text.as_ref()));
				}
			},
			Event::CData(data) => {
				if let Some((_, buffer)) = collecting.as_mut() {
					buffer.push_str(&String::from_utf8_lossy(data.as_ref()));
				}
			},
			// quick-xml reports `&amp;` and friends as their own event, so a
			// title around one arrives in three pieces, not one.
			Event::GeneralRef(reference) => {
				if let Some((_, buffer)) = collecting.as_mut() {
					buffer.push_str(&resolve_reference(reference.as_ref()));
				}
			},
			Event::End(_) => {
				if let Some((name, buffer)) = collecting.take() {
					let trimmed = buffer.trim();
					if !trimmed.is_empty() {
						match name.as_slice() {
							b"title" => opf.titles.push(trimmed.to_string()),
							b"language" => opf.languages.push(trimmed.to_string()),
							_ => {},
						}
					}
				}
				stack.pop();
			},
			Event::Eof => break,
			_ => {},
		}
	}

	Ok(opf)
}

fn read_element(
	opf: &mut Opf,
	element: &BytesStart<'_>,
	name: &[u8],
	parent: Option<&[u8]>,
) -> Result<(), quick_xml::Error> {
	if let Some(id) = attribute(element, "id")? {
		opf.all_ids.insert(id);
	}

	match (name, parent) {
		(b"package", None) => {
			opf.version = attribute(element, "version")?;
			opf.unique_identifier = attribute(element, "unique-identifier")?
				.filter(|value| !value.trim().is_empty());
		},
		(b"identifier", Some(b"metadata")) => {
			opf.identifier_ids.push(attribute(element, "id")?);
		},
		(b"meta", Some(b"metadata")) => {
			if attribute(element, "name")?.as_deref() == Some("cover") {
				opf.meta_cover = attribute(element, "content")?
					.filter(|value| !value.trim().is_empty());
			}
		},
		(b"item", Some(b"manifest")) => {
			let id = attribute(element, "id")?.unwrap_or_default();
			let href = attribute(element, "href")?.unwrap_or_default();
			if id.is_empty() || href.is_empty() {
				return Ok(());
			}
			opf.items.push(ManifestItem {
				id,
				href,
				media_type: attribute(element, "media-type")?,
				properties: attribute(element, "properties")?,
			});
		},
		(b"itemref", Some(b"spine")) => {
			if let Some(idref) =
				attribute(element, "idref")?.filter(|value| !value.is_empty())
			{
				opf.itemrefs.push(idref);
			}
		},
		(b"spine", Some(b"package")) => {
			opf.spine_toc = attribute(element, "toc")?.filter(|value| !value.is_empty());
		},
		_ => {},
	}
	Ok(())
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

pub(crate) fn local_name(name: &[u8]) -> &[u8] {
	match name.iter().position(|byte| *byte == b':') {
		Some(index) => &name[index + 1..],
		None => name,
	}
}

pub(crate) fn local_string(name: &[u8]) -> String {
	String::from_utf8_lossy(local_name(name)).into_owned()
}

/// Unescape leniently: a checker must not abort on an exotic entity.
pub(crate) fn decode(bytes: &[u8]) -> String {
	let raw = String::from_utf8_lossy(bytes);
	match unescape(&raw) {
		Ok(Cow::Owned(value)) => value,
		Ok(Cow::Borrowed(_)) | Err(_) => raw.into_owned(),
	}
}

/// The text a `&name;`/`&#nn;` reference stands for, or the reference itself
/// when it names an entity this crate cannot resolve.
pub(crate) fn resolve_reference(content: &[u8]) -> String {
	decode(format!("&{};", String::from_utf8_lossy(content)).as_bytes())
}

pub(crate) fn attribute(
	element: &BytesStart<'_>,
	key: &str,
) -> Result<Option<String>, quick_xml::Error> {
	for attribute in element.attributes() {
		let attribute = attribute?;
		if local_name(attribute.key.as_ref()) == key.as_bytes() {
			return Ok(Some(decode(&attribute.value)));
		}
	}
	Ok(None)
}

/// Replace `key`'s value, appending the attribute when it is absent. Every
/// other attribute keeps its original bytes and position.
pub(crate) fn set_attribute(
	element: &mut BytesStart<'_>,
	key: &str,
	value: &str,
) -> Result<(), quick_xml::Error> {
	let mut rebuilt =
		BytesStart::new(String::from_utf8_lossy(element.name().as_ref()).into_owned());
	let mut replaced = false;
	for attribute in element.attributes() {
		let attribute = attribute?;
		if local_name(attribute.key.as_ref()) == key.as_bytes() {
			rebuilt.push_attribute((
				String::from_utf8_lossy(attribute.key.as_ref()).as_ref(),
				value,
			));
			replaced = true;
			continue;
		}
		rebuilt.push_attribute(attribute);
	}
	if !replaced {
		rebuilt.push_attribute((key, value));
	}
	*element = rebuilt.into_owned();
	Ok(())
}

/// Drop `key`, keeping every other attribute's original bytes and position.
fn remove_attribute(
	element: &mut BytesStart<'_>,
	key: &str,
) -> Result<(), quick_xml::Error> {
	let mut rebuilt =
		BytesStart::new(String::from_utf8_lossy(element.name().as_ref()).into_owned());
	for attribute in element.attributes() {
		let attribute = attribute?;
		if local_name(attribute.key.as_ref()) == key.as_bytes() {
			continue;
		}
		rebuilt.push_attribute(attribute);
	}
	*element = rebuilt.into_owned();
	Ok(())
}

pub(crate) fn is_whitespace(bytes: &[u8]) -> bool {
	!bytes.is_empty() && bytes.iter().all(u8::is_ascii_whitespace)
}

pub(crate) fn zip_parent(path: &str) -> &str {
	match path.rfind('/') {
		Some(index) => &path[..index],
		None => "",
	}
}

fn normalize_zip_path(path: &str) -> String {
	resolve_href("", path).unwrap_or_else(|| path.to_string())
}

/// Resolve a manifest `href` against the package document's directory: strip
/// the fragment and query, percent-decode, normalize `.`/`..`. `None` means
/// "not an archive-local path" (absolute URL, or `..` escaping the root).
pub(crate) fn resolve_href(base: &str, href: &str) -> Option<String> {
	let href = href.split(['#', '?']).next().unwrap_or(href);
	if href.is_empty()
		|| href.contains("://")
		|| href.starts_with("//")
		|| href.starts_with("mailto:")
	{
		return None;
	}
	let decoded = percent_decode(href);

	let mut parts: Vec<&str> = Vec::new();
	if !decoded.starts_with('/') && !base.is_empty() {
		parts.extend(base.split('/').filter(|part| !part.is_empty()));
	}
	for part in decoded.split('/') {
		match part {
			"" | "." => {},
			".." => {
				parts.pop()?;
			},
			part => parts.push(part),
		}
	}
	(!parts.is_empty()).then(|| parts.join("/"))
}

fn percent_decode(input: &str) -> String {
	if !input.contains('%') {
		return input.to_string();
	}
	let bytes = input.as_bytes();
	let mut out = Vec::with_capacity(bytes.len());
	let mut index = 0;
	while index < bytes.len() {
		if bytes[index] == b'%' && index + 2 < bytes.len() {
			let decoded = std::str::from_utf8(&bytes[index + 1..index + 3])
				.ok()
				.and_then(|hex| u8::from_str_radix(hex, 16).ok());
			if let Some(byte) = decoded {
				out.push(byte);
				index += 3;
				continue;
			}
		}
		out.push(bytes[index]);
		index += 1;
	}
	String::from_utf8_lossy(&out).into_owned()
}

/// Expand the caller's paths into the EPUBs to check: a file is taken as-is, a
/// directory is walked recursively for `*.epub`.
pub(crate) fn collect_epubs(paths: &[PathBuf]) -> ToolResult<Vec<PathBuf>> {
	if paths.is_empty() {
		return Err(ToolError::Invalid("no input paths".to_string()));
	}

	let mut targets = Vec::new();
	for path in paths {
		let metadata = std::fs::metadata(path).map_err(|error| {
			ToolError::Invalid(format!("{}: {error}", path.display()))
		})?;
		if metadata.is_dir() {
			targets.extend(
				util::sorted_files(path, true)?
					.into_iter()
					.filter(|path| is_epub(path)),
			);
		} else {
			targets.push(path.clone());
		}
	}

	if targets.is_empty() {
		return Err(ToolError::Invalid(
			"no .epub files found in the given paths".to_string(),
		));
	}
	Ok(targets)
}

fn is_epub(path: &Path) -> bool {
	path.extension()
		.is_some_and(|extension| extension.eq_ignore_ascii_case("epub"))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::NoopProgress;
	use pretty_assertions::assert_eq;
	use tempfile::TempDir;

	const NAV: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Contents</title></head>
<body><nav epub:type="toc"><ol><li><a href="ch1.xhtml">One</a></li></ol></nav></body>
</html>"#;

	const CHAPTER: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>One</title></head>
<body><p>Hello.</p></body></html>"#;

	const NCX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
<head/><docTitle><text>Book</text></docTitle>
<navMap><navPoint id="p1" playOrder="1"><navLabel><text>One</text></navLabel>
<content src="ch1.xhtml"/></navPoint></navMap></ncx>"#;

	const CONTAINER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
	<rootfiles>
		<rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
	</rootfiles>
</container>"#;

	/// EPUB 3 package with no `dc:language` and a manifest item whose file is
	/// not in the archive.
	const BROKEN_OPF: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid">
	<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
		<dc:identifier id="bookid">urn:uuid:6b1c4e0a-1d1f-4a5b-9c3e-2a7f1d0c9b11</dc:identifier>
		<dc:title>A Broken Book</dc:title>
	</metadata>
	<manifest>
		<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
		<item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
		<item id="cover" href="cover.jpg" media-type="image/jpeg" properties="cover-image"/>
		<item id="ghost" href="ghost.xhtml" media-type="application/xhtml+xml"/>
	</manifest>
	<spine>
		<itemref idref="ch1"/>
	</spine>
</package>"#;

	fn stored() -> CompressionMethod {
		CompressionMethod::Stored
	}

	fn deflated() -> CompressionMethod {
		CompressionMethod::Deflated
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

	/// The acceptance fixture: `mimetype` is the *second* entry, the package has
	/// no `dc:language`, and `ghost.xhtml` is not in the archive.
	fn broken_epub(dir: &Path) -> PathBuf {
		let path = dir.join("broken.epub");
		write_epub(
			&path,
			&[
				(CONTAINER_ENTRY, CONTAINER.as_bytes(), stored()),
				(MIMETYPE_ENTRY, EPUB_MEDIA_TYPE.as_bytes(), stored()),
				("OEBPS/content.opf", BROKEN_OPF.as_bytes(), deflated()),
				("OEBPS/nav.xhtml", NAV.as_bytes(), deflated()),
				("OEBPS/ch1.xhtml", CHAPTER.as_bytes(), deflated()),
				(
					"OEBPS/cover.jpg",
					b"\xff\xd8\xff\xe0 not-a-real-jpeg",
					stored(),
				),
			],
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

	/// Entry names in *archive order*, which is what the OCF rules are about.
	fn entry_names(path: &Path) -> Vec<String> {
		let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
		(0..archive.len())
			.map(|index| archive.by_index(index).unwrap().name().to_owned())
			.collect()
	}

	fn plan_and_apply(path: &Path) -> Report {
		let tool = EpubCheck;
		let input = ToolInput::new(vec![path.to_path_buf()])
			.with_options(serde_json::json!({ "fix": true }));
		let plan = tool.plan(&input).unwrap();
		tool.apply(&plan, &mut NoopProgress).unwrap()
	}

	#[test]
	fn reports_the_three_seeded_violations() {
		let dir = TempDir::new().unwrap();
		let path = broken_epub(dir.path());

		let found = audit(&path).unwrap();

		assert_eq!(
			found.codes(),
			vec![
				codes::MIMETYPE_NOT_FIRST,
				codes::OPF_LANGUAGE_MISSING,
				codes::MANIFEST_ITEM_MISSING,
			]
		);
		assert!(found.findings.iter().all(|finding| finding.fixable));
		assert_eq!(
			found.findings[2].path,
			Some(path.join("OEBPS/content.opf")),
			"a package finding points at the package document"
		);
		let repair = found.repair.expect("all three findings are fixable");
		assert!(repair.rewrite_mimetype);
		assert!(repair.add_language);
		assert_eq!(repair.drop_item_ids, vec!["ghost".to_string()]);
	}

	#[test]
	fn repaired_epub_is_clean_and_readable() {
		let dir = TempDir::new().unwrap();
		let path = broken_epub(dir.path());
		let chapter_before = entry_bytes(&path, "OEBPS/ch1.xhtml");

		let report = plan_and_apply(&path);
		assert_eq!(report.applied.len(), 1);
		assert!(report.skipped.is_empty());

		let found = audit(&path).unwrap();
		assert_eq!(found.codes(), Vec::<&str>::new());
		assert_eq!(found.repair, None);

		assert_eq!(
			entry_names(&path).first().map(String::as_str),
			Some(MIMETYPE_ENTRY)
		);
		assert_eq!(
			entry_bytes(&path, MIMETYPE_ENTRY),
			EPUB_MEDIA_TYPE.as_bytes()
		);

		let opf = String::from_utf8(entry_bytes(&path, "OEBPS/content.opf")).unwrap();
		assert!(
			opf.contains("<dc:language>und</dc:language>"),
			"language was inserted: {opf}"
		);
		assert!(!opf.contains("ghost"), "dangling item was dropped: {opf}");
		assert!(
			opf.contains("<dc:title>A Broken Book</dc:title>"),
			"untouched metadata is preserved verbatim: {opf}"
		);
		assert_eq!(
			entry_bytes(&path, "OEBPS/ch1.xhtml"),
			chapter_before,
			"content documents are copied byte-for-byte"
		);

		stump_media::EpubProcessor::open(path.to_str().unwrap())
			.expect("repaired epub opens with stump_media");
	}

	#[test]
	fn compressed_and_padded_mimetype_is_normalized() {
		let dir = TempDir::new().unwrap();
		let path = dir.path().join("mime.epub");
		write_epub(
			&path,
			&[
				(MIMETYPE_ENTRY, b"application/epub+zip\n", deflated()),
				(CONTAINER_ENTRY, CONTAINER.as_bytes(), stored()),
				("OEBPS/content.opf", BROKEN_OPF.as_bytes(), stored()),
				("OEBPS/nav.xhtml", NAV.as_bytes(), stored()),
				("OEBPS/ch1.xhtml", CHAPTER.as_bytes(), stored()),
				("OEBPS/cover.jpg", b"\xff\xd8\xff\xe0", stored()),
			],
		);

		assert_eq!(
			audit(&path).unwrap().codes(),
			vec![
				codes::MIMETYPE_COMPRESSED,
				codes::MIMETYPE_CONTENT,
				codes::OPF_LANGUAGE_MISSING,
				codes::MANIFEST_ITEM_MISSING,
			]
		);

		plan_and_apply(&path);

		let mut archive = ZipArchive::new(File::open(&path).unwrap()).unwrap();
		let first = archive.by_index(0).unwrap();
		assert_eq!(first.name(), MIMETYPE_ENTRY);
		assert_eq!(first.compression(), CompressionMethod::Stored);
		assert_eq!(
			first.data_start(),
			(30 + MIMETYPE_ENTRY.len()) as u64,
			"EPUB 3.3 §4.3.3: the mimetype local header carries no extra field"
		);
		drop(first);
		assert_eq!(
			entry_bytes(&path, MIMETYPE_ENTRY),
			EPUB_MEDIA_TYPE.as_bytes()
		);
		assert_eq!(audit(&path).unwrap().codes(), Vec::<&str>::new());
	}

	#[test]
	fn plan_without_fix_never_writes() {
		let dir = TempDir::new().unwrap();
		let path = broken_epub(dir.path());
		let before = std::fs::read(&path).unwrap();

		let tool = EpubCheck;
		let plan = tool.plan(&ToolInput::new(vec![path.clone()])).unwrap();
		assert_eq!(plan.warnings.len(), 3);
		assert!(plan.actions.is_empty(), "a check-only plan has no actions");

		let report = tool.apply(&plan, &mut NoopProgress).unwrap();
		assert!(report.applied.is_empty());
		assert_eq!(report.warnings.len(), 3, "findings survive into the report");
		assert_eq!(std::fs::read(&path).unwrap(), before, "original untouched");
	}

	#[test]
	fn missing_identifier_and_cover_property_are_repaired() {
		let dir = TempDir::new().unwrap();
		let path = dir.path().join("meta.epub");
		let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
	<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
		<dc:title>Nameless</dc:title>
		<dc:language>en</dc:language>
	</metadata>
	<manifest>
		<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
		<item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
		<item id="cover" href="cover.jpg" media-type="image/jpeg"/>
	</manifest>
	<spine><itemref idref="ch1"/></spine>
</package>"#;
		write_epub(
			&path,
			&[
				(MIMETYPE_ENTRY, EPUB_MEDIA_TYPE.as_bytes(), stored()),
				(CONTAINER_ENTRY, CONTAINER.as_bytes(), stored()),
				("OEBPS/content.opf", opf.as_bytes(), stored()),
				("OEBPS/nav.xhtml", NAV.as_bytes(), stored()),
				("OEBPS/ch1.xhtml", CHAPTER.as_bytes(), stored()),
				("OEBPS/cover.jpg", b"\xff\xd8\xff\xe0", stored()),
			],
		);

		assert_eq!(
			audit(&path).unwrap().codes(),
			vec![
				codes::OPF_IDENTIFIER_MISSING,
				codes::COVER_IMAGE_PROPERTY_MISSING,
			]
		);

		plan_and_apply(&path);

		let repaired =
			String::from_utf8(entry_bytes(&path, "OEBPS/content.opf")).unwrap();
		assert!(
			repaired.contains(r#"unique-identifier="bookid""#),
			"package gained a unique-identifier: {repaired}"
		);
		assert!(
			repaired.contains(r#"<dc:identifier id="bookid">urn:uuid:"#),
			"a urn:uuid identifier was inserted: {repaired}"
		);
		assert!(
			repaired.contains(r#"properties="cover-image""#),
			"the cover item gained the property: {repaired}"
		);
		assert_eq!(audit(&path).unwrap().codes(), Vec::<&str>::new());
		stump_media::EpubProcessor::open(path.to_str().unwrap()).unwrap();
	}

	#[test]
	fn dropping_a_manifest_item_drops_its_spine_itemref() {
		let dir = TempDir::new().unwrap();
		let path = dir.path().join("spine.epub");
		let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid">
	<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
		<dc:identifier id="bookid">urn:uuid:0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0</dc:identifier>
		<dc:title>Gapped</dc:title>
		<dc:language>en</dc:language>
	</metadata>
	<manifest>
		<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
		<item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
		<item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
	</manifest>
	<spine>
		<itemref idref="ch1"/>
		<itemref idref="ch2"/>
	</spine>
</package>"#;
		write_epub(
			&path,
			&[
				(MIMETYPE_ENTRY, EPUB_MEDIA_TYPE.as_bytes(), stored()),
				(CONTAINER_ENTRY, CONTAINER.as_bytes(), stored()),
				("OEBPS/content.opf", opf.as_bytes(), stored()),
				("OEBPS/nav.xhtml", NAV.as_bytes(), stored()),
				("OEBPS/ch1.xhtml", CHAPTER.as_bytes(), stored()),
			],
		);

		assert_eq!(
			audit(&path).unwrap().codes(),
			vec![codes::MANIFEST_ITEM_MISSING],
			"a spine item with no file is one finding, not two"
		);

		plan_and_apply(&path);

		let repaired =
			String::from_utf8(entry_bytes(&path, "OEBPS/content.opf")).unwrap();
		assert!(
			!repaired.contains("ch2"),
			"item and itemref both dropped: {repaired}"
		);
		assert!(repaired.contains(r#"<itemref idref="ch1"/>"#));
		assert_eq!(audit(&path).unwrap().codes(), Vec::<&str>::new());
		stump_media::EpubProcessor::open(path.to_str().unwrap()).unwrap();
	}

	#[test]
	fn epub2_without_title_or_ncx_reports_unfixable_findings() {
		let dir = TempDir::new().unwrap();
		let path = dir.path().join("epub2.epub");
		let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="bookid">
	<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
		<dc:identifier id="bookid">isbn:1234</dc:identifier>
		<dc:language>en</dc:language>
	</metadata>
	<manifest>
		<item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
	</manifest>
	<spine><itemref idref="ch1"/></spine>
</package>"#;
		write_epub(
			&path,
			&[
				(MIMETYPE_ENTRY, EPUB_MEDIA_TYPE.as_bytes(), stored()),
				(CONTAINER_ENTRY, CONTAINER.as_bytes(), stored()),
				("OEBPS/content.opf", opf.as_bytes(), stored()),
				("OEBPS/ch1.xhtml", CHAPTER.as_bytes(), stored()),
			],
		);

		let found = audit(&path).unwrap();
		assert_eq!(
			found.codes(),
			vec![codes::OPF_TITLE_MISSING, codes::NCX_MISSING]
		);
		assert!(
			found.findings.iter().all(|finding| !finding.fixable),
			"neither a title nor a navigation document can be invented"
		);
		assert_eq!(found.repair, None, "nothing to apply");
	}

	#[test]
	fn epub2_with_ncx_is_clean() {
		let dir = TempDir::new().unwrap();
		let path = dir.path().join("epub2-ok.epub");
		let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="bookid">
	<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
		<dc:identifier id="bookid">isbn:1234</dc:identifier>
		<dc:title>Old Book</dc:title>
		<dc:language>en</dc:language>
		<meta name="cover" content="cover"/>
	</metadata>
	<manifest>
		<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
		<item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
		<item id="cover" href="cover.jpg" media-type="image/jpeg"/>
	</manifest>
	<spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#;
		write_epub(
			&path,
			&[
				(MIMETYPE_ENTRY, EPUB_MEDIA_TYPE.as_bytes(), stored()),
				(CONTAINER_ENTRY, CONTAINER.as_bytes(), stored()),
				("OEBPS/content.opf", opf.as_bytes(), stored()),
				("OEBPS/toc.ncx", NCX.as_bytes(), stored()),
				("OEBPS/ch1.xhtml", CHAPTER.as_bytes(), stored()),
				("OEBPS/cover.jpg", b"\xff\xd8\xff\xe0", stored()),
			],
		);

		assert_eq!(
			audit(&path).unwrap().codes(),
			Vec::<&str>::new(),
			"EPUB 2 needs an NCX, not a nav document or a cover-image property"
		);
	}

	#[test]
	fn dropping_the_ncx_clears_the_spine_toc_pointer() {
		let dir = TempDir::new().unwrap();
		let path = dir.path().join("dangling-ncx.epub");
		let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="bookid">
	<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
		<dc:identifier id="bookid">isbn:1234</dc:identifier>
		<dc:title>No Contents</dc:title>
		<dc:language>en</dc:language>
	</metadata>
	<manifest>
		<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
		<item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
	</manifest>
	<spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#;
		write_epub(
			&path,
			&[
				(MIMETYPE_ENTRY, EPUB_MEDIA_TYPE.as_bytes(), stored()),
				(CONTAINER_ENTRY, CONTAINER.as_bytes(), stored()),
				("OEBPS/content.opf", opf.as_bytes(), stored()),
				("OEBPS/ch1.xhtml", CHAPTER.as_bytes(), stored()),
			],
		);

		let found = audit(&path).unwrap();
		assert_eq!(
			found.codes(),
			vec![codes::MANIFEST_ITEM_MISSING, codes::NCX_MISSING]
		);
		assert!(found.repair.as_ref().unwrap().clear_spine_toc);

		plan_and_apply(&path);

		let repaired =
			String::from_utf8(entry_bytes(&path, "OEBPS/content.opf")).unwrap();
		assert!(
			repaired.contains("<spine>"),
			"a dropped NCX takes spine/@toc with it, no dangling IDREF: {repaired}"
		);
		assert!(!repaired.contains("toc.ncx"));
		assert_eq!(
			audit(&path).unwrap().codes(),
			vec![codes::NCX_MISSING],
			"the repair fixes what it can and reports the rest unchanged"
		);
	}

	#[test]
	fn a_spine_with_no_readable_document_is_reported() {
		let dir = TempDir::new().unwrap();
		let path = dir.path().join("empty-spine.epub");
		let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid">
	<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
		<dc:identifier id="bookid">urn:uuid:0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0</dc:identifier>
		<dc:title>Hollow</dc:title>
		<dc:language>en</dc:language>
	</metadata>
	<manifest>
		<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
		<item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
	</manifest>
	<spine><itemref idref="ch1"/></spine>
</package>"#;
		write_epub(
			&path,
			&[
				(MIMETYPE_ENTRY, EPUB_MEDIA_TYPE.as_bytes(), stored()),
				(CONTAINER_ENTRY, CONTAINER.as_bytes(), stored()),
				("OEBPS/content.opf", opf.as_bytes(), stored()),
				("OEBPS/nav.xhtml", NAV.as_bytes(), stored()),
			],
		);

		assert_eq!(
			audit(&path).unwrap().codes(),
			vec![codes::MANIFEST_ITEM_MISSING, codes::SPINE_EMPTY],
			"dropping the only spine document leaves an unreadable book"
		);
	}

	#[test]
	fn percent_encoded_and_relative_hrefs_resolve() {
		assert_eq!(
			resolve_href("OEBPS/text", "../images/a%20b.png").as_deref(),
			Some("OEBPS/images/a b.png")
		);
		assert_eq!(
			resolve_href("OEBPS", "./ch1.xhtml#part2").as_deref(),
			Some("OEBPS/ch1.xhtml")
		);
		assert_eq!(resolve_href("OEBPS", "https://example.com/a.png"), None);
		assert_eq!(
			resolve_href("OEBPS", "images/plot.svg?rev=2").as_deref(),
			Some("OEBPS/images/plot.svg"),
			"a query string is not part of the archive entry name"
		);
		assert_eq!(resolve_href("OEBPS", "//cdn.example.com/a.png"), None);
		assert_eq!(resolve_href("OEBPS", "../../escape.png"), None);
	}

	#[test]
	fn registered_in_the_tool_registry() {
		let tool = crate::find(ID).expect("epub-check is registered");
		assert_eq!(tool.id(), ID);
	}
}
