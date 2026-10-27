use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "omnibroom",
    version,
    about = "OmniBroom: know what is safe to delete, what is risky, and what to never touch."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Classify findings under paths: size, category, verdict, reason.
    Scan(ScanArgs),
    /// Delete safe findings. Dry run by default; real deletion needs --yes.
    Sweep(SweepArgs),
    /// Explain the verdict for one path: which rules claim it, and why.
    Why {
        /// The path to explain.
        path: PathBuf,
    },
}

#[derive(Debug, Args, Clone)]
pub struct ScanArgs {
    /// Paths to scan. Defaults to the home directory.
    #[arg(value_name = "PATH")]
    pub paths: Vec<PathBuf>,

    /// Emit findings as JSON instead of a table.
    #[arg(long)]
    pub json: bool,

    /// Skip system facts providers (docker, journalctl, git).
    #[arg(long)]
    pub no_facts: bool,

    /// Maximum directory depth to walk. Default: no limit.
    #[arg(long, value_name = "N")]
    pub max_depth: Option<usize>,
}

#[derive(Debug, Args, Clone)]
pub struct SweepArgs {
    /// Paths to sweep. Defaults to the home directory.
    #[arg(value_name = "PATH")]
    pub paths: Vec<PathBuf>,

    /// Actually delete. Without it, sweep is a dry run that deletes nothing.
    #[arg(long)]
    pub yes: bool,

    /// Skip system facts providers (docker, journalctl, git).
    #[arg(long)]
    pub no_facts: bool,

    /// Maximum directory depth to walk. Default: no limit.
    #[arg(long, value_name = "N")]
    pub max_depth: Option<usize>,
}
