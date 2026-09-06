//! Throwaway smoke runner for `sources-import` (deleted before hand-off).
//!
//! `cargo run -p stump_tools --example sources_import_run -- <checkout> <out> [apply] [theme,…]`

use std::path::PathBuf;

use stump_tools::{sources_import::SourcesImport, NoopProgress, Tool, ToolInput};

fn main() {
	let args = std::env::args().skip(1).collect::<Vec<_>>();
	let checkout = args.first().expect("checkout path");
	let output = args.get(1).expect("output path");
	let apply = args.iter().any(|arg| arg == "apply");
	let themes = args
		.iter()
		.find(|arg| arg.contains(','))
		.map(|arg| arg.split(',').map(str::to_string).collect::<Vec<_>>());

	let mut options = serde_json::json!({
		"extensions_source": checkout,
		"output_dir": output,
	});
	if let Some(themes) = themes {
		options["themes"] = serde_json::json!(themes);
	}

	let input = ToolInput::new(vec![PathBuf::from(checkout)]).with_options(options);
	let plan = SourcesImport.plan(&input).expect("plan");

	for warning in &plan.warnings {
		println!("warning [{}] {}", warning.code, warning.message);
	}
	let stats = plan
		.actions
		.iter()
		.find(|action| action.kind == "report-stats")
		.expect("stats action");
	println!("{}", stats.detail["table"].as_str().unwrap_or_default());
	println!(
		"extensions={} derived={} unsupported={} filtered={} commit={}",
		stats.detail["extensions"],
		stats.detail["derived"],
		stats.detail["unsupported"],
		stats.detail["filtered"],
		stats.detail["commit"],
	);

	let unsupported = plan
		.actions
		.iter()
		.find(|action| action.kind == "write-unsupported")
		.expect("unsupported action");
	let mut by_code = std::collections::BTreeMap::<String, usize>::new();
	for row in unsupported.detail["rows"].as_array().unwrap_or(&Vec::new()) {
		*by_code
			.entry(row["code"].as_str().unwrap_or_default().to_string())
			.or_default() += 1;
	}
	println!("\nunsupported by code:");
	for (code, count) in &by_code {
		println!("  {count:>5}  {code}");
	}

	if let Some(id) = args.iter().find(|arg| arg.starts_with("show=")) {
		let id = id.trim_start_matches("show=");
		for action in &plan.actions {
			if action.kind == "write-definition" && action.detail["id"] == *id {
				println!("\n{}", serde_json::to_string_pretty(&action.detail).unwrap());
			}
		}
		for row in unsupported.detail["rows"].as_array().unwrap_or(&Vec::new()) {
			if row["id"] == *id {
				println!("\nUNSUPPORTED {}", serde_json::to_string_pretty(row).unwrap());
			}
		}
	}

	if apply {
		let report = SourcesImport
			.apply(&plan, &mut NoopProgress)
			.expect("apply");
		println!(
			"\napplied={} skipped={}",
			report.applied.len(),
			report.skipped.len()
		);
	}
}
