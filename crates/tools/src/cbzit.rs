//! `cbzit`: turn folders of page images into cleanly named CBZ archives, and
//! optionally consolidate chapter archives into per-volume ones.
//!
//! Behaviour source: Kavita's external tools guide lists `cbzit` as a tool that
//! "takes a folder of images and creates a properly named cbz"
//! (<https://wiki.kavitareader.com/guides/external-tools/>). Nothing is copied
//! from that (GPL) tool. The naming grammar here is Stump's own and is shared
//! with the scanner: numbers are parsed with
//! [`stump_scanner::parse_identifier_of`] and names are pre-cleaned with
//! [`stump_scanner::clean_name`], so a file this tool writes parses back to the
//! same volume/chapter it was named from (`names_round_trip_through_the_scanner_parser`).
//!
//! Detection is bottom-up: the directory that holds the images decides the
//! chapter, its ancestors fill in what it does not say, and the series falls
//! back to the shallowest ancestor with real text in its name. Every guess is
//! reported as a [`Warning`], and nothing is written by `plan`.

use std::{
	collections::HashMap,
	fs::File,
	io::Read,
	path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use stump_media::PathUtils;
use stump_scanner::{clean_name, parse_identifier_of, SequenceKind, SequenceNumber};
use zip::ZipArchive;

use crate::{
	util, Action, Plan, ProgressSink, Report, Severity, Tool, ToolError, ToolInput,
	ToolResult, Warning,
};

pub const ID: &str = "cbzit";
/// Pack one folder of images into one CBZ.
pub const ACTION_PACK: &str = "pack";
/// Consolidate chapter CBZs into one volume CBZ.
pub const ACTION_MERGE: &str = "merge";

/// Default ceiling for a merged volume: readers and browsers cope badly with
/// multi-gigabyte archives, and a runaway merge is almost always a detection
/// mistake rather than a real volume.
pub const DEFAULT_MAX_MERGE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub struct Cbzit;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CbzitOptions {
	/// Where archives are written. Default: the input root itself.
	pub output_dir: Option<PathBuf>,
	/// Override the detected series name.
	pub series: Option<String>,
	/// Override the detected language tag.
	pub language: Option<String>,
	/// Replace existing archives instead of skipping them.
	pub overwrite: bool,
	/// Write a `ComicInfo.xml` describing the detected identity.
	pub comic_info: bool,
	/// Folders with fewer page images than this are reported, not packed.
	pub min_pages: usize,
	/// Merge mode: consolidate chapter `.cbz` files into per-volume archives
	/// instead of packing image folders.
	pub merge: bool,
	/// Merge-mode guard: skip a volume whose pages exceed this many bytes.
	pub max_merge_bytes: u64,
}

impl Default for CbzitOptions {
	fn default() -> Self {
		Self {
			output_dir: None,
			series: None,
			language: None,
			overwrite: false,
			comic_info: true,
			min_pages: 1,
			merge: false,
			max_merge_bytes: DEFAULT_MAX_MERGE_BYTES,
		}
	}
}

/// Everything `apply` needs to pack one archive; a plan is self-contained by
/// contract, so the exact page order is recorded here rather than re-derived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PackDetail {
	pages: usize,
	series: Option<String>,
	volume: Option<i32>,
	chapter: Option<String>,
	title: Option<String>,
	language: Option<String>,
	comic_info: bool,
	overwrite: bool,
	/// Page images in reading order; entries are renumbered on write, so
	/// identical file names in nested folders cannot collide.
	sources: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MergeDetail {
	pages: usize,
	bytes: u64,
	chapters: usize,
	series: Option<String>,
	volume: Option<i32>,
	language: Option<String>,
	comic_info: bool,
	overwrite: bool,
	/// Chapter archives in chapter order.
	sources: Vec<PathBuf>,
}

impl Tool for Cbzit {
	fn id(&self) -> &'static str {
		ID
	}

	fn describe(&self) -> &'static str {
		"Pack folders of page images into cleanly named CBZ archives (and merge chapters into volumes)"
	}

	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError> {
		let options = input.parse_options::<CbzitOptions>()?;
		if input.paths.is_empty() {
			return Err(ToolError::Invalid("no input paths given".to_string()));
		}
		for path in &input.paths {
			if !path.exists() {
				return Err(ToolError::Invalid(format!(
					"path does not exist: {}",
					path.display()
				)));
			}
		}

		if options.merge {
			plan_merge(&input.paths, &options)
		} else {
			plan_pack(&input.paths, &options)
		}
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
			let Some(target) = action.target.as_ref() else {
				report.skipped(action.clone(), "action has no target");
				continue;
			};
			sink.progress(index, total, &format!("writing {}", target.display()));

			let outcome = match action.kind.as_str() {
				ACTION_PACK => apply_pack(action, target),
				ACTION_MERGE => apply_merge(action, target),
				kind => Err(ToolError::Invalid(format!(
					"unsupported action kind {kind:?}"
				))),
			};

			match outcome {
				Ok(Some(pages)) => {
					tracing::debug!(?target, pages, "wrote archive");
					report.applied(action.clone());
				},
				Ok(None) => report.skipped(action.clone(), "target already exists"),
				Err(error) => report.skipped(action.clone(), error.to_string()),
			}
		}
		sink.progress(total, total, "done");

		Ok(report)
	}
}

/// The identity of one archive, detected bottom-up from the path.
#[derive(Debug, Clone, Default, PartialEq)]
struct Identity {
	series: String,
	volume: Option<SequenceNumber>,
	chapter: Option<SequenceNumber>,
	title: Option<String>,
	language: Option<String>,
}

impl Identity {
	/// The archive file name: `Series v01 c003 - Title [en].cbz`.
	///
	/// Volumes are padded to two digits and chapters to three so a directory
	/// listing sorts in reading order, and the identifier letters (`v`, `c`)
	/// keep the name parseable by the scanner's grammar.
	fn archive_name(&self) -> String {
		let mut name = self.series.clone();
		if let Some(volume) = self.volume {
			name.push_str(&format!(" v{}", pad(volume, 2)));
		}
		if let Some(chapter) = self.chapter {
			name.push_str(&format!(" c{}", pad(chapter, 3)));
		}
		if let Some(title) = &self.title {
			name.push_str(&format!(" - {title}"));
		}
		if let Some(language) = &self.language {
			name.push_str(&format!(" [{language}]"));
		}

		format!("{}.cbz", util::sanitize_file_stem(&name))
	}
}

/// Zero-pad the integer part of a sequence number, keeping any decimal.
fn pad(number: SequenceNumber, width: usize) -> String {
	let integer = number.integer_part();
	let padded = format!("{:0width$}", integer, width = width);
	let fraction = (number.thousandths() - integer * 1000).abs();
	if fraction == 0 {
		return padded;
	}

	let decimals = format!("{fraction:03}").trim_end_matches('0').to_string();
	format!("{padded}.{decimals}")
}

fn plan_pack(paths: &[PathBuf], options: &CbzitOptions) -> Result<Plan, ToolError> {
	let mut plan = Plan::new(ID);
	let mut claimed: HashMap<PathBuf, usize> = HashMap::new();

	for root in paths {
		if !root.is_dir() {
			plan.warn(
				Warning::new(
					"not-a-directory",
					"pack mode takes directories of page images; pass merge for archives",
				)
				.at(root)
				.with_severity(Severity::Error),
			);
			continue;
		}

		let folders = image_folders(root)?;
		if folders.is_empty() {
			plan.warn(
				Warning::new("no-images", "no page images found under this path")
					.at(root)
					.with_severity(Severity::Error),
			);
			continue;
		}

		for folder in folders {
			if folder.pages.len() < options.min_pages.max(1) {
				plan.warn(
					Warning::new(
						"too-few-pages",
						format!(
							"{} page(s), fewer than the {} required",
							folder.pages.len(),
							options.min_pages.max(1)
						),
					)
					.at(&folder.dir),
				);
				continue;
			}
			if folder.mixed {
				plan.warn(
					Warning::new(
						"mixed-folder",
						"this folder holds both page images and image subfolders; its own images are packed separately",
					)
					.at(&folder.dir),
				);
			}

			let (identity, warnings) = identify(root, &folder.dir, options);
			for warning in warnings {
				plan.warn(warning);
			}

			let output_dir = options.output_dir.clone().unwrap_or_else(|| root.clone());
			let mut target = output_dir.join(identity.archive_name());
			let duplicates = claimed.entry(target.clone()).or_insert(0);
			*duplicates += 1;
			if *duplicates > 1 {
				// Duplicate chapters are kept, never dropped: the second copy
				// is suffixed so both survive for the user to compare.
				let suffix = *duplicates;
				let stem = target
					.file_stem()
					.map(|stem| stem.to_string_lossy().into_owned())
					.unwrap_or_default();
				target = output_dir.join(format!("{stem} ({suffix}).cbz"));
				plan.warn(
					Warning::new(
						"duplicate-chapter",
						format!(
							"another folder already resolved to this name; kept as {}",
							target.file_name().unwrap_or_default().to_string_lossy()
						),
					)
					.at(&folder.dir),
				);
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
					.at(&folder.dir),
				);
				continue;
			}

			let detail = PackDetail {
				pages: folder.pages.len(),
				series: (!identity.series.is_empty()).then(|| identity.series.clone()),
				volume: identity.volume.map(|volume| volume.integer_part() as i32),
				chapter: identity.chapter.map(|chapter| chapter.to_string()),
				title: identity.title.clone(),
				language: identity.language.clone(),
				comic_info: options.comic_info,
				overwrite: options.overwrite,
				sources: folder.pages,
			};
			plan.push(
				Action::new(ACTION_PACK)
					.with_source(&folder.dir)
					.with_target(&target)
					.with_detail(serde_json::to_value(&detail)?),
			);
		}
	}

	Ok(plan)
}

fn plan_merge(paths: &[PathBuf], options: &CbzitOptions) -> Result<Plan, ToolError> {
	let mut plan = Plan::new(ID);
	let mut groups: Vec<VolumeGroup> = Vec::new();

	for root in paths {
		let archives = if root.is_dir() {
			util::sorted_files(root, true)?
				.into_iter()
				.filter(|path| is_cbz(path))
				.collect::<Vec<_>>()
		} else if is_cbz(root) {
			vec![root.clone()]
		} else {
			plan.warn(
				Warning::new("not-an-archive", "merge mode takes .cbz files")
					.at(root)
					.with_severity(Severity::Error),
			);
			continue;
		};

		if archives.is_empty() {
			plan.warn(
				Warning::new("no-archives", "no .cbz files found under this path")
					.at(root)
					.with_severity(Severity::Error),
			);
			continue;
		}

		let output_dir = options.output_dir.clone().unwrap_or_else(|| {
			if root.is_dir() {
				root.clone()
			} else {
				root.parent().map(Path::to_path_buf).unwrap_or_default()
			}
		});

		for archive in archives {
			let (identity, _) = identify(root, &archive, options);
			let Some(volume) = identity.volume else {
				plan.warn(
					Warning::new(
						"no-volume",
						"no volume identifier in the name or its folders; left alone",
					)
					.at(&archive),
				);
				continue;
			};

			let key = (identity.series.clone(), volume.thousandths());
			let group = match groups.iter_mut().find(|group| group.key == key) {
				Some(group) => group,
				None => {
					groups.push(VolumeGroup {
						key,
						identity: Identity {
							title: None,
							chapter: None,
							..identity.clone()
						},
						output_dir: output_dir.clone(),
						chapters: Vec::new(),
					});
					groups.last_mut().expect("just pushed")
				},
			};
			group.chapters.push((identity.chapter, archive));
		}
	}

	for mut group in groups {
		// Chapter order first, then name, so an unnumbered extra lands last.
		group
			.chapters
			.sort_by(|(left, left_path), (right, right_path)| {
				left.map(|number| number.thousandths())
					.unwrap_or(i64::MAX)
					.cmp(&right.map(|number| number.thousandths()).unwrap_or(i64::MAX))
					.then_with(|| {
						alphanumeric_sort::compare_str(
							left_path.to_string_lossy(),
							right_path.to_string_lossy(),
						)
					})
			});

		if group.chapters.len() < 2 {
			plan.warn(
				Warning::new(
					"single-chapter",
					format!(
						"only one archive resolved to {}; nothing to merge",
						group.identity.archive_name()
					),
				)
				.at(&group.chapters[0].1)
				.with_severity(Severity::Info),
			);
			continue;
		}

		let sources = group
			.chapters
			.iter()
			.map(|(_, path)| path.clone())
			.collect::<Vec<_>>();
		let mut pages = 0;
		let mut bytes = 0;
		let mut unreadable = false;
		for source in &sources {
			match archive_pages(source) {
				Ok(entries) => {
					pages += entries.len();
					bytes += entries.iter().map(|entry| entry.size).sum::<u64>();
				},
				Err(error) => {
					plan.warn(
						Warning::new(
							"unreadable",
							format!("cannot read archive: {error}"),
						)
						.at(source)
						.with_severity(Severity::Error),
					);
					unreadable = true;
				},
			}
		}
		if unreadable {
			continue;
		}
		if pages == 0 {
			plan.warn(
				Warning::new("no-images", "the grouped archives hold no page images")
					.at(&sources[0])
					.with_severity(Severity::Error),
			);
			continue;
		}
		if bytes > options.max_merge_bytes {
			plan.warn(
				Warning::new(
					"merge-too-large",
					format!(
						"{bytes} bytes of pages exceeds the {} byte guard; chapters left alone",
						options.max_merge_bytes
					),
				)
				.at(&sources[0])
				.with_severity(Severity::Error),
			);
			continue;
		}

		let target = group.output_dir.join(group.identity.archive_name());
		if target.exists() && !options.overwrite {
			plan.warn(
				Warning::new(
					"target-exists",
					format!("{} exists; pass overwrite to replace it", target.display()),
				)
				.at(&sources[0]),
			);
			continue;
		}
		if sources.contains(&target) {
			plan.warn(
				Warning::new(
					"target-is-source",
					format!(
						"{} is one of the chapters being merged; rename it first",
						target.display()
					),
				)
				.at(&target)
				.with_severity(Severity::Error),
			);
			continue;
		}

		let detail = MergeDetail {
			pages,
			bytes,
			chapters: sources.len(),
			series: (!group.identity.series.is_empty())
				.then(|| group.identity.series.clone()),
			volume: group
				.identity
				.volume
				.map(|volume| volume.integer_part() as i32),
			language: group.identity.language.clone(),
			comic_info: options.comic_info,
			overwrite: options.overwrite,
			sources,
		};
		plan.push(
			Action::new(ACTION_MERGE)
				.with_target(&target)
				.with_detail(serde_json::to_value(&detail)?),
		);
	}

	Ok(plan)
}

struct VolumeGroup {
	key: (String, i64),
	identity: Identity,
	output_dir: PathBuf,
	chapters: Vec<(Option<SequenceNumber>, PathBuf)>,
}

/// One folder that directly holds page images.
#[derive(Debug)]
struct ImageFolder {
	dir: PathBuf,
	pages: Vec<PathBuf>,
	/// True when the folder also has image-bearing subfolders.
	mixed: bool,
}

/// Every folder under `root` (inclusive) that directly holds page images, in
/// natural depth-first order.
fn image_folders(root: &Path) -> ToolResult<Vec<ImageFolder>> {
	let mut folders = Vec::new();
	visit_folder(root, &mut folders)?;

	Ok(folders)
}

fn visit_folder(dir: &Path, folders: &mut Vec<ImageFolder>) -> ToolResult<bool> {
	let pages = util::sorted_files(dir, false)?
		.into_iter()
		.filter(|path| util::is_page_file(path))
		.collect::<Vec<_>>();
	let position = folders.len();
	let has_pages = !pages.is_empty();
	if has_pages {
		folders.push(ImageFolder {
			dir: dir.to_path_buf(),
			pages,
			mixed: false,
		});
	}

	let mut nested = false;
	for sub in util::sorted_dirs(dir)? {
		nested |= visit_folder(&sub, folders)?;
	}
	if has_pages && nested {
		folders[position].mixed = true;
	}

	Ok(has_pages || nested)
}

fn is_cbz(path: &Path) -> bool {
	!path.is_hidden_file()
		&& path
			.extension()
			.is_some_and(|ext| ext.eq_ignore_ascii_case("cbz"))
}

/// Detect the identity of `path` bottom-up.
///
/// The chain is the path components between `root` and `path`. The deepest name
/// wins for chapter, volume and language; the series is the shallowest ancestor
/// name with real text, else the deepest name's own leading text, else `root`'s
/// name (see [`split_name`]).
fn identify(
	root: &Path,
	path: &Path,
	options: &CbzitOptions,
) -> (Identity, Vec<Warning>) {
	let mut warnings = Vec::new();
	let mut chain = path
		.strip_prefix(root)
		.map(|relative| {
			relative
				.components()
				.map(|component| component.as_os_str().to_string_lossy().into_owned())
				.collect::<Vec<_>>()
		})
		.unwrap_or_default();
	if chain.is_empty() {
		chain = root
			.file_name()
			.map(|name| vec![name.to_string_lossy().into_owned()])
			.unwrap_or_default();
	}
	// A `.cbz` in merge mode: the extension is not part of the name grammar.
	if let Some(last) = chain.last_mut() {
		if let Some(stem) = Path::new(last.as_str())
			.file_stem()
			.map(|stem| stem.to_string_lossy().into_owned())
		{
			if is_cbz(Path::new(last.as_str())) {
				*last = stem;
			}
		}
	}

	let mut chapter = None;
	let mut volume = None;
	let mut language = None;
	for name in chain.iter().rev() {
		let found_chapter =
			parse_identifier_of(name, SequenceKind::Chapter).map(|parsed| parsed.start);
		let found_volume =
			parse_identifier_of(name, SequenceKind::Volume).map(|parsed| parsed.start);

		if let (Some(deep), Some(shallow)) = (volume, found_volume) {
			if deep != shallow {
				warnings.push(
					Warning::new(
						"conflicting-volume",
						format!("volume {shallow} in {name:?} disagrees with {deep} from a deeper folder; keeping {deep}"),
					)
					.at(path),
				);
			}
		}
		chapter = chapter.or(found_chapter);
		volume = volume.or(found_volume);
		language = language.or_else(|| language_tag(name));
	}

	// The deepest name describes *this* archive, so its leading text is only a
	// series when no ancestor supplies one, and its trailing text is a title.
	let split = chain.split_last();
	let (deepest_series, title) = split
		.map(|(deepest, _)| split_name(deepest))
		.unwrap_or((None, None));
	let series = options
		.series
		.clone()
		.or_else(|| {
			split
				.into_iter()
				.flat_map(|(_, ancestors)| ancestors)
				.find_map(|name| {
					let (before, after) = split_name(name);
					before.or(after)
				})
		})
		.or(deepest_series)
		.or_else(|| {
			root.file_name()
				.map(|name| clean_name(&name.to_string_lossy()).trim().to_string())
		})
		.unwrap_or_default();

	if series.is_empty() {
		warnings.push(
			Warning::new(
				"ambiguous-series",
				"no series name could be derived from the folder names",
			)
			.at(path)
			.with_severity(Severity::Error),
		);
	}
	if chapter.is_none() && volume.is_none() {
		warnings.push(
			Warning::new(
				"ambiguous-number",
				"no chapter or volume identifier in the name or its folders; naming from the title only",
			)
			.at(path),
		);
	}

	let identity = Identity {
		series,
		volume,
		chapter,
		title,
		language: options.language.clone().or(language),
	};

	(identity, warnings)
}

/// Common language tags, so `[en]`, `(Japanese)` or a `ja` folder all become a
/// ComicInfo `LanguageISO` value.
const LANGUAGES: [(&str, &str); 12] = [
	("en", "english"),
	("ja", "japanese"),
	("ko", "korean"),
	("zh", "chinese"),
	("es", "spanish"),
	("fr", "french"),
	("de", "german"),
	("it", "italian"),
	("pt", "portuguese"),
	("ru", "russian"),
	("nl", "dutch"),
	("pl", "polish"),
];

/// A language tag from a bracketed/parenthesised span, or from the whole name.
fn language_tag(name: &str) -> Option<String> {
	let mut candidates = Vec::new();
	let mut current = String::new();
	let mut depth = 0usize;
	for character in name.chars() {
		match character {
			'[' | '(' => depth += 1,
			']' | ')' => {
				depth = depth.saturating_sub(1);
				if !current.trim().is_empty() {
					candidates.push(current.trim().to_string());
				}
				current.clear();
			},
			_ if depth > 0 => current.push(character),
			_ => {},
		}
	}
	candidates.push(name.trim().to_string());

	candidates.iter().find_map(|candidate| {
		let lowered = candidate.to_lowercase();
		LANGUAGES
			.iter()
			.find(|(code, english)| lowered == *code || lowered == *english)
			.map(|(code, _)| code.to_string())
	})
}

/// Split `name` around its chapter/volume identifiers: the text *before* the
/// first identifier is a series, the text *after* the last one is a title. A
/// name with no identifier at all is all series text.
///
/// This is the only naming rule `cbzit` owns: bracketed spans and separators
/// are removed by [`stump_scanner::clean_name`] and the numbers themselves are
/// parsed by [`stump_scanner::parse_identifier_of`], so `Series v01 c003 - The
/// Brand` splits into `("Series", "The Brand")` under one shared grammar. The
/// identifier words mirror the scanner's (Chapter/Chap/Ch/C/Episode/Ep and
/// Volume/Vol/V), agreed with the `missing-sequence` work.
fn split_name(name: &str) -> (Option<String>, Option<String>) {
	let cleaned = clean_name(name);
	let mut rest = cleaned.as_str();
	let mut before = None;
	let mut tail = rest;
	let mut found = false;

	while let Some((start, end)) = identifier_span(rest) {
		if !found {
			before = tidy(&rest[..start]);
			found = true;
		}
		rest = &rest[end..];
		// Only the run after the *last* identifier can be a title.
		tail = rest;
	}

	if !found {
		return (tidy(tail), None);
	}

	(before, tidy(tail))
}

/// Trim separator noise and collapse whitespace; `None` when nothing is left.
fn tidy(text: &str) -> Option<String> {
	let text = text
		.trim_matches(|c: char| {
			c.is_whitespace() || matches!(c, '-' | '_' | ':' | '.' | '~')
		})
		.split_whitespace()
		.collect::<Vec<_>>()
		.join(" ");

	(!text.is_empty()).then_some(text)
}

/// The byte range of the first `<identifier><number>` phrase in `text`.
fn identifier_span(text: &str) -> Option<(usize, usize)> {
	const WORDS: [&str; 8] = [
		"chapter", "chap", "ch", "c", "episode", "ep", "volume", "vol",
	];

	let lowered = text.to_lowercase();
	let bytes = lowered.as_bytes();
	let mut index = 0;
	while index < bytes.len() {
		if index > 0 && !is_boundary(bytes[index - 1]) {
			index += 1;
			continue;
		}

		let word = WORDS
			.iter()
			.find(|word| lowered[index..].starts_with(**word))
			.copied()
			.or_else(|| {
				// A bare `v` prefix (`v01`) is a volume in every scanner test.
				lowered[index..].starts_with('v').then_some("v")
			});
		let Some(word) = word else {
			index += 1;
			continue;
		};

		let mut cursor = index + word.len();
		while cursor < bytes.len()
			&& matches!(bytes[cursor], b' ' | b'.' | b'_' | b'-' | b'#')
		{
			cursor += 1;
		}
		let digits_start = cursor;
		while cursor < bytes.len()
			&& (bytes[cursor].is_ascii_digit() || bytes[cursor] == b'.')
		{
			cursor += 1;
		}
		if cursor > digits_start {
			// Trailing `.` belongs to the sentence, not the number.
			while cursor > digits_start && bytes[cursor - 1] == b'.' {
				cursor -= 1;
			}
			return Some((index, cursor));
		}

		index += word.len().max(1);
	}

	None
}

fn is_boundary(byte: u8) -> bool {
	!byte.is_ascii_alphanumeric()
}

/// One page entry inside a source archive.
#[derive(Debug, Clone, PartialEq)]
struct ArchiveEntry {
	index: usize,
	extension: String,
	size: u64,
}

/// The page entries of a CBZ in natural name order, skipping hidden entries,
/// directories, and `ComicInfo.xml` — the same rules the ZIP processor uses.
fn archive_pages(path: &Path) -> ToolResult<Vec<ArchiveEntry>> {
	let mut archive = ZipArchive::new(File::open(path)?)?;
	let mut entries = Vec::new();
	for index in 0..archive.len() {
		let entry = archive.by_index(index)?;
		if entry.is_dir() {
			continue;
		}
		let name = entry
			.enclosed_name()
			.unwrap_or_else(|| PathBuf::from(entry.name()));
		if !util::is_page_file(&name) {
			continue;
		}
		entries.push((
			name.to_string_lossy().into_owned(),
			ArchiveEntry {
				index,
				extension: name
					.extension()
					.map(|ext| ext.to_string_lossy().to_lowercase())
					.unwrap_or_else(|| "jpg".to_string()),
				size: entry.size(),
			},
		));
	}
	entries.sort_by(|(left, _), (right, _)| alphanumeric_sort::compare_str(left, right));

	Ok(entries.into_iter().map(|(_, entry)| entry).collect())
}

/// A lazily-read run of page entries from one archive, so merging a volume
/// never holds more than one page in memory.
struct ArchiveStream {
	archive: ZipArchive<File>,
	entries: std::vec::IntoIter<ArchiveEntry>,
}

impl ArchiveStream {
	fn open(path: &Path) -> ToolResult<Self> {
		let entries = archive_pages(path)?;
		Ok(Self {
			archive: ZipArchive::new(File::open(path)?)?,
			entries: entries.into_iter(),
		})
	}
}

impl Iterator for ArchiveStream {
	type Item = ToolResult<(String, Vec<u8>)>;

	fn next(&mut self) -> Option<Self::Item> {
		let entry = self.entries.next()?;
		let mut file = match self.archive.by_index(entry.index) {
			Ok(file) => file,
			Err(error) => return Some(Err(error.into())),
		};
		let mut bytes = Vec::with_capacity(entry.size as usize);
		match file.read_to_end(&mut bytes) {
			Ok(_) => Some(Ok((entry.extension, bytes))),
			Err(error) => Some(Err(error.into())),
		}
	}
}

fn apply_pack(action: &Action, target: &Path) -> ToolResult<Option<usize>> {
	let detail = serde_json::from_value::<PackDetail>(action.detail.clone())
		.map_err(|error| ToolError::Invalid(format!("malformed action: {error}")))?;
	if target.exists() && !detail.overwrite {
		return Ok(None);
	}

	let comic_info = detail.comic_info.then(|| {
		util::ComicInfo {
			title: detail.title.clone(),
			series: detail.series.clone(),
			number: detail.chapter.clone(),
			volume: detail.volume,
			page_count: Some(detail.pages),
			language: detail.language.clone(),
			..Default::default()
		}
		.to_xml()
	});

	let pages = detail.sources.iter().enumerate().map(|(index, source)| {
		let bytes = std::fs::read(source)?;
		let extension = source
			.extension()
			.map(|ext| ext.to_string_lossy().to_lowercase())
			.unwrap_or_else(|| "jpg".to_string());

		Ok((util::page_entry_name(index, &extension), bytes))
	});

	util::write_cbz_file(target, comic_info.as_ref().map(|xml| xml.as_bytes()), pages)
		.map(Some)
}

fn apply_merge(action: &Action, target: &Path) -> ToolResult<Option<usize>> {
	let detail = serde_json::from_value::<MergeDetail>(action.detail.clone())
		.map_err(|error| ToolError::Invalid(format!("malformed action: {error}")))?;
	if target.exists() && !detail.overwrite {
		return Ok(None);
	}

	let comic_info = detail.comic_info.then(|| {
		util::ComicInfo {
			series: detail.series.clone(),
			volume: detail.volume,
			number: detail.volume.map(|volume| volume.to_string()),
			page_count: Some(detail.pages),
			language: detail.language.clone(),
			..Default::default()
		}
		.to_xml()
	});

	let streams = detail
		.sources
		.iter()
		.map(|source| ArchiveStream::open(source))
		.collect::<ToolResult<Vec<_>>>()?;
	let pages = streams
		.into_iter()
		.flatten()
		.enumerate()
		.map(|(index, page)| {
			let (extension, bytes) = page?;
			Ok((util::page_entry_name(index, &extension), bytes))
		});

	util::write_cbz_file(target, comic_info.as_ref().map(|xml| xml.as_bytes()), pages)
		.map(Some)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::NoopProgress;
	use std::io::Write;

	fn write_pages(dir: &Path, names: &[&str]) {
		std::fs::create_dir_all(dir).unwrap();
		for name in names {
			std::fs::write(dir.join(name), format!("bytes:{name}")).unwrap();
		}
	}

	fn entries(path: &Path) -> Vec<String> {
		let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
		(0..archive.len())
			.map(|index| archive.by_index(index).unwrap().name().to_string())
			.collect()
	}

	fn read_entry(path: &Path, name: &str) -> Vec<u8> {
		let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
		let mut entry = archive.by_name(name).unwrap();
		let mut bytes = Vec::new();
		entry.read_to_end(&mut bytes).unwrap();
		bytes
	}

	fn plan_for(paths: Vec<PathBuf>, options: serde_json::Value) -> Plan {
		Cbzit
			.plan(&ToolInput::new(paths).with_options(options))
			.unwrap()
	}

	#[test]
	fn packs_a_series_tree_into_cleanly_named_archives() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("Berserk");
		write_pages(&root.join("v01/c001"), &["1.jpg", "2.jpg", "10.jpg"]);
		write_pages(&root.join("v01/c002 - The Brand"), &["1.jpg", "2.jpg"]);
		write_pages(&root.join("v02/Chapter 10.5"), &["a.png"]);

		let plan = plan_for(vec![root.clone()], serde_json::json!({}));
		let names = plan
			.actions
			.iter()
			.map(|action| {
				action
					.target
					.as_ref()
					.unwrap()
					.file_name()
					.unwrap()
					.to_string_lossy()
					.into_owned()
			})
			.collect::<Vec<_>>();
		assert_eq!(
			names,
			vec![
				"Berserk v01 c001.cbz",
				"Berserk v01 c002 - The Brand.cbz",
				"Berserk v02 c010.5.cbz",
			]
		);
		assert_eq!(plan.actions[0].detail["pages"], 3);
		assert!(
			plan.warnings.is_empty(),
			"clean tree must not warn: {:?}",
			plan.warnings
		);

		let report = Cbzit.apply(&plan, &mut NoopProgress).unwrap();
		assert_eq!(report.applied.len(), 3);
		assert_eq!(report.skipped, vec![]);

		let target = root.join("Berserk v01 c001.cbz");
		assert_eq!(
			entries(&target),
			vec!["ComicInfo.xml", "0001.jpg", "0002.jpg", "0003.jpg"],
			"pages are renumbered in natural order"
		);
		assert_eq!(read_entry(&target, "0003.jpg"), b"bytes:10.jpg");
		assert!(
			root.join("v01/c001/1.jpg").exists(),
			"sources must not be touched"
		);

		let xml = String::from_utf8(read_entry(&target, "ComicInfo.xml")).unwrap();
		let parsed: stump_media::ProcessedMediaMetadata =
			quick_xml::de::from_str(&xml).unwrap();
		assert_eq!(parsed.series.as_deref(), Some("Berserk"));
		assert_eq!(parsed.number, Some(1.0));
		assert_eq!(parsed.volume, Some(1));
		assert_eq!(parsed.page_count, Some(3));
	}

	#[test]
	fn names_round_trip_through_the_scanner_parser() {
		let identity = Identity {
			series: "Berserk".to_string(),
			volume: Some(SequenceNumber::from_integer(3)),
			chapter: Some(SequenceNumber::from_thousandths(10_500)),
			title: Some("The Brand".to_string()),
			language: Some("ja".to_string()),
		};
		let name = identity.archive_name();
		assert_eq!(name, "Berserk v03 c010.5 - The Brand [ja].cbz");

		let volume = parse_identifier_of(&name, SequenceKind::Volume).unwrap();
		let chapter = parse_identifier_of(&name, SequenceKind::Chapter).unwrap();
		assert_eq!(volume.start, SequenceNumber::from_integer(3));
		assert_eq!(chapter.start, SequenceNumber::from_thousandths(10_500));
	}

	#[test]
	fn bottom_up_detection_prefers_the_deepest_name() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("Manga");
		write_pages(&root.join("Vinland Saga [en]/v02/c014"), &["1.jpg"]);

		let plan = plan_for(vec![root], serde_json::json!({}));
		assert_eq!(plan.actions.len(), 1);
		let detail: PackDetail =
			serde_json::from_value(plan.actions[0].detail.clone()).unwrap();
		assert_eq!(detail.series.as_deref(), Some("Vinland Saga"));
		assert_eq!(detail.volume, Some(2));
		assert_eq!(detail.chapter.as_deref(), Some("14"));
		assert_eq!(detail.language.as_deref(), Some("en"));
		assert_eq!(detail.title, None);
	}

	#[test]
	fn ambiguities_and_conflicts_are_warnings_not_failures() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("Loose");
		write_pages(&root.join("random folder"), &["1.jpg"]);
		write_pages(&root.join("v01/v02/c003"), &["1.jpg"]);

		let plan = plan_for(vec![root], serde_json::json!({}));
		let codes = plan
			.warnings
			.iter()
			.map(|warning| warning.code.as_str())
			.collect::<Vec<_>>();
		assert!(codes.contains(&"ambiguous-number"), "{codes:?}");
		assert!(codes.contains(&"conflicting-volume"), "{codes:?}");
		assert_eq!(plan.actions.len(), 2, "warnings never drop work");
	}

	#[test]
	fn duplicate_chapters_are_both_kept() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("Dup");
		write_pages(&root.join("c001"), &["1.jpg"]);
		write_pages(&root.join("Chapter 01"), &["1.jpg"]);

		let plan = plan_for(vec![root.clone()], serde_json::json!({}));
		let names = plan
			.actions
			.iter()
			.map(|action| {
				action
					.target
					.as_ref()
					.unwrap()
					.file_name()
					.unwrap()
					.to_string_lossy()
					.into_owned()
			})
			.collect::<Vec<_>>();
		assert_eq!(names, vec!["Dup c001.cbz", "Dup c001 (2).cbz"]);
		assert!(plan
			.warnings
			.iter()
			.any(|warning| warning.code == "duplicate-chapter"));

		Cbzit.apply(&plan, &mut NoopProgress).unwrap();
		assert!(root.join("Dup c001.cbz").exists());
		assert!(root.join("Dup c001 (2).cbz").exists());
	}

	#[test]
	fn existing_archives_are_never_clobbered_without_overwrite() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("Keep");
		write_pages(&root.join("c001"), &["1.jpg"]);
		let target = root.join("Keep c001.cbz");
		std::fs::write(&target, b"original").unwrap();

		let plan = plan_for(vec![root.clone()], serde_json::json!({}));
		assert!(plan.is_empty());
		assert_eq!(plan.warnings[0].code, "target-exists");
		assert_eq!(std::fs::read(&target).unwrap(), b"original");

		// A plan made before the file appeared must still refuse to clobber.
		std::fs::remove_file(&target).unwrap();
		let plan = plan_for(vec![root.clone()], serde_json::json!({}));
		std::fs::write(&target, b"original").unwrap();
		let report = Cbzit.apply(&plan, &mut NoopProgress).unwrap();
		assert_eq!(report.applied, vec![]);
		assert_eq!(report.skipped.len(), 1);
		assert_eq!(report.skipped[0].1, "target already exists");
		assert_eq!(std::fs::read(&target).unwrap(), b"original");

		let plan = plan_for(vec![root], serde_json::json!({ "overwrite": true }));
		Cbzit.apply(&plan, &mut NoopProgress).unwrap();
		assert_eq!(entries(&target).len(), 2);
	}

	#[test]
	fn options_override_detection_and_output_location() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("in");
		let out = dir.path().join("out");
		write_pages(&root.join("c007"), &["1.jpg", "2.jpg"]);

		let plan = plan_for(
			vec![root],
			serde_json::json!({
				"series": "Real Series",
				"language": "fr",
				"comic_info": false,
				"output_dir": out,
			}),
		);
		Cbzit.apply(&plan, &mut NoopProgress).unwrap();

		let target = out.join("Real Series c007 [fr].cbz");
		assert_eq!(entries(&target), vec!["0001.jpg", "0002.jpg"]);
	}

	#[test]
	fn too_few_pages_is_reported_and_skipped() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("Thin");
		write_pages(&root.join("c001"), &["1.jpg"]);
		write_pages(&root.join("c002"), &["1.jpg", "2.jpg", "3.jpg"]);

		let plan = plan_for(vec![root], serde_json::json!({ "min_pages": 2 }));
		assert_eq!(plan.actions.len(), 1);
		assert_eq!(plan.warnings[0].code, "too-few-pages");
	}

	#[test]
	fn merge_consolidates_chapters_into_a_volume_and_guards_size() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("Berserk");
		write_pages(&root.join("v01/c001"), &["1.jpg", "2.jpg"]);
		write_pages(&root.join("v01/c002"), &["9.jpg"]);
		write_pages(&root.join("v02/c003"), &["1.jpg"]);

		let plan = plan_for(vec![root.clone()], serde_json::json!({}));
		Cbzit.apply(&plan, &mut NoopProgress).unwrap();

		let plan = plan_for(vec![root.clone()], serde_json::json!({ "merge": true }));
		assert_eq!(plan.actions.len(), 1, "only v01 has multiple chapters");
		let action = &plan.actions[0];
		assert_eq!(action.kind, ACTION_MERGE);
		assert_eq!(action.detail["pages"], 3);
		assert_eq!(action.detail["chapters"], 2);
		assert!(plan
			.warnings
			.iter()
			.any(|warning| warning.code == "single-chapter"));

		let report = Cbzit.apply(&plan, &mut NoopProgress).unwrap();
		assert_eq!(report.applied.len(), 1);

		let target = root.join("Berserk v01.cbz");
		assert_eq!(
			entries(&target),
			vec!["ComicInfo.xml", "0001.jpg", "0002.jpg", "0003.jpg"]
		);
		assert_eq!(
			read_entry(&target, "0003.jpg"),
			b"bytes:9.jpg",
			"c002's page follows c001's pages"
		);
		assert!(
			root.join("Berserk v01 c001.cbz").exists(),
			"merged chapters are left in place"
		);

		let plan = plan_for(
			vec![root],
			serde_json::json!({ "merge": true, "max_merge_bytes": 4, "overwrite": true }),
		);
		assert!(plan.is_empty());
		assert!(plan
			.warnings
			.iter()
			.any(|warning| warning.code == "merge-too-large"));
	}

	#[test]
	fn merge_refuses_to_overwrite_one_of_its_own_sources() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("Odd");
		std::fs::create_dir_all(&root).unwrap();
		for name in ["Odd v01.cbz", "Odd v01 c002.cbz"] {
			let mut zip = zip::ZipWriter::new(File::create(root.join(name)).unwrap());
			zip.start_file("0001.jpg", zip::write::SimpleFileOptions::default())
				.unwrap();
			zip.write_all(b"page").unwrap();
			zip.finish().unwrap();
		}

		let plan = plan_for(
			vec![root],
			serde_json::json!({ "merge": true, "overwrite": true }),
		);
		assert!(plan.is_empty());
		assert_eq!(plan.warnings[0].code, "target-is-source");
	}

	#[test]
	fn names_split_into_series_text_before_and_title_text_after() {
		let some = |text: &str| Some(text.to_string());

		assert_eq!(split_name("c001"), (None, None));
		assert_eq!(split_name("v01"), (None, None));
		assert_eq!(split_name("Chapter 10.5"), (None, None));
		// No identifier: the whole name is series text, never a title.
		assert_eq!(split_name("Berserk"), (some("Berserk"), None));
		assert_eq!(
			split_name("Berserk - c003 - The Brand"),
			(some("Berserk"), some("The Brand"))
		);
		assert_eq!(
			split_name("Berserk v01 c003 - The Brand"),
			(some("Berserk"), some("The Brand")),
			"text between identifiers is not a title"
		);
		assert_eq!(
			split_name("Vinland Saga [en] (2019)"),
			(some("Vinland Saga"), None),
			"bracketed spans are removed by the scanner's clean_name"
		);
		assert_eq!(
			split_name("Chainsaw Man Ep 5"),
			(some("Chainsaw Man"), None)
		);
	}

	#[test]
	fn language_tags_come_from_brackets_or_the_whole_name() {
		assert_eq!(language_tag("Vinland Saga [en]"), Some("en".to_string()));
		assert_eq!(language_tag("Berserk (Japanese)"), Some("ja".to_string()));
		assert_eq!(language_tag("ja"), Some("ja".to_string()));
		assert_eq!(language_tag("Berserk"), None);
		assert_eq!(language_tag("[HQ] Berserk"), None);
	}

	#[test]
	fn padding_keeps_reading_order_and_decimals() {
		assert_eq!(pad(SequenceNumber::from_integer(1), 3), "001");
		assert_eq!(pad(SequenceNumber::from_integer(112), 3), "112");
		assert_eq!(pad(SequenceNumber::from_thousandths(10_500), 3), "010.5");
		assert_eq!(pad(SequenceNumber::from_integer(3), 2), "03");
		assert_eq!(pad(SequenceNumber::from_thousandths(1_250), 3), "001.25");
	}

	#[test]
	fn empty_and_wrong_inputs_error_instead_of_guessing() {
		let error = Cbzit.plan(&ToolInput::new(vec![])).unwrap_err();
		assert!(matches!(error, ToolError::Invalid(_)), "{error}");

		let error = Cbzit
			.plan(&ToolInput::new(vec![PathBuf::from("/nope/missing")]))
			.unwrap_err();
		assert!(matches!(error, ToolError::Invalid(_)), "{error}");

		let error = Cbzit
			.apply(&Plan::new("epub2cbz"), &mut NoopProgress)
			.unwrap_err();
		assert!(matches!(error, ToolError::PlanMismatch { .. }), "{error}");

		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
		let plan = plan_for(vec![dir.path().to_path_buf()], serde_json::json!({}));
		assert!(plan.is_empty());
		assert_eq!(plan.warnings[0].code, "no-images");
	}
}
