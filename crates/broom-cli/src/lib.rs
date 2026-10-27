//! The OmniBroom CLI surface: argument parsing, output rendering, and the
//! facts providers that shell out (docker, journalctl, git). The engine never
//! runs a subprocess; everything OS-specific lives here, behind the traits
//! broom-core defines.

pub mod args;
pub mod commands;
pub mod facts;
pub mod output;

use anyhow::Result;
use clap::Parser;

/// Entry point shared by the binary and integration tests.
pub fn run() -> Result<()> {
    let cli = args::Cli::parse();
    match cli.command {
        args::Command::Broom { command } => match command {
            args::BroomCommand::Scan(scan) => commands::scan::run(scan),
            args::BroomCommand::Sweep(sweep) => commands::sweep::run(sweep),
            args::BroomCommand::Why { path } => commands::why::run(path),
        },
    }
}
