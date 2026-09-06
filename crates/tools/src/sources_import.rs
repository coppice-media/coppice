//! `sources-import`: derive data-only source definitions from a checkout of
//! [keiyoushi/extensions-source](https://github.com/keiyoushi/extensions-source).
//!
//! Stump does not run Mihon/Tachiyomi extension code: there is no Android
//! runtime, no Kotlin, no APK. This tool reads the extension repository as
//! *text* and emits one JSON definition per source — base URL, language,
//! theme, and the per-site knobs that theme needs — which the native theme
//! engines (`crates/provider-themes`) then execute. Schema v1, the knob
//! vocabulary, the output layout and the unsupported-reason codes are frozen in
//! `docs/content/docs/developer/source-definitions.mdx`; this module is the
//! only writer of that format.
//!
//! Upstream layout the recogniser is written against (`CONTRIBUTING.md`
//! "build.gradle.kts" / "Source declaration"): every extension is
//! `src/<lang>/<dir>/` with a `build.gradle.kts` holding a `keiyoushi { name,
//! versionCode, contentWarning, libVersion, theme, source { lang, baseUrl } }`
//! DSL, plus one `@Source`-annotated Kotlin class that extends a
//! `lib-multisrc/<theme>/` base class. `name`, `lang`, `id` and `baseUrl` are
//! injected by KSP from the DSL, so the Kotlin body contains only overrides:
//! the ones that are literal data become knobs, and anything with behaviour in
//! it makes the source `unsupported` with a reason instead of a half-derived
//! definition. The pre-KSP form (metadata as base-class constructor arguments)
//! is still recognised so an older pin can be imported.
//!
//! keiyoushi/extensions-source is Apache-2.0. Only *data* is derived here and
//! no upstream code is copied into this crate; each definition names the commit
//! it came from, and [`NOTICE_TEMPLATE`] puts the same attribution in the
//! generated output directory.

use std::{
	collections::{BTreeMap, BTreeSet},
	io::Write,
	path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
	util, Action, Plan, ProgressSink, Report, Severity, Tool, ToolError, ToolInput,
	ToolResult, Warning,
};

const ID: &str = "sources-import";

/// The frozen schema version every emitted definition carries.
pub const SCHEMA: u32 = 1;

/// Fallback when the checkout has no `origin` remote.
const DEFAULT_REPO: &str = "keiyoushi/extensions-source";

const KIND_DEFINITION: &str = "write-definition";
const KIND_INDEX: &str = "write-index";
const KIND_UNSUPPORTED: &str = "write-unsupported";
const KIND_NOTICE: &str = "write-notice";
const KIND_STATS: &str = "report-stats";

const INDEX_FILE: &str = "index.json";
const UNSUPPORTED_FILE: &str = "unsupported.json";
const NOTICE_FILE: &str = "NOTICE";

/// Marker standing in for the source's own `baseUrl` inside a knob string, so
/// one parse of a shared Kotlin class can serve several `source {}` blocks.
const BASE_URL_MARKER: char = '\u{0}';

/// Attribution written to `<output_dir>/NOTICE`; `{repo}`/`{commit}` are
/// substituted with the checkout's provenance.
pub const NOTICE_TEMPLATE: &str = "\
Stump source definitions
========================

Data-only source definitions derived from

    {repo}
    commit {commit}

which is licensed under the Apache License, Version 2.0. These definitions are
a derived work: names, languages, base URLs, themes and per-site settings were
extracted mechanically from that repository's Gradle DSL and Kotlin sources by
`stump tools apply sources-import` (crates/tools/src/sources_import.rs). No
upstream source code is included, compiled, or executed.

A copy of the Apache License is available at

    http://www.apache.org/licenses/LICENSE-2.0

Layout: index.json (every derived definition), unsupported.json (every source
that was not derived, with the reason), <lang>/<id>.json (the definitions).
The schema is documented in Stump's `docs/content/docs/developer/\
source-definitions.mdx`.
";

/// Derives data-only source definitions from an extensions-source checkout.
pub struct SourcesImport;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SourcesImportOptions {
	/// Checkout root; must contain `src/` and `lib-multisrc/`. Defaults to the
	/// first input path.
	pub extensions_source: Option<PathBuf>,
	/// Existing directory the definition repo is written into.
	pub output_dir: Option<PathBuf>,
	/// Only derive these themes; everything else is counted as filtered.
	pub themes: Option<Vec<String>>,
	/// Only derive these source languages.
	pub langs: Option<Vec<String>>,
}

/// One source, as data. Schema v1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Definition {
	pub schema: u32,
	pub id: String,
	pub name: String,
	pub lang: String,
	pub base_url: String,
	pub theme: String,
	pub version: i64,
	pub nsfw: bool,
	pub knobs: BTreeMap<String, Knob>,
	pub upstream: Upstream,
}

/// A knob value: string, bool or int, never null/array/object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Knob {
	Bool(bool),
	Int(i64),
	Str(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Upstream {
	pub repo: String,
	pub commit: String,
	pub path: String,
}

/// One `index.json` row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexRow {
	pub id: String,
	pub name: String,
	pub lang: String,
	pub theme: String,
	pub nsfw: bool,
	pub version: i64,
	pub file: String,
}

/// One `unsupported.json` row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnsupportedRow {
	pub id: String,
	pub name: String,
	pub lang: String,
	pub theme: Option<String>,
	pub path: String,
	pub code: String,
	pub reason: String,
}

/// Per-theme derived/unsupported counts, the tool's own report.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ThemeStat {
	pub theme: String,
	pub derived: usize,
	pub unsupported: usize,
}

impl Tool for SourcesImport {
	fn id(&self) -> &'static str {
		ID
	}

	fn describe(&self) -> &'static str {
		"Derive data-only source definitions from a keiyoushi/extensions-source checkout"
	}

	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError> {
		let options = input.parse_options::<SourcesImportOptions>()?;
		let root = options
			.extensions_source
			.clone()
			.or_else(|| input.paths.first().cloned())
			.ok_or_else(|| {
				ToolError::Invalid(
					"no checkout given: pass a path or extensions_source".to_string(),
				)
			})?;
		if !root.is_dir() {
			return Err(ToolError::Invalid(format!(
				"extensions_source {} is not a directory",
				root.display()
			)));
		}
		let sources_root = root.join("src");
		if !sources_root.is_dir() {
			return Err(ToolError::Invalid(format!(
				"{} is not an extensions-source checkout: no src/ directory",
				root.display()
			)));
		}
		let output_dir = options
			.output_dir
			.clone()
			.ok_or_else(|| ToolError::Invalid("output_dir is required".to_string()))?;
		if !output_dir.is_dir() {
			return Err(ToolError::Invalid(format!(
				"output_dir {} is not a directory",
				output_dir.display()
			)));
		}

		let mut plan = Plan::new(ID);
		let themes = Theme::modules(&root.join("lib-multisrc"))?;
		if themes.is_empty() {
			plan.warn(
				Warning::new(
					"no-themes",
					"no lib-multisrc/ modules found: every source without an \
					 explicit theme will be unsupported",
				)
				.at(&root),
			);
		}

		let repo = resolve_repo(&root).unwrap_or_else(|| DEFAULT_REPO.to_string());
		let commit = match resolve_commit(&root) {
			Some(commit) => commit,
			None => {
				plan.warn(
					Warning::new(
						"unresolved-upstream-commit",
						"could not resolve the checked-out commit: definitions \
						 will record it as \"unknown\"",
					)
					.at(&root)
					.with_severity(Severity::Warn),
				);
				"unknown".to_string()
			},
		};

		let theme_filter = options.themes.as_ref().map(|themes| {
			themes
				.iter()
				.map(|theme| theme.to_ascii_lowercase())
				.collect::<BTreeSet<_>>()
		});
		let lang_filter = options
			.langs
			.as_ref()
			.map(|langs| langs.iter().cloned().collect::<BTreeSet<_>>());

		let mut definitions = Vec::new();
		let mut unsupported = Vec::new();
		let mut filtered = 0usize;
		let mut extensions = 0usize;

		for extension_dir in extension_dirs(&sources_root)? {
			extensions += 1;
			let outcome = derive(&extension_dir, &root, &themes, &repo, &commit);
			for source in outcome {
				match source {
					Derived::Definition(definition) => {
						let keep = theme_filter
							.as_ref()
							.is_none_or(|filter| filter.contains(&definition.theme))
							&& lang_filter
								.as_ref()
								.is_none_or(|filter| filter.contains(&definition.lang));
						if keep {
							definitions.push(definition);
						} else {
							filtered += 1;
						}
					},
					Derived::Unsupported(row) => {
						let excluded = theme_filter.as_ref().is_some_and(|filter| {
							row.theme
								.as_ref()
								.is_none_or(|theme| !filter.contains(theme))
						}) || lang_filter
							.as_ref()
							.is_some_and(|filter| !filter.contains(&row.lang));
						if excluded {
							filtered += 1;
						} else {
							unsupported.push(row);
						}
					},
				}
			}
		}

		definitions.sort_by(|left, right| left.id.cmp(&right.id));
		unsupported.sort_by(|left, right| left.id.cmp(&right.id));

		let mut duplicates = BTreeSet::new();
		let mut seen = BTreeSet::new();
		for definition in &definitions {
			if !seen.insert(definition.id.clone()) {
				duplicates.insert(definition.id.clone());
			}
		}
		for id in &duplicates {
			plan.warn(Warning::new(
				"duplicate-id",
				format!("id {id:?} was derived more than once; only one file is written"),
			));
		}

		let mut stats: BTreeMap<String, ThemeStat> = BTreeMap::new();
		for definition in &definitions {
			stats
				.entry(definition.theme.clone())
				.or_insert_with(|| ThemeStat {
					theme: definition.theme.clone(),
					..ThemeStat::default()
				})
				.derived += 1;
		}
		for row in &unsupported {
			let theme = row.theme.clone().unwrap_or_else(|| "(none)".to_string());
			stats
				.entry(theme.clone())
				.or_insert_with(|| ThemeStat {
					theme,
					..ThemeStat::default()
				})
				.unsupported += 1;
		}

		let index = definitions
			.iter()
			.map(|definition| IndexRow {
				id: definition.id.clone(),
				name: definition.name.clone(),
				lang: definition.lang.clone(),
				theme: definition.theme.clone(),
				nsfw: definition.nsfw,
				version: definition.version,
				file: definition_file(definition),
			})
			.collect::<Vec<_>>();

		for definition in &definitions {
			let target = output_dir.join(definition_file(definition));
			plan.push(
				Action::new(KIND_DEFINITION)
					.with_source(root.join(&definition.upstream.path))
					.with_target(target)
					.with_detail(serde_json::to_value(definition)?),
			);
		}

		plan.push(
			Action::new(KIND_INDEX)
				.with_target(output_dir.join(INDEX_FILE))
				.with_detail(serde_json::json!({
					"count": index.len(),
					"rows": index,
				})),
		);
		plan.push(
			Action::new(KIND_UNSUPPORTED)
				.with_target(output_dir.join(UNSUPPORTED_FILE))
				.with_detail(serde_json::json!({
					"count": unsupported.len(),
					"rows": unsupported,
				})),
		);
		plan.push(
			Action::new(KIND_NOTICE)
				.with_target(output_dir.join(NOTICE_FILE))
				.with_detail(serde_json::json!({
					"text": NOTICE_TEMPLATE
						.replace("{repo}", &repo)
						.replace("{commit}", &commit),
				})),
		);

		let rows = stats.into_values().collect::<Vec<_>>();
		plan.push(Action::new(KIND_STATS).with_source(&root).with_detail(
			serde_json::json!({
				"table": stats_table(&rows, definitions.len(), unsupported.len()),
				"themes": rows,
				"extensions": extensions,
				"derived": definitions.len(),
				"unsupported": unsupported.len(),
				"filtered": filtered,
				"commit": commit,
				"repo": repo,
			}),
		));

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

		let mut written = BTreeSet::new();
		for (done, action) in plan.actions.iter().enumerate() {
			sink.progress(done, total, &action.kind);
			let target = action.target.clone();
			let body = match action.kind.as_str() {
				KIND_DEFINITION => Some(pretty(&action.detail)?),
				KIND_INDEX | KIND_UNSUPPORTED => {
					let rows = action.detail.get("rows").ok_or_else(|| {
						ToolError::Invalid(format!("{} action has no rows", action.kind))
					})?;
					Some(pretty(rows)?)
				},
				KIND_NOTICE => Some(
					action
						.detail
						.get("text")
						.and_then(serde_json::Value::as_str)
						.ok_or_else(|| {
							ToolError::Invalid("notice action has no text".to_string())
						})?
						.as_bytes()
						.to_vec(),
				),
				KIND_STATS => None,
				other => {
					return Err(ToolError::Invalid(format!(
						"unknown action kind: {other}"
					)))
				},
			};

			let Some(body) = body else {
				report.applied(action.clone());
				continue;
			};
			let Some(target) = target else {
				return Err(ToolError::Invalid(format!(
					"{} action has no target",
					action.kind
				)));
			};
			if !written.insert(target.clone()) {
				report.skipped(action.clone(), "another action already wrote this file");
				continue;
			}
			util::write_atomic(&target, |file| {
				file.write_all(&body)?;
				Ok(())
			})?;
			report.applied(action.clone());
		}

		sink.progress(total, total, "done");
		Ok(report)
	}
}

fn pretty(value: &serde_json::Value) -> ToolResult<Vec<u8>> {
	let mut body = serde_json::to_vec_pretty(value)?;
	body.push(b'\n');
	Ok(body)
}

fn definition_file(definition: &Definition) -> String {
	format!("{}/{}.json", definition.lang, definition.id)
}

fn stats_table(rows: &[ThemeStat], derived: usize, unsupported: usize) -> String {
	let width = rows
		.iter()
		.map(|row| row.theme.chars().count())
		.chain(std::iter::once("theme".len()))
		.chain(std::iter::once("TOTAL".len()))
		.max()
		.unwrap_or(5);
	let mut table = format!(
		"{:<width$}  {:>7}  {:>11}\n",
		"theme", "derived", "unsupported"
	);
	let mut ordered = rows.to_vec();
	ordered.sort_by(|left, right| {
		right
			.derived
			.cmp(&left.derived)
			.then_with(|| right.unsupported.cmp(&left.unsupported))
			.then_with(|| left.theme.cmp(&right.theme))
	});
	for row in &ordered {
		table.push_str(&format!(
			"{:<width$}  {:>7}  {:>11}\n",
			row.theme, row.derived, row.unsupported
		));
	}
	table.push_str(&format!(
		"{:<width$}  {:>7}  {:>11}\n",
		"TOTAL", derived, unsupported
	));
	table
}

/// Every `src/<lang>/<dir>` holding a `build.gradle.kts`, in path order.
fn extension_dirs(sources_root: &Path) -> ToolResult<Vec<PathBuf>> {
	let mut langs = std::fs::read_dir(sources_root)?
		.filter_map(Result::ok)
		.map(|entry| entry.path())
		.filter(|path| path.is_dir())
		.collect::<Vec<_>>();
	langs.sort_unstable();

	let mut extensions = Vec::new();
	for lang in langs {
		let mut dirs = std::fs::read_dir(&lang)?
			.filter_map(Result::ok)
			.map(|entry| entry.path())
			.filter(|path| path.is_dir() && path.join("build.gradle.kts").is_file())
			.collect::<Vec<_>>();
		dirs.sort_unstable();
		extensions.extend(dirs);
	}
	Ok(extensions)
}

/// What one extension directory yielded: a definition per `source {}` block, or
/// a reason it could not be derived.
enum Derived {
	Definition(Definition),
	Unsupported(UnsupportedRow),
}

/// A `lib-multisrc/<module>` theme: the module name plus the base classes it
/// declares.
#[derive(Debug, Clone, Default)]
struct Theme {
	classes: BTreeSet<String>,
	has_no_ajax_variant: bool,
}

impl Theme {
	/// Scan `lib-multisrc/` for the theme modules and their base classes.
	fn modules(root: &Path) -> ToolResult<BTreeMap<String, Theme>> {
		let mut modules = BTreeMap::new();
		if !root.is_dir() {
			return Ok(modules);
		}
		let mut dirs = std::fs::read_dir(root)?
			.filter_map(Result::ok)
			.map(|entry| entry.path())
			.filter(|path| path.is_dir())
			.collect::<Vec<_>>();
		dirs.sort_unstable();

		for dir in dirs {
			let Some(name) = dir.file_name().and_then(|name| name.to_str()) else {
				continue;
			};
			let mut theme = Theme::default();
			for file in kotlin_files(&dir)? {
				let Ok(text) = std::fs::read_to_string(&file) else {
					continue;
				};
				for class in declared_classes(&strip_comments(&text)) {
					theme.classes.insert(class);
				}
			}
			theme.has_no_ajax_variant = theme
				.classes
				.iter()
				.any(|class| class.to_ascii_lowercase().ends_with("noajax"));
			modules.insert(name.to_ascii_lowercase(), theme);
		}
		Ok(modules)
	}
}

/// Every `.kt` file under `root`, in path order.
fn kotlin_files(root: &Path) -> ToolResult<Vec<PathBuf>> {
	let mut files = Vec::new();
	collect_kotlin(root, &mut files)?;
	files.sort_unstable();
	Ok(files)
}

fn collect_kotlin(dir: &Path, files: &mut Vec<PathBuf>) -> ToolResult<()> {
	for entry in std::fs::read_dir(dir)?.filter_map(Result::ok) {
		let path = entry.path();
		if path.is_dir() {
			collect_kotlin(&path, files)?;
		} else if path.extension().is_some_and(|ext| ext == "kt") {
			files.push(path);
		}
	}
	Ok(())
}

/// The simple names of the top-level `class`/`abstract class`/`open class`
/// declarations in `text`.
fn declared_classes(text: &str) -> Vec<String> {
	let mut classes = Vec::new();
	for line in text.lines() {
		let trimmed = line.trim_start();
		if trimmed.len() != line.len() {
			// Indented: a nested declaration, not a theme base class.
			continue;
		}
		let mut rest = trimmed;
		for modifier in ["public ", "abstract ", "open ", "sealed ", "internal "] {
			if let Some(stripped) = rest.strip_prefix(modifier) {
				rest = stripped.trim_start();
			}
		}
		let Some(rest) = rest.strip_prefix("class ") else {
			continue;
		};
		let name = rest
			.trim_start()
			.split(|c: char| !c.is_alphanumeric() && c != '_')
			.next()
			.unwrap_or_default();
		if !name.is_empty() {
			classes.push(name.to_string());
		}
	}
	classes
}

// ---------------------------------------------------------------------------
// Derivation
// ---------------------------------------------------------------------------

/// Why a source could not be derived: a stable code plus operator-facing prose.
struct Reject {
	code: &'static str,
	reason: String,
}

impl Reject {
	fn new(code: &'static str, reason: impl Into<String>) -> Self {
		Self {
			code,
			reason: reason.into(),
		}
	}
}

/// Derive every source of one extension directory.
fn derive(
	dir: &Path,
	root: &Path,
	themes: &BTreeMap<String, Theme>,
	repo: &str,
	commit: &str,
) -> Vec<Derived> {
	let rel = relative_path(root, dir);
	let dir_name = dir
		.file_name()
		.and_then(|name| name.to_str())
		.unwrap_or_default()
		.to_string();
	let lang_dir = dir
		.parent()
		.and_then(Path::file_name)
		.and_then(|name| name.to_str())
		.unwrap_or_default()
		.to_string();

	let gradle_text = match std::fs::read_to_string(dir.join("build.gradle.kts")) {
		Ok(text) => text,
		Err(error) => {
			return vec![reject_extension(
				&format!("{lang_dir}.{dir_name}"),
				&dir_name,
				&lang_dir,
				None,
				&rel,
				&Reject::new(
					"unparsed-gradle",
					format!("unreadable build.gradle.kts: {error}"),
				),
			)]
		},
	};

	let gradle = match parse_gradle(&gradle_text) {
		Ok(gradle) => gradle,
		Err(reject) => {
			return vec![reject_extension(
				&format!("{lang_dir}.{dir_name}"),
				&dir_name,
				&lang_dir,
				None,
				&rel,
				&reject,
			)]
		},
	};

	let pkg = gradle
		.pkg_name
		.clone()
		.unwrap_or_else(|| format!("{lang_dir}.{dir_name}"));
	let blocks = &gradle.sources;
	let ids = source_ids(&pkg, blocks, &gradle.name);

	// The class declaration decides the theme, so it is parsed before the body:
	// a hand-written source must be reported as `no-theme`, not as whichever
	// override happens to carry logic.
	let mut class = match source_class(dir) {
		Ok(class) => class,
		Err(reject) => {
			return blocks
				.iter()
				.zip(&ids)
				.map(|(block, id)| {
					reject_source(id, block, &gradle, &lang_dir, None, &rel, &reject)
				})
				.collect()
		},
	};

	let theme = match resolve_theme(&gradle.theme, &class, themes) {
		Ok(theme) => theme,
		Err(reject) => {
			return blocks
				.iter()
				.zip(&ids)
				.map(|(block, id)| {
					reject_source(
						id,
						block,
						&gradle,
						&lang_dir,
						gradle.theme.clone(),
						&rel,
						&reject,
					)
				})
				.collect()
		},
	};

	let knobs = match parse_knobs(&mut class)
		.and_then(|()| class_knobs(&class, themes.get(&theme)))
	{
		Ok(knobs) => knobs,
		Err(reject) => {
			return blocks
				.iter()
				.zip(&ids)
				.map(|(block, id)| {
					reject_source(
						id,
						block,
						&gradle,
						&lang_dir,
						Some(theme.clone()),
						&rel,
						&reject,
					)
				})
				.collect()
		},
	};

	blocks
		.iter()
		.zip(&ids)
		.map(|(block, id)| {
			let name = block
				.name
				.clone()
				.or_else(|| class.ctor_name.clone())
				.unwrap_or_else(|| gradle.name.clone());
			let lang = block
				.lang
				.clone()
				.or_else(|| class.ctor_lang.clone())
				.unwrap_or_else(|| lang_dir.clone());
			let base_url = block
				.base_url
				.clone()
				.or_else(|| class.ctor_base_url.clone());
			let Some(base_url) = base_url else {
				return reject_source(
					id,
					block,
					&gradle,
					&lang_dir,
					Some(theme.clone()),
					&rel,
					&Reject::new(
						"no-base-url",
						"the source block declares no static base URL",
					),
				);
			};
			let base_url = base_url.trim_end_matches('/').to_string();

			Derived::Definition(Definition {
				schema: SCHEMA,
				id: id.clone(),
				name,
				lang,
				base_url: base_url.clone(),
				theme: theme.clone(),
				version: gradle.version,
				nsfw: gradle.nsfw,
				knobs: knobs
					.iter()
					.map(|(key, knob)| (key.clone(), knob.resolve(&base_url)))
					.collect(),
				upstream: Upstream {
					repo: repo.to_string(),
					commit: commit.to_string(),
					path: rel.clone(),
				},
			})
		})
		.collect()
}

fn reject_extension(
	id: &str,
	name: &str,
	lang: &str,
	theme: Option<String>,
	path: &str,
	reject: &Reject,
) -> Derived {
	Derived::Unsupported(UnsupportedRow {
		id: id.to_string(),
		name: name.to_string(),
		lang: lang.to_string(),
		theme,
		path: path.to_string(),
		code: reject.code.to_string(),
		reason: reject.reason.clone(),
	})
}

fn reject_source(
	id: &str,
	block: &SourceBlock,
	gradle: &Gradle,
	lang_dir: &str,
	theme: Option<String>,
	path: &str,
	reject: &Reject,
) -> Derived {
	reject_extension(
		id,
		block.name.as_deref().unwrap_or(&gradle.name),
		block.lang.as_deref().unwrap_or(lang_dir),
		theme,
		path,
		reject,
	)
}

/// Definition ids for one extension: the package suffix for a single source,
/// and a stable per-block disambiguator when the extension yields several.
/// Blocks are commonly distinguished by name, but a multi-language extension
/// repeats one name and differs only in `lang`.
fn source_ids(pkg: &str, blocks: &[SourceBlock], fallback: &str) -> Vec<String> {
	if blocks.len() <= 1 {
		return vec![pkg.to_string()];
	}
	let unique = |candidates: &[String]| {
		let mut seen = candidates.to_vec();
		seen.sort_unstable();
		seen.dedup();
		seen.len() == candidates.len()
			&& candidates.iter().all(|candidate| !candidate.is_empty())
	};
	let names = blocks
		.iter()
		.map(|block| slug(block.name.as_deref().unwrap_or(fallback)))
		.collect::<Vec<_>>();
	let langs = blocks
		.iter()
		.map(|block| slug(block.lang.as_deref().unwrap_or_default()))
		.collect::<Vec<_>>();
	let discriminators = if unique(&names) {
		names
	} else if unique(&langs) {
		langs
	} else {
		(1..=blocks.len())
			.map(|index| index.to_string())
			.collect::<Vec<_>>()
	};
	discriminators
		.into_iter()
		.map(|discriminator| format!("{pkg}.{discriminator}"))
		.collect()
}

fn slug(name: &str) -> String {
	name.chars()
		.filter(|c| c.is_ascii_alphanumeric())
		.map(|c| c.to_ascii_lowercase())
		.collect()
}

fn resolve_theme(
	declared: &Option<String>,
	class: &SourceClass,
	themes: &BTreeMap<String, Theme>,
) -> Result<String, Reject> {
	if let Some(declared) = declared {
		// The Gradle `theme` names the `lib-multisrc/` module that is actually
		// compiled in, and it wins: `lib-multisrc/madaralegacy` declares its
		// classes in package `…multisrc.madara`, so the import path cannot tell
		// the two Madara generations apart.
		let declared = declared.to_ascii_lowercase();
		let module = themes.get(&declared);
		let declares_class =
			module.is_some_and(|theme| theme.classes.contains(&class.base_class));
		let unknown_module = module.is_none_or(|theme| theme.classes.is_empty());
		if declares_class || unknown_module {
			return Ok(declared);
		}
		return Err(Reject::new(
			"unknown-base-class",
			format!(
				"declares theme {declared:?} but extends {}, which that theme does not declare",
				class.base_class
			),
		));
	}

	let imported = class.theme_module.clone();

	if let Some(imported) = imported {
		return Ok(imported);
	}
	let derived = class.base_class.to_ascii_lowercase();
	if themes
		.get(&derived)
		.is_some_and(|theme| theme.classes.contains(&class.base_class))
	{
		return Ok(derived);
	}
	Err(Reject::new(
		"no-theme",
		format!(
			"no theme in the Gradle DSL and {} is not a lib-multisrc base class",
			class.base_class
		),
	))
}

/// The knobs of one extension: the class's own overrides plus the universal
/// `base_class`/`browse_mode` pair.
fn class_knobs(
	class: &SourceClass,
	theme: Option<&Theme>,
) -> Result<BTreeMap<String, RawKnob>, Reject> {
	let mut knobs = class.knobs.clone();
	knobs.insert(
		"base_class".to_string(),
		RawKnob::Str(class.base_class.clone()),
	);
	if theme.is_some_and(|theme| theme.has_no_ajax_variant) {
		let mode = if class.base_class.to_ascii_lowercase().ends_with("noajax") {
			"no_ajax"
		} else {
			"ajax"
		};
		knobs.insert("browse_mode".to_string(), RawKnob::Str(mode.to_string()));
	}
	Ok(knobs)
}

fn relative_path(root: &Path, path: &Path) -> String {
	path.strip_prefix(root)
		.unwrap_or(path)
		.components()
		.map(|component| component.as_os_str().to_string_lossy().into_owned())
		.collect::<Vec<_>>()
		.join("/")
}

// ---------------------------------------------------------------------------
// Provenance
// ---------------------------------------------------------------------------

/// `owner/repo` of the checkout's `origin` remote.
fn resolve_repo(root: &Path) -> Option<String> {
	let config = std::fs::read_to_string(git_dir(root)?.join("config")).ok()?;
	let mut in_origin = false;
	for line in config.lines() {
		let line = line.trim();
		if line.starts_with('[') {
			in_origin = line.replace(char::is_whitespace, "") == "[remote\"origin\"]";
			continue;
		}
		if !in_origin {
			continue;
		}
		let Some((key, value)) = line.split_once('=') else {
			continue;
		};
		if key.trim() != "url" {
			continue;
		}
		let url = value.trim().trim_end_matches(".git");
		let path = url
			.rsplit_once(':')
			.filter(|(host, _)| !host.contains('/'))
			.map(|(_, path)| path)
			.or_else(|| url.split_once("//").map(|(_, rest)| rest))
			.unwrap_or(url);
		let segments = path.trim_matches('/').split('/').collect::<Vec<_>>();
		if segments.len() >= 2 {
			return Some(segments[segments.len() - 2..].join("/"));
		}
	}
	None
}

/// The commit the checkout is on, resolved without invoking git.
fn resolve_commit(root: &Path) -> Option<String> {
	let git = git_dir(root)?;
	let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
	let head = head.trim();
	let Some(reference) = head.strip_prefix("ref:") else {
		return object_id(head);
	};
	let reference = reference.trim();
	if let Ok(direct) = std::fs::read_to_string(git.join(reference)) {
		if let Some(id) = object_id(direct.trim()) {
			return Some(id);
		}
	}
	let packed = std::fs::read_to_string(git.join("packed-refs")).ok()?;
	packed.lines().find_map(|line| {
		let (id, name) = line.split_once(' ')?;
		(name.trim() == reference).then_some(())?;
		object_id(id)
	})
}

fn git_dir(root: &Path) -> Option<PathBuf> {
	let git = root.join(".git");
	if git.is_dir() {
		return Some(git);
	}
	// A worktree or submodule checkout: `.git` is a file pointing at the real
	// directory.
	let pointer = std::fs::read_to_string(&git).ok()?;
	let path = pointer.trim().strip_prefix("gitdir:")?.trim();
	let path = Path::new(path);
	Some(if path.is_absolute() {
		path.to_path_buf()
	} else {
		root.join(path)
	})
}

fn object_id(text: &str) -> Option<String> {
	let text = text.trim();
	let hex = text.len() == 40 || text.len() == 64;
	(hex && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
		.then(|| text.to_ascii_lowercase())
}

// ---------------------------------------------------------------------------
// Gradle DSL
// ---------------------------------------------------------------------------

/// The `keiyoushi { ... }` block of one extension.
#[derive(Debug, Clone, Default, PartialEq)]
struct Gradle {
	name: String,
	version: i64,
	nsfw: bool,
	theme: Option<String>,
	pkg_name: Option<String>,
	sources: Vec<SourceBlock>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct SourceBlock {
	name: Option<String>,
	lang: Option<String>,
	base_url: Option<String>,
}

/// Keys the `keiyoushi {}` block may carry; anything else means the DSL grew a
/// knob this tool would silently drop.
const GRADLE_KEYS: &[&str] = &[
	"name",
	"versionCode",
	"contentWarning",
	"libVersion",
	"theme",
	"pkgName",
	"baseVersionCode",
];
const GRADLE_BLOCKS: &[&str] = &["source", "deeplink"];
const SOURCE_KEYS: &[&str] = &["name", "lang", "baseUrl", "id", "versionId"];
const SOURCE_BLOCKS: &[&str] = &["baseUrl", "deeplink"];

fn parse_gradle(text: &str) -> Result<Gradle, Reject> {
	let stripped = strip_comments(text);
	let (start, end) = find_block(&stripped, "keiyoushi")
		.ok_or_else(|| Reject::new("unparsed-gradle", "no keiyoushi { } block"))?;
	let body = &stripped[start..end];

	let mut gradle = Gradle::default();
	let mut content_warning = None;
	for statement in statements(body) {
		match statement {
			Statement::Assign { key, value } => {
				if !GRADLE_KEYS.contains(&key) {
					return Err(Reject::new(
						"unparsed-gradle",
						format!("unknown key {key:?} in keiyoushi {{ }}"),
					));
				}
				match key {
					"name" => gradle.name = string_literal(value).unwrap_or_default(),
					"versionCode" => gradle.version = int_literal(value).unwrap_or(0),
					"theme" => gradle.theme = string_literal(value),
					"pkgName" => gradle.pkg_name = string_literal(value),
					"contentWarning" => {
						content_warning = value
							.rsplit_once('.')
							.map(|(_, variant)| variant.trim().to_string())
					},
					_ => {},
				}
			},
			Statement::Block { key, body } => {
				if !GRADLE_BLOCKS.contains(&key) {
					return Err(Reject::new(
						"unparsed-gradle",
						format!("unknown block {key:?} in keiyoushi {{ }}"),
					));
				}
				if key == "source" {
					gradle.sources.push(parse_source_block(body)?);
				}
			},
			Statement::Call => {},
		}
	}

	let warning = content_warning.ok_or_else(|| {
		Reject::new("unparsed-gradle", "no contentWarning in keiyoushi { }")
	})?;
	gradle.nsfw = match warning.as_str() {
		"SAFE" => false,
		"MIXED" | "NSFW" => true,
		other => {
			return Err(Reject::new(
				"unparsed-gradle",
				format!("unknown contentWarning {other:?}"),
			))
		},
	};
	if gradle.sources.is_empty() {
		return Err(Reject::new(
			"unparsed-gradle",
			"no source { } block in keiyoushi { }",
		));
	}
	Ok(gradle)
}

fn parse_source_block(body: &str) -> Result<SourceBlock, Reject> {
	let mut block = SourceBlock::default();
	for statement in statements(body) {
		match statement {
			Statement::Assign { key, value } => {
				if !SOURCE_KEYS.contains(&key) {
					return Err(Reject::new(
						"unparsed-gradle",
						format!("unknown key {key:?} in source {{ }}"),
					));
				}
				match key {
					"name" => block.name = string_literal(value),
					"lang" => block.lang = string_literal(value),
					"baseUrl" => block.base_url = string_literal(value),
					_ => {},
				}
			},
			Statement::Block { key, body } => {
				if !SOURCE_BLOCKS.contains(&key) {
					return Err(Reject::new(
						"unparsed-gradle",
						format!("unknown block {key:?} in source {{ }}"),
					));
				}
				if key == "baseUrl" {
					// `custom("url")` is the default of a user-editable URL and
					// `mirrors(a, b, ...)` defaults to its first entry, which may
					// be a `"label" to "url"` pair.
					block.base_url = string_literals(body)
						.into_iter()
						.find(|literal| literal.starts_with("http"));
				}
			},
			Statement::Call => {},
		}
	}
	Ok(block)
}

/// One statement of a Gradle DSL block.
enum Statement<'a> {
	Assign { key: &'a str, value: &'a str },
	Block { key: &'a str, body: &'a str },
	Call,
}

/// Split a DSL block body into its top-level statements.
fn statements(body: &str) -> Vec<Statement<'_>> {
	let bytes = body.as_bytes();
	let mut out = Vec::new();
	let mut i = 0;
	while i < bytes.len() {
		while i < bytes.len() && !is_ident_start(bytes[i]) {
			i = match bytes[i] {
				b'"' => skip_string(bytes, i),
				b'{' => match_brace(bytes, i).map_or(bytes.len(), |end| end + 1),
				_ => i + 1,
			};
		}
		if i >= bytes.len() {
			break;
		}
		let key_start = i;
		while i < bytes.len() && is_ident_byte(bytes[i]) {
			i += 1;
		}
		let key = &body[key_start..i];
		let mut probe = i;
		while probe < bytes.len() && (bytes[probe] == b' ' || bytes[probe] == b'\t') {
			probe += 1;
		}
		match bytes.get(probe) {
			Some(b'=') if bytes.get(probe + 1) != Some(&b'=') => {
				let value_start = probe + 1;
				let mut end = value_start;
				let mut depth = 0i32;
				while end < bytes.len() {
					match bytes[end] {
						b'"' => {
							end = skip_string(bytes, end);
							continue;
						},
						b'(' | b'[' | b'{' => depth += 1,
						b')' | b']' | b'}' => depth -= 1,
						b'\n' if depth <= 0 => break,
						_ => {},
					}
					end += 1;
				}
				out.push(Statement::Assign {
					key,
					value: body[value_start..end].trim(),
				});
				i = end;
			},
			Some(b'{') => {
				let Some(close) = match_brace(bytes, probe) else {
					break;
				};
				out.push(Statement::Block {
					key,
					body: &body[probe + 1..close],
				});
				i = close + 1;
			},
			Some(b'(') => {
				let Some(close) = match_paren(bytes, probe) else {
					break;
				};
				out.push(Statement::Call);
				i = close + 1;
			},
			_ => {},
		}
	}
	out
}

// ---------------------------------------------------------------------------
// Kotlin
// ---------------------------------------------------------------------------

/// A knob before the source's own `baseUrl` is known: one Kotlin class can back
/// several `source {}` blocks, so a `"$baseUrl/api"` knob is resolved per block.
#[derive(Debug, Clone, PartialEq)]
enum RawKnob {
	Bool(bool),
	Int(i64),
	Str(String),
}

impl RawKnob {
	fn resolve(&self, base_url: &str) -> Knob {
		match self {
			RawKnob::Bool(value) => Knob::Bool(*value),
			RawKnob::Int(value) => Knob::Int(*value),
			RawKnob::Str(value) => Knob::Str(if value.contains(BASE_URL_MARKER) {
				value.replace(BASE_URL_MARKER, base_url)
			} else {
				value.clone()
			}),
		}
	}
}

/// The `@Source` class of one extension. The declaration is parsed first,
/// because it decides the theme: a hand-written source has to be reported as
/// `no-theme` and not as whichever of its many overrides carries logic.
#[derive(Debug, Clone, Default, PartialEq)]
struct SourceClass {
	base_class: String,
	/// `lib-multisrc` module the base class was imported from.
	theme_module: Option<String>,
	/// Supertypes beyond the base class: behaviour the theme does not have.
	extra_supertypes: Vec<String>,
	/// Still-unparsed base-class constructor call (pre-KSP layout).
	ctor_args: String,
	/// Still-unparsed class body.
	body: String,
	knobs: BTreeMap<String, RawKnob>,
	/// Metadata from a pre-KSP base-class constructor call.
	ctor_name: Option<String>,
	ctor_lang: Option<String>,
	ctor_base_url: Option<String>,
}

/// Find and parse the declaration of the one `@Source` class of an extension.
fn source_class(dir: &Path) -> Result<SourceClass, Reject> {
	let files = kotlin_files(dir).map_err(|error| {
		Reject::new("no-source-class", format!("unreadable sources: {error}"))
	})?;
	let mut candidates = Vec::new();
	for file in files {
		let Ok(text) = std::fs::read_to_string(&file) else {
			continue;
		};
		let stripped = strip_comments(&text);
		let mut from = 0;
		while let Some(at) = find_word(&stripped, "@Source", from) {
			candidates.push((stripped.clone(), at));
			from = at + "@Source".len();
		}
	}
	match candidates.len() {
		0 => Err(Reject::new(
			"no-source-class",
			"no @Source-annotated class in the extension",
		)),
		1 => {
			let (text, at) = &candidates[0];
			parse_source_class(text, *at)
		},
		count => Err(Reject::new(
			"multiple-source-classes",
			format!("{count} @Source classes: source blocks cannot be mapped to a class"),
		)),
	}
}

fn parse_source_class(text: &str, annotation: usize) -> Result<SourceClass, Reject> {
	let bytes = text.as_bytes();
	let class_at = find_word(text, "class", annotation).ok_or_else(|| {
		Reject::new("no-source-class", "@Source is not followed by a class")
	})?;
	// Nothing but modifiers and annotations may sit between the two.
	let between = &text[annotation + "@Source".len()..class_at];
	for word in between.split_whitespace() {
		let allowed = word.starts_with('@')
			|| matches!(
				word,
				"abstract" | "open" | "internal" | "public" | "sealed" | "final"
			);
		if !allowed {
			return Err(Reject::new(
				"no-source-class",
				format!("unexpected {word:?} between @Source and the class"),
			));
		}
	}

	let mut i = class_at + "class".len();
	while i < bytes.len() && bytes[i].is_ascii_whitespace() {
		i += 1;
	}
	let name_start = i;
	while i < bytes.len() && is_ident_byte(bytes[i]) {
		i += 1;
	}
	if name_start == i {
		return Err(Reject::new("no-source-class", "class has no name"));
	}

	// Supertype list: everything from `:` to the body brace (or the end of the
	// declaration when the class has no body).
	let mut supertypes = "";
	let mut body = "";
	let mut depth = 0i32;
	let mut colon = None;
	while i < bytes.len() {
		match bytes[i] {
			b'"' => {
				i = skip_string(bytes, i);
				continue;
			},
			b'(' | b'[' | b'<' => depth += 1,
			b')' | b']' | b'>' => depth -= 1,
			b':' if depth <= 0 && colon.is_none() => colon = Some(i + 1),
			b'{' if depth <= 0 => {
				let close = match_brace(bytes, i).ok_or_else(|| {
					Reject::new("no-source-class", "unbalanced class body")
				})?;
				if let Some(colon) = colon {
					supertypes = text[colon..i].trim();
				}
				body = &text[i + 1..close];
				break;
			},
			b'\n' if depth <= 0 => {
				let last = last_significant(&text[class_at..i]);
				let next = next_significant(bytes, i);
				if !continues(last, next) {
					if let Some(colon) = colon {
						supertypes = text[colon..i].trim();
					}
					break;
				}
			},
			_ => {},
		}
		i += 1;
	}
	if supertypes.is_empty() {
		if let Some(colon) = colon.filter(|_| i >= bytes.len()) {
			supertypes = text[colon..].trim();
		}
	}
	if supertypes.is_empty() {
		return Err(Reject::new(
			"no-theme",
			"the class extends nothing: not a themed source",
		));
	}

	let parts = split_top_level(supertypes, b',');
	let base = parts.first().copied().unwrap_or_default().trim();
	let extra_supertypes = parts[1..]
		.iter()
		.map(|part| part.trim().to_string())
		.filter(|part| !part.is_empty())
		.collect::<Vec<_>>();

	let (base_class, args) = match base.find('(') {
		Some(open) => {
			let close = match_paren(base.as_bytes(), open).ok_or_else(|| {
				Reject::new("ctor-args", "unbalanced base class constructor call")
			})?;
			(base[..open].trim(), base[open + 1..close].trim())
		},
		None => (base, ""),
	};
	let base_class = base_class
		.split(|c: char| !c.is_alphanumeric() && c != '_')
		.next()
		.unwrap_or_default()
		.to_string();
	if base_class.is_empty() {
		return Err(Reject::new("no-theme", "the class extends nothing"));
	}

	Ok(SourceClass {
		theme_module: theme_module(text, &base_class),
		base_class,
		extra_supertypes,
		ctor_args: args.to_string(),
		body: body.to_string(),
		..SourceClass::default()
	})
}

/// Turn the declaration's constructor call and body into knobs. Run after the
/// theme is known, so every rejection is attributed to a theme.
fn parse_knobs(class: &mut SourceClass) -> Result<(), Reject> {
	if !class.extra_supertypes.is_empty() {
		return Err(Reject::new(
			"extra-supertype",
			format!("also implements {}", class.extra_supertypes.join(", ")),
		));
	}
	let args = std::mem::take(&mut class.ctor_args);
	if !args.is_empty() {
		parse_ctor_args(&args, class)?;
	}
	let body = std::mem::take(&mut class.body);
	if !body.is_empty() {
		parse_class_body(&body, class)?;
	}
	Ok(())
}

/// The `lib-multisrc` module a base class was imported from.
fn theme_module(text: &str, base_class: &str) -> Option<String> {
	const PREFIX: &str = "eu.kanade.tachiyomi.multisrc.";
	for line in text.lines() {
		let line = line.trim();
		let Some(path) = line.strip_prefix("import ") else {
			continue;
		};
		let path = path.trim().trim_end_matches(';');
		let Some(rest) = path.strip_prefix(PREFIX) else {
			continue;
		};
		let mut segments = rest.split('.');
		let module = segments.next()?;
		if segments.next() == Some(base_class) {
			return Some(module.to_ascii_lowercase());
		}
	}
	None
}

/// Metadata and knobs from a pre-KSP `Base("Name", "https://x", "en", knob =
/// ...)` constructor call.
fn parse_ctor_args(args: &str, class: &mut SourceClass) -> Result<(), Reject> {
	let mut positional = 0usize;
	for arg in split_top_level(args, b',') {
		let arg = arg.trim();
		if arg.is_empty() {
			continue;
		}
		let named = named_argument(arg);
		match named {
			Some((name, value)) => {
				let key = snake_case(name);
				match key.as_str() {
					"name" => class.ctor_name = string_literal(value),
					"lang" => class.ctor_lang = string_literal(value),
					"base_url" => class.ctor_base_url = string_literal(value),
					_ => {
						let knobs = recognise(&key, value).map_err(|reason| {
							Reject::new(
								"ctor-args",
								format!("constructor argument {name}: {reason}"),
							)
						})?;
						class.knobs.extend(knobs);
					},
				}
			},
			None => {
				let value = string_literal(arg).ok_or_else(|| {
					Reject::new(
						"ctor-args",
						format!("positional constructor argument {arg} is not a string"),
					)
				})?;
				match positional {
					0 => class.ctor_name = Some(value),
					1 => class.ctor_base_url = Some(value),
					2 => class.ctor_lang = Some(value),
					_ => {
						return Err(Reject::new(
							"ctor-args",
							format!("unexpected positional constructor argument {arg}"),
						))
					},
				}
				positional += 1;
			},
		}
	}
	Ok(())
}

/// Split a class body into members and turn every `override` of a literal into
/// a knob. Anything else is behaviour, and rejects the source.
///
/// Every blocking member is collected, not just the first: `unsupported.json`
/// is the theme engines' work list, and a source that needs six members ported
/// must not read as if it needed one. The reject keeps the *first* blocker's
/// code and reason in file order and appends the names of the rest.
fn parse_class_body(body: &str, class: &mut SourceClass) -> Result<(), Reject> {
	// `override fun latestUpdatesSelector() = popularMangaSelector()` is one of
	// the theme idioms: an alias for another member of the same class, which may
	// be declared further down.
	let mut aliases: Vec<(String, String, &'static str)> = Vec::new();
	let mut blockers: Vec<Reject> = Vec::new();

	for member in split_members(body) {
		let member = member.trim();
		if member.is_empty() {
			continue;
		}
		if starts_with_word(member, "init") {
			blockers.push(Reject::new(
				"init-block",
				"init: the class has an init { } block",
			));
			continue;
		}
		let Some(declaration) = declaration(member) else {
			continue;
		};
		if !declaration.modifiers.iter().any(|word| *word == "override") {
			continue;
		}

		let parsed = match declaration.kind {
			DeclarationKind::Property => property(declaration.rest)
				.map(|(name, expression)| (name, Vec::new(), expression)),
			DeclarationKind::Function => function(declaration.rest),
		};
		let (member_name, parameters, expression) = match parsed {
			Ok(parsed) => parsed,
			Err(reject) => {
				blockers.push(reject);
				continue;
			},
		};
		let key = snake_case(&member_name);
		let code = match key.as_str() {
			"client" => "custom-client",
			_ if declaration.kind == DeclarationKind::Function => "custom-override",
			_ => "unparsed-knob",
		};
		match key.as_str() {
			"name" => class.ctor_name = string_literal(expression),
			"lang" => class.ctor_lang = string_literal(expression),
			"base_url" => class.ctor_base_url = string_literal(expression),
			_ => {
				if let Some(target) = alias_target(expression) {
					aliases.push((member_name, target, code));
					continue;
				}
				let recognised = match request_url(&key, expression, &parameters) {
					Some(result) => result,
					None => recognise(&key, expression),
				}
				// Reasons quote the upstream identifier, not the derived knob
				// key, so a reader can go straight back to the Kotlin.
				.map_err(|reason| Reject::new(code, format!("{member_name}: {reason}")));
				match recognised {
					Ok(knobs) => class.knobs.extend(knobs),
					Err(reject) => blockers.push(reject),
				}
			},
		}
	}

	for (member_name, target, code) in aliases {
		match class.knobs.get(&target).cloned() {
			Some(knob) => {
				class.knobs.insert(snake_case(&member_name), knob);
			},
			None => blockers.push(Reject::new(
				code,
				format!("{member_name}: aliases {target}, which is not a knob"),
			)),
		}
	}

	let mut blockers = blockers.into_iter();
	let Some(first) = blockers.next() else {
		return Ok(());
	};
	let rest = blockers
		.map(|reject| member_of(&reject.reason).to_string())
		.collect::<Vec<_>>();
	if rest.is_empty() {
		return Err(first);
	}
	Err(Reject::new(
		first.code,
		format!(
			"{} (and {} more: {})",
			first.reason,
			rest.len(),
			rest.join(", ")
		),
	))
}

/// The `<member>: ` a reject reason leads with — every member-level reason is
/// built that way — so the summary can list names instead of whole reasons.
fn member_of(reason: &str) -> &str {
	match reason.split_once(':') {
		Some((head, _)) if !head.is_empty() && head.bytes().all(is_ident_byte) => head,
		_ => reason,
	}
}

/// The knob key a `= otherMember()` / `= otherMember` body aliases.
fn alias_target(expression: &str) -> Option<String> {
	let expression = expression.trim();
	let name = expression.strip_suffix("()").unwrap_or(expression);
	let name = name.trim();
	if name.is_empty() || !name.bytes().all(is_ident_byte) {
		return None;
	}
	if !is_ident_start(name.as_bytes()[0]) {
		return None;
	}
	if matches!(name, "true" | "false" | "null" | "this" | "super") {
		return None;
	}
	if int_literal(name).is_some() {
		return None;
	}
	Some(snake_case(name))
}

/// `override fun popularMangaRequest(page: Int) = GET("$baseUrl/list?p=$page")`
/// is a URL template: the only non-selector extraction point a theme subclass
/// states as data. Returns `None` when the member is not a request override.
fn request_url(
	key: &str,
	expression: &str,
	parameters: &[String],
) -> Option<Result<Vec<(String, RawKnob)>, String>> {
	let stem = key.strip_suffix("_request")?;
	let url_key = format!("{stem}_url");
	let expression = expression.trim();
	let rest = expression.strip_prefix("GET")?.trim_start();
	let open = rest.find('(').filter(|open| *open == 0)?;
	let Some(close) = match_paren(rest.as_bytes(), open) else {
		return Some(Err(truncate(expression)));
	};
	if !rest[close + 1..].trim().is_empty() {
		return Some(Err(truncate(expression)));
	}

	let arguments = split_top_level(&rest[open + 1..close], b',');
	// Only `headers`/`headersBuilder…` may follow the URL: anything else is a
	// body, a cache directive or a computed request.
	for argument in arguments.iter().skip(1) {
		let argument = named_argument(argument).map_or(*argument, |(_, value)| value);
		if !matches!(argument.trim(), "headers" | "") {
			return Some(Err(truncate(expression)));
		}
	}
	let template = arguments.first().map(|argument| argument.trim())?;
	let Some(url) = parse_string(template, parameters) else {
		return Some(Err(truncate(template)));
	};
	Some(Ok(vec![(url_key, RawKnob::Str(url))]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclarationKind {
	Property,
	Function,
}

struct Declaration<'a> {
	kind: DeclarationKind,
	modifiers: Vec<&'a str>,
	rest: &'a str,
}

/// Split a member into its modifiers and the declaration that follows, or
/// `None` when it is not a property or function.
fn declaration(member: &str) -> Option<Declaration<'_>> {
	let bytes = member.as_bytes();
	let mut i = 0;
	let mut modifiers = Vec::new();
	while i < bytes.len() {
		while i < bytes.len() && bytes[i].is_ascii_whitespace() {
			i += 1;
		}
		if i >= bytes.len() {
			return None;
		}
		if bytes[i] == b'@' {
			// Annotation, possibly with arguments.
			i += 1;
			while i < bytes.len() && is_ident_byte(bytes[i]) {
				i += 1;
			}
			if bytes.get(i) == Some(&b'(') {
				i = match_paren(bytes, i).map_or(bytes.len(), |end| end + 1);
			}
			continue;
		}
		let start = i;
		while i < bytes.len() && is_ident_byte(bytes[i]) {
			i += 1;
		}
		if start == i {
			return None;
		}
		let word = &member[start..i];
		let kind = match word {
			"val" | "var" => DeclarationKind::Property,
			"fun" => DeclarationKind::Function,
			_ => {
				modifiers.push(word);
				continue;
			},
		};
		return Some(Declaration {
			kind,
			modifiers,
			rest: &member[i..],
		});
	}
	None
}

/// `<name>[: Type] = <expression>` of a property declaration.
fn property(rest: &str) -> Result<(String, &str), Reject> {
	let bytes = rest.as_bytes();
	let mut i = 0;
	while i < bytes.len() && bytes[i].is_ascii_whitespace() {
		i += 1;
	}
	let start = i;
	while i < bytes.len() && is_ident_byte(bytes[i]) {
		i += 1;
	}
	let name = rest[start..i].to_string();
	if name.is_empty() {
		return Err(Reject::new("unparsed-knob", "property has no name"));
	}
	if find_word(rest, "by", i).is_some() {
		return Err(Reject::new(
			"unparsed-knob",
			format!("{name}: delegated property"),
		));
	}
	let Some(equals) = top_level_byte(rest, i, b'=') else {
		return Err(Reject::new(
			"unparsed-knob",
			format!("{name}: no single-expression initialiser"),
		));
	};
	let expression = rest[equals + 1..].trim();
	if expression.starts_with('{') {
		return Err(Reject::new(
			"unparsed-knob",
			format!("{name}: block initialiser"),
		));
	}
	Ok((name, expression))
}

/// `<receiver.>?<name>(params)[: Type] = <expression>` of a function
/// declaration; a block body is behaviour and rejects the source. The
/// parameter names come back because a request override renders them into a
/// URL template.
fn function(rest: &str) -> Result<(String, Vec<String>, &str), Reject> {
	let bytes = rest.as_bytes();
	let open = top_level_byte(rest, 0, b'(').ok_or_else(|| {
		Reject::new(
			"custom-override",
			format!("{}: function has no parameter list", truncate(rest)),
		)
	})?;
	let mut start = open;
	while start > 0 && is_ident_byte(bytes[start - 1]) {
		start -= 1;
	}
	let name = rest[start..open].to_string();
	let close = match_paren(bytes, open).ok_or_else(|| {
		Reject::new("custom-override", format!("{name}: unbalanced parameters"))
	})?;
	let parameters = split_top_level(&rest[open + 1..close], b',')
		.into_iter()
		.filter_map(|parameter| {
			let parameter = parameter.trim();
			let parameter = parameter.split(':').next()?.trim();
			let parameter = parameter
				.rsplit_once(' ')
				.map_or(parameter, |(_, last)| last);
			(!parameter.is_empty() && parameter.bytes().all(is_ident_byte))
				.then(|| parameter.to_string())
		})
		.collect::<Vec<_>>();
	let Some(equals) = top_level_byte(rest, close + 1, b'=') else {
		return Err(Reject::new(
			"custom-override",
			format!("{name}: block body, not a single expression"),
		));
	};
	Ok((name, parameters, rest[equals + 1..].trim()))
}

/// Turn one recognised Kotlin expression into knobs, or say why it is not data.
fn recognise(key: &str, expression: &str) -> Result<Vec<(String, RawKnob)>, String> {
	let expression = expression.trim();
	if expression.is_empty() {
		return Err("empty expression".to_string());
	}

	// Networking policy: the only non-literal forms that are still pure data.
	if key == "configure_client" {
		return rate_limit(expression)
			.map_err(|reason| format!("{reason} (configureClient)"));
	}
	if key == "client" {
		let chain = expression
			.strip_prefix("super.client.newBuilder()")
			.or_else(|| expression.strip_prefix("network.client.newBuilder()"))
			.ok_or_else(|| truncate(expression))?;
		let chain = chain.trim();
		let chain = chain
			.strip_suffix(".build()")
			.ok_or_else(|| truncate(expression))?;
		let call = chain
			.trim()
			.strip_prefix('.')
			.ok_or_else(|| truncate(expression))?;
		if split_top_level(call, b'.').len() != 1 {
			return Err(truncate(expression));
		}
		let call = call
			.trim()
			.strip_prefix("rateLimit")
			.ok_or_else(|| truncate(expression))?;
		return rate_limit(&format!("rateLimit{call}"));
	}

	if let Some(literal) = string_literal(expression) {
		return Ok(vec![(key.to_string(), RawKnob::Str(literal))]);
	}
	if expression == "true" || expression == "false" {
		return Ok(vec![(key.to_string(), RawKnob::Bool(expression == "true"))]);
	}
	if let Some(value) = int_literal(expression) {
		return Ok(vec![(key.to_string(), RawKnob::Int(value))]);
	}
	if let Some(knobs) = date_format(key, expression)? {
		return Ok(knobs);
	}
	if let Some(constant) = enum_constant(expression) {
		return Ok(vec![(key.to_string(), RawKnob::Str(constant))]);
	}
	Err(truncate(expression))
}

/// `SimpleDateFormat("p", Locale.X)` / `DateTimeFormatter.ofPattern("p", …)`
/// become the pattern plus a BCP-47 locale knob.
fn date_format(
	key: &str,
	expression: &str,
) -> Result<Option<Vec<(String, RawKnob)>>, String> {
	let args = ["SimpleDateFormat", "DateTimeFormatter.ofPattern"]
		.into_iter()
		.find_map(|prefix| {
			let rest = expression.strip_prefix(prefix)?.trim_start();
			let open = rest.find('(')?;
			if !rest[..open].trim().is_empty() {
				return None;
			}
			let close = match_paren(rest.as_bytes(), open)?;
			if !rest[close + 1..].trim().is_empty() {
				return None;
			}
			Some(rest[open + 1..close].to_string())
		});
	let Some(args) = args else {
		return Ok(None);
	};

	let args = split_top_level(&args, b',');
	let pattern = args
		.first()
		.and_then(|arg| string_literal(arg.trim()))
		.ok_or_else(|| "date pattern is not a literal".to_string())?;
	let mut knobs = vec![(key.to_string(), RawKnob::Str(pattern))];

	let locale_arg = args
		.get(1)
		.map(|arg| arg.trim())
		.filter(|arg| !arg.is_empty());
	if let Some(arg) = locale_arg {
		let arg = named_argument(arg).map_or(arg, |(_, value)| value);
		let locale = locale(arg).ok_or_else(|| format!("locale {}", truncate(arg)))?;
		let locale_key = key
			.strip_suffix("_format")
			.map(|stem| format!("{stem}_locale"))
			.unwrap_or_else(|| format!("{key}_locale"));
		knobs.push((locale_key, RawKnob::Str(locale)));
	}
	Ok(Some(knobs))
}

/// `java.util.Locale` as a BCP-47 tag; `Locale.getDefault()` is not data.
fn locale(expression: &str) -> Option<String> {
	const CONSTANTS: &[(&str, &str)] = &[
		("ROOT", ""),
		("ENGLISH", "en"),
		("US", "en-US"),
		("UK", "en-GB"),
		("CANADA", "en-CA"),
		("CANADA_FRENCH", "fr-CA"),
		("FRENCH", "fr"),
		("FRANCE", "fr-FR"),
		("GERMAN", "de"),
		("GERMANY", "de-DE"),
		("ITALIAN", "it"),
		("ITALY", "it-IT"),
		("JAPANESE", "ja"),
		("JAPAN", "ja-JP"),
		("KOREAN", "ko"),
		("KOREA", "ko-KR"),
		("CHINESE", "zh"),
		("SIMPLIFIED_CHINESE", "zh-CN"),
		("TRADITIONAL_CHINESE", "zh-TW"),
		("CHINA", "zh-CN"),
		("PRC", "zh-CN"),
		("TAIWAN", "zh-TW"),
	];

	let expression = expression.trim();
	if let Some(constant) = expression.strip_prefix("Locale.") {
		let constant = constant.trim();
		if let Some(rest) = constant.strip_prefix("forLanguageTag") {
			let open = rest.find('(')?;
			let close = match_paren(rest.as_bytes(), open)?;
			return string_literal(rest[open + 1..close].trim());
		}
		return CONSTANTS
			.iter()
			.find(|(name, _)| *name == constant)
			.map(|(_, tag)| tag.to_string());
	}
	let rest = expression.strip_prefix("Locale")?.trim_start();
	let open = rest.find('(')?;
	if !rest[..open].trim().is_empty() {
		return None;
	}
	let close = match_paren(rest.as_bytes(), open)?;
	let parts = split_top_level(&rest[open + 1..close], b',');
	let language = string_literal(parts.first()?.trim())?;
	match parts
		.get(1)
		.map(|part| part.trim())
		.filter(|part| !part.is_empty())
	{
		Some(region) => Some(format!("{language}-{}", string_literal(region)?)),
		None => Some(language),
	}
}

/// `rateLimit(permits[, period])`, with the trailing host/path predicate
/// dropped: ignoring it can only make an engine more polite.
fn rate_limit(expression: &str) -> Result<Vec<(String, RawKnob)>, String> {
	let rest = expression
		.trim()
		.strip_prefix("rateLimit")
		.or_else(|| expression.trim().strip_prefix("this.rateLimit"))
		.ok_or_else(|| truncate(expression))?;
	let rest = rest.trim_start();
	let open = rest
		.find('(')
		.filter(|open| *open == 0)
		.ok_or_else(|| truncate(expression))?;
	let close = match_paren(rest.as_bytes(), open).ok_or_else(|| truncate(expression))?;
	let tail = rest[close + 1..].trim();
	if !tail.is_empty() && !tail.starts_with('{') {
		return Err(truncate(expression));
	}

	let args = split_top_level(&rest[open + 1..close], b',');
	let mut permits = None;
	let mut period = 1i64;
	for (index, arg) in args.iter().enumerate() {
		let arg = arg.trim();
		if arg.is_empty() {
			continue;
		}
		let (name, value) = match named_argument(arg) {
			Some((name, value)) => (name.to_string(), value),
			None => (
				match index {
					0 => "permits".to_string(),
					1 => "period".to_string(),
					_ => return Err(truncate(expression)),
				},
				arg,
			),
		};
		match name.as_str() {
			"permits" => {
				permits = Some(int_literal(value).ok_or_else(|| truncate(expression))?)
			},
			"period" => {
				period = duration_seconds(value).ok_or_else(|| truncate(expression))?
			},
			"interval" | "shouldLimit" => {},
			_ => return Err(truncate(expression)),
		}
	}
	let permits = permits.ok_or_else(|| truncate(expression))?;
	Ok(vec![
		("rate_limit_permits".to_string(), RawKnob::Int(permits)),
		(
			"rate_limit_period_seconds".to_string(),
			RawKnob::Int(period),
		),
	])
}

/// `2.seconds`, `1.minutes`, `1.hours` as whole seconds.
fn duration_seconds(expression: &str) -> Option<i64> {
	let (value, unit) = expression.trim().rsplit_once('.')?;
	let value = int_literal(value.trim())?;
	match unit.trim() {
		"seconds" => Some(value),
		"minutes" => Some(value * 60),
		"hours" => Some(value * 3600),
		_ => None,
	}
}

/// `Enum.Constant` as the `snake_case` of the constant.
fn enum_constant(expression: &str) -> Option<String> {
	let expression = expression.trim();
	if expression.contains(|c: char| c.is_whitespace() || "(){}[]\"+-".contains(c)) {
		return None;
	}
	let (owner, constant) = expression.rsplit_once('.')?;
	let owner = owner.rsplit_once('.').map_or(owner, |(_, last)| last);
	if !owner.starts_with(|c: char| c.is_ascii_uppercase()) {
		return None;
	}
	if constant.is_empty() || !constant.starts_with(|c: char| c.is_ascii_alphabetic()) {
		return None;
	}
	Some(snake_case(constant))
}

fn truncate(expression: &str) -> String {
	let flat = expression.split_whitespace().collect::<Vec<_>>().join(" ");
	if flat.chars().count() <= 60 {
		return flat;
	}
	let head = flat.chars().take(57).collect::<String>();
	format!("{head}...")
}

// ---------------------------------------------------------------------------
// Scanning helpers
// ---------------------------------------------------------------------------

/// Replace every comment with spaces, keeping newlines so declarations stay on
/// their own lines. String and character literals are preserved verbatim.
fn strip_comments(source: &str) -> String {
	let chars = source.chars().collect::<Vec<_>>();
	let mut out = String::with_capacity(source.len());
	let mut i = 0;
	while i < chars.len() {
		let current = chars[i];
		let next = chars.get(i + 1).copied();
		match current {
			'/' if next == Some('/') => {
				while i < chars.len() && chars[i] != '\n' {
					out.push(' ');
					i += 1;
				}
			},
			'/' if next == Some('*') => {
				let mut depth = 1usize;
				out.push_str("  ");
				i += 2;
				while i < chars.len() && depth > 0 {
					if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
						depth -= 1;
						out.push_str("  ");
						i += 2;
						continue;
					}
					if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
						depth += 1;
						out.push_str("  ");
						i += 2;
						continue;
					}
					out.push(if chars[i] == '\n' { '\n' } else { ' ' });
					i += 1;
				}
			},
			'"' if next == Some('"') && chars.get(i + 2) == Some(&'"') => {
				out.push_str("\"\"\"");
				i += 3;
				while i < chars.len() {
					if chars[i] == '"'
						&& chars.get(i + 1) == Some(&'"')
						&& chars.get(i + 2) == Some(&'"')
					{
						out.push_str("\"\"\"");
						i += 3;
						break;
					}
					out.push(chars[i]);
					i += 1;
				}
			},
			'"' | '\'' => {
				out.push(current);
				i += 1;
				while i < chars.len() {
					let ch = chars[i];
					out.push(ch);
					i += 1;
					if ch == '\\' {
						if let Some(escaped) = chars.get(i) {
							out.push(*escaped);
							i += 1;
						}
						continue;
					}
					if ch == current || ch == '\n' {
						break;
					}
				}
			},
			_ => {
				out.push(current);
				i += 1;
			},
		}
	}
	out
}

fn is_ident_start(byte: u8) -> bool {
	byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident_byte(byte: u8) -> bool {
	byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Index just past a `"…"` or `"""…"""` literal starting at `at`.
fn skip_string(bytes: &[u8], at: usize) -> usize {
	if bytes.get(at) != Some(&b'"') {
		return at + 1;
	}
	if bytes.get(at + 1) == Some(&b'"') && bytes.get(at + 2) == Some(&b'"') {
		let mut i = at + 3;
		while i + 2 < bytes.len() {
			if bytes[i] == b'"' && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
				return i + 3;
			}
			i += 1;
		}
		return bytes.len();
	}
	let mut i = at + 1;
	while i < bytes.len() {
		match bytes[i] {
			b'\\' => i += 2,
			b'"' => return i + 1,
			b'\n' => return i,
			_ => i += 1,
		}
	}
	bytes.len()
}

/// Index of the `}` matching the `{` at `at`.
fn match_brace(bytes: &[u8], at: usize) -> Option<usize> {
	match_delimiter(bytes, at, b'{', b'}')
}

/// Index of the `)` matching the `(` at `at`.
fn match_paren(bytes: &[u8], at: usize) -> Option<usize> {
	match_delimiter(bytes, at, b'(', b')')
}

fn match_delimiter(bytes: &[u8], at: usize, open: u8, close: u8) -> Option<usize> {
	if bytes.get(at) != Some(&open) {
		return None;
	}
	let mut depth = 0usize;
	let mut i = at;
	while i < bytes.len() {
		let byte = bytes[i];
		if byte == b'"' {
			i = skip_string(bytes, i);
			continue;
		}
		if byte == open {
			depth += 1;
		} else if byte == close {
			depth -= 1;
			if depth == 0 {
				return Some(i);
			}
		}
		i += 1;
	}
	None
}

/// Byte range of the body of `name { … }`, exclusive of the braces.
fn find_block(source: &str, name: &str) -> Option<(usize, usize)> {
	let bytes = source.as_bytes();
	let mut from = 0;
	while let Some(at) = find_word(source, name, from) {
		let mut probe = at + name.len();
		while probe < bytes.len() && bytes[probe].is_ascii_whitespace() {
			probe += 1;
		}
		if bytes.get(probe) == Some(&b'{') {
			let close = match_brace(bytes, probe)?;
			return Some((probe + 1, close));
		}
		from = at + name.len();
	}
	None
}

/// Index of `word` in `source` at or after `from`, as a whole word.
fn find_word(source: &str, word: &str, from: usize) -> Option<usize> {
	let bytes = source.as_bytes();
	let mut at = from;
	while let Some(found) = source.get(at..)?.find(word) {
		let index = at + found;
		let before = index
			.checked_sub(1)
			.and_then(|i| bytes.get(i).copied())
			.unwrap_or(b' ');
		let after = bytes.get(index + word.len()).copied().unwrap_or(b' ');
		let starts_with_symbol = !is_ident_byte(word.as_bytes()[0]);
		let boundary_before = starts_with_symbol || !is_ident_byte(before);
		if boundary_before && !is_ident_byte(after) {
			return Some(index);
		}
		at = index + word.len();
	}
	None
}

/// Index of the first `byte` in `source` at or after `from` that is not inside
/// a string, parenthesis, bracket or brace.
fn top_level_byte(source: &str, from: usize, byte: u8) -> Option<usize> {
	let bytes = source.as_bytes();
	let mut depth = 0i32;
	let mut i = from;
	while i < bytes.len() {
		match bytes[i] {
			b'"' => {
				i = skip_string(bytes, i);
				continue;
			},
			// The sought byte is tested first, so searching for an opening
			// delimiter finds it instead of counting it.
			found if found == byte && depth <= 0 => return Some(i),
			b'(' | b'[' | b'{' | b'<' => depth += 1,
			b')' | b']' | b'}' | b'>' => depth -= 1,
			_ => {},
		}
		i += 1;
	}
	None
}

/// Split on a top-level separator, ignoring strings and nested delimiters.
fn split_top_level(source: &str, separator: u8) -> Vec<&str> {
	let bytes = source.as_bytes();
	let mut parts = Vec::new();
	let mut depth = 0i32;
	let mut start = 0;
	let mut i = 0;
	while i < bytes.len() {
		match bytes[i] {
			b'"' => {
				i = skip_string(bytes, i);
				continue;
			},
			b'(' | b'[' | b'{' => depth += 1,
			b')' | b']' | b'}' => depth -= 1,
			found if found == separator && depth <= 0 => {
				parts.push(&source[start..i]);
				start = i + 1;
			},
			_ => {},
		}
		i += 1;
	}
	parts.push(&source[start..]);
	parts
}

/// `name = value` of a named argument, if the argument is one.
fn named_argument(argument: &str) -> Option<(&str, &str)> {
	let equals = top_level_byte(argument, 0, b'=')?;
	if argument.as_bytes().get(equals + 1) == Some(&b'=') {
		return None;
	}
	let name = argument[..equals].trim();
	if name.is_empty() || !name.bytes().all(is_ident_byte) {
		return None;
	}
	Some((name, argument[equals + 1..].trim()))
}

fn starts_with_word(source: &str, word: &str) -> bool {
	source.strip_prefix(word).is_some_and(|rest| {
		rest.as_bytes()
			.first()
			.is_none_or(|byte| !is_ident_byte(*byte))
	})
}

/// Split a class body into member declarations.
fn split_members(body: &str) -> Vec<&str> {
	let bytes = body.as_bytes();
	let mut members = Vec::new();
	let mut i = 0;
	while i < bytes.len() {
		while i < bytes.len() && bytes[i].is_ascii_whitespace() {
			i += 1;
		}
		if i >= bytes.len() {
			break;
		}
		let start = i;
		let mut depth = 0i32;
		let mut end = None;
		while i < bytes.len() {
			match bytes[i] {
				b'"' => {
					i = skip_string(bytes, i);
					continue;
				},
				b'{' | b'(' | b'[' => {
					depth += 1;
					i += 1;
				},
				b'}' | b')' | b']' => {
					depth -= 1;
					i += 1;
				},
				b'\n' if depth <= 0 => {
					let last = last_significant(&body[start..i]);
					let next = next_significant(bytes, i);
					if continues(last, next) {
						i += 1;
						continue;
					}
					end = Some(i);
					i += 1;
					break;
				},
				_ => i += 1,
			}
		}
		members.push(&body[start..end.unwrap_or(body.len())]);
	}
	members
}

fn last_significant(source: &str) -> u8 {
	source
		.bytes()
		.rev()
		.find(|byte| !byte.is_ascii_whitespace())
		.unwrap_or(b'\n')
}

fn next_significant(bytes: &[u8], from: usize) -> u8 {
	bytes[from..]
		.iter()
		.copied()
		.find(|byte| !byte.is_ascii_whitespace())
		.unwrap_or(b'\n')
}

/// Whether a declaration continues past a newline: an operator or an opening
/// delimiter before it, or a chained call, argument or block after it.
fn continues(last: u8, next: u8) -> bool {
	const TRAILING: &[u8] = b"=+-*/%,.([{:?&|<>";
	const LEADING: &[u8] = b".,)]?:{+";
	TRAILING.contains(&last) || LEADING.contains(&next)
}

/// The one string literal `source` consists of; `$baseUrl` becomes
/// [`BASE_URL_MARKER`] and any other template makes it not a literal.
fn string_literal(source: &str) -> Option<String> {
	parse_string(source, &[])
}

/// Like [`string_literal`], but a reference to one of `parameters` renders as
/// a `{name}` placeholder instead of rejecting the string, which is what turns
/// `GET("$baseUrl/list?p=$page")` into a URL template.
fn parse_string(source: &str, parameters: &[String]) -> Option<String> {
	let source = source.trim();
	let bytes = source.as_bytes();
	if bytes.first() != Some(&b'"') {
		return None;
	}
	let raw = bytes.get(1) == Some(&b'"') && bytes.get(2) == Some(&b'"');
	let end = skip_string(bytes, 0);
	if end != source.len() {
		return None;
	}
	let inner = if raw {
		&source[3..source.len() - 3]
	} else {
		&source[1..source.len() - 1]
	};

	let mut out = String::with_capacity(inner.len());
	let mut chars = inner.chars().peekable();
	while let Some(c) = chars.next() {
		match c {
			'\\' if !raw => match chars.next()? {
				'n' => out.push('\n'),
				't' => out.push('\t'),
				'r' => out.push('\r'),
				'\\' => out.push('\\'),
				'"' => out.push('"'),
				'\'' => out.push('\''),
				'$' => out.push('$'),
				'u' => {
					let hex = (0..4).map(|_| chars.next()).collect::<Option<String>>()?;
					out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
				},
				_ => return None,
			},
			'$' => {
				let rest = chars.clone().collect::<String>();
				let reference = if let Some(braced) = rest.strip_prefix('{') {
					braced
						.split_once('}')
						.map(|(name, _)| (name, name.len() + 2))
				} else {
					let name = rest
						.split(|c: char| !c.is_alphanumeric() && c != '_')
						.next()
						.unwrap_or_default();
					(!name.is_empty()).then_some((name, name.len()))
				};
				let Some((name, consumed)) = reference else {
					return None;
				};
				let name = name.trim();
				if name == "baseUrl" {
					out.push(BASE_URL_MARKER);
				} else if parameters.iter().any(|parameter| parameter == name) {
					out.push('{');
					out.push_str(name);
					out.push('}');
				} else {
					return None;
				}
				for _ in 0..consumed {
					chars.next();
				}
			},
			_ => out.push(c),
		}
	}
	Some(out)
}

/// Every string literal in `source`, in order.
fn string_literals(source: &str) -> Vec<String> {
	let bytes = source.as_bytes();
	let mut literals = Vec::new();
	let mut i = 0;
	while i < bytes.len() {
		if bytes[i] == b'"' {
			let end = skip_string(bytes, i);
			if let Some(literal) = string_literal(&source[i..end]) {
				literals.push(literal);
			}
			i = end;
			continue;
		}
		i += 1;
	}
	literals
}

fn int_literal(source: &str) -> Option<i64> {
	let source = source.trim().trim_end_matches(['L', 'l']);
	let cleaned = source.replace('_', "");
	if cleaned.is_empty() {
		return None;
	}
	cleaned.parse::<i64>().ok()
}

/// `mangaSubString` -> `manga_sub_string`, `MangaAjaxPaginated` ->
/// `manga_ajax_paginated`, `SEARCH_URL` -> `search_url`.
fn snake_case(name: &str) -> String {
	let chars = name.chars().collect::<Vec<_>>();
	let mut out = String::with_capacity(name.len() + 4);
	for (index, c) in chars.iter().enumerate() {
		if *c == '_' || *c == '-' || c.is_whitespace() {
			if !out.ends_with('_') && !out.is_empty() {
				out.push('_');
			}
			continue;
		}
		if c.is_uppercase() {
			let previous = index.checked_sub(1).map(|i| chars[i]);
			let next = chars.get(index + 1).copied();
			let boundary = previous.is_some_and(|previous| {
				previous.is_lowercase()
					|| previous.is_numeric()
					|| (previous.is_uppercase() && next.is_some_and(char::is_lowercase))
			});
			if boundary && !out.ends_with('_') && !out.is_empty() {
				out.push('_');
			}
			out.extend(c.to_lowercase());
			continue;
		}
		out.push(*c);
	}
	out
}

#[cfg(test)]
mod tests;
