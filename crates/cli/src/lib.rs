//! Stump's operator CLI: the `stump` clap root plus the `account`, `system`
//! and `tools` subcommand trees. See `crates/cli/README.md` for the crate
//! contract and decisions.

#![warn(clippy::dbg_macro)]

mod commands;
mod config;
mod error;

pub use commands::{handle_command, Commands};
pub use config::CliConfig;
pub use error::CliError;

pub use clap::Parser;

/// A CLI for Stump. If no subcommand is provided, the server will be started.
#[derive(Parser)]
#[command(name = "stump")]
#[command(author, version, about, long_about = None)]
pub struct Cli {
	#[clap(flatten)]
	pub config: CliConfig,

	/// The available subcommands. If no subcommand is provided, the server will be started
	#[command(subcommand)]
	pub command: Option<Commands>,
}
