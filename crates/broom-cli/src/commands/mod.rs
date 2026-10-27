//! Command implementations: scan, sweep, why. Shared plumbing (roots,
//! engine construction, progress) lives here.

pub mod scan;
pub mod sweep;
pub mod why;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use broom_core::{Engine, FactsProvider, ProgressSink, WalkConfig};
use broom_rules::builtin_registry;
use indicatif::{ProgressBar, ProgressStyle};

use crate::args::{ScanArgs, SweepArgs};
use crate::facts;

/// Default scan/sweep root: the user's home directory. Home is where the
/// caches this tool exists to judge actually live.
pub fn resolve_roots(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    if !paths.is_empty() {
        return Ok(paths.to_vec());
    }
    let home =
        dirs::home_dir().context("could not determine home directory; pass explicit paths")?;
    Ok(vec![home])
}

/// Build the engine: built-in rule packs plus injected facts providers.
/// `no_facts` (used by tests and `--no-facts`) swaps providers for an empty
/// set: the engine only ever sees the trait.
pub fn build_engine(no_facts: bool, max_depth: Option<usize>) -> Result<Engine> {
    let registry = builtin_registry().context("built-in rule packs failed to compile")?;
    let providers: Vec<Arc<dyn FactsProvider>> = if no_facts {
        Vec::new()
    } else {
        facts::default_providers()
    };
    Ok(Engine::new(
        registry,
        providers,
        WalkConfig {
            max_depth,
            follow_symlinks: false,
        },
    ))
}

/// Progress bar wired into the walker. Hidden automatically when stderr is
/// not a TTY (indicatif default), so tests and pipes stay clean.
pub struct WalkProgress {
    bar: ProgressBar,
}

impl WalkProgress {
    pub fn start(label: &str) -> Self {
        let bar = ProgressBar::new_spinner();
        let style = ProgressStyle::with_template("{spinner} {msg} {pos}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner());
        bar.set_style(style);
        bar.set_message(format!("{label}..."));
        bar.enable_steady_tick(std::time::Duration::from_millis(80));
        Self { bar }
    }

    pub fn finish(&self) {
        self.bar.finish_and_clear();
    }

    pub fn arc(self: Arc<Self>) -> Arc<dyn ProgressSink> {
        self
    }
}

impl ProgressSink for WalkProgress {
    fn entries_visited(&self, count: u64) {
        self.bar.set_position(count);
    }
}

/// From scan args, run a scan and return the report (shared by scan and
/// sweep so both classify identically).
pub fn scan_with_args(args: &ScanArgs) -> Result<broom_core::ScanReport> {
    let roots = resolve_roots(&args.paths)?;
    let engine = build_engine(args.no_facts, args.max_depth)?;
    let progress = Arc::new(WalkProgress::start("scanning"));
    let report = engine.scan(&roots, Some(progress.clone().arc()))?;
    progress.finish();
    Ok(report)
}

/// Same for sweep args (no --json on sweep in v1).
pub fn scan_for_sweep(args: &SweepArgs) -> Result<broom_core::ScanReport> {
    let roots = resolve_roots(&args.paths)?;
    let engine = build_engine(args.no_facts, args.max_depth)?;
    let progress = Arc::new(WalkProgress::start("scanning"));
    let report = engine.scan(&roots, Some(progress.clone().arc()))?;
    progress.finish();
    Ok(report)
}
