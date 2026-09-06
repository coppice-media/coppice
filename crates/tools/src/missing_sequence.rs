//! `missing-sequence`: report chapter/volume gaps in a folder of books.
//!
//! Behaviour mirrors the Kavita-recommended `CBZ-Missing-Sequence-Checker`
//! documented at
//! <https://wiki.kavitareader.com/guides/external-tools/cbz-missing-sequence-checker/>:
//! recursive per-folder analysis, the `Chapter/Chap/Ch/C/Episode/Ep/Volume/
//! Vol/v` identifier vocabulary, omnibus ranges, decimal chapters that never
//! open a gap, `()`/`[]` metadata stripping, the changing-numeric-token
//! fallback, gap-only reporting by default, and the detailed (`-d`),
//! unidentified (`-u`) and all-folders (`-a`) report modes.
//!
//! The parsing itself lives in [`stump_scanner::sequence`], the one sequence
//! parser in the workspace: `stump_core`'s `missing_chapters_in_series`
//! quality check reports the same gaps for a persisted series.
//!
//! This tool is report-only. [`MissingSequence::plan`] produces the findings
//! and [`MissingSequence::apply`] writes nothing: it re-emits them as the
//! report, so `plan` and `apply` can never disagree.

use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use stump_media::PathUtils;
use stump_scanner::sequence::{analyze_sequence, SequenceAnalysis};

use crate::{
	util::sorted_files, Action, Plan, ProgressSink, Report, Severity, Tool, ToolError,
	ToolInput, Warning,
};

const ID: &str = "missing-sequence";

/// A folder whose numbering skips at least one chapter/volume.
const KIND_GAPS: &str = "report-gaps";
/// A folder whose numbering is gap-free (only with `all`).
const KIND_COMPLETE: &str = "report-complete";
/// A folder no number could be established for (only with `all` or
/// `unidentified`).
const KIND_UNSEQUENCED: &str = "report-unsequenced";
/// A folder with unidentified files but no gaps (only with `unidentified`).
const KIND_UNIDENTIFIED: &str = "report-unidentified";

/// Reports missing chapter/volume numbers per folder.
pub struct MissingSequence;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MissingSequenceOptions {
	/// Include the found ranges (`48-80`, `82-92`) in every report detail.
	pub detailed: bool,
	/// Include the names that carry no recognizable chapter/volume number,
	/// and report folders that only have such names.
	pub unidentified: bool,
	/// Report every folder, including gap-free ones.
	pub all: bool,
}

/// The per-folder finding carried by an action's `detail`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct FolderDetail {
	/// Book files considered in the folder.
	files: usize,
	/// Files a number was established for.
	sequenced: usize,
	/// `chapter`, `volume`, or absent in fallback mode.
	#[serde(skip_serializing_if = "Option::is_none")]
	kind: Option<String>,
	/// `identifier`, `fallback`, or `unidentified`.
	source: String,
	/// Only with `detailed`.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	found_ranges: Vec<String>,
	missing: Vec<i64>,
	/// `missing` as a count and as one compact line (`81`, `18, 22-24`): the
	/// CLI table can only render scalar detail fields, and a report tool whose
	/// finding is invisible without `--json` is useless.
	missing_count: usize,
	#[serde(skip_serializing_if = "String::is_empty")]
	gaps: String,
	/// Interstitial numbers seen (`112.1`); never counted as gaps.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	decimals: Vec<String>,
	/// Only with `unidentified`.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	unidentified: Vec<String>,
	/// The numbering spans more than the parser will enumerate, so `missing`
	/// is deliberately empty.
	#[serde(skip_serializing_if = "is_false")]
	span_exceeded: bool,
}

fn is_false(value: &bool) -> bool {
	!*value
}

impl Tool for MissingSequence {
	fn id(&self) -> &'static str {
		ID
	}

	fn describe(&self) -> &'static str {
		"Report missing chapter/volume numbers per folder, with omnibus range, decimal and unidentified-file handling"
	}

	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError> {
		let options = input.parse_options::<MissingSequenceOptions>()?;
		if input.paths.is_empty() {
			return Err(ToolError::Invalid("no input paths given".to_string()));
		}

		let mut plan = Plan::new(ID);
		let folders = collect_folders(&input.paths, &mut plan)?;
		if folders.is_empty() {
			plan.warn(
				Warning::new("no-books", "no supported book files in the given paths")
					.with_severity(Severity::Error),
			);
			return Ok(plan);
		}

		for (folder, files) in folders {
			let names = files
				.iter()
				.map(|file| file_name(file).to_string())
				.collect::<Vec<_>>();
			let analysis = analyze_sequence(&names);
			let detail = detail(&analysis, &options);

			let kind = if analysis.span_exceeded {
				plan.warn(
					Warning::new(
						"span-exceeded",
						format!(
							"numbering spans {} distinct runs too far apart to enumerate; gaps not reported",
							analysis.found_ranges.len()
						),
					)
					.at(&folder),
				);
				(options.all || options.unidentified).then_some(KIND_UNSEQUENCED)
			} else if analysis.identified_count() == 0 {
				(options.all || options.unidentified).then_some(KIND_UNSEQUENCED)
			} else if analysis.has_gaps() {
				Some(KIND_GAPS)
			} else if options.all {
				Some(KIND_COMPLETE)
			} else if options.unidentified && !analysis.unidentified.is_empty() {
				Some(KIND_UNIDENTIFIED)
			} else {
				None
			};

			if let Some(kind) = kind {
				plan.push(
					Action::new(kind)
						.with_source(&folder)
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
		// Report-only: the plan is the finding set, so nothing is written and
		// every finding is reported as applied.
		let mut report = Report::for_plan(plan);
		let total = plan.actions.len();
		for (index, action) in plan.actions.iter().enumerate() {
			let folder = action
				.source
				.as_deref()
				.unwrap_or(Path::new(""))
				.to_string_lossy();
			sink.progress(index + 1, total, &folder);
			report.applied(action.clone());
		}
		Ok(report)
	}
}

fn detail(analysis: &SequenceAnalysis, options: &MissingSequenceOptions) -> FolderDetail {
	FolderDetail {
		files: analysis.entries.len(),
		sequenced: analysis.identified_count(),
		kind: analysis.kind.map(|kind| kind.as_str().to_string()),
		source: analysis.source.as_str().to_string(),
		found_ranges: if options.detailed {
			analysis.found_ranges_display()
		} else {
			Vec::new()
		},
		missing: analysis.missing.clone(),
		missing_count: analysis.missing.len(),
		gaps: analysis.missing_summary(),
		decimals: if options.detailed {
			analysis
				.decimals
				.iter()
				.map(ToString::to_string)
				.collect::<Vec<_>>()
		} else {
			Vec::new()
		},
		unidentified: if options.unidentified {
			analysis.unidentified.clone()
		} else {
			Vec::new()
		},
		span_exceeded: analysis.span_exceeded,
	}
}

/// Group every book file under the given paths by its own folder. Directories
/// are walked recursively, as the reference tool does; a file argument is
/// grouped with the other arguments from the same folder, never with its
/// unlisted siblings.
fn collect_folders(
	paths: &[PathBuf],
	plan: &mut Plan,
) -> Result<BTreeMap<PathBuf, Vec<PathBuf>>, ToolError> {
	let mut folders: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
	for path in paths {
		if path.is_dir() {
			for file in sorted_files(path, true)? {
				push_book(&mut folders, file);
			}
		} else if path.is_file() {
			push_book(&mut folders, path.clone());
		} else {
			plan.warn(
				Warning::new("missing-path", "path does not exist")
					.at(path)
					.with_severity(Severity::Error),
			);
		}
	}
	Ok(folders)
}

fn push_book(folders: &mut BTreeMap<PathBuf, Vec<PathBuf>>, file: PathBuf) {
	// The scanner's own predicate: hidden files and formats Stump cannot read
	// are not books and must not influence a sequence.
	if file.as_path().is_default_ignored() {
		return;
	}
	let folder = file
		.parent()
		.map(Path::to_path_buf)
		.unwrap_or_else(|| PathBuf::from("."));
	folders.entry(folder).or_default().push(file);
}

fn file_name(path: &Path) -> &str {
	path.file_name()
		.and_then(|name| name.to_str())
		.unwrap_or_default()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::NoopProgress;
	use std::fs;
	use tempfile::TempDir;

	fn library(folders: &[(&str, &[&str])]) -> TempDir {
		let root = TempDir::new().expect("temp library");
		for (folder, files) in folders {
			let dir = root.path().join(folder);
			fs::create_dir_all(&dir).expect("create folder");
			for file in *files {
				fs::write(dir.join(file), b"").expect("create book file");
			}
		}
		root
	}

	fn plan_for(root: &Path, options: serde_json::Value) -> Plan {
		MissingSequence
			.plan(&ToolInput::new(vec![root.to_path_buf()]).with_options(options))
			.expect("plan succeeds")
	}

	fn detail_of(action: &Action) -> FolderDetail {
		serde_json::from_value(action.detail.clone()).expect("decode detail")
	}

	/// The wiki's detailed-mode example: 48-80 and 82-92 found, 81 missing.
	#[test]
	fn reports_the_reference_gap() {
		let names = (48..=92)
			.filter(|chapter| *chapter != 81)
			.map(|chapter| format!("XYZ Series Chapter {chapter:03}.cbz"))
			.collect::<Vec<_>>();
		let borrowed = names.iter().map(String::as_str).collect::<Vec<_>>();
		let root = library(&[("XYZ Series", &borrowed)]);

		let plan = plan_for(root.path(), serde_json::json!({"detailed": true}));
		assert_eq!(plan.actions.len(), 1);
		assert_eq!(plan.actions[0].kind, KIND_GAPS);
		let detail = detail_of(&plan.actions[0]);
		assert_eq!(detail.missing, vec![81]);
		assert_eq!(detail.missing_count, 1);
		assert_eq!(detail.gaps, "81");
		assert_eq!(detail.found_ranges, vec!["48-80", "82-92"]);
		assert_eq!(detail.files, 44);
		assert_eq!(detail.sequenced, 44);
		assert_eq!(detail.kind.as_deref(), Some("chapter"));
		assert_eq!(detail.source, "identifier");
	}

	/// `Series Chapter 01-03.cbz` is an omnibus: chapters 1, 2 and 3 are all
	/// present, so a folder holding it plus chapter 4 has no gap.
	#[test]
	fn omnibus_range_closes_the_gap_it_covers() {
		let root = library(&[(
			"Series",
			&["Series Chapter 01-03.cbz", "Series Chapter 04.cbz"],
		)]);
		let plan = plan_for(
			root.path(),
			serde_json::json!({"all": true, "detailed": true}),
		);
		assert_eq!(plan.actions.len(), 1);
		assert_eq!(plan.actions[0].kind, KIND_COMPLETE);
		let detail = detail_of(&plan.actions[0]);
		assert!(detail.missing.is_empty());
		assert_eq!(detail.found_ranges, vec!["1-4"]);

		// Without the omnibus file the same folder is missing 1, 2 and 3, and
		// a run of gaps renders as one range.
		let root = library(&[(
			"Series",
			&["Series Chapter 04.cbz", "Series Chapter 01.cbz"],
		)]);
		let plan = plan_for(root.path(), serde_json::json!({}));
		let detail = detail_of(&plan.actions[0]);
		assert_eq!(detail.missing, vec![2, 3]);
		assert_eq!(detail.gaps, "2-3");
	}

	/// `XYZ 017.cbz`: no identifier anywhere in the folder, so the changing
	/// numeric token establishes the order and the static year is ignored.
	#[test]
	fn fallback_sequences_names_without_identifiers() {
		let root = library(&[(
			"XYZ",
			&[
				"XYZ (2025) 017.cbz",
				"XYZ (2025) 018.cbz",
				"XYZ (2025) 020.cbz",
			],
		)]);
		let plan = plan_for(root.path(), serde_json::json!({"detailed": true}));
		assert_eq!(plan.actions.len(), 1);
		let detail = detail_of(&plan.actions[0]);
		assert_eq!(detail.source, "fallback");
		assert_eq!(detail.kind, None);
		assert_eq!(detail.missing, vec![19]);
		assert_eq!(detail.found_ranges, vec!["17-18", "20"]);
	}

	/// `Prologue.cbz` has no schema at all: it is unidentified, reported only
	/// with `unidentified`, and never treated as a gap.
	#[test]
	fn unidentified_files_are_opt_in() {
		let root = library(&[("XYZ", &["XYZ 017.cbz", "XYZ 018.cbz", "Prologue.cbz"])]);

		// Default: the folder is gap-free, so nothing is reported at all.
		assert!(plan_for(root.path(), serde_json::json!({}))
			.actions
			.is_empty());

		let plan = plan_for(root.path(), serde_json::json!({"unidentified": true}));
		assert_eq!(plan.actions.len(), 1);
		assert_eq!(plan.actions[0].kind, KIND_UNIDENTIFIED);
		let detail = detail_of(&plan.actions[0]);
		assert_eq!(detail.unidentified, vec!["Prologue.cbz".to_string()]);
		assert!(detail.missing.is_empty());
		assert_eq!(detail.files, 3);
		assert_eq!(detail.sequenced, 2);

		// A folder of nothing but unidentified names cannot be sequenced.
		let root = library(&[("Extras", &["Prologue.cbz", "Epilogue.cbz"])]);
		let plan = plan_for(root.path(), serde_json::json!({"unidentified": true}));
		assert_eq!(plan.actions[0].kind, KIND_UNSEQUENCED);
		assert_eq!(detail_of(&plan.actions[0]).sequenced, 0);
	}

	/// Decimal chapters are interstitial: they belong to their integer chapter
	/// and must not be reported as missing.
	#[test]
	fn decimal_chapters_never_open_a_gap() {
		let root = library(&[(
			"Series",
			&[
				"Series Ch 112.cbz",
				"Series Ch 112.1.cbz",
				"Series Ch 113.cbz",
			],
		)]);
		let plan = plan_for(
			root.path(),
			serde_json::json!({"detailed": true, "all": true}),
		);
		assert_eq!(plan.actions[0].kind, KIND_COMPLETE);
		let detail = detail_of(&plan.actions[0]);
		assert!(detail.missing.is_empty());
		assert_eq!(detail.decimals, vec!["112.1".to_string()]);
	}

	/// Only folders with gaps are reported by default; `all` adds the rest.
	#[test]
	fn default_reports_only_folders_with_gaps() {
		let root = library(&[
			("Broken", &["Broken Ch 1.cbz", "Broken Ch 3.cbz"]),
			("Whole", &["Whole Ch 1.cbz", "Whole Ch 2.cbz"]),
		]);

		let plan = plan_for(root.path(), serde_json::json!({}));
		assert_eq!(plan.actions.len(), 1);
		assert_eq!(
			plan.actions[0].source.as_deref(),
			Some(root.path().join("Broken").as_path())
		);
		assert_eq!(detail_of(&plan.actions[0]).missing, vec![2]);

		let plan = plan_for(root.path(), serde_json::json!({"all": true}));
		assert_eq!(plan.actions.len(), 2);
		assert_eq!(plan.actions[1].kind, KIND_COMPLETE);
	}

	/// Images, hidden files and formats Stump cannot read never take part in a
	/// sequence. A `.txt` sidecar is deliberately absent from this list: Stump
	/// serves text files as books (`ContentType::from_extension`), so one in a
	/// series folder is a book, not clutter.
	#[test]
	fn ignored_files_do_not_influence_the_sequence() {
		let root = library(&[(
			"Series",
			&[
				"Series Ch 1.cbz",
				"Series Ch 2.cbz",
				"cover.jpg",
				".DS_Store",
				"release 9.nfo",
			],
		)]);
		let plan = plan_for(root.path(), serde_json::json!({"all": true}));
		assert_eq!(plan.actions.len(), 1);
		let detail = detail_of(&plan.actions[0]);
		assert_eq!(detail.files, 2);
		assert!(detail.missing.is_empty());
	}

	#[test]
	fn apply_writes_nothing_and_re_emits_the_findings() {
		let root = library(&[("Series", &["Series Ch 1.cbz", "Series Ch 3.cbz"])]);
		let plan = plan_for(root.path(), serde_json::json!({}));
		let before = fs::read_dir(root.path().join("Series"))
			.expect("read folder")
			.count();

		let report = MissingSequence
			.apply(&plan, &mut NoopProgress)
			.expect("apply succeeds");
		assert_eq!(report.applied, plan.actions);
		assert!(report.skipped.is_empty());
		assert_eq!(
			fs::read_dir(root.path().join("Series"))
				.expect("read folder")
				.count(),
			before
		);
	}

	#[test]
	fn a_plan_from_another_tool_is_refused() {
		let foreign = Plan::new("epub2cbz");
		assert!(matches!(
			MissingSequence.apply(&foreign, &mut NoopProgress),
			Err(ToolError::PlanMismatch { .. })
		));
	}

	#[test]
	fn empty_input_and_unknown_options_are_errors() {
		assert!(matches!(
			MissingSequence.plan(&ToolInput::default()),
			Err(ToolError::Invalid(_))
		));
		let root = library(&[("Series", &["Series Ch 1.cbz"])]);
		assert!(matches!(
			MissingSequence.plan(
				&ToolInput::new(vec![root.path().to_path_buf()])
					.with_options(serde_json::json!({"nope": true}))
			),
			Err(ToolError::Options(_))
		));
	}

	#[test]
	fn a_missing_path_is_warned_about() {
		let root = library(&[("Series", &["Series Ch 1.cbz", "Series Ch 3.cbz"])]);
		let plan = MissingSequence
			.plan(&ToolInput::new(vec![
				root.path().to_path_buf(),
				root.path().join("nope"),
			]))
			.expect("plan succeeds");
		assert_eq!(plan.warnings.len(), 1);
		assert_eq!(plan.warnings[0].code, "missing-path");
		assert_eq!(plan.actions.len(), 1);
	}
}
