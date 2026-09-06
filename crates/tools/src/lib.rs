//! Library maintenance tools: the Kavita "external tools" feature set as
//! first-class, in-process pipeline modules.
//!
//! Every tool implements [`Tool`]: a pure [`Tool::plan`] pass that only reads
//! the filesystem and returns a printable [`Plan`], and a [`Tool::apply`] pass
//! that performs exactly the planned actions. Originals are never modified
//! unless the caller applies a plan.
//!
//! Behaviour provenance, the option table for every tool, and verification
//! commands are in `crates/tools/README.md`.

#![warn(clippy::dbg_macro)]

mod error;
pub mod external;
mod plan;
#[cfg(test)]
mod test_support;
pub mod util;

// One module per tool; module name is the tool id with underscores.
pub mod boko;
pub mod calibre;
pub mod calibre_polish;
pub mod cbz_covers;
pub mod cbzit;
pub mod epub2cbz;
pub mod epub_check;
pub mod epub_polish;
pub mod meta_edit;
pub mod missing_sequence;
pub mod mobi2epub;
pub mod sources_import;
pub mod webp_convert;

pub use error::{ToolError, ToolResult};
pub use plan::{Action, Plan, Report, Severity, Warning};

use std::path::PathBuf;

use serde::de::DeserializeOwned;

/// The paths a tool operates on plus its tool-specific JSON options.
#[derive(Debug, Clone, Default)]
pub struct ToolInput {
	pub paths: Vec<PathBuf>,
	pub options: serde_json::Value,
}

impl ToolInput {
	pub fn new(paths: Vec<PathBuf>) -> Self {
		Self {
			paths,
			options: serde_json::Value::Null,
		}
	}

	pub fn with_options(mut self, options: serde_json::Value) -> Self {
		self.options = options;
		self
	}

	/// Deserialize [`Self::options`] into a tool's option struct. A `null` or
	/// absent blob yields the option struct's defaults, so every tool is
	/// runnable with no options at all.
	pub fn parse_options<T: DeserializeOwned + Default>(&self) -> ToolResult<T> {
		if self.options.is_null() {
			return Ok(T::default());
		}
		serde_json::from_value(self.options.clone())
			.map_err(|error| ToolError::Options(error.to_string()))
	}
}

/// Progress reporting for [`Tool::apply`]. `plan` never reports progress: it is
/// pure and cheap by contract.
pub trait ProgressSink {
	fn progress(&mut self, done: usize, total: usize, message: &str);
}

/// A sink that drops every update; used by tests and non-interactive callers.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopProgress;

impl ProgressSink for NoopProgress {
	fn progress(&mut self, _done: usize, _total: usize, _message: &str) {}
}

/// One library maintenance tool.
///
/// Implementations must be pure in [`Tool::plan`] (reads only) and must apply
/// only what the plan describes, so `plan` output is a truthful dry run.
pub trait Tool: Send + Sync {
	/// Stable kebab-case id, e.g. `"epub2cbz"`.
	fn id(&self) -> &'static str;
	/// One-line description shown by `stump tools list`.
	fn describe(&self) -> &'static str;
	/// Dry run: inspect the input and return the actions an apply would take.
	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError>;
	/// Perform the actions of a plan produced by this tool.
	fn apply(
		&self,
		plan: &Plan,
		sink: &mut dyn ProgressSink,
	) -> Result<Report, ToolError>;
}

/// Every tool known to the build, in registration order.
pub fn registry() -> Vec<Box<dyn Tool>> {
	vec![
		Box::new(boko::BokoConvert),
		Box::new(calibre::CalibreConvert),
		Box::new(calibre::CalibreMeta),
		Box::new(calibre_polish::CalibrePolish),
		Box::new(cbz_covers::CbzCovers),
		Box::new(cbzit::Cbzit),
		Box::new(epub2cbz::Epub2Cbz),
		Box::new(epub_check::EpubCheck),
		Box::new(epub_polish::EpubPolish),
		Box::new(meta_edit::MetaEdit),
		Box::new(missing_sequence::MissingSequence),
		Box::new(mobi2epub::Mobi2Epub),
		Box::new(sources_import::SourcesImport),
		Box::new(webp_convert::WebpConvert),
	]
}

/// Look up a tool by [`Tool::id`].
pub fn find(id: &str) -> Option<Box<dyn Tool>> {
	registry().into_iter().find(|tool| tool.id() == id)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn registry_ids_are_unique_kebab_case_and_findable() {
		let tools = registry();
		assert!(!tools.is_empty(), "registry must not be empty");

		let mut ids = tools.iter().map(|tool| tool.id()).collect::<Vec<_>>();
		let count = ids.len();
		ids.sort_unstable();
		ids.dedup();
		assert_eq!(ids.len(), count, "duplicate tool id in registry");

		for tool in &tools {
			let id = tool.id();
			assert!(
				id.chars()
					.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
				"tool id {id:?} is not kebab-case"
			);
			assert!(
				!tool.describe().is_empty(),
				"tool {id:?} has no description"
			);
			assert!(find(id).is_some(), "tool {id:?} is not findable by id");
		}
		assert!(find("nope-not-a-tool").is_none());
	}

	#[test]
	fn options_default_when_null_and_error_when_malformed() {
		#[derive(Debug, Default, PartialEq, serde::Deserialize)]
		#[serde(default, deny_unknown_fields)]
		struct Options {
			flag: bool,
		}

		let input = ToolInput::new(vec![PathBuf::from("/tmp")]);
		assert_eq!(
			input.parse_options::<Options>().unwrap(),
			Options::default()
		);

		let input = input.with_options(serde_json::json!({ "flag": "yes" }));
		let error = input.parse_options::<Options>().unwrap_err();
		assert!(matches!(error, ToolError::Options(_)), "{error}");
	}

	#[test]
	fn apply_refuses_a_plan_from_another_tool() {
		let plan = Plan::new("other-tool");
		let error = plan.expect_tool("epub2cbz").unwrap_err();
		assert!(
			matches!(&error, ToolError::PlanMismatch { tool, plan } if tool == "epub2cbz" && plan == "other-tool"),
			"{error}"
		);
	}
}
