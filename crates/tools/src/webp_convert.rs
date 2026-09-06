//! `webp-convert`: re-encode the pages of a CBZ as WebP.
//!
//! Behaviour source: Kavita's external tools guide lists a WebP converter that
//! "converts your images to webp" to shrink a library
//! (<https://wiki.kavitareader.com/guides/external-tools/>). Nothing is copied
//! from that (GPL) tool, and no new image dependency is introduced: the pixels
//! go through the very same pipeline Stump already uses to prepare pages for a
//! device ([`stump_media::transform::transform_page_bytes`] with a
//! [`TransformProfile`] whose format is
//! [`TransformFormat::Webp`]), so the encoder, the resize kernel and the
//! never-upscale rule are the ones the koreader/phone presets are validated
//! against.
//!
//! What is *not* re-encoded is as important as what is:
//!
//! - a page that is already WebP is copied with its original bytes, so a second
//!   run is a no-op rather than a generational quality loss;
//! - `ComicInfo.xml`, `.stump-covers.json`, and every other non-page member are
//!   raw-copied (same compressed bytes), so metadata written by the other tools
//!   survives untouched;
//! - a page keeps its entry name, only the extension changes, which is what
//!   keeps the archive's page order (and therefore a reader's progress) intact.
//!
//! The option table, action kinds and warning codes are in
//! `crates/tools/README.md`.

use std::{
	collections::{BTreeSet, HashMap},
	fs::File,
	io::{Read, Write},
	path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use stump_media::transform::{
	transform_page_bytes, ComicContainer, TransformFormat, TransformProfile,
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

use crate::{
	util, Action, Plan, ProgressSink, Report, Severity, Tool, ToolError, ToolInput,
	ToolResult, Warning,
};

/// The registered tool id.
pub const ID: &str = "webp-convert";
/// The only [`Action::kind`] this tool plans.
pub const ACTION_CONVERT: &str = "convert-pages";

/// Default WebP quality. 80 is the knee of the size/quality curve for scanned
/// comic pages: visually indistinguishable from the JPEG originals at roughly
/// two thirds of their size.
pub const DEFAULT_QUALITY: u8 = 80;

/// Default stem suffix for the converted sibling archive.
///
/// Bracketed, because [`stump_scanner::clean_name`] strips `[...]` spans: the
/// converted file still identifies as the same series, volume and chapter when
/// the library is scanned.
pub const DEFAULT_SUFFIX: &str = " [webp]";

/// The extension of a page that is already WebP and must not be re-encoded.
const WEBP_EXTENSION: &str = "webp";

pub struct WebpConvert;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct WebpConvertOptions {
	/// WebP quality, `1..=100`.
	pub quality: u8,
	/// Largest allowed page width; pages are only ever downscaled.
	pub max_width: Option<u32>,
	/// Largest allowed page height; pages are only ever downscaled.
	pub max_height: Option<u32>,
	/// Where converted archives are written. Default: next to the source.
	pub output_dir: Option<PathBuf>,
	/// Stem suffix used when the output lands in the source's own directory.
	pub suffix: String,
	/// Descend into subdirectories when a given path is a directory.
	pub recursive: bool,
	/// Replace an existing target (or an existing backup) instead of skipping.
	pub overwrite: bool,
	/// Replace the source archive, keeping a `<file name>.bak` next to it.
	pub in_place: bool,
}

impl Default for WebpConvertOptions {
	fn default() -> Self {
		Self {
			quality: DEFAULT_QUALITY,
			max_width: None,
			max_height: None,
			output_dir: None,
			suffix: DEFAULT_SUFFIX.to_string(),
			recursive: false,
			overwrite: false,
			in_place: false,
		}
	}
}

/// What one page becomes. Recorded in the plan so an apply re-derives nothing:
/// the dry run already names every entry the new archive will hold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PagePlan {
	/// Entry name in the source archive.
	entry: String,
	/// Entry name in the converted archive.
	target: String,
	/// `false` for a page that is already WebP and is copied verbatim.
	convert: bool,
}

/// Everything `apply` needs to convert one archive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ConvertDetail {
	quality: u8,
	#[serde(default)]
	max_width: Option<u32>,
	#[serde(default)]
	max_height: Option<u32>,
	/// Pages in archive order, including the already-WebP ones.
	pages: Vec<PagePlan>,
	/// Pages that are already WebP and will be copied, not re-encoded.
	kept_webp: usize,
	before_bytes: u64,
	#[serde(default)]
	overwrite: bool,
	#[serde(default)]
	in_place: bool,
	/// Where the original is preserved when `in_place` is set.
	#[serde(default)]
	backup: Option<PathBuf>,
}

impl ConvertDetail {
	fn profile(&self) -> TransformProfile {
		TransformProfile {
			max_width: self.max_width,
			max_height: self.max_height,
			format: TransformFormat::Webp {
				quality: self.quality,
			},
			container: ComicContainer::Cbz,
			..Default::default()
		}
	}

	fn converted(&self) -> usize {
		self.pages.iter().filter(|page| page.convert).count()
	}
}

/// What an applied conversion achieved, reported per file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ConvertResult {
	pages: usize,
	converted: usize,
	kept_webp: usize,
	before_bytes: u64,
	after_bytes: u64,
	/// Positive when the archive shrank; negative archives happen and are
	/// reported rather than hidden.
	saved_bytes: i64,
}

impl Tool for WebpConvert {
	fn id(&self) -> &'static str {
		ID
	}

	fn describe(&self) -> &'static str {
		"Re-encode CBZ pages as WebP with Stump's own page pipeline"
	}

	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError> {
		let options = input.parse_options::<WebpConvertOptions>()?;
		if input.paths.is_empty() {
			return Err(ToolError::Invalid("no input paths given".to_string()));
		}
		if !(1..=100).contains(&options.quality) {
			return Err(ToolError::Options(format!(
				"quality must be between 1 and 100, got {}",
				options.quality
			)));
		}
		if let Some(dir) = &options.output_dir {
			if options.in_place {
				return Err(ToolError::Invalid(
					"in_place and output_dir are mutually exclusive".to_string(),
				));
			}
			if !dir.is_dir() {
				return Err(ToolError::Invalid(format!(
					"output_dir {} is not a directory",
					dir.display()
				)));
			}
		}

		let mut plan = Plan::new(ID);
		let sources = collect_archives(&input.paths, options.recursive, &mut plan)?;
		if sources.is_empty() {
			plan.warn(
				Warning::new("no-archives", "no .cbz files found in the given paths")
					.with_severity(Severity::Error),
			);
			return Ok(plan);
		}

		for source in sources {
			plan_archive(&source, &options, &mut plan)?;
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
			let (Some(source), Some(target)) = (&action.source, &action.target) else {
				report.skipped(action.clone(), "action has no source or target");
				continue;
			};
			let detail = serde_json::from_value::<ConvertDetail>(action.detail.clone())
				.map_err(|error| ToolError::Options(error.to_string()))?;

			sink.progress(index, total, &format!("converting {}", source.display()));

			if !source.is_file() {
				report.skipped(action.clone(), "source disappeared");
				continue;
			}
			if !detail.in_place && target.exists() && !detail.overwrite {
				report.skipped(
					action.clone(),
					format!("{} already exists", target.display()),
				);
				continue;
			}

			// The original is only ever replaced with a backup in hand.
			let backup = detail.backup.as_deref();
			if let Some(backup) = backup {
				if backup.exists() && !detail.overwrite {
					report.skipped(
						action.clone(),
						format!("backup {} already exists", backup.display()),
					);
					continue;
				}
				std::fs::copy(source, backup)?;
			}

			match convert_archive(source, target, &detail) {
				Ok(result) => {
					report.applied(
						Action::new(ACTION_CONVERT)
							.with_source(source)
							.with_target(target)
							.with_detail(serde_json::to_value(&result)?),
					);
				},
				Err(error) => {
					// `write_atomic` never leaves a partial file, but a target
					// from a previous overwrite attempt must not survive as a
					// half-converted archive.
					if !detail.in_place && target.exists() {
						let _ = std::fs::remove_file(target);
					}
					// The source was not touched, so the backup is redundant.
					if let Some(backup) = backup {
						let _ = std::fs::remove_file(backup);
					}
					report.skipped(action.clone(), error.to_string());
				},
			}
		}

		sink.progress(total, total, "done");

		Ok(report)
	}
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

fn plan_archive(
	source: &Path,
	options: &WebpConvertOptions,
	plan: &mut Plan,
) -> ToolResult<()> {
	let members = match read_members(source) {
		Ok(members) => members,
		Err(error) => {
			plan.warn(
				Warning::new("unreadable-archive", format!("cannot read: {error}"))
					.at(source)
					.with_severity(Severity::Error),
			);
			return Ok(());
		},
	};

	let pages = members
		.iter()
		.filter(|member| member.is_page)
		.map(|member| PagePlan {
			entry: member.name.clone(),
			target: webp_name(&member.name),
			convert: !member.is_webp,
		})
		.collect::<Vec<_>>();

	if pages.is_empty() {
		plan.warn(
			Warning::new("no-pages", "archive holds no page images")
				.at(source)
				.with_severity(Severity::Error),
		);
		return Ok(());
	}

	// Swapping the extension can collide when two pages differ only by it
	// (`001.jpg` and `001.png`). Renaming one of them would change the page
	// order, so the archive is left alone instead.
	let mut names = BTreeSet::new();
	let mut collision = None;
	for member in &members {
		let name = match pages.iter().find(|page| page.entry == member.name) {
			Some(page) => page.target.clone(),
			None => member.name.clone(),
		};
		if !names.insert(name.clone()) {
			collision = Some(name);
			break;
		}
	}
	if let Some(name) = collision {
		plan.warn(
			Warning::new(
				"page-name-collision",
				format!("two entries would both become {name}"),
			)
			.at(source)
			.with_severity(Severity::Error),
		);
		return Ok(());
	}

	let kept_webp = pages.iter().filter(|page| !page.convert).count();
	if kept_webp == pages.len() {
		plan.warn(
			Warning::new("already-webp", "every page is already WebP")
				.at(source)
				.with_severity(Severity::Info),
		);
		return Ok(());
	}

	let target = if options.in_place {
		source.to_path_buf()
	} else {
		util::derived_target(
			source,
			options.output_dir.as_deref(),
			&options.suffix,
			"cbz",
		)?
	};
	if !options.in_place {
		if target == source {
			plan.warn(
				Warning::new(
					"target-is-source",
					"conversion would overwrite the original; pass in_place to \
					 do that deliberately",
				)
				.at(source)
				.with_severity(Severity::Error),
			);
			return Ok(());
		}
		if target.exists() && !options.overwrite {
			plan.warn(
				Warning::new(
					"target-exists",
					format!("{} exists; pass overwrite to replace it", target.display()),
				)
				.at(source),
			);
			return Ok(());
		}
	}

	let backup = options.in_place.then(|| backup_path(source));
	if let Some(backup) = &backup {
		if backup.exists() && !options.overwrite {
			plan.warn(
				Warning::new(
					"backup-exists",
					format!("{} exists; pass overwrite to replace it", backup.display()),
				)
				.at(source),
			);
			return Ok(());
		}
	}

	let detail = ConvertDetail {
		quality: options.quality,
		max_width: options.max_width,
		max_height: options.max_height,
		kept_webp,
		pages,
		before_bytes: std::fs::metadata(source)?.len(),
		overwrite: options.overwrite,
		in_place: options.in_place,
		backup,
	};
	plan.push(
		Action::new(ACTION_CONVERT)
			.with_source(source)
			.with_target(&target)
			.with_detail(serde_json::to_value(&detail)?),
	);

	Ok(())
}

/// Expand the caller's paths into the CBZs to convert: a file is taken as-is,
/// a directory is walked (recursively only when asked).
fn collect_archives(
	paths: &[PathBuf],
	recursive: bool,
	plan: &mut Plan,
) -> ToolResult<Vec<PathBuf>> {
	let mut archives = Vec::new();

	for path in paths {
		if path.is_dir() {
			archives.extend(
				util::sorted_files(path, recursive)?
					.into_iter()
					.filter(|file| is_cbz(file)),
			);
			continue;
		}
		if !path.is_file() {
			plan.warn(
				Warning::new("missing-path", "path does not exist")
					.at(path)
					.with_severity(Severity::Error),
			);
			continue;
		}
		if !is_cbz(path) {
			plan.warn(
				Warning::new("not-an-archive", "not a .cbz file")
					.at(path)
					.with_severity(Severity::Error),
			);
			continue;
		}
		archives.push(path.clone());
	}

	archives.dedup();

	Ok(archives)
}

// ---------------------------------------------------------------------------
// Applying
// ---------------------------------------------------------------------------

/// Write the converted archive, re-encoding only the planned pages.
///
/// Non-page members and already-WebP pages are `raw_copy_file`d, so their
/// bytes and compression method are preserved exactly; the new pages are
/// stored (uncompressed), like every other CBZ Stump writes.
fn convert_archive(
	source: &Path,
	target: &Path,
	detail: &ConvertDetail,
) -> ToolResult<ConvertResult> {
	let plans: HashMap<&str, &PagePlan> = detail
		.pages
		.iter()
		.map(|page| (page.entry.as_str(), page))
		.collect();
	let profile = detail.profile();
	let permissions = std::fs::metadata(source)
		.map(|metadata| metadata.permissions())
		.ok();

	let mut archive = ZipArchive::new(File::open(source)?)?;
	let names = (0..archive.len())
		.map(|index| {
			archive
				.name_for_index(index)
				.unwrap_or_default()
				.to_string()
		})
		.collect::<Vec<_>>();
	let stored = SimpleFileOptions::default()
		.compression_method(CompressionMethod::Stored)
		.unix_permissions(0o644);

	util::write_atomic(target, |sink| {
		let mut writer = ZipWriter::new(sink);

		for (index, name) in names.iter().enumerate() {
			let page = plans.get(name.as_str()).copied();
			let convert = page.is_some_and(|page| page.convert);

			if !convert {
				let member = archive.by_index_raw(index)?;
				if member.is_dir() {
					let name = member.name().to_string();
					writer.add_directory(name, SimpleFileOptions::default())?;
				} else {
					writer.raw_copy_file(member)?;
				}
				continue;
			}

			let target_name = page
				.map(|page| page.target.clone())
				.unwrap_or_else(|| webp_name(name));
			let mut member = archive.by_index(index)?;
			let mut bytes = Vec::with_capacity(member.size() as usize);
			member.read_to_end(&mut bytes)?;
			drop(member);

			let encoded = transform_page_bytes(&bytes, &profile)?
				.into_iter()
				.next()
				.ok_or_else(|| {
					ToolError::Invalid(format!("{name}: page encoded to nothing"))
				})?;

			writer.start_file(target_name, stored)?;
			writer.write_all(&encoded.bytes)?;
		}

		writer.finish()?;

		Ok(())
	})?;

	if detail.in_place {
		if let Some(permissions) = permissions {
			let _ = std::fs::set_permissions(target, permissions);
		}
	}

	let after_bytes = std::fs::metadata(target)?.len();

	Ok(ConvertResult {
		pages: detail.pages.len(),
		converted: detail.converted(),
		kept_webp: detail.kept_webp,
		before_bytes: detail.before_bytes,
		after_bytes,
		saved_bytes: detail.before_bytes as i64 - after_bytes as i64,
	})
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// One member of a source archive, in archive order.
struct Member {
	name: String,
	is_page: bool,
	is_webp: bool,
}

fn read_members(path: &Path) -> ToolResult<Vec<Member>> {
	let mut archive = ZipArchive::new(File::open(path)?)?;
	let mut members = Vec::with_capacity(archive.len());

	for index in 0..archive.len() {
		let entry = archive.by_index(index)?;
		let name = entry.name().to_string();
		let is_page = !entry.is_dir() && util::is_page_file(Path::new(&name));
		let is_webp = is_page && extension_of(&name) == WEBP_EXTENSION;
		members.push(Member {
			name,
			is_page,
			is_webp,
		});
	}

	Ok(members)
}

/// The entry name with its extension swapped for `webp`, keeping the stem so
/// the archive's page order is unchanged.
fn webp_name(entry: &str) -> String {
	match entry.rsplit_once('.') {
		Some((stem, _)) if !stem.is_empty() => format!("{stem}.{WEBP_EXTENSION}"),
		_ => format!("{entry}.{WEBP_EXTENSION}"),
	}
}

fn extension_of(entry: &str) -> String {
	Path::new(entry)
		.extension()
		.map(|extension| extension.to_string_lossy().to_lowercase())
		.unwrap_or_default()
}

/// `Book.cbz` -> `Book.cbz.bak`: the whole original name is kept, so the
/// backup can never be mistaken for a library file.
fn backup_path(source: &Path) -> PathBuf {
	let mut name = source.as_os_str().to_owned();
	name.push(".bak");
	PathBuf::from(name)
}

fn is_cbz(path: &Path) -> bool {
	path.extension()
		.and_then(|extension| extension.to_str())
		.is_some_and(|extension| extension.eq_ignore_ascii_case("cbz"))
}

#[cfg(test)]
mod tests;
