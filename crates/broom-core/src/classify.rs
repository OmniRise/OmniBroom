use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::facts::Fact;
use crate::walk::WalkEntry;

/// Environment a path-matching rule can see: whether the path sits inside an
/// active git worktree and where that worktree starts. Built by the
/// classifier on demand so entries no rule claims cost zero stats.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PathContext {
    /// The path sits inside an active git worktree (any ancestor has .git).
    pub inside_git_worktree: bool,
    /// Absolute path of the worktree root if inside one.
    pub git_root: Option<PathBuf>,
}

/// One rule's verdict for one target, ready to become a finding or an answer
/// from `why`. Carries the reason verbatim from the rule pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleDecision {
    pub rule_id: String,
    pub category: String,
    pub verdict: crate::Verdict,
    pub reason: String,
}

/// The bridge between engine and rules. broom-rules implements this over TOML
/// packs; the engine only ever sees this trait, so rule logic never hardcodes
/// into the core. Built once per run, then read from many walker threads, so
/// implementations must be cheap to borrow immutably.
///
/// `classify_path` takes a lazy context getter instead of a context value:
/// rules that do not need the git context (everything except worktree
/// conditions) must not pay an ancestor walk per entry, and entries no rule
/// claims must not pay anything at all.
pub trait Classifier: Send + Sync {
    /// Classify a walked entry against every filesystem rule. Pure pattern
    /// matching over precompiled globs: no I/O unless a matched rule carries
    /// a condition and calls `ctx`.
    fn classify_path(&self, entry: &WalkEntry, ctx: &dyn Fn() -> PathContext) -> Vec<RuleDecision>;

    /// Build a PathContext for `path` by checking ancestors for .git. Callers
    /// memoize this per entry; `why` calls it exactly once.
    fn context_for(&self, path: &Path) -> PathContext;

    /// Classify a system fact (docker, journalctl, git) against fact rules.
    fn classify_fact(&self, fact: &Fact) -> Vec<RuleDecision>;
}
