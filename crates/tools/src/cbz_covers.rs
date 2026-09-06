//! `cbz-covers`: front/back cover management for CBZ archives.
//!
//! Behaviour source: the Kavita community "CBZ Cover Manager" external tool
//! documented at <https://wiki.kavitareader.com/guides/external-tools/> — set a
//! front and/or back cover on a comic archive, drop the existing first/last
//! page, bulk-assign loose cover images to archives by volume number, and undo
//! a previous run. Only the *documented behaviour* is mirrored; the
//! implementation here is original.
//!
//! Two rules make the "undo" honest:
//!
//! 1. Entries this tool adds are named from a fixed, sort-stable pattern
//!    ([`FRONT_STEM`] / [`BACK_STEM`]) so Stump's own page ordering
//!    (`crates/media/src/media/format/zip.rs:194-229`, which
//!    alphanumeric-sorts entry names and takes the first image as page 1)
//!    picks them up as the first and last page.
//! 2. Every added entry is also recorded in [`MARKER_ENTRY`], a hidden JSON
//!    manifest inside the archive. It is hidden (leading `.`) so
//!    `PathUtils::is_hidden_file` keeps it out of the page count, and it is
//!    independent of `ComicInfo.xml` so metadata tools cannot clobber it.
//!
//! The option table, warning codes, and verification commands are in
//! `crates/tools/README.md`.

use std::{
	cmp::Ordering,
	collections::{BTreeMap, BTreeSet, HashMap, HashSet},
	fs::{self, File},
	io::{Read, Seek, SeekFrom, Write},
	path::{Path, PathBuf},
	sync::LazyLock,
};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

use crate::{
	util, Action, Plan, ProgressSink, Report, Severity, Tool, ToolError, ToolInput,
	ToolResult, Warning,
};

/// The registered tool id.
pub const TOOL_ID: &str = "cbz-covers";

/// Hidden manifest listing the entries this tool added, so `remove_added` can
/// strip exactly those and nothing else.
pub const MARKER_ENTRY: &str = ".stump-covers.json";

/// Front cover entry stem. `!` (0x21) sorts before digits and letters under
/// `alphanumeric_sort`, which is what Stump uses to order archive pages.
pub const FRONT_STEM: &str = "!000-cover";

/// Back cover entry stem; sorts after digit- and most letter-prefixed names.
pub const BACK_STEM: &str = "zzz-back";

const MARKER_VERSION: u32 = 1;

/// Volume tokens accepted by `auto_assign_dir`.
///
/// Mirrors the volume subset of the ingest filename grammar
/// (`core/src/ingest/quality/filename.rs:272-275`), including its
/// normalisation of `_`/`.` to spaces, so `v01`, `vol.7`, `vol 7` and
/// `volume 12` parse the same way here as they do during a scan. Longest
/// alternative first so `vol`/`volume` are not shadowed by `v`.
static VOLUME_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r"(?i)\b(?:volume|vol|v)\s*([0-9]+)\b")
		.expect("volume token regex is valid")
});

/// Options for [`CbzCovers`]. Every field is optional; see the option table in
/// `crates/tools/README.md`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CoverOptions {
	/// Image to insert as the first page.
	pub front: Option<PathBuf>,
	/// Image to insert as the last page.
	pub back: Option<PathBuf>,
	/// Drop the current first page.
	pub delete_first: bool,
	/// Drop the current last page.
	pub delete_last: bool,
	/// Strip the entries a previous run of this tool added.
	pub remove_added: bool,
	/// Directory of loose cover images to match to archives by volume number.
	pub auto_assign_dir: Option<PathBuf>,
}

/// Which end of the book a cover belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
	Front,
	Back,
}

impl Role {
	fn stem(self) -> &'static str {
		match self {
			Self::Front => FRONT_STEM,
			Self::Back => BACK_STEM,
		}
	}

	fn kind(self) -> &'static str {
		match self {
			Self::Front => "add-front-cover",
			Self::Back => "add-back-cover",
		}
	}

	fn from_kind(kind: &str) -> Option<Self> {
		match kind {
			"add-front-cover" => Some(Self::Front),
			"add-back-cover" => Some(Self::Back),
			_ => None,
		}
	}

	fn label(self) -> &'static str {
		match self {
			Self::Front => "front",
			Self::Back => "back",
		}
	}
}

/// The in-archive manifest of entries added by this tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Marker {
	version: u32,
	tool: String,
	added: Vec<AddedEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AddedEntry {
	/// Entry name inside the archive.
	name: String,
	role: Role,
	/// File name (not path) the entry came from, for provenance only.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	source: Option<String>,
}

/// CBZ front/back cover manager.
#[derive(Debug, Default, Clone, Copy)]
pub struct CbzCovers;

impl Tool for CbzCovers {
	fn id(&self) -> &'static str {
		TOOL_ID
	}

	fn describe(&self) -> &'static str {
		"Add or replace front/back covers on CBZ archives, drop the current first/last page, bulk-assign loose covers by volume number, and undo a previous run"
	}

	fn plan(&self, input: &ToolInput) -> ToolResult<Plan> {
		let options = input.parse_options::<CoverOptions>()?;
		if !options.wants_work() {
			return Err(ToolError::Invalid(
				"nothing to do: set at least one of front, back, delete_first, delete_last, remove_added, auto_assign_dir".to_string(),
			));
		}

		let front = validate_image(options.front.as_deref(), "front")?;
		let back = validate_image(options.back.as_deref(), "back")?;

		let mut plan = Plan::new(TOOL_ID);
		let archives = collect_archives(&input.paths, &mut plan)?;
		if archives.is_empty() {
			return Err(ToolError::Invalid(
				"no CBZ archives found in the given paths".to_string(),
			));
		}

		let mut auto = match options.auto_assign_dir.as_deref() {
			Some(dir) => Some(AutoAssign::scan(dir, &mut plan)?),
			None => None,
		};

		// A single explicit target is a *manual* assignment; the same option
		// spread over many archives is the *global* fallback. Precedence is
		// manual > auto > global, as in the reference tool.
		let manual = archives.len() == 1;

		for archive in &archives {
			plan_archive(
				archive,
				&options,
				front.as_deref(),
				back.as_deref(),
				manual,
				auto.as_mut(),
				&mut plan,
			);
		}

		if let Some(auto) = auto.as_ref() {
			for (volume, image) in auto.unmatched() {
				plan.warn(
					Warning::new(
						"auto-assign-unmatched",
						format!("no archive matches volume {volume}"),
					)
					.at(image),
				);
			}
		}

		Ok(plan)
	}

	fn apply(&self, plan: &Plan, sink: &mut dyn ProgressSink) -> ToolResult<Report> {
		plan.expect_tool(TOOL_ID)?;

		let mut report = Report::for_plan(plan);
		let groups = group_by_target(plan, &mut report);
		let total = groups.len();

		for (done, (target, actions)) in groups.into_iter().enumerate() {
			sink.progress(done, total, &target.to_string_lossy());
			apply_archive(&target, &actions, &mut report)?;
		}

		sink.progress(total, total, "done");

		Ok(report)
	}
}

impl CoverOptions {
	fn wants_work(&self) -> bool {
		self.front.is_some()
			|| self.back.is_some()
			|| self.delete_first
			|| self.delete_last
			|| self.remove_added
			|| self.auto_assign_dir.is_some()
	}
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn plan_archive(
	path: &Path,
	options: &CoverOptions,
	front: Option<&Path>,
	back: Option<&Path>,
	manual: bool,
	auto: Option<&mut AutoAssign>,
	plan: &mut Plan,
) {
	let view = match ArchiveView::inspect(path) {
		Ok(view) => view,
		Err(error) => {
			plan.warn(
				Warning::new(
					"unreadable-archive",
					format!("cannot read archive: {error}"),
				)
				.at(path)
				.with_severity(Severity::Error),
			);
			return;
		},
	};

	// Entries a previous run added that are still in the archive.
	let tracked = view.tracked_added();

	// The page list as it will look once earlier actions in this plan run.
	let mut pages = view.pages.clone();
	let mut removed: Vec<String> = Vec::new();

	// 1. Undo first, so `delete_first`/`delete_last` act on the original pages.
	if options.remove_added && !tracked.is_empty() {
		let names = tracked
			.iter()
			.map(|entry| entry.name.clone())
			.collect::<Vec<_>>();
		pages.retain(|page| !names.contains(page));
		removed.extend(names.iter().cloned());
		plan.push(
			Action::new("remove-added")
				.with_target(path)
				.with_detail(json!({ "entries": names, "marker": MARKER_ENTRY })),
		);
	}

	// 2. Deletions of existing pages.
	if options.delete_first {
		plan_delete(path, &mut pages, &mut removed, "first", plan);
	}
	if options.delete_last {
		plan_delete(path, &mut pages, &mut removed, "last", plan);
	}

	// 3. Additions. `front` may come from a manual, auto, or global source;
	//    `back` is never auto-assigned (a loose image carries no side token).
	let front_source = resolve_front(path, front, manual, auto);
	let back_source = back.map(|image| {
		(
			image.to_path_buf(),
			if manual { "manual" } else { "global" },
		)
	});

	for (role, source) in [(Role::Front, front_source), (Role::Back, back_source)] {
		let Some((image, origin)) = source else {
			continue;
		};
		plan_add(
			path,
			role,
			&image,
			origin,
			&view,
			&tracked,
			&mut pages,
			&mut removed,
			plan,
		);
	}
}

fn plan_delete(
	path: &Path,
	pages: &mut Vec<String>,
	removed: &mut Vec<String>,
	position: &'static str,
	plan: &mut Plan,
) {
	let entry = match position {
		"first" => pages.first().cloned(),
		_ => pages.last().cloned(),
	};

	let Some(entry) = entry else {
		plan.warn(
			Warning::new(
				"no-pages",
				format!("archive has no {position} page left to delete"),
			)
			.at(path),
		);
		return;
	};

	pages.retain(|page| page != &entry);
	removed.push(entry.clone());
	plan.push(
		Action::new("delete-page")
			.with_target(path)
			.with_detail(json!({ "entry": entry, "position": position })),
	);
}

fn resolve_front(
	archive: &Path,
	front: Option<&Path>,
	manual: bool,
	auto: Option<&mut AutoAssign>,
) -> Option<(PathBuf, &'static str)> {
	if manual {
		if let Some(image) = front {
			return Some((image.to_path_buf(), "manual"));
		}
	}

	if let Some(auto) = auto {
		if let Some(image) = auto.take_for(archive) {
			return Some((image, "auto"));
		}
	}

	front.map(|image| (image.to_path_buf(), "global"))
}

#[allow(clippy::too_many_arguments)]
fn plan_add(
	path: &Path,
	role: Role,
	image: &Path,
	origin: &'static str,
	view: &ArchiveView,
	tracked: &[AddedEntry],
	pages: &mut Vec<String>,
	removed: &mut Vec<String>,
	plan: &mut Plan,
) {
	let Some(extension) = accepted_extension(image) else {
		plan.warn(
			Warning::new(
				"unsupported-cover-image",
				format!("{} is not a supported image", image.display()),
			)
			.at(image),
		);
		return;
	};
	let entry = format!("{}.{extension}", role.stem());

	// Everything this add supersedes: the previous cover of the same role, and
	// any pre-existing entry that would collide with the new name.
	let mut replaces: Vec<String> = Vec::new();
	if let Some(previous) = tracked.iter().find(|added| added.role == role) {
		if !removed.contains(&previous.name) {
			replaces.push(previous.name.clone());
		}
	}
	if view.contains(&entry) && !removed.contains(&entry) && !replaces.contains(&entry) {
		replaces.push(entry.clone());
		plan.warn(
			Warning::new(
				"cover-entry-replaced",
				format!("`{entry}` already exists in the archive and will be replaced"),
			)
			.at(path),
		);
	}

	pages.retain(|page| !replaces.contains(page));
	removed.extend(replaces.iter().cloned());

	// The name only works as a cover if it really sorts to the right end under
	// the ordering Stump uses to number pages.
	let misplaced = pages.iter().any(|page| match role {
		Role::Front => alphanumeric_sort::compare_str(&entry, page) != Ordering::Less,
		Role::Back => alphanumeric_sort::compare_str(&entry, page) != Ordering::Greater,
	});
	if misplaced {
		plan.warn(
			Warning::new(
				"cover-not-at-edge",
				format!(
					"`{entry}` does not sort {} the existing pages; it will not be read as the {} cover",
					if role == Role::Front { "before" } else { "after" },
					role.label()
				),
			)
			.at(path),
		);
	}

	plan.push(
		Action::new(role.kind())
			.with_source(image)
			.with_target(path)
			.with_detail(json!({
				"entry": entry,
				"role": role,
				"origin": origin,
				"replaces": replaces,
				"marker": MARKER_ENTRY,
			})),
	);
}

fn collect_archives(paths: &[PathBuf], plan: &mut Plan) -> ToolResult<Vec<PathBuf>> {
	if paths.is_empty() {
		return Err(ToolError::Invalid("no paths given".to_string()));
	}

	let mut archives = Vec::new();
	let mut seen = HashSet::new();

	for path in paths {
		if path.is_dir() {
			let mut found = fs::read_dir(path)?
				.filter_map(Result::ok)
				.map(|entry| entry.path())
				.filter(|entry| entry.is_file() && is_cbz(entry))
				.collect::<Vec<_>>();
			found.sort();
			if found.is_empty() {
				plan.warn(
					Warning::new("no-archives", "directory contains no CBZ files")
						.at(path),
				);
			}
			for archive in found {
				if seen.insert(archive.clone()) {
					archives.push(archive);
				}
			}
		} else if path.is_file() {
			if is_cbz(path) {
				if seen.insert(path.clone()) {
					archives.push(path.clone());
				}
			} else {
				plan.warn(Warning::new("unsupported-path", "not a CBZ archive").at(path));
			}
		} else {
			plan.warn(
				Warning::new("missing-path", "path does not exist")
					.at(path)
					.with_severity(Severity::Error),
			);
		}
	}

	Ok(archives)
}

// ---------------------------------------------------------------------------
// Auto assignment
// ---------------------------------------------------------------------------

/// Loose cover images keyed by the volume number parsed from their file name.
struct AutoAssign {
	by_volume: BTreeMap<u32, PathBuf>,
	used: BTreeSet<u32>,
}

impl AutoAssign {
	fn scan(dir: &Path, plan: &mut Plan) -> ToolResult<Self> {
		if !dir.is_dir() {
			return Err(ToolError::Invalid(format!(
				"auto_assign_dir is not a directory: {}",
				dir.display()
			)));
		}

		let mut images = fs::read_dir(dir)?
			.filter_map(Result::ok)
			.map(|entry| entry.path())
			.filter(|entry| entry.is_file() && accepted_extension(entry).is_some())
			.collect::<Vec<_>>();
		images.sort();

		let mut by_volume: BTreeMap<u32, PathBuf> = BTreeMap::new();
		let mut ambiguous: BTreeSet<u32> = BTreeSet::new();

		for image in images {
			let numbers = volume_numbers(&file_stem(&image));
			match numbers.len() {
				0 => plan.warn(
					Warning::new(
						"auto-assign-no-volume",
						"no volume token (v01 / vol.7 / volume 12) in the file name",
					)
					.at(&image),
				),
				1 => {
					let volume = numbers[0];
					if by_volume.insert(volume, image.clone()).is_some() {
						ambiguous.insert(volume);
						plan.warn(
							Warning::new(
								"auto-assign-duplicate-volume",
								format!("more than one image claims volume {volume}; none will be assigned"),
							)
							.at(&image),
						);
					}
				},
				_ => plan.warn(
					Warning::new(
						"auto-assign-ambiguous",
						format!(
							"file name contains several volume numbers ({})",
							numbers
								.iter()
								.map(u32::to_string)
								.collect::<Vec<_>>()
								.join(", ")
						),
					)
					.at(&image),
				),
			}
		}

		for volume in &ambiguous {
			by_volume.remove(volume);
		}

		Ok(Self {
			by_volume,
			used: BTreeSet::new(),
		})
	}

	/// The image for the archive's volume, if exactly one volume number can be
	/// read from the archive's file name and an image claims it.
	fn take_for(&mut self, archive: &Path) -> Option<PathBuf> {
		let numbers = volume_numbers(&file_stem(archive));
		if numbers.len() != 1 {
			return None;
		}
		let volume = numbers[0];
		let image = self.by_volume.get(&volume)?.clone();
		self.used.insert(volume);
		Some(image)
	}

	fn unmatched(&self) -> impl Iterator<Item = (u32, &PathBuf)> + '_ {
		self.by_volume
			.iter()
			.filter(|(volume, _)| !self.used.contains(volume))
			.map(|(volume, image)| (*volume, image))
	}
}

/// Distinct volume numbers in a file stem, in first-seen order.
///
/// `_` and `.` are normalised to spaces first, matching
/// `core/src/ingest/quality/filename.rs:215-223`, so `vol.7` reads as `vol 7`.
pub fn volume_numbers(stem: &str) -> Vec<u32> {
	let normalized = stem.replace(['_', '.'], " ");
	let mut numbers = Vec::new();
	for captures in VOLUME_TOKEN.captures_iter(&normalized) {
		let Some(number) = captures
			.get(1)
			.and_then(|group| group.as_str().parse::<u32>().ok())
		else {
			continue;
		};
		if !numbers.contains(&number) {
			numbers.push(number);
		}
	}
	numbers
}

// ---------------------------------------------------------------------------
// Applying
// ---------------------------------------------------------------------------

fn group_by_target(plan: &Plan, report: &mut Report) -> Vec<(PathBuf, Vec<Action>)> {
	let mut order: Vec<PathBuf> = Vec::new();
	let mut grouped: HashMap<PathBuf, Vec<Action>> = HashMap::new();

	for action in &plan.actions {
		match action.target.as_ref() {
			Some(target) => {
				let bucket = grouped.entry(target.clone()).or_insert_with(|| {
					order.push(target.clone());
					Vec::new()
				});
				bucket.push(action.clone());
			},
			None => report.skipped(action.clone(), "action has no target"),
		}
	}

	order
		.into_iter()
		.map(|target| {
			let actions = grouped.remove(&target).unwrap_or_default();
			(target, actions)
		})
		.collect()
}

fn apply_archive(path: &Path, actions: &[Action], report: &mut Report) -> ToolResult<()> {
	let view = match ArchiveView::inspect(path) {
		Ok(view) => view,
		Err(error) => {
			for action in actions {
				report.skipped(action.clone(), format!("cannot read archive: {error}"));
			}
			return Ok(());
		},
	};

	let mut remove: Vec<String> = Vec::new();
	let mut adds: Vec<(String, PathBuf, Role)> = Vec::new();
	let mut accepted: Vec<Action> = Vec::new();

	for action in actions {
		match action.kind.as_str() {
			"remove-added" => {
				let entries = detail_strings(action, "entries");
				let present = entries
					.into_iter()
					.filter(|entry| view.contains(entry))
					.collect::<Vec<_>>();
				if present.is_empty() {
					report.skipped(
						action.clone(),
						"no tool-added entries remain in the archive",
					);
					continue;
				}
				remove.extend(present);
				accepted.push(action.clone());
			},
			"delete-page" => {
				let Some(entry) = detail_string(action, "entry") else {
					report.skipped(action.clone(), "action has no `entry` detail");
					continue;
				};
				if !view.contains(&entry) {
					report.skipped(
						action.clone(),
						format!("`{entry}` is not in the archive"),
					);
					continue;
				}
				if remove.contains(&entry) {
					report.skipped(
						action.clone(),
						format!("`{entry}` was already removed by an earlier action"),
					);
					continue;
				}
				remove.push(entry);
				accepted.push(action.clone());
			},
			kind if Role::from_kind(kind).is_some() => {
				let role = Role::from_kind(kind).expect("checked by the guard");
				let Some(entry) = detail_string(action, "entry") else {
					report.skipped(action.clone(), "action has no `entry` detail");
					continue;
				};
				let Some(source) = action.source.clone() else {
					report.skipped(action.clone(), "action has no source image");
					continue;
				};
				if !source.is_file() {
					report.skipped(
						action.clone(),
						format!("source image is missing: {}", source.display()),
					);
					continue;
				}
				for replaced in detail_strings(action, "replaces") {
					if view.contains(&replaced) && !remove.contains(&replaced) {
						remove.push(replaced);
					}
				}
				adds.push((entry, source, role));
				accepted.push(action.clone());
			},
			other => {
				report.skipped(
					action.clone(),
					format!("`{other}` is not a cbz-covers action"),
				);
			},
		}
	}

	if accepted.is_empty() {
		return Ok(());
	}

	// The manifest keeps every still-present tracked entry that survived, plus
	// whatever this run adds.
	let mut added = view
		.tracked_added()
		.into_iter()
		.filter(|entry| {
			!remove.contains(&entry.name)
				&& !adds.iter().any(|(name, _, _)| name == &entry.name)
		})
		.collect::<Vec<_>>();
	for (entry, source, role) in &adds {
		added.push(AddedEntry {
			name: entry.clone(),
			role: *role,
			source: source
				.file_name()
				.map(|name| name.to_string_lossy().to_string()),
		});
	}

	let mut new_entries: Vec<(String, Vec<u8>)> = Vec::with_capacity(adds.len() + 1);
	for (entry, source, _) in &adds {
		new_entries.push((entry.clone(), fs::read(source)?));
	}
	if !added.is_empty() {
		let marker = Marker {
			version: MARKER_VERSION,
			tool: TOOL_ID.to_string(),
			added,
		};
		new_entries.push((MARKER_ENTRY.to_string(), serde_json::to_vec(&marker)?));
	}

	let mut remove: BTreeSet<String> = remove.into_iter().collect();
	if view.has_marker_entry {
		// A manifest already exists; it can only be updated by rewriting.
		remove.insert(MARKER_ENTRY.to_string());
	}

	// Fast path: pure addition to an archive whose local headers all carry real
	// sizes. `ZipWriter::new_append` leaves every existing byte untouched, which
	// preserves per-entry extra fields and comments that `raw_copy_file` drops.
	// It re-emits the existing central directory records verbatim, and
	// `zip-1.1.3/src/write.rs:1552-1558` computes the general purpose bit flag
	// from the file name alone — so an archive whose entries use data
	// descriptors (bit 3) would come out with local headers claiming a
	// descriptor and central headers denying it. Those rewrite instead.
	if remove.is_empty() && !view.uses_data_descriptors() {
		append_entries(path, &new_entries)?;
	} else {
		rewrite(path, &remove, &new_entries)?;
	}

	for action in accepted {
		report.applied(action);
	}

	Ok(())
}

/// Copy the archive, append the new entries to the copy, swap it in.
fn append_entries(path: &Path, entries: &[(String, Vec<u8>)]) -> ToolResult<()> {
	let permissions = fs::metadata(path).map(|meta| meta.permissions()).ok();
	util::write_atomic(path, |temp| {
		std::io::copy(&mut File::open(path)?, &mut *temp)?;
		let mut writer = ZipWriter::new_append(temp)?;
		write_entries(&mut writer, entries)?;
		writer.finish()?;
		Ok(())
	})?;
	restore_permissions(path, permissions);
	Ok(())
}

/// Rebuild the archive without `remove`, copying every retained entry's already
/// compressed bytes verbatim (`raw_copy_file`, no decompress/recompress cycle).
fn rewrite(
	path: &Path,
	remove: &BTreeSet<String>,
	entries: &[(String, Vec<u8>)],
) -> ToolResult<()> {
	let permissions = fs::metadata(path).map(|meta| meta.permissions()).ok();
	let mut archive = ZipArchive::new(File::open(path)?)?;
	util::write_atomic(path, |temp| {
		let mut writer = ZipWriter::new(temp);
		for index in 0..archive.len() {
			let entry = archive.by_index_raw(index)?;
			let name = entry.name().to_string();
			if remove.contains(&name) || entries.iter().any(|(new, _)| new == &name) {
				continue;
			}
			writer.raw_copy_file(entry)?;
		}
		write_entries(&mut writer, entries)?;
		writer.finish()?;
		Ok(())
	})?;
	restore_permissions(path, permissions);
	Ok(())
}

fn write_entries<W: Write + Seek>(
	writer: &mut ZipWriter<W>,
	entries: &[(String, Vec<u8>)],
) -> ToolResult<()> {
	// Stored, like every other CBZ Stump writes (`crates/media/src/transform/
	// container.rs:63-66`): the payloads are already-compressed images plus a
	// tiny manifest.
	let options = SimpleFileOptions::default()
		.compression_method(CompressionMethod::Stored)
		.unix_permissions(0o644);

	for (name, bytes) in entries {
		writer.start_file(name.as_str(), options)?;
		writer.write_all(bytes)?;
	}

	Ok(())
}

/// `util::write_atomic` stages through a 0600 temp file, so an in-place update
/// has to put the library file's own mode back. The mtime is deliberately *not*
/// preserved: the scanner uses it to notice the change.
fn restore_permissions(path: &Path, permissions: Option<fs::Permissions>) {
	if let Some(permissions) = permissions {
		let _ = fs::set_permissions(path, permissions);
	}
}

// ---------------------------------------------------------------------------
// Archive inspection
// ---------------------------------------------------------------------------

struct ArchiveEntry {
	name: String,
	is_dir: bool,
	uses_data_descriptor: bool,
}

struct ArchiveView {
	entries: Vec<ArchiveEntry>,
	/// Image entries in the order Stump numbers pages.
	pages: Vec<String>,
	marker: Option<Marker>,
	has_marker_entry: bool,
}

impl ArchiveView {
	fn inspect(path: &Path) -> ToolResult<Self> {
		let mut archive = ZipArchive::new(File::open(path)?)?;

		let mut entries = Vec::with_capacity(archive.len());
		let mut header_starts = Vec::with_capacity(archive.len());
		let mut marker = None;
		let mut has_marker_entry = false;

		for index in 0..archive.len() {
			let mut entry = archive.by_index(index)?;
			let name = entry.name().to_string();
			let is_dir = entry.is_dir();
			header_starts.push(entry.header_start());

			if !is_dir && name == MARKER_ENTRY {
				has_marker_entry = true;
				let mut raw = String::new();
				if entry.read_to_string(&mut raw).is_ok() {
					marker = serde_json::from_str::<Marker>(&raw).ok();
				}
			}

			entries.push(ArchiveEntry {
				name,
				is_dir,
				uses_data_descriptor: false,
			});
		}

		let mut file = archive.into_inner();
		for (entry, header_start) in entries.iter_mut().zip(header_starts) {
			entry.uses_data_descriptor =
				local_header_has_data_descriptor(&mut file, header_start)?;
		}

		let mut pages = entries
			.iter()
			.filter(|entry| !entry.is_dir && is_page(&entry.name))
			.map(|entry| entry.name.clone())
			.collect::<Vec<_>>();
		// Same ordering as `crates/media/src/media/utils.rs:31-36`, which is
		// what turns entry names into page numbers.
		alphanumeric_sort::sort_str_slice(&mut pages);

		Ok(Self {
			entries,
			pages,
			marker,
			has_marker_entry,
		})
	}

	fn contains(&self, name: &str) -> bool {
		self.entries.iter().any(|entry| entry.name == name)
	}

	fn uses_data_descriptors(&self) -> bool {
		self.entries.iter().any(|entry| entry.uses_data_descriptor)
	}

	/// Manifest entries that are actually still in the archive.
	fn tracked_added(&self) -> Vec<AddedEntry> {
		self.marker
			.as_ref()
			.map(|marker| marker.added.clone())
			.unwrap_or_default()
			.into_iter()
			.filter(|entry| self.contains(&entry.name))
			.collect()
	}
}

/// Read bit 3 (data descriptor) of a local file header's general purpose flag.
fn local_header_has_data_descriptor(
	file: &mut File,
	header_start: u64,
) -> ToolResult<bool> {
	// Local file header: signature (4) + version needed (2) + flags (2).
	file.seek(SeekFrom::Start(header_start + 6))?;
	let mut flags = [0u8; 2];
	file.read_exact(&mut flags)?;
	Ok(u16::from_le_bytes(flags) & 0x08 != 0)
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// Page detection uses the shared predicate so the tool's idea of a page is the
/// scanner's (`crate::util::is_page_file`).
fn is_page(name: &str) -> bool {
	util::is_page_file(Path::new(name))
}

fn is_cbz(path: &Path) -> bool {
	path.extension()
		.and_then(|extension| extension.to_str())
		.is_some_and(|extension| extension.eq_ignore_ascii_case("cbz"))
}

/// The lowercased extension of an image Stump can decode, if it is one.
fn accepted_extension(path: &Path) -> Option<String> {
	if !util::is_page_file(path) {
		return None;
	}
	Some(path.extension()?.to_str()?.to_ascii_lowercase())
}

fn file_stem(path: &Path) -> String {
	path.file_stem()
		.map(|stem| stem.to_string_lossy().to_string())
		.unwrap_or_default()
}

fn validate_image(path: Option<&Path>, label: &str) -> ToolResult<Option<PathBuf>> {
	let Some(path) = path else {
		return Ok(None);
	};
	if !path.is_file() {
		return Err(ToolError::Invalid(format!(
			"{label} cover is not a file: {}",
			path.display()
		)));
	}
	if accepted_extension(path).is_none() {
		return Err(ToolError::Invalid(format!(
			"{label} cover is not a supported image: {}",
			path.display()
		)));
	}
	Ok(Some(path.to_path_buf()))
}

fn detail_string(action: &Action, key: &str) -> Option<String> {
	action
		.detail
		.get(key)
		.and_then(|value| value.as_str())
		.map(ToString::to_string)
}

fn detail_strings(action: &Action, key: &str) -> Vec<String> {
	action
		.detail
		.get(key)
		.and_then(|value| value.as_array())
		.map(|values| {
			values
				.iter()
				.filter_map(|value| value.as_str().map(ToString::to_string))
				.collect()
		})
		.unwrap_or_default()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::NoopProgress;
	use pretty_assertions::assert_eq;
	use tempfile::TempDir;

	/// A one-pixel-ish payload; the tool never decodes images, only moves bytes.
	fn image_bytes(seed: u8) -> Vec<u8> {
		vec![seed; 64]
	}

	fn write_cbz(path: &Path, entries: &[(&str, Vec<u8>)]) {
		let file = File::create(path).unwrap();
		let mut writer = ZipWriter::new(file);
		let options =
			SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
		for (name, bytes) in entries {
			writer.start_file(*name, options).unwrap();
			writer.write_all(bytes).unwrap();
		}
		writer.finish().unwrap();
	}

	fn write_image(path: &Path, seed: u8) {
		fs::write(path, image_bytes(seed)).unwrap();
	}

	/// Every entry, in central directory order, with its CRC and contents.
	fn entries_of(path: &Path) -> Vec<(String, u32, Vec<u8>)> {
		let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
		let mut out = Vec::new();
		for index in 0..archive.len() {
			let mut entry = archive.by_index(index).unwrap();
			let name = entry.name().to_string();
			let crc = entry.crc32();
			let mut bytes = Vec::new();
			entry.read_to_end(&mut bytes).unwrap();
			out.push((name, crc, bytes));
		}
		out
	}

	/// The page list exactly as Stump would number it.
	fn pages_of(path: &Path) -> Vec<String> {
		ArchiveView::inspect(path).unwrap().pages
	}

	fn marker_of(path: &Path) -> Option<Marker> {
		ArchiveView::inspect(path).unwrap().marker
	}

	fn simple_cbz(dir: &Path, name: &str) -> PathBuf {
		let path = dir.join(name);
		write_cbz(
			&path,
			&[
				("ComicInfo.xml", b"<ComicInfo/>".to_vec()),
				("0001.jpg", image_bytes(1)),
				("0002.jpg", image_bytes(2)),
				("0003.jpg", image_bytes(3)),
			],
		);
		path
	}

	fn run(input: ToolInput) -> (Plan, Report) {
		let tool = CbzCovers;
		let plan = tool.plan(&input).expect("plan");
		let report = tool.apply(&plan, &mut NoopProgress).expect("apply");
		(plan, report)
	}

	#[test]
	fn adds_front_and_back_covers_at_the_page_edges() {
		let temp = TempDir::new().unwrap();
		let archive = simple_cbz(temp.path(), "Series v01.cbz");
		let front = temp.path().join("front.png");
		let back = temp.path().join("back.jpg");
		write_image(&front, 10);
		write_image(&back, 20);

		let (plan, report) = run(ToolInput::new(vec![archive.clone()])
			.with_options(json!({ "front": front, "back": back })));

		assert_eq!(
			plan.actions
				.iter()
				.map(|action| action.kind.as_str())
				.collect::<Vec<_>>(),
			vec!["add-front-cover", "add-back-cover"]
		);
		assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
		assert_eq!(report.applied.len(), 2);
		assert!(report.skipped.is_empty());

		assert_eq!(
			pages_of(&archive),
			vec![
				"!000-cover.png",
				"0001.jpg",
				"0002.jpg",
				"0003.jpg",
				"zzz-back.jpg"
			]
		);

		// The cover bytes are the source bytes, and nothing else was disturbed.
		let entries = entries_of(&archive);
		let by_name = entries
			.iter()
			.map(|(name, _, bytes)| (name.as_str(), bytes.clone()))
			.collect::<HashMap<_, _>>();
		assert_eq!(by_name["!000-cover.png"], image_bytes(10));
		assert_eq!(by_name["zzz-back.jpg"], image_bytes(20));
		assert_eq!(by_name["ComicInfo.xml"], b"<ComicInfo/>".to_vec());

		let marker = marker_of(&archive).expect("marker written");
		assert_eq!(marker.tool, TOOL_ID);
		assert_eq!(
			marker.added,
			vec![
				AddedEntry {
					name: "!000-cover.png".to_string(),
					role: Role::Front,
					source: Some("front.png".to_string()),
				},
				AddedEntry {
					name: "zzz-back.jpg".to_string(),
					role: Role::Back,
					source: Some("back.jpg".to_string()),
				},
			]
		);
	}

	#[test]
	fn dry_run_writes_nothing() {
		let temp = TempDir::new().unwrap();
		let archive = simple_cbz(temp.path(), "Series v01.cbz");
		let front = temp.path().join("front.png");
		write_image(&front, 10);
		let before = fs::read(&archive).unwrap();
		let listing_before = fs::read_dir(temp.path()).unwrap().count();

		let plan = CbzCovers
			.plan(&ToolInput::new(vec![archive.clone()]).with_options(json!({
				"front": front,
				"delete_first": true,
				"delete_last": true,
				"remove_added": true,
			})))
			.expect("plan");

		assert!(!plan.actions.is_empty());
		assert_eq!(fs::read(&archive).unwrap(), before);
		assert_eq!(fs::read_dir(temp.path()).unwrap().count(), listing_before);
	}

	#[test]
	fn deletes_the_current_first_and_last_page() {
		let temp = TempDir::new().unwrap();
		let archive = simple_cbz(temp.path(), "Series v01.cbz");

		let (plan, report) = run(ToolInput::new(vec![archive.clone()])
			.with_options(json!({ "delete_first": true, "delete_last": true })));

		let deleted = plan
			.actions
			.iter()
			.map(|action| detail_string(action, "entry").unwrap())
			.collect::<Vec<_>>();
		assert_eq!(deleted, vec!["0001.jpg", "0003.jpg"]);
		assert_eq!(report.applied.len(), 2);

		assert_eq!(pages_of(&archive), vec!["0002.jpg"]);
		// A delete-only run leaves no manifest behind.
		assert!(marker_of(&archive).is_none());
		assert!(!ArchiveView::inspect(&archive).unwrap().has_marker_entry);
		// Untouched entries survive.
		assert!(ArchiveView::inspect(&archive)
			.unwrap()
			.contains("ComicInfo.xml"));
	}

	#[test]
	fn delete_last_warns_when_the_archive_runs_out_of_pages() {
		let temp = TempDir::new().unwrap();
		let archive = temp.path().join("Single.cbz");
		write_cbz(&archive, &[("0001.jpg", image_bytes(1))]);

		let plan = CbzCovers
			.plan(
				&ToolInput::new(vec![archive.clone()])
					.with_options(json!({ "delete_first": true, "delete_last": true })),
			)
			.expect("plan");

		assert_eq!(plan.actions.len(), 1);
		assert_eq!(
			plan.warnings
				.iter()
				.map(|warning| warning.code.as_str())
				.collect::<Vec<_>>(),
			vec!["no-pages"]
		);
	}

	#[test]
	fn remove_added_restores_a_byte_identical_page_list() {
		let temp = TempDir::new().unwrap();
		let archive = simple_cbz(temp.path(), "Series v01.cbz");
		let front = temp.path().join("front.png");
		let back = temp.path().join("back.jpg");
		write_image(&front, 10);
		write_image(&back, 20);

		let original = entries_of(&archive);

		run(ToolInput::new(vec![archive.clone()])
			.with_options(json!({ "front": front, "back": back })));
		assert_eq!(pages_of(&archive).len(), 5);

		let (_, report) = run(ToolInput::new(vec![archive.clone()])
			.with_options(json!({ "remove_added": true })));
		assert_eq!(report.applied.len(), 1);

		// Every original entry is back, byte for byte, with its original CRC,
		// and the manifest is gone with it.
		assert_eq!(entries_of(&archive), original);
		assert!(marker_of(&archive).is_none());
		assert!(!ArchiveView::inspect(&archive).unwrap().has_marker_entry);
	}

	#[test]
	fn remove_added_leaves_pages_the_tool_did_not_add() {
		let temp = TempDir::new().unwrap();
		let archive = temp.path().join("Series v01.cbz");
		// A hand-made "cover" that this tool never added must survive.
		write_cbz(
			&archive,
			&[
				("!000-cover.png", image_bytes(9)),
				("0001.jpg", image_bytes(1)),
			],
		);
		let front = temp.path().join("front.jpg");
		write_image(&front, 10);

		let (plan, _) =
			run(ToolInput::new(vec![archive.clone()])
				.with_options(json!({ "front": front })));
		assert!(plan
			.warnings
			.iter()
			.all(|warning| warning.code != "cover-entry-replaced"));

		run(ToolInput::new(vec![archive.clone()])
			.with_options(json!({ "remove_added": true })));

		assert_eq!(pages_of(&archive), vec!["!000-cover.png", "0001.jpg"]);
	}

	#[test]
	fn a_second_front_cover_replaces_the_first() {
		let temp = TempDir::new().unwrap();
		let archive = simple_cbz(temp.path(), "Series v01.cbz");
		let first = temp.path().join("first.png");
		let second = temp.path().join("second.jpg");
		write_image(&first, 10);
		write_image(&second, 11);

		run(ToolInput::new(vec![archive.clone()]).with_options(json!({ "front": first })));
		let (plan, report) = run(ToolInput::new(vec![archive.clone()])
			.with_options(json!({ "front": second })));

		assert_eq!(
			detail_strings(&plan.actions[0], "replaces"),
			vec!["!000-cover.png"]
		);
		assert_eq!(report.applied.len(), 1);
		assert_eq!(
			pages_of(&archive),
			vec!["!000-cover.jpg", "0001.jpg", "0002.jpg", "0003.jpg"]
		);
		let marker = marker_of(&archive).unwrap();
		assert_eq!(marker.added.len(), 1);
		assert_eq!(marker.added[0].name, "!000-cover.jpg");
	}

	/// Set the local-header data descriptor bit (general purpose bit 3) on the
	/// entry starting at `header_start`, leaving the central directory alone.
	fn set_data_descriptor_bit(path: &Path, header_start: u64) {
		let mut file = fs::OpenOptions::new()
			.read(true)
			.write(true)
			.open(path)
			.unwrap();
		file.seek(SeekFrom::Start(header_start + 6)).unwrap();
		let mut flags = [0u8; 2];
		file.read_exact(&mut flags).unwrap();
		let patched = u16::from_le_bytes(flags) | 0x08;
		file.seek(SeekFrom::Start(header_start + 6)).unwrap();
		file.write_all(&patched.to_le_bytes()).unwrap();
	}

	#[test]
	fn an_archive_with_data_descriptors_is_rewritten_not_appended() {
		let temp = TempDir::new().unwrap();
		let archive = simple_cbz(temp.path(), "Series v01.cbz");
		let header_start = {
			let mut zip = ZipArchive::new(File::open(&archive).unwrap()).unwrap();
			let start = zip.by_index(1).unwrap().header_start();
			start
		};
		set_data_descriptor_bit(&archive, header_start);
		assert!(ArchiveView::inspect(&archive)
			.unwrap()
			.uses_data_descriptors());

		let front = temp.path().join("front.png");
		write_image(&front, 10);
		let original = entries_of(&archive);

		let (_, report) =
			run(ToolInput::new(vec![archive.clone()])
				.with_options(json!({ "front": front })));
		assert_eq!(report.applied.len(), 1);

		// The rewrite path wrote fresh local headers, so the inconsistent flag
		// is gone and every original entry survived unchanged.
		assert!(!ArchiveView::inspect(&archive)
			.unwrap()
			.uses_data_descriptors());
		let after = entries_of(&archive);
		for entry in &original {
			assert!(after.contains(entry), "{} changed", entry.0);
		}
		assert_eq!(pages_of(&archive)[0], "!000-cover.png");
	}

	#[test]
	fn auto_assign_matches_volume_tokens_and_warns_about_the_rest() {
		let temp = TempDir::new().unwrap();
		let library = temp.path().join("library");
		let covers = temp.path().join("covers");
		fs::create_dir(&library).unwrap();
		fs::create_dir(&covers).unwrap();

		let one = simple_cbz(&library, "Series v01.cbz");
		let seven = simple_cbz(&library, "Series vol.7.cbz");
		let twelve = simple_cbz(&library, "Series volume 12.cbz");
		let extras = simple_cbz(&library, "Series Extras.cbz");

		for (name, seed) in [
			("Series v01.png", 1u8),
			("Series vol.7.png", 7),
			("Series volume 12.png", 12),
			("Series v99.png", 99),
			("loose art.png", 0),
		] {
			write_image(&covers.join(name), seed);
		}

		let (plan, report) = run(ToolInput::new(vec![library.clone()])
			.with_options(json!({ "auto_assign_dir": covers })));

		let assigned = plan
			.actions
			.iter()
			.map(|action| {
				(
					action.target.clone().unwrap(),
					action.source.clone().unwrap(),
					detail_string(action, "origin").unwrap(),
				)
			})
			.collect::<Vec<_>>();
		assert_eq!(
			assigned,
			vec![
				(
					one.clone(),
					covers.join("Series v01.png"),
					"auto".to_string()
				),
				(
					seven.clone(),
					covers.join("Series vol.7.png"),
					"auto".to_string()
				),
				(
					twelve.clone(),
					covers.join("Series volume 12.png"),
					"auto".to_string()
				),
			]
		);
		assert_eq!(report.applied.len(), 3);
		assert!(report.skipped.is_empty());

		// `Series v99.png` has no archive and `loose art.png` has no volume
		// token; both are reported rather than silently dropped.
		let mut warnings = plan
			.warnings
			.iter()
			.map(|warning| {
				(
					warning.code.clone(),
					warning
						.path
						.as_ref()
						.and_then(|path| path.file_name())
						.map(|name| name.to_string_lossy().to_string())
						.unwrap_or_default(),
				)
			})
			.collect::<Vec<_>>();
		warnings.sort();
		assert_eq!(
			warnings,
			vec![
				(
					"auto-assign-no-volume".to_string(),
					"loose art.png".to_string()
				),
				(
					"auto-assign-unmatched".to_string(),
					"Series v99.png".to_string()
				),
			]
		);

		assert_eq!(pages_of(&one)[0], "!000-cover.png");
		assert_eq!(pages_of(&seven)[0], "!000-cover.png");
		assert_eq!(pages_of(&twelve)[0], "!000-cover.png");
		// The volume-less archive is left alone.
		assert_eq!(pages_of(&extras).len(), 3);
	}

	#[test]
	fn cover_precedence_is_manual_then_auto_then_global() {
		let temp = TempDir::new().unwrap();
		let library = temp.path().join("library");
		let covers = temp.path().join("covers");
		fs::create_dir(&library).unwrap();
		fs::create_dir(&covers).unwrap();

		let one = simple_cbz(&library, "Series v01.cbz");
		let two = simple_cbz(&library, "Series v02.cbz");
		let auto_cover = covers.join("Series v01.png");
		write_image(&auto_cover, 1);
		let explicit = temp.path().join("explicit.png");
		write_image(&explicit, 42);

		// One explicit target: manual wins over the volume match.
		let plan = CbzCovers
			.plan(&ToolInput::new(vec![one.clone()]).with_options(json!({
				"front": explicit,
				"auto_assign_dir": covers,
			})))
			.expect("plan");
		assert_eq!(
			(
				plan.actions[0].source.clone().unwrap(),
				detail_string(&plan.actions[0], "origin").unwrap()
			),
			(explicit.clone(), "manual".to_string())
		);

		// Several targets: the volume match beats the shared `front`, and
		// archives with no match fall back to it.
		let plan = CbzCovers
			.plan(
				&ToolInput::new(vec![one.clone(), two.clone()]).with_options(json!({
					"front": explicit,
					"auto_assign_dir": covers,
				})),
			)
			.expect("plan");
		assert_eq!(
			plan.actions
				.iter()
				.map(|action| {
					(
						action.target.clone().unwrap(),
						action.source.clone().unwrap(),
						detail_string(action, "origin").unwrap(),
					)
				})
				.collect::<Vec<_>>(),
			vec![
				(one, auto_cover, "auto".to_string()),
				(two, explicit, "global".to_string()),
			]
		);
	}

	#[test]
	fn volume_token_grammar() {
		assert_eq!(volume_numbers("Series v01"), vec![1]);
		assert_eq!(volume_numbers("Series vol.7"), vec![7]);
		assert_eq!(volume_numbers("Series vol 7"), vec![7]);
		assert_eq!(volume_numbers("Series volume 12"), vec![12]);
		assert_eq!(volume_numbers("Series V003 (2020)"), vec![3]);
		assert_eq!(volume_numbers("Series_v04_extra"), vec![4]);
		// No token at all, and no false positive on an embedded `v`.
		assert!(volume_numbers("Series Extras").is_empty());
		assert!(volume_numbers("November special").is_empty());
		// Distinct numbers are all reported so the caller can refuse to guess.
		assert_eq!(volume_numbers("Series v01 vol.2"), vec![1, 2]);
		// The same number twice is not ambiguous.
		assert_eq!(volume_numbers("Series v01 volume 1"), vec![1]);
	}
}
