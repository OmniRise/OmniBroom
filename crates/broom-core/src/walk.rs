use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use jwalk::WalkDir;
use rayon::prelude::*;

/// One entry the walker saw, with its resolved size for files.
#[derive(Debug, Clone)]
pub struct WalkEntry {
    pub path: PathBuf,
    pub is_dir: bool,
    pub depth: usize,
    pub size_bytes: Option<u64>,
}

/// What a walk produced: entries, non-fatal warnings (permission corners,
/// broken symlinks), and how many entries were visited for progress and
/// reporting. Walk errors never abort the scan; a full disk with unreadable
/// corners is exactly the machine this tool exists for.
#[derive(Debug, Default)]
pub struct WalkOutcome {
    pub entries: Vec<WalkEntry>,
    pub warnings: Vec<String>,
    pub visited: u64,
}

/// Progress callback the surface can wire to indicatif. Called from the walk
/// with a running count of visited entries. Must be cheap: this runs hot.
pub trait ProgressSink: Send + Sync {
    fn entries_visited(&self, count: u64);
}

/// Serializes progress updates from walker threads through one atomic counter.
pub struct AtomicProgress {
    counter: AtomicU64,
    sink: std::sync::Arc<dyn ProgressSink>,
}

impl AtomicProgress {
    pub fn new(sink: std::sync::Arc<dyn ProgressSink>) -> Self {
        Self {
            counter: AtomicU64::new(0),
            sink,
        }
    }

    pub fn tick(&self) {
        let count = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        if count % 4096 == 0 {
            self.sink.entries_visited(count);
        }
    }

    pub fn total(&self) -> u64 {
        self.counter.load(Ordering::Relaxed)
    }
}

/// Knobs for the parallel walker. Defaults are the sane ones for a scan:
/// unlimited depth, no symlink following (loops hang walkers, and a swept
/// symlink target is a surprise).
#[derive(Debug, Clone, Default)]
pub struct WalkConfig {
    /// Stop descending below this depth. Depth 0 is the root itself.
    pub max_depth: Option<usize>,
    /// Follow symlinks instead of skipping them.
    pub follow_symlinks: bool,
}

/// How many individual walk errors get a warning line before the rest are
/// folded into a count. Keeps a hostile filesystem from flooding the output.
const MAX_WALK_WARNINGS: usize = 10;

/// Walk `roots` in parallel (jwalk is rayon-backed) and collect every entry
/// with its file size. Directories are reported without sizes: the engine
/// sums matched subtrees on demand so an untouched million-file tree costs
/// nothing extra.
///
/// Unreadable entries are collected as warnings, never as errors: the scan
/// must survive a full disk with permission corners.
pub fn walk(
    roots: &[PathBuf],
    config: &WalkConfig,
    progress: Option<&AtomicProgress>,
) -> WalkOutcome {
    let mut outcome = WalkOutcome::default();
    let mut hidden_errors: usize = 0;
    for root in roots {
        let walker = WalkDir::new(root)
            .follow_links(config.follow_symlinks)
            .max_depth(config.max_depth.unwrap_or(usize::MAX))
            // Hidden entries ON: the caches this tool judges (.npm, .cache)
            // are exactly the dotfiles jwalk skips by default.
            .skip_hidden(false)
            // Stable per-directory order so output and snapshots do not jitter.
            .sort(true);
        for result in walker {
            match result {
                Ok(entry) => {
                    if let Some(progress) = progress {
                        progress.tick();
                    }
                    let depth = entry.depth;
                    let path = entry.path();
                    let is_dir = entry.file_type().is_dir();
                    let size_bytes = if is_dir {
                        None
                    } else {
                        entry.metadata().ok().map(|m| m.len())
                    };
                    outcome.entries.push(WalkEntry {
                        path,
                        is_dir,
                        depth,
                        size_bytes,
                    });
                }
                Err(err) => {
                    if outcome.warnings.len() < MAX_WALK_WARNINGS {
                        outcome.warnings.push(format!("walk error: {err}"));
                    } else {
                        hidden_errors += 1;
                    }
                }
            }
        }
    }
    if hidden_errors > 0 {
        outcome.warnings.push(format!(
            "and {hidden_errors} more path(s) could not be read (permission or I/O errors)"
        ));
    }
    outcome.visited = progress
        .map(|p| p.total())
        .unwrap_or(outcome.entries.len() as u64);
    outcome
}

/// Sum the sizes of all regular files below `dir`, in parallel. Returns 0 for
/// a missing or unreadable directory: size is a display concern, a failure to
/// measure must not fail the scan.
pub fn dir_size(dir: &Path) -> u64 {
    if !dir.is_dir() {
        return std::fs::symlink_metadata(dir).map(|m| m.len()).unwrap_or(0);
    }
    WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .par_bridge()
        .map(|entry| entry.metadata().map(|m| m.len()).unwrap_or(0))
        .sum()
}
