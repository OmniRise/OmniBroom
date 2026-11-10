use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::output::{GroupKey, SortKey};

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

    /// Show only the N rows that sort first (default: 25).
    #[arg(long, value_name = "N")]
    pub top: Option<usize>,

    /// Show every finding, overriding --top.
    #[arg(long)]
    pub all: bool,

    /// Table sort order. Default: size, biggest first.
    #[arg(long, value_enum, default_value_t = SortKey::Size)]
    pub sort: SortKey,

    /// Group rows into labelled sections.
    #[arg(long, value_enum)]
    pub group_by: Option<GroupKey>,
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

    /// Show only the N rows that sort first (default: 25).
    #[arg(long, value_name = "N")]
    pub top: Option<usize>,

    /// Show every finding, overriding --top.
    #[arg(long)]
    pub all: bool,
}
