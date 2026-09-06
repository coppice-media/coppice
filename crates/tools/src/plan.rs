//! The plan/report value types shared by every tool.
//!
//! A [`Plan`] is the *dry run*: it is pure data, safe to print, and describes
//! exactly what an `apply` would do. A [`Report`] is what actually happened.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How bad a [`Warning`] is. Tools decide; the CLI only renders it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
	/// Worth telling the user, nothing is wrong.
	Info,
	/// Something is ambiguous or was guessed; the action still runs.
	#[default]
	Warn,
	/// The input is broken; the related action was dropped from the plan.
	Error,
}

impl Severity {
	pub fn as_str(&self) -> &'static str {
		match self {
			Severity::Info => "info",
			Severity::Warn => "warn",
			Severity::Error => "error",
		}
	}
}

/// One unit of work a tool intends to perform (plan) or performed (report).
///
/// `kind` is a tool-defined, kebab-case verb (`"write-cbz"`, `"rename"`, ...).
/// `detail` carries whatever the tool wants to show or replay; it must be a
/// JSON object or null so the CLI can flatten it into a table cell.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Action {
	pub kind: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub source: Option<PathBuf>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub target: Option<PathBuf>,
	#[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
	pub detail: serde_json::Value,
}

impl Action {
	pub fn new(kind: impl Into<String>) -> Self {
		Self {
			kind: kind.into(),
			source: None,
			target: None,
			detail: serde_json::Value::Null,
		}
	}

	pub fn with_source(mut self, source: impl AsRef<Path>) -> Self {
		self.source = Some(source.as_ref().to_path_buf());
		self
	}

	pub fn with_target(mut self, target: impl AsRef<Path>) -> Self {
		self.target = Some(target.as_ref().to_path_buf());
		self
	}

	pub fn with_detail(mut self, detail: serde_json::Value) -> Self {
		self.detail = detail;
		self
	}
}

/// A non-fatal finding attached to a plan or report.
///
/// `code` is a stable, kebab-case identifier so callers can filter without
/// matching on prose. `fixable` means *this tool* can repair it (an `apply`
/// will), which lets a check-only tool report findings and repairs in one pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Warning {
	pub code: String,
	pub message: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub path: Option<PathBuf>,
	#[serde(default)]
	pub severity: Severity,
	#[serde(default, skip_serializing_if = "is_false")]
	pub fixable: bool,
}

fn is_false(value: &bool) -> bool {
	!*value
}

impl Warning {
	/// A `Severity::Warn`, non-fixable finding.
	pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
		Self {
			code: code.into(),
			message: message.into(),
			path: None,
			severity: Severity::Warn,
			fixable: false,
		}
	}

	pub fn at(mut self, path: impl AsRef<Path>) -> Self {
		self.path = Some(path.as_ref().to_path_buf());
		self
	}

	pub fn with_severity(mut self, severity: Severity) -> Self {
		self.severity = severity;
		self
	}

	pub fn fixable(mut self) -> Self {
		self.fixable = true;
		self
	}
}

/// The dry run of a tool: what it would do, and what it noticed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Plan {
	pub tool: String,
	pub actions: Vec<Action>,
	pub warnings: Vec<Warning>,
}

impl Plan {
	pub fn new(tool: impl Into<String>) -> Self {
		Self {
			tool: tool.into(),
			actions: Vec::new(),
			warnings: Vec::new(),
		}
	}

	pub fn push(&mut self, action: Action) {
		self.actions.push(action);
	}

	pub fn warn(&mut self, warning: Warning) {
		self.warnings.push(warning);
	}

	pub fn is_empty(&self) -> bool {
		self.actions.is_empty()
	}

	/// Guard used at the top of [`crate::Tool::apply`] so a plan from tool A
	/// can never be applied by tool B.
	pub fn expect_tool(&self, tool: &str) -> Result<(), crate::ToolError> {
		if self.tool == tool {
			Ok(())
		} else {
			Err(crate::ToolError::PlanMismatch {
				tool: tool.to_string(),
				plan: self.tool.clone(),
			})
		}
	}
}

/// What an `apply` actually did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
	pub applied: Vec<Action>,
	/// Planned actions that were not performed, each with the reason why.
	pub skipped: Vec<(Action, String)>,
	pub warnings: Vec<Warning>,
}

impl Report {
	/// A report that inherits the plan's warnings and nothing else.
	pub fn for_plan(plan: &Plan) -> Self {
		Self {
			applied: Vec::new(),
			skipped: Vec::new(),
			warnings: plan.warnings.clone(),
		}
	}

	pub fn applied(&mut self, action: Action) {
		self.applied.push(action);
	}

	pub fn skipped(&mut self, action: Action, reason: impl Into<String>) {
		self.skipped.push((action, reason.into()));
	}

	pub fn warn(&mut self, warning: Warning) {
		self.warnings.push(warning);
	}
}
