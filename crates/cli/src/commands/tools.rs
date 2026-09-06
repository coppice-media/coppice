use std::path::PathBuf;

use clap::Subcommand;
use indicatif::{ProgressBar, ProgressStyle};
use stump_tools::{Plan, ProgressSink, Report, Severity, Tool, ToolError, ToolInput};

use crate::error::{CliError, CliResult};

/// Subcommands for the library maintenance tools (`stump_tools`).
///
/// `plan` is always a dry run: it reads the given paths and prints what an
/// `apply` would do. `apply` refuses to run without `--yes`.
#[derive(Subcommand, Debug)]
pub enum Tools {
	/// List every registered tool
	List {
		/// Print JSON instead of a table
		#[clap(long)]
		json: bool,
	},
	/// Dry run a tool: print the plan without changing anything
	Plan {
		/// The tool id, e.g. `epub2cbz`
		tool: String,
		/// The paths the tool operates on
		#[clap(required = true)]
		paths: Vec<PathBuf>,
		/// Tool options as a JSON object, e.g. `{"recursive":true}`
		#[clap(long)]
		options: Option<String>,
		/// Print JSON instead of a table
		#[clap(long)]
		json: bool,
	},
	/// Apply a tool: plan, then perform the planned actions
	Apply {
		/// The tool id, e.g. `epub2cbz`
		tool: String,
		/// The paths the tool operates on
		#[clap(required = true)]
		paths: Vec<PathBuf>,
		/// Tool options as a JSON object, e.g. `{"recursive":true}`
		#[clap(long)]
		options: Option<String>,
		/// Confirm the plan should be applied. Required: without it nothing runs
		#[clap(long)]
		yes: bool,
		/// Print JSON instead of a table
		#[clap(long)]
		json: bool,
	},
}

pub fn handle_tools_command(command: Tools) -> CliResult<()> {
	match command {
		Tools::List { json } => list(json),
		Tools::Plan {
			tool,
			paths,
			options,
			json,
		} => plan(&tool, paths, options.as_deref(), json),
		Tools::Apply {
			tool,
			paths,
			options,
			yes,
			json,
		} => apply(&tool, paths, options.as_deref(), yes, json),
	}
}

fn list(json: bool) -> CliResult<()> {
	let tools = stump_tools::registry();

	if json {
		let entries = tools
			.iter()
			.map(
				|tool| serde_json::json!({ "id": tool.id(), "describe": tool.describe() }),
			)
			.collect::<Vec<_>>();
		println!("{}", to_json(&serde_json::Value::Array(entries))?);
		return Ok(());
	}

	let mut table = prettytable::Table::new();
	table.add_row(prettytable::row!["Tool", "Description"]);
	for tool in &tools {
		table.add_row(prettytable::row![tool.id(), tool.describe()]);
	}
	table.printstd();

	Ok(())
}

fn plan(
	id: &str,
	paths: Vec<PathBuf>,
	options: Option<&str>,
	json: bool,
) -> CliResult<()> {
	let tool = find(id)?;
	let plan = tool.plan(&input(paths, options)?)?;

	if json {
		println!("{}", to_json(&plan)?);
	} else {
		print_plan(&plan);
		println!(
			"\nDry run only. Re-run with `apply ... --yes` to perform {} action(s).",
			plan.actions.len()
		);
	}

	Ok(())
}

fn apply(
	id: &str,
	paths: Vec<PathBuf>,
	options: Option<&str>,
	yes: bool,
	json: bool,
) -> CliResult<()> {
	let tool = find(id)?;
	let plan = tool.plan(&input(paths, options)?)?;

	if !yes {
		if json {
			println!("{}", to_json(&plan)?);
		} else {
			print_plan(&plan);
		}
		return Err(CliError::OperationFailed(format!(
			"refusing to apply {} action(s) without --yes",
			plan.actions.len()
		)));
	}

	let mut sink = BarSink::new(plan.actions.len(), json);
	let report = tool.apply(&plan, &mut sink)?;
	sink.finish();

	if json {
		println!("{}", to_json(&report)?);
	} else {
		print_report(&report);
	}

	Ok(())
}

fn find(id: &str) -> CliResult<Box<dyn Tool>> {
	stump_tools::find(id).ok_or_else(|| {
		let known = stump_tools::registry()
			.iter()
			.map(|tool| tool.id())
			.collect::<Vec<_>>()
			.join(", ");
		CliError::Tool(ToolError::UnknownTool(format!(
			"{id}; known tools: {known}"
		)))
	})
}

fn input(paths: Vec<PathBuf>, options: Option<&str>) -> CliResult<ToolInput> {
	let options = match options {
		Some(raw) => serde_json::from_str(raw).map_err(|error| {
			CliError::OperationFailed(format!("--options is not valid JSON: {error}"))
		})?,
		None => serde_json::Value::Null,
	};

	Ok(ToolInput::new(paths).with_options(options))
}

fn to_json<T: serde::Serialize>(value: &T) -> CliResult<String> {
	serde_json::to_string_pretty(value)
		.map_err(|error| CliError::OperationFailed(error.to_string()))
}

fn print_plan(plan: &Plan) {
	if plan.actions.is_empty() {
		println!("{}: nothing to do", plan.tool);
	} else {
		let mut table = prettytable::Table::new();
		table.add_row(prettytable::row![
			"#", "Action", "Source", "Target", "Detail"
		]);
		for (index, action) in plan.actions.iter().enumerate() {
			table.add_row(prettytable::row![
				index + 1,
				action.kind,
				display(action.source.as_deref()),
				display(action.target.as_deref()),
				summarize(&action.detail)
			]);
		}
		table.printstd();
	}

	print_warnings(&plan.warnings);
}

fn print_report(report: &Report) {
	println!("Applied {} action(s)", report.applied.len());
	if !report.skipped.is_empty() {
		let mut table = prettytable::Table::new();
		table.add_row(prettytable::row!["Skipped", "Target", "Reason"]);
		for (action, reason) in &report.skipped {
			table.add_row(prettytable::row![
				action.kind,
				display(action.target.as_deref()),
				reason
			]);
		}
		table.printstd();
	}

	print_warnings(&report.warnings);
}

fn print_warnings(warnings: &[stump_tools::Warning]) {
	if warnings.is_empty() {
		return;
	}

	let mut table = prettytable::Table::new();
	table.add_row(prettytable::row!["Severity", "Code", "Path", "Message"]);
	for warning in warnings {
		table.add_row(prettytable::row![
			warning.severity.as_str(),
			warning.code,
			display(warning.path.as_deref()),
			warning.message
		]);
	}
	table.printstd();

	let errors = warnings
		.iter()
		.filter(|warning| warning.severity == Severity::Error)
		.count();
	if errors > 0 {
		println!("{errors} warning(s) at error severity");
	}
}

fn display(path: Option<&std::path::Path>) -> String {
	path.map(|path| path.display().to_string())
		.unwrap_or_default()
}

/// Flatten the scalar members of an action's detail into `k=v` pairs. Nested
/// arrays and objects (such as the full page list) are only shown by `--json`.
fn summarize(detail: &serde_json::Value) -> String {
	let Some(object) = detail.as_object() else {
		return String::new();
	};

	object
		.iter()
		.filter_map(|(key, value)| match value {
			serde_json::Value::Null
			| serde_json::Value::Array(_)
			| serde_json::Value::Object(_) => None,
			serde_json::Value::String(value) => Some(format!("{key}={value}")),
			value => Some(format!("{key}={value}")),
		})
		.collect::<Vec<_>>()
		.join(" ")
}

/// A [`ProgressSink`] backed by an `indicatif` bar; hidden for `--json` so the
/// only stdout output stays parseable.
struct BarSink {
	bar: ProgressBar,
}

impl BarSink {
	fn new(total: usize, quiet: bool) -> Self {
		if quiet {
			return Self {
				bar: ProgressBar::hidden(),
			};
		}

		let bar = ProgressBar::new(total as u64);
		bar.set_style(
			ProgressStyle::with_template("{spinner} [{pos}/{len}] {msg}")
				.unwrap_or_else(|_| ProgressStyle::default_bar()),
		);

		Self { bar }
	}

	fn finish(&self) {
		self.bar.finish_and_clear();
	}
}

impl ProgressSink for BarSink {
	fn progress(&mut self, done: usize, total: usize, message: &str) {
		self.bar.set_length(total as u64);
		self.bar.set_position(done as u64);
		self.bar.set_message(message.to_string());
	}
}
