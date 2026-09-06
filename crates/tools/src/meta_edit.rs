//! `meta-edit`: bulk-edit the metadata Stump reads out of comic archives and
//! e-books, then rename the files from the values it just wrote.
//!
//! Behaviour provenance (read, never copied — MangaManager is GPL-3.0): the
//! Kavita external-tools guide lists MangaManager as the GUI that edits
//! `ComicInfo.xml` metadata in bulk and renames files from that metadata
//! (<https://wiki.kavitareader.com/guides/external-tools/>). The element names
//! are the ComicInfo v2.0 schema's
//! (<https://anansi-project.github.io/docs/comicinfo/schemas/v2.0>) and Dublin
//! Core as EPUB 3.3 uses it
//! (<https://www.w3.org/TR/epub-33/#sec-opf-dcmes-optional>); series and series
//! index live in calibre's `<meta name="calibre:series">` pair, which is what
//! `crate::calibre::parse_opf` already reads back.
//!
//! Values are set, never cleared, and they travel as strings so `3.5` and
//! `007` survive a round trip; `volume`/`year` must still parse as integers
//! and `number` as a decimal, because those elements are typed.
//!
//! A container is rewritten event by event, so the declaration, comments,
//! unknown elements, attribute order and indentation of a hand-made file all
//! survive — the same rule [`crate::epub_check`] follows — and the new bytes
//! go back through [`util::replace_archive_entry`], which raw-copies every
//! other member. A page, a content document, a font or a style sheet therefore
//! cannot change. The current value of a field is read from the element that
//! stores it (not from a normalised model), so a reported `from`/`to` pair is
//! about bytes in the file and "nothing to change" means exactly that.
//!
//! Renaming renders `pattern` from the *post-edit* metadata: `{series}`,
//! `{number}`/`{chapter}`, `{volume}`, `{title}`, `{writer}`, `{publisher}`,
//! `{year}`, `{language}`, each optionally zero-padded with `:0N` (the integer
//! part is padded and a decimal is kept, so `{volume:02}` renders `3.5` as
//! `03.5`). The pattern is split on whitespace and any chunk holding a
//! placeholder with no value is dropped whole, so `v{volume:02}` leaves no
//! orphan `v`; a chunk with no letters or digits (`-`) then survives only
//! between two chunks that have some, which is what keeps ` - ` from ending up
//! leading, trailing or doubled. The result goes through
//! [`util::sanitize_file_stem`], keeps the original extension, and never
//! leaves the file's own directory.
//!
//! See `crates/tools/README.md` for the option table.

use std::{
	borrow::Cow,
	collections::BTreeSet,
	fs::File,
	io::Read,
	path::{Path, PathBuf},
};

use quick_xml::{
	escape::unescape,
	events::{BytesEnd, BytesStart, BytesText, Event},
	Reader, Writer,
};
use serde::{Deserialize, Serialize};
use stump_media::ProcessedMediaMetadata;
use zip::{CompressionMethod, ZipArchive};

use crate::{
	util, Action, Plan, ProgressSink, Report, Severity, Tool, ToolError, ToolInput,
	ToolResult, Warning,
};

/// The tool id, also its `stump tools <id>` subcommand name.
pub const ID: &str = "meta-edit";
/// A metadata edit, with or without a rename.
pub const ACTION_EDIT: &str = "edit-metadata";
/// A rename with no metadata change.
pub const ACTION_RENAME: &str = "rename";
/// The default rename pattern: `Series v01 c003 - Title`, the naming `cbzit`
/// writes and the scanner parses back.
pub const DEFAULT_PATTERN: &str = "{series} v{volume:02} c{chapter:03} - {title}";

const CONTAINER_ENTRY: &str = "META-INF/container.xml";
const OPF_MEDIA_TYPE: &str = "application/oebps-package+xml";
const DC_NAMESPACE: &str = "http://purl.org/dc/elements/1.1/";
const CALIBRE_SERIES: &str = "calibre:series";
const CALIBRE_SERIES_INDEX: &str = "calibre:series_index";
/// Widest zero-padding a placeholder may ask for.
const MAX_PAD_WIDTH: usize = 16;

/// Stable warning codes: callers filter on these, never on prose.
pub mod codes {
	pub const NO_BOOKS: &str = "no-books";
	pub const NOT_A_FILE: &str = "not-a-file";
	pub const UNSUPPORTED_FORMAT: &str = "unsupported-format";
	pub const UNREADABLE: &str = "unreadable";
	pub const OPF_MISSING: &str = "opf-missing";
	pub const OPF_UNPARSABLE: &str = "opf-unparsable";
	pub const FIELD_UNSUPPORTED: &str = "field-unsupported";
	pub const RENAME_TARGET_EXISTS: &str = "rename-target-exists";
	pub const NO_CHANGES: &str = "no-changes";
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// `meta-edit` options.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct MetaEditOptions {
	/// The values to write. A field left out is a field this run has no
	/// opinion about.
	pub set: FieldSet,
	/// Rename every file from its post-edit metadata.
	pub rename_from_metadata: bool,
	/// The rename pattern; see the module docs for the placeholders.
	pub pattern: String,
	/// Descend into subdirectories of a given folder.
	pub recursive: bool,
	/// Replace a rename target that already exists.
	pub overwrite: bool,
}

impl Default for MetaEditOptions {
	fn default() -> Self {
		Self {
			set: FieldSet::default(),
			rename_from_metadata: false,
			pattern: DEFAULT_PATTERN.to_string(),
			recursive: false,
			overwrite: false,
		}
	}
}

/// The metadata values to write. Numbers are strings on the wire so `3.5` and
/// `007` survive a round trip.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct FieldSet {
	/// `Series` / `<meta name="calibre:series">`.
	pub series: Option<String>,
	/// `Number` / `<meta name="calibre:series_index">`; may be a decimal.
	pub number: Option<String>,
	/// `Volume`; an integer, and a `ComicInfo.xml` field only.
	pub volume: Option<String>,
	/// `Title` / `dc:title`.
	pub title: Option<String>,
	/// `Writer` / `dc:creator`.
	pub writer: Option<String>,
	/// `Publisher` / `dc:publisher`.
	pub publisher: Option<String>,
	/// `Year` / the `YYYY` of `dc:date`; an integer.
	pub year: Option<String>,
	/// `LanguageISO` / `dc:language`.
	pub language: Option<String>,
	/// `Tags` / one `dc:subject` per tag. Replaces the file's tag list
	/// wholesale; empty means "no opinion".
	pub tags: Vec<String>,
}

impl FieldSet {
	/// The fields this run sets, trimmed, validated, in [`Field`] order.
	fn wanted(&self) -> ToolResult<Vec<(Field, String)>> {
		let mut wanted = Vec::new();
		for (field, value) in [
			(Field::Series, self.series.as_deref()),
			(Field::Number, self.number.as_deref()),
			(Field::Volume, self.volume.as_deref()),
			(Field::Title, self.title.as_deref()),
			(Field::Writer, self.writer.as_deref()),
			(Field::Publisher, self.publisher.as_deref()),
			(Field::Year, self.year.as_deref()),
			(Field::Language, self.language.as_deref()),
		] {
			let Some(value) = value.map(str::trim) else {
				continue;
			};
			if value.is_empty() {
				return Err(ToolError::Options(format!(
					"{} is empty: meta-edit sets values, it does not clear them",
					field.as_str()
				)));
			}
			validate(field, value)?;
			wanted.push((field, value.to_string()));
		}

		// One comma-separated element, in the operator's own order, matching
		// how `stump_media` splits `Tags`.
		let tags = self
			.tags
			.iter()
			.map(|tag| tag.trim())
			.filter(|tag| !tag.is_empty())
			.collect::<Vec<_>>();
		if !tags.is_empty() {
			wanted.push((Field::Tags, tags.join(", ")));
		}

		Ok(wanted)
	}
}

/// Reject a value the target element cannot store, at plan time, for the whole
/// run: a typo must not write `Volume` twice and fail on the third book.
fn validate(field: Field, value: &str) -> ToolResult<()> {
	let (ok, expected) = match field {
		Field::Volume | Field::Year => (value.parse::<i32>().is_ok(), "an integer"),
		Field::Number => (is_decimal(value), "a decimal number"),
		_ => (true, ""),
	};
	if ok {
		return Ok(());
	}
	Err(ToolError::Options(format!(
		"{} must be {expected}: {value:?}",
		field.as_str()
	)))
}

/// `[+-]?digits[.digits]`. Deliberately not `f64::from_str`, which accepts
/// `inf` and `NaN` and would happily write them into a `Number` element.
fn is_decimal(value: &str) -> bool {
	let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
	let mut parts = digits.split('.');
	let integer = parts.next().unwrap_or_default();
	let fraction = parts.next();
	let all_digits = |text: &str| text.bytes().all(|byte| byte.is_ascii_digit());

	parts.next().is_none()
		&& !integer.is_empty()
		&& all_digits(integer)
		&& fraction.is_none_or(|fraction| !fraction.is_empty() && all_digits(fraction))
}

// ---------------------------------------------------------------------------
// Fields
// ---------------------------------------------------------------------------

/// One editable metadata field. The declaration order is the order changes are
/// reported and missing elements are inserted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
	/// Series name.
	Series,
	/// Chapter/issue number, also the calibre series index.
	Number,
	/// Volume number; `ComicInfo.xml` only.
	Volume,
	/// Title of this file.
	Title,
	/// Writer/author.
	Writer,
	/// Publisher.
	Publisher,
	/// Publication year.
	Year,
	/// Language tag.
	Language,
	/// Free-form tag list.
	Tags,
}

impl Field {
	/// The option key and warning-message name of this field.
	pub fn as_str(self) -> &'static str {
		match self {
			Field::Series => "series",
			Field::Number => "number",
			Field::Volume => "volume",
			Field::Title => "title",
			Field::Writer => "writer",
			Field::Publisher => "publisher",
			Field::Year => "year",
			Field::Language => "language",
			Field::Tags => "tags",
		}
	}
}

/// Which container a file's metadata lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Container {
	/// A CBZ/ZIP archive's root `ComicInfo.xml`.
	ComicInfo,
	/// An EPUB's package document.
	Opf,
}

impl Container {
	fn label(self) -> &'static str {
		match self {
			Container::ComicInfo => "ComicInfo.xml",
			Container::Opf => "the package document",
		}
	}
}

/// The current value of every editable field, as text the container holds.
#[derive(Debug, Clone, Default, PartialEq)]
struct Values {
	series: Option<String>,
	number: Option<String>,
	volume: Option<String>,
	title: Option<String>,
	writer: Option<String>,
	publisher: Option<String>,
	year: Option<String>,
	language: Option<String>,
	tags: Option<String>,
}

impl Values {
	fn slot(&mut self, field: Field) -> &mut Option<String> {
		match field {
			Field::Series => &mut self.series,
			Field::Number => &mut self.number,
			Field::Volume => &mut self.volume,
			Field::Title => &mut self.title,
			Field::Writer => &mut self.writer,
			Field::Publisher => &mut self.publisher,
			Field::Year => &mut self.year,
			Field::Language => &mut self.language,
			Field::Tags => &mut self.tags,
		}
	}

	fn get(&self, field: Field) -> Option<&str> {
		match field {
			Field::Series => self.series.as_deref(),
			Field::Number => self.number.as_deref(),
			Field::Volume => self.volume.as_deref(),
			Field::Title => self.title.as_deref(),
			Field::Writer => self.writer.as_deref(),
			Field::Publisher => self.publisher.as_deref(),
			Field::Year => self.year.as_deref(),
			Field::Language => self.language.as_deref(),
			Field::Tags => self.tags.as_deref(),
		}
	}

	fn set(&mut self, field: Field, value: String) {
		*self.slot(field) = Some(value);
	}

	/// First value in document order wins, so a duplicated element cannot
	/// change what the diff compares against.
	fn fill(&mut self, field: Field, value: String) {
		let slot = self.slot(field);
		if slot.is_none() {
			*slot = Some(value);
		}
	}
}

// ---------------------------------------------------------------------------
// Plan detail
// ---------------------------------------------------------------------------

/// Everything `apply` needs for one file. A plan is self-contained by
/// contract: `apply` never sees the options, so the container, the entry to
/// rewrite, the exact per-field edits, the rename and the `overwrite` license
/// all live here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditDetail {
	pub container: Container,
	/// The archive entry the edit rewrites.
	pub entry: String,
	/// Only the fields whose stored value actually differs, in [`Field`]
	/// order.
	pub changes: Vec<FieldChange>,
	/// The new path, when the file is also renamed.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub rename: Option<PathBuf>,
	/// Whether an existing rename target may be replaced.
	#[serde(default)]
	pub overwrite: bool,
}

/// One field's stored value and the value replacing it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldChange {
	pub field: Field,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub from: Option<String>,
	pub to: String,
}

// ---------------------------------------------------------------------------
// Tool
// ---------------------------------------------------------------------------

/// Bulk metadata editor and metadata-driven renamer.
pub struct MetaEdit;

impl Tool for MetaEdit {
	fn id(&self) -> &'static str {
		ID
	}

	fn describe(&self) -> &'static str {
		"Bulk-edit ComicInfo.xml and EPUB package metadata, and rename files from it"
	}

	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError> {
		let options = input.parse_options::<MetaEditOptions>()?;
		if input.paths.is_empty() {
			return Err(ToolError::Invalid("no input paths given".to_string()));
		}
		let wanted = options.set.wanted()?;
		let pattern = Pattern::parse(&options.pattern)?;
		if wanted.is_empty() && !options.rename_from_metadata {
			return Err(ToolError::Options(
				"nothing to do: set at least one field or enable rename_from_metadata"
					.to_string(),
			));
		}

		let mut plan = Plan::new(ID);
		let targets = collect_targets(&input.paths, options.recursive, &mut plan)?;
		if targets.is_empty() {
			plan.warn(
				Warning::new(codes::NO_BOOKS, "no CBZ or EPUB files in the given paths")
					.with_severity(Severity::Error),
			);
			return Ok(plan);
		}

		let mut claimed = BTreeSet::new();
		for target in targets {
			plan_file(
				&target,
				&options,
				&wanted,
				&pattern,
				&mut claimed,
				&mut plan,
			)?;
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
			if action.kind != ACTION_EDIT && action.kind != ACTION_RENAME {
				report.skipped(
					action.clone(),
					format!("unsupported action kind {:?}", action.kind),
				);
				continue;
			}
			let Some(source) = action.source.clone() else {
				report.skipped(action.clone(), "action has no source");
				continue;
			};
			if !source.is_file() {
				// A plan can be minutes old; never resurrect a moved book.
				report.skipped(
					action.clone(),
					format!("{} is no longer a file", source.display()),
				);
				continue;
			}
			let detail = match serde_json::from_value::<EditDetail>(action.detail.clone())
			{
				Ok(detail) => detail,
				Err(error) => {
					report.skipped(action.clone(), format!("unreadable detail: {error}"));
					continue;
				},
			};
			// Both re-checks happen before anything is written, so a
			// collision that appeared since the plan leaves the file exactly
			// as it was rather than half-edited.
			if let Some(target) = &detail.rename {
				if *target != source && target.exists() && !detail.overwrite {
					report.skipped(
						action.clone(),
						format!("{} now exists", target.display()),
					);
					continue;
				}
			}
			sink.progress(index, total, &format!("editing {}", source.display()));

			if !detail.changes.is_empty() {
				if let Err(error) = write_metadata(&source, &detail) {
					report.skipped(action.clone(), error.to_string());
					continue;
				}
			}
			if let Some(target) = &detail.rename {
				if let Err(error) = std::fs::rename(&source, target) {
					report.skipped(
						action.clone(),
						format!("metadata written, rename failed: {error}"),
					);
					continue;
				}
			}
			report.applied(action.clone());
		}
		sink.progress(total, total, "meta-edit complete");

		Ok(report)
	}
}

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

/// The books to edit. A directory contributes only the files this tool can
/// write; an explicitly named file that it cannot is reported, because the
/// caller asked for it by name.
fn collect_targets(
	paths: &[PathBuf],
	recursive: bool,
	plan: &mut Plan,
) -> ToolResult<Vec<PathBuf>> {
	if paths.is_empty() {
		return Err(ToolError::Invalid("no input paths".to_string()));
	}

	let mut targets = Vec::new();
	for path in paths {
		if path.is_dir() {
			targets.extend(
				util::sorted_files(path, recursive)?
					.into_iter()
					.filter(|path| container_of(path).is_some()),
			);
			continue;
		}
		if !path.is_file() {
			plan.warn(
				Warning::new(
					codes::NOT_A_FILE,
					format!("{} is not a file", path.display()),
				)
				.at(path),
			);
			continue;
		}
		if container_of(path).is_none() {
			plan.warn(
				Warning::new(
					codes::UNSUPPORTED_FORMAT,
					format!("{} is not a CBZ or EPUB", path.display()),
				)
				.at(path),
			);
			continue;
		}
		targets.push(path.clone());
	}

	Ok(targets)
}

/// Which container a path's metadata lives in, by extension. `.cbr` is absent
/// on purpose: RAR is read-only here.
fn container_of(path: &Path) -> Option<Container> {
	let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
	match extension.as_str() {
		"cbz" | "zip" => Some(Container::ComicInfo),
		"epub" => Some(Container::Opf),
		_ => None,
	}
}

/// What one file's metadata container holds right now.
struct State {
	/// The archive entry an edit rewrites.
	entry: String,
	/// The stored value of every field.
	values: Values,
	/// Fields this container cannot store.
	unsupported: &'static [Field],
}

/// Why one file was skipped at plan time.
struct Skip {
	code: &'static str,
	message: String,
}

impl Skip {
	fn new(code: &'static str, message: impl Into<String>) -> Self {
		Self {
			code,
			message: message.into(),
		}
	}
}

fn plan_file(
	path: &Path,
	options: &MetaEditOptions,
	wanted: &[(Field, String)],
	pattern: &Pattern,
	claimed: &mut BTreeSet<PathBuf>,
	plan: &mut Plan,
) -> ToolResult<()> {
	let Some(container) = container_of(path) else {
		return Ok(());
	};
	let state = match container {
		Container::ComicInfo => read_comic_info(path),
		Container::Opf => read_opf(path),
	};
	let state = match state {
		Ok(state) => state,
		Err(skip) => {
			plan.warn(Warning::new(skip.code, skip.message).at(path));
			return Ok(());
		},
	};

	let mut changes = Vec::new();
	for (field, value) in wanted {
		if state.unsupported.contains(field) {
			plan.warn(
				Warning::new(
					codes::FIELD_UNSUPPORTED,
					format!(
						"{} cannot store {}; left unchanged",
						container.label(),
						field.as_str()
					),
				)
				.at(path),
			);
			continue;
		}
		let from = state.values.get(*field).map(str::to_string);
		if from.as_deref() == Some(value.as_str()) {
			continue;
		}
		changes.push(FieldChange {
			field: *field,
			from,
			to: value.clone(),
		});
	}

	// The rename is rendered from what the file will hold, not what it holds.
	let mut after = state.values.clone();
	for change in &changes {
		after.set(change.field, change.to.clone());
	}

	let mut rename = None;
	let mut blocked = false;
	if options.rename_from_metadata {
		if let Some(target) = rename_target(path, pattern, &after) {
			// `claimed` catches two books in one run rendering to one name,
			// which no `exists` check can see yet.
			if target.exists() || claimed.contains(&target) {
				if options.overwrite {
					rename = Some(target);
				} else {
					plan.warn(
						Warning::new(
							codes::RENAME_TARGET_EXISTS,
							format!("{} already exists", target.display()),
						)
						.at(path),
					);
					blocked = true;
				}
			} else {
				rename = Some(target);
			}
		}
	}
	if let Some(target) = &rename {
		claimed.insert(target.clone());
	}

	if changes.is_empty() && rename.is_none() {
		if !blocked {
			plan.warn(
				Warning::new(
					codes::NO_CHANGES,
					format!("{} already holds these values", path.display()),
				)
				.at(path)
				.with_severity(Severity::Info),
			);
		}
		return Ok(());
	}

	let kind = if changes.is_empty() {
		ACTION_RENAME
	} else {
		ACTION_EDIT
	};
	let detail = EditDetail {
		container,
		entry: state.entry,
		changes,
		rename: rename.clone(),
		overwrite: options.overwrite,
	};
	plan.push(
		Action::new(kind)
			.with_source(path)
			.with_target(rename.unwrap_or_else(|| path.to_path_buf()))
			.with_detail(serde_json::to_value(&detail)?),
	);

	Ok(())
}

/// The path a rename would produce: the rendered stem plus the original
/// extension, always in the file's own directory. `None` when the pattern
/// renders to nothing or to the name the file already has.
fn rename_target(path: &Path, pattern: &Pattern, values: &Values) -> Option<PathBuf> {
	let stem = pattern.render(values)?;
	let name = match path.extension() {
		Some(extension) => format!("{stem}.{}", extension.to_string_lossy()),
		None => stem,
	};
	let target = path.with_file_name(name);
	(target != *path).then_some(target)
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Read the archive's root `ComicInfo.xml`.
///
/// The entry name is matched exactly, so a `__MACOSX/ComicInfo.xml` resource
/// fork or a hidden `.ComicInfo.xml` is never read and never written: Stump's
/// processors only import the root entry.
fn read_comic_info(path: &Path) -> Result<State, Skip> {
	let entry = util::COMIC_INFO_ENTRY.to_string();
	let xml = entry_text(path, util::COMIC_INFO_ENTRY)
		.map_err(|error| Skip::new(codes::UNREADABLE, error.to_string()))?;
	// No `ComicInfo.xml` at all: every set field is new, and `apply`
	// generates a schema-ordered document.
	let Some(xml) = xml else {
		return Ok(State {
			entry,
			values: Values::default(),
			unsupported: &[],
		});
	};

	// The reader Stump itself uses decides whether a document is usable: a
	// file Stump cannot import is one this tool refuses to rewrite.
	let readable = quick_xml::de::from_str::<ProcessedMediaMetadata>(without_bom(&xml));
	if let Err(error) = readable {
		return Err(Skip::new(
			codes::UNREADABLE,
			format!("ComicInfo.xml is not readable by Stump: {error}"),
		));
	}

	let children = children_of(&xml, None)
		.map_err(|error| Skip::new(codes::UNREADABLE, error.to_string()))?
		.ok_or_else(|| {
			Skip::new(codes::UNREADABLE, "ComicInfo.xml has no root element")
		})?;

	let mut values = Values::default();
	let mut language_iso = None;
	for child in children {
		let Some(text) = child.text else {
			continue;
		};
		match child.name.as_str() {
			"Series" => values.fill(Field::Series, text),
			"Number" => values.fill(Field::Number, text),
			"Volume" => values.fill(Field::Volume, text),
			"Title" => values.fill(Field::Title, text),
			"Writer" => values.fill(Field::Writer, text),
			"Publisher" => values.fill(Field::Publisher, text),
			"Year" => values.fill(Field::Year, text),
			"Tags" => values.fill(Field::Tags, text),
			// `Language` is `stump_media`'s own alias, not a v2.0 element;
			// the schema element wins when a file carries both.
			"Language" => values.fill(Field::Language, text),
			"LanguageISO" if language_iso.is_none() => language_iso = Some(text),
			_ => {},
		}
	}
	if let Some(language) = language_iso {
		values.set(Field::Language, language);
	}

	Ok(State {
		entry,
		values,
		unsupported: &[],
	})
}

fn read_opf(path: &Path) -> Result<State, Skip> {
	let unreadable = |error: ToolError| Skip::new(codes::UNREADABLE, error.to_string());

	let container = entry_text(path, CONTAINER_ENTRY)
		.map_err(unreadable)?
		.ok_or_else(|| Skip::new(codes::OPF_MISSING, "no META-INF/container.xml"))?;
	let entry = rootfile_path(&container)
		.map_err(|error| {
			Skip::new(
				codes::OPF_UNPARSABLE,
				format!("META-INF/container.xml: {error}"),
			)
		})?
		.ok_or_else(|| {
			Skip::new(codes::OPF_MISSING, "container.xml declares no rootfile")
		})?;
	let xml = entry_text(path, &entry)
		.map_err(unreadable)?
		.ok_or_else(|| {
			Skip::new(codes::OPF_MISSING, format!("{entry} is not in the archive"))
		})?;

	let children = children_of(&xml, Some("metadata"))
		.map_err(|error| Skip::new(codes::OPF_UNPARSABLE, error.to_string()))?
		.ok_or_else(|| {
			Skip::new(
				codes::OPF_UNPARSABLE,
				format!("{entry} has no metadata element"),
			)
		})?;

	let mut values = Values::default();
	let mut creators = Vec::new();
	let mut subjects = Vec::new();
	for child in children {
		if child.name == "meta" {
			let name = attribute(&child.start, "name")
				.map_err(|error| Skip::new(codes::OPF_UNPARSABLE, error.to_string()))?;
			let content = attribute(&child.start, "content")
				.map_err(|error| Skip::new(codes::OPF_UNPARSABLE, error.to_string()))?
				.filter(|content| !content.is_empty());
			match (name.as_deref(), content) {
				(Some(CALIBRE_SERIES), Some(content)) => {
					values.fill(Field::Series, content)
				},
				(Some(CALIBRE_SERIES_INDEX), Some(content)) => {
					values.fill(Field::Number, content)
				},
				_ => {},
			}
			continue;
		}
		let Some(text) = child.text else {
			continue;
		};
		match child.name.as_str() {
			"title" => values.fill(Field::Title, text),
			"creator" => creators.push(text),
			"publisher" => values.fill(Field::Publisher, text),
			"language" => values.fill(Field::Language, text),
			"subject" => subjects.push(text),
			"date" => {
				if let Some(year) = leading_year(&text) {
					values.fill(Field::Year, year);
				}
			},
			_ => {},
		}
	}
	if !creators.is_empty() {
		values.fill(Field::Writer, creators.join(", "));
	}
	if !subjects.is_empty() {
		values.fill(Field::Tags, subjects.join(", "));
	}

	Ok(State {
		entry,
		values,
		// A package document has no volume: EPUB series numbering is the
		// calibre series index, which is `number`.
		unsupported: &[Field::Volume],
	})
}

/// The `YYYY` of an OPF date, which is all a `year` field is about; the rest
/// of a `dc:date` is left alone unless the year itself changes.
fn leading_year(date: &str) -> Option<String> {
	let year = date.split(['-', 'T', ' ']).next()?;
	(year.len() == 4 && year.bytes().all(|byte| byte.is_ascii_digit()))
		.then(|| year.to_string())
}

/// The `full-path` of the first `<rootfile>` with the package media type, or
/// of the first `<rootfile>` at all when none declares it.
fn rootfile_path(xml: &str) -> Result<Option<String>, quick_xml::Error> {
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
		let full_path = normalize_entry(&full_path);
		if attribute(&element, "media-type")?.as_deref() == Some(OPF_MEDIA_TYPE) {
			return Ok(Some(full_path));
		}
		fallback.get_or_insert(full_path);
	}
	Ok(fallback)
}

/// Collapse `.` and `..` so a `full-path` of `./OEBPS/content.opf` still names
/// the archive member `OEBPS/content.opf`.
fn normalize_entry(path: &str) -> String {
	let mut parts: Vec<&str> = Vec::new();
	for part in path.trim().split('/') {
		match part {
			"" | "." => {},
			".." => {
				parts.pop();
			},
			part => parts.push(part),
		}
	}
	parts.join("/")
}

/// The UTF-8 text of one archive entry, or `None` when it is absent.
fn entry_text(path: &Path, entry: &str) -> ToolResult<Option<String>> {
	let mut archive = ZipArchive::new(File::open(path)?)?;
	let mut member = match archive.by_name(entry) {
		Ok(member) => member,
		Err(zip::result::ZipError::FileNotFound) => return Ok(None),
		Err(error) => return Err(error.into()),
	};
	let mut bytes = Vec::new();
	member.read_to_end(&mut bytes)?;

	String::from_utf8(bytes).map(Some).map_err(|error| {
		ToolError::Invalid(format!("{entry} is not valid UTF-8: {error}"))
	})
}

/// A byte order mark is legal in front of an XML declaration but chokes a
/// serde model; the rewrite keeps it, so it is only stripped for parsing.
fn without_bom(xml: &str) -> &str {
	xml.strip_prefix('\u{feff}').unwrap_or(xml)
}

/// One direct child element of a metadata container.
struct Child {
	/// Local name, so `dc:title` and `title` are one thing.
	name: String,
	/// The start tag, for attribute reads.
	start: BytesStart<'static>,
	/// Trimmed, unescaped text content.
	text: Option<String>,
}

/// Every direct child of `container` (the root element when `None`), in
/// document order. `None` means the container element is not in the document.
///
/// quick-xml reports every entity reference as its own `GeneralRef` event, so
/// the text is reassembled before a single unescape pass — the rule
/// `calibre::parse_opf` already follows, without which `a &amp; b` would
/// silently truncate at the ampersand.
fn children_of(xml: &str, container: Option<&str>) -> ToolResult<Option<Vec<Child>>> {
	let mut reader = Reader::from_str(xml);
	let mut stack: Vec<String> = Vec::new();
	let mut children = Vec::new();
	let mut found = false;
	// The child currently collecting text, and the depth it opened at.
	let mut open: Option<(String, BytesStart<'static>, usize)> = None;
	let mut raw = String::new();

	loop {
		match reader.read_event()? {
			Event::Start(start) => {
				let name = local_string(start.name().as_ref());
				found |= is_container_start(&stack, container, &name);
				if open.is_none() && is_child_of(&stack, container) {
					open = Some((name.clone(), start.into_owned(), stack.len()));
					raw.clear();
				}
				stack.push(name);
			},
			Event::Empty(empty) => {
				let name = local_string(empty.name().as_ref());
				if is_child_of(&stack, container) {
					children.push(Child {
						name,
						start: empty.into_owned(),
						text: None,
					});
				}
			},
			Event::Text(text) if open.is_some() => {
				raw.push_str(&String::from_utf8_lossy(text.as_ref()));
			},
			Event::GeneralRef(reference) if open.is_some() => {
				raw.push('&');
				raw.push_str(&String::from_utf8_lossy(reference.as_ref()));
				raw.push(';');
			},
			Event::End(_) => {
				stack.pop();
				let closed = open
					.as_ref()
					.is_some_and(|(_, _, depth)| *depth == stack.len());
				if closed {
					let (name, start, _) = open.take().expect("checked above");
					children.push(Child {
						name,
						start,
						text: decode_text(&raw),
					});
					raw.clear();
				}
			},
			Event::Eof => break,
			_ => {},
		}
	}

	Ok(found.then_some(children))
}

/// True when an element opening at this stack depth is a direct child of the
/// container being edited.
fn is_child_of(stack: &[String], container: Option<&str>) -> bool {
	match container {
		Some(container) => stack.last().map(String::as_str) == Some(container),
		None => stack.len() == 1,
	}
}

/// True when the element `name` opening at this stack depth *is* the
/// container: the root element, or the named one wherever it sits.
fn is_container_start(stack: &[String], container: Option<&str>, name: &str) -> bool {
	match container {
		Some(container) => name == container,
		None => stack.is_empty(),
	}
}

/// True when this `End` closes the container: the stack still holds the
/// container itself, and nothing above it.
fn is_container_end(stack: &[String], container: Option<&str>, name: &str) -> bool {
	match container {
		Some(container) => name == container,
		None => stack.len() == 1,
	}
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

fn write_metadata(source: &Path, detail: &EditDetail) -> ToolResult<()> {
	match detail.container {
		Container::ComicInfo => {
			let bytes = match entry_text(source, &detail.entry)? {
				Some(xml) => {
					let mut edits = comic_info_edits(&detail.changes);
					rewrite(&xml, None, &mut edits, &mut [])?
				},
				None => generated_comic_info(&detail.changes).into_bytes(),
			};
			// Stored, like every `ComicInfo.xml` Stump writes.
			util::replace_archive_entry(
				source,
				&detail.entry,
				&bytes,
				CompressionMethod::Stored,
			)
		},
		Container::Opf => {
			let Some(xml) = entry_text(source, &detail.entry)? else {
				return Err(ToolError::Invalid(format!(
					"{} is no longer in {}",
					detail.entry,
					source.display()
				)));
			};
			let naming = DcNaming::detect(&xml)?;
			let (mut edits, mut metas) = opf_edits(&detail.changes, &naming);
			let bytes = rewrite(&xml, Some("metadata"), &mut edits, &mut metas)?;
			util::replace_archive_entry(
				source,
				&detail.entry,
				&bytes,
				CompressionMethod::Deflated,
			)
		},
	}
}

/// One element a rewrite owns: matched on `local`, written from `template`.
struct ElementEdit {
	local: &'static str,
	/// The start tag used when the element has to be written from scratch.
	template: BytesStart<'static>,
	/// One element per value, so a tag list becomes one element each.
	values: Vec<String>,
	/// Insert before the container's closing tag when absent. A non-schema
	/// alias is patched when present but never introduced.
	insert: bool,
	written: bool,
}

impl ElementEdit {
	fn new(local: &'static str, template: BytesStart<'static>, value: String) -> Self {
		Self {
			local,
			template,
			values: vec![value],
			insert: true,
			written: false,
		}
	}
}

/// A `<meta name=... content=...>` pair a rewrite owns.
struct MetaPair {
	name: &'static str,
	content: String,
	written: bool,
}

fn comic_info_edits(changes: &[FieldChange]) -> Vec<ElementEdit> {
	let mut edits = Vec::with_capacity(changes.len());
	for change in changes {
		let element = |name: &'static str| {
			ElementEdit::new(name, BytesStart::new(name), change.to.clone())
		};
		match change.field {
			Field::Series => edits.push(element("Series")),
			Field::Number => edits.push(element("Number")),
			Field::Volume => edits.push(element("Volume")),
			Field::Title => edits.push(element("Title")),
			Field::Writer => edits.push(element("Writer")),
			Field::Publisher => edits.push(element("Publisher")),
			Field::Year => edits.push(element("Year")),
			Field::Tags => edits.push(element("Tags")),
			Field::Language => {
				edits.push(element("LanguageISO"));
				// Patch `stump_media`'s alias too when a file has it, so the
				// document cannot end up naming two different languages.
				let mut alias = element("Language");
				alias.insert = false;
				edits.push(alias);
			},
		}
	}
	edits
}

fn opf_edits(
	changes: &[FieldChange],
	naming: &DcNaming,
) -> (Vec<ElementEdit>, Vec<MetaPair>) {
	let mut edits = Vec::new();
	let mut metas = Vec::new();
	for change in changes {
		let dc = |local: &'static str, values: Vec<String>| ElementEdit {
			local,
			template: naming.template(local),
			values,
			insert: true,
			written: false,
		};
		match change.field {
			Field::Title => edits.push(dc("title", vec![change.to.clone()])),
			Field::Writer => edits.push(dc("creator", vec![change.to.clone()])),
			Field::Publisher => edits.push(dc("publisher", vec![change.to.clone()])),
			Field::Language => edits.push(dc("language", vec![change.to.clone()])),
			// The bare year: a `dc:date` is only rewritten when the year the
			// field stands for actually changes.
			Field::Year => edits.push(dc("date", vec![change.to.clone()])),
			Field::Tags => edits.push(dc("subject", split_tags(&change.to))),
			Field::Series => metas.push(MetaPair {
				name: CALIBRE_SERIES,
				content: change.to.clone(),
				written: false,
			}),
			Field::Number => metas.push(MetaPair {
				name: CALIBRE_SERIES_INDEX,
				content: change.to.clone(),
				written: false,
			}),
			// Refused at plan time with a `field-unsupported` warning.
			Field::Volume => {},
		}
	}
	(edits, metas)
}

/// A generated `ComicInfo.xml` for an archive that has none, in schema
/// element order.
fn generated_comic_info(changes: &[FieldChange]) -> String {
	let mut info = util::ComicInfo::default();
	for change in changes {
		let value = change.to.clone();
		match change.field {
			Field::Series => info.series = Some(value),
			Field::Number => info.number = Some(value),
			Field::Volume => info.volume = value.parse().ok(),
			Field::Title => info.title = Some(value),
			Field::Writer => info.writers = vec![value],
			Field::Publisher => info.publisher = Some(value),
			Field::Year => info.year = value.parse().ok(),
			Field::Language => info.language = Some(value),
			Field::Tags => info.tags = split_tags(&value),
		}
	}
	info.to_xml()
}

fn split_tags(joined: &str) -> Vec<String> {
	joined
		.split(',')
		.map(|tag| tag.trim().to_string())
		.filter(|tag| !tag.is_empty())
		.collect()
}

/// Rewrite an XML metadata document, touching only the elements in `edits` and
/// the `<meta name=...>` pairs in `metas`, and inserting the ones the document
/// does not have yet immediately before the container's closing tag.
///
/// Everything else — the declaration, comments, unknown elements, attribute
/// order and indentation — is written back byte for byte. A repeated element a
/// rewrite owns is dropped after its first occurrence, so a field ends up
/// stored exactly once.
fn rewrite(
	xml: &str,
	container: Option<&str>,
	edits: &mut [ElementEdit],
	metas: &mut [MetaPair],
) -> ToolResult<Vec<u8>> {
	let mut reader = Reader::from_str(xml);
	let mut writer = Writer::new(Vec::with_capacity(xml.len() + 256));

	let mut stack: Vec<String> = Vec::new();
	let mut pending_ws: Option<BytesText<'static>> = None;
	let mut observed_indent: Option<BytesText<'static>> = None;
	let mut skip_depth: Option<usize> = None;

	loop {
		let event = reader.read_event()?;

		// Inside an element this rewrite replaced or dropped.
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
				if is_child_of(&stack, container) && observed_indent.is_none() {
					observed_indent = Some(text.clone().into_owned());
				}
				pending_ws = Some(text.into_owned());
			},
			Event::End(end)
				if is_container_end(
					&stack,
					container,
					&local_string(end.name().as_ref()),
				) =>
			{
				let indent = indent_or(&observed_indent, container);
				for edit in edits.iter_mut().filter(|edit| edit.insert && !edit.written) {
					for value in &edit.values {
						writer.write_event(Event::Text(indent.clone()))?;
						write_element(&mut writer, &edit.template, value)?;
					}
					edit.written = true;
				}
				for meta in metas.iter_mut().filter(|meta| !meta.written) {
					writer.write_event(Event::Text(indent.clone()))?;
					write_meta(&mut writer, meta.name, &meta.content)?;
					meta.written = true;
				}
				flush(&mut writer, &mut pending_ws)?;
				writer.write_event(Event::End(end))?;
				stack.pop();
			},
			Event::Start(start) => {
				let name = local_string(start.name().as_ref());
				if is_child_of(&stack, container) {
					if let Some(index) = edit_index(edits, &name) {
						write_values(
							&mut writer,
							&mut edits[index],
							Some(&start),
							&indent_or(&observed_indent, container),
							&mut pending_ws,
						)?;
						skip_depth = Some(1);
						continue;
					}
					if let Some(index) = meta_index(metas, &name, &start)? {
						if metas[index].written {
							pending_ws = None;
							skip_depth = Some(1);
							continue;
						}
						let mut patched = start;
						set_attribute(&mut patched, "content", &metas[index].content)?;
						metas[index].written = true;
						flush(&mut writer, &mut pending_ws)?;
						writer.write_event(Event::Start(patched))?;
						stack.push(name);
						continue;
					}
				}
				flush(&mut writer, &mut pending_ws)?;
				writer.write_event(Event::Start(start))?;
				stack.push(name);
			},
			Event::Empty(empty) => {
				let name = local_string(empty.name().as_ref());
				if is_child_of(&stack, container) {
					if let Some(index) = edit_index(edits, &name) {
						write_values(
							&mut writer,
							&mut edits[index],
							Some(&empty),
							&indent_or(&observed_indent, container),
							&mut pending_ws,
						)?;
						continue;
					}
					if let Some(index) = meta_index(metas, &name, &empty)? {
						if metas[index].written {
							pending_ws = None;
							continue;
						}
						let mut patched = empty;
						set_attribute(&mut patched, "content", &metas[index].content)?;
						metas[index].written = true;
						flush(&mut writer, &mut pending_ws)?;
						writer.write_event(Event::Empty(patched))?;
						continue;
					}
				}
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

/// Write an owned element's values in place of the document's own element, or
/// drop the element when the field was already written once.
fn write_values(
	writer: &mut Writer<Vec<u8>>,
	edit: &mut ElementEdit,
	original: Option<&BytesStart<'_>>,
	indent: &BytesText<'static>,
	pending: &mut Option<BytesText<'static>>,
) -> ToolResult<()> {
	if edit.written {
		// Swallow the duplicate together with its own indentation.
		*pending = None;
		return Ok(());
	}
	edit.written = true;
	flush(writer, pending)?;
	for (index, value) in edit.values.iter().enumerate() {
		if index > 0 {
			writer.write_event(Event::Text(indent.clone()))?;
		}
		// The first value keeps the document's own start tag, so attributes
		// such as `opf:role` or `xml:lang` are preserved.
		let start = match (index, original) {
			(0, Some(original)) => original,
			_ => &edit.template,
		};
		write_element(writer, start, value)?;
	}
	Ok(())
}

fn edit_index(edits: &[ElementEdit], name: &str) -> Option<usize> {
	edits.iter().position(|edit| edit.local == name)
}

fn meta_index(
	metas: &[MetaPair],
	name: &str,
	element: &BytesStart<'_>,
) -> Result<Option<usize>, quick_xml::Error> {
	if metas.is_empty() || name != "meta" {
		return Ok(None);
	}
	let Some(attribute) = attribute(element, "name")? else {
		return Ok(None);
	};
	Ok(metas.iter().position(|meta| meta.name == attribute))
}

/// `<name ...original attributes...>text</name>`.
fn write_element(
	writer: &mut Writer<Vec<u8>>,
	start: &BytesStart<'_>,
	text: &str,
) -> ToolResult<()> {
	let name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
	writer.write_event(Event::Start(start.clone()))?;
	writer.write_event(Event::Text(BytesText::new(text)))?;
	writer.write_event(Event::End(BytesEnd::new(name)))?;
	Ok(())
}

/// `<meta name=".." content=".."/>`, the empty-element form calibre writes and
/// `calibre::parse_opf` reads.
fn write_meta(writer: &mut Writer<Vec<u8>>, name: &str, content: &str) -> ToolResult<()> {
	let mut start = BytesStart::new("meta");
	start.push_attribute(("name", name));
	start.push_attribute(("content", content));
	writer.write_event(Event::Empty(start))?;
	Ok(())
}

/// The indentation an inserted element gets: the container's own, else a
/// sensible default for a document written on one line.
fn indent_or(
	observed: &Option<BytesText<'static>>,
	container: Option<&str>,
) -> BytesText<'static> {
	observed.clone().unwrap_or_else(|| match container {
		Some(_) => BytesText::from_escaped("\n\t\t"),
		None => BytesText::from_escaped("\n  "),
	})
}

fn flush(
	writer: &mut Writer<Vec<u8>>,
	pending: &mut Option<BytesText<'static>>,
) -> ToolResult<()> {
	if let Some(text) = pending.take() {
		writer.write_event(Event::Text(text))?;
	}
	Ok(())
}

/// The `dc` prefix (and whether the namespace still needs declaring) for
/// inserted Dublin Core elements.
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
		b"subject",
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

	fn template(&self, local: &str) -> BytesStart<'static> {
		let name = match self.prefix.as_deref() {
			Some(prefix) => format!("{prefix}:{local}"),
			None => local.to_string(),
		};
		let mut start = BytesStart::new(name).into_owned();
		if self.declare_namespace {
			start.push_attribute(("xmlns:dc", DC_NAMESPACE));
		}
		start
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

// ---------------------------------------------------------------------------
// Rename patterns
// ---------------------------------------------------------------------------

/// A parsed rename pattern: whitespace-delimited chunks of literal text and
/// placeholders. See the module docs for the rendering rule.
#[derive(Debug, Clone, PartialEq)]
struct Pattern {
	chunks: Vec<Chunk>,
}

#[derive(Debug, Clone, PartialEq)]
struct Chunk {
	pieces: Vec<Piece>,
}

#[derive(Debug, Clone, PartialEq)]
enum Piece {
	Literal(String),
	Placeholder { field: Field, width: usize },
}

impl Pattern {
	fn parse(pattern: &str) -> ToolResult<Self> {
		let chunks = pattern
			.split_whitespace()
			.map(Chunk::parse)
			.collect::<ToolResult<Vec<_>>>()?;
		Ok(Self { chunks })
	}

	/// Render `values` into a sanitized file stem, or `None` when nothing
	/// survives.
	fn render(&self, values: &Values) -> Option<String> {
		let rendered = self
			.chunks
			.iter()
			.filter_map(|chunk| chunk.render(values))
			.collect::<Vec<_>>();
		let separators = rendered
			.iter()
			.map(|text| !text.chars().any(char::is_alphanumeric))
			.collect::<Vec<_>>();

		let mut parts = Vec::with_capacity(rendered.len());
		for (index, text) in rendered.iter().enumerate() {
			// A separator only earns its place between two chunks that carry
			// something, which drops it when it is leading, trailing, or
			// next to another separator.
			if separators[index] {
				let before = index
					.checked_sub(1)
					.is_some_and(|previous| !separators[previous]);
				let after = separators.get(index + 1).is_some_and(|sep| !sep);
				if !(before && after) {
					continue;
				}
			}
			parts.push(text.as_str());
		}

		let joined = parts.join(" ");
		(!joined.trim().is_empty()).then(|| util::sanitize_file_stem(&joined))
	}
}

impl Chunk {
	fn parse(word: &str) -> ToolResult<Self> {
		let mut pieces = Vec::new();
		let mut rest = word;
		while let Some(open) = rest.find('{') {
			if open > 0 {
				pieces.push(Piece::Literal(rest[..open].to_string()));
			}
			let after = &rest[open + 1..];
			let Some(close) = after.find('}') else {
				return Err(ToolError::Options(format!(
					"unterminated placeholder in pattern: {word:?}"
				)));
			};
			pieces.push(Piece::parse(&after[..close])?);
			rest = &after[close + 1..];
		}
		if !rest.is_empty() {
			pieces.push(Piece::Literal(rest.to_string()));
		}
		Ok(Self { pieces })
	}

	/// `None` when the chunk holds a placeholder the file has no value for:
	/// the chunk goes away whole, so `v{volume:02}` leaves no orphan `v`.
	fn render(&self, values: &Values) -> Option<String> {
		let mut out = String::new();
		for piece in &self.pieces {
			match piece {
				Piece::Literal(text) => out.push_str(text),
				Piece::Placeholder { field, width } => {
					out.push_str(&pad(values.get(*field)?, *width));
				},
			}
		}
		Some(out)
	}
}

impl Piece {
	fn parse(spec: &str) -> ToolResult<Self> {
		let (name, width) = match spec.split_once(':') {
			Some((name, pad)) => (name, parse_width(pad, spec)?),
			None => (spec, 0),
		};
		let field = placeholder_field(name).ok_or_else(|| {
			ToolError::Options(format!("unknown placeholder {{{spec}}} in pattern"))
		})?;
		Ok(Piece::Placeholder { field, width })
	}
}

fn parse_width(pad: &str, spec: &str) -> ToolResult<usize> {
	let invalid = || {
		ToolError::Options(format!(
			"placeholder {{{spec}}} must pad with :0N, e.g. {{volume:02}}"
		))
	};
	if pad.is_empty() || !pad.bytes().all(|byte| byte.is_ascii_digit()) {
		return Err(invalid());
	}
	let width = pad.parse::<usize>().map_err(|_| invalid())?;
	if width > MAX_PAD_WIDTH {
		return Err(ToolError::Options(format!(
			"placeholder {{{spec}}} pads wider than {MAX_PAD_WIDTH}"
		)));
	}
	Ok(width)
}

/// `{chapter}` is an alias of `{number}`: the ComicInfo chapter/issue number
/// is the calibre series index.
fn placeholder_field(name: &str) -> Option<Field> {
	match name {
		"series" => Some(Field::Series),
		"number" | "chapter" => Some(Field::Number),
		"volume" => Some(Field::Volume),
		"title" => Some(Field::Title),
		"writer" => Some(Field::Writer),
		"publisher" => Some(Field::Publisher),
		"year" => Some(Field::Year),
		"language" => Some(Field::Language),
		_ => None,
	}
}

/// Zero-pad the integer part, keep any decimal: width 2 renders `3` as `03`
/// and `3.5` as `03.5`. A value that is not a number is left alone.
fn pad(value: &str, width: usize) -> String {
	let (integer, fraction) = match value.split_once('.') {
		Some((integer, fraction)) => (integer, Some(fraction)),
		None => (value, None),
	};
	if width == 0
		|| integer.len() >= width
		|| integer.is_empty()
		|| !integer.bytes().all(|byte| byte.is_ascii_digit())
	{
		return value.to_string();
	}

	let mut padded = "0".repeat(width - integer.len());
	padded.push_str(integer);
	if let Some(fraction) = fraction {
		padded.push('.');
		padded.push_str(fraction);
	}
	padded
}

// ---------------------------------------------------------------------------
// Small XML helpers
// ---------------------------------------------------------------------------

fn local_name(name: &[u8]) -> &[u8] {
	match name.iter().position(|byte| *byte == b':') {
		Some(index) => &name[index + 1..],
		None => name,
	}
}

fn local_string(name: &[u8]) -> String {
	String::from_utf8_lossy(local_name(name)).into_owned()
}

/// Unescape leniently: an exotic entity must not abort a bulk run.
fn decode(bytes: &[u8]) -> String {
	let raw = String::from_utf8_lossy(bytes);
	match unescape(&raw) {
		Ok(Cow::Owned(value)) => value,
		Ok(Cow::Borrowed(_)) | Err(_) => raw.into_owned(),
	}
}

/// Unescaped, trimmed element text; `None` when nothing is left.
fn decode_text(raw: &str) -> Option<String> {
	let decoded = match unescape(raw) {
		Ok(value) => value.into_owned(),
		Err(_) => raw.to_string(),
	};
	let trimmed = decoded.trim();
	(!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn attribute(
	element: &BytesStart<'_>,
	key: &str,
) -> Result<Option<String>, quick_xml::Error> {
	for attribute in element.attributes() {
		let attribute = attribute?;
		if local_name(attribute.key.as_ref()) == key.as_bytes() {
			return Ok(Some(decode(&attribute.value).trim().to_string()));
		}
	}
	Ok(None)
}

/// Replace `key`'s value, appending the attribute when it is absent. Every
/// other attribute keeps its original bytes and position.
fn set_attribute(
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

fn is_whitespace(bytes: &[u8]) -> bool {
	!bytes.is_empty() && bytes.iter().all(u8::is_ascii_whitespace)
}

#[cfg(test)]
mod tests;
