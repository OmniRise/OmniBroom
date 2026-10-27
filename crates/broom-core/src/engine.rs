use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use serde::Serialize;

use crate::classify::{Classifier, PathContext};
use crate::error::CoreError;
use crate::facts::{FactsContext, FactsProvider};
use crate::finding::{Finding, FindingOrigin};
use crate::verdict::Verdict;
use crate::walk::{AtomicProgress, ProgressSink, WalkConfig, WalkEntry, WalkOutcome, dir_size};

/// Roll-up numbers shown at the bottom of scan output and in JSON summaries.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Summary {
    pub findings: usize,
    pub safe: usize,
    pub risky: usize,
    pub never: usize,
    pub reclaimable_bytes: u64,
    pub never_bytes: u64,
}

/// Everything one scan produced. Warnings are non-fatal: a down docker daemon
/// or an unreadable directory is reported, not thrown.
#[derive(Debug, Serialize)]
pub struct ScanReport {
    pub roots: Vec<PathBuf>,
    pub findings: Vec<Finding>,
    pub warnings: Vec<String>,
    pub entries_visited: u64,
    pub summary: Summary,
}

/// One rule decision attached to a path, the `why` answer shape.
#[derive(Debug, Clone, Serialize)]
pub struct WhyMatch {
    pub rule_id: String,
    pub category: String,
    pub verdict: Verdict,
    pub reason: String,
    /// "filesystem" for path rules, "facts:<provider>" for fact rules.
    pub source: String,
}

/// The full explanation for a single path: every rule that fires, the path
/// context, and any facts attached to it. Empty `matches` means no rule has
/// an opinion, which the CLI reports as unclassified.
#[derive(Debug, Serialize)]
pub struct WhyReport {
    pub path: PathBuf,
    pub exists: bool,
    pub context: PathContext,
    pub matches: Vec<WhyMatch>,
    pub warnings: Vec<String>,
}

/// The engine: walks, classifies, gathers facts. Pure orchestration. Every
/// rule-shaped decision comes from the injected [`Classifier`], every system
/// observation from injected [`FactsProvider`]s. This crate never runs a
/// subprocess and never hardcodes a rule.
pub struct Engine {
    classifier: Arc<dyn Classifier>,
    providers: Vec<Arc<dyn FactsProvider>>,
    config: WalkConfig,
}

impl Engine {
    pub fn new(
        classifier: Arc<dyn Classifier>,
        providers: Vec<Arc<dyn FactsProvider>>,
        config: WalkConfig,
    ) -> Self {
        Self {
            classifier,
            providers,
            config,
        }
    }

    /// Scan `roots` and return classified findings sorted by size, biggest
    /// first. Directory sizes are summed lazily and in parallel, only for
    /// subtrees that matched a rule.
    pub fn scan(
        &self,
        roots: &[PathBuf],
        progress: Option<Arc<dyn ProgressSink>>,
    ) -> Result<ScanReport, CoreError> {
        let progress = progress.map(AtomicProgress::new);
        let WalkOutcome {
            entries,
            mut warnings,
            visited,
        } = crate::walk::walk(roots, &self.config, progress.as_ref());

        // Phase 1: classify in parallel. Path rules are pure pattern matching
        // over precompiled globs, so this scales with rayon. The context
        // closure runs at most once per entry, and only if a rule that
        // matched actually carries a condition.
        let candidates: Vec<(WalkEntry, Vec<crate::classify::RuleDecision>)> = entries
            .par_iter()
            .filter_map(|entry| {
                let ctx_fn = || self.classifier.context_for(&entry.path);
                let decisions = self.classifier.classify_path(entry, &ctx_fn);
                if decisions.is_empty() {
                    None
                } else {
                    Some((entry.clone(), decisions))
                }
            })
            .collect();

        // Phase 2: resolve sizes for matched directories, in parallel.
        let mut findings: Vec<Finding> = candidates
            .into_iter()
            .map(|(entry, decisions)| {
                let size_bytes = if entry.is_dir {
                    Some(dir_size(&entry.path))
                } else {
                    entry.size_bytes
                };
                let decision = pick_decision(decisions);
                Finding {
                    path: entry.path,
                    verdict: decision.verdict,
                    category: decision.category,
                    reason: decision.reason,
                    rule_id: decision.rule_id,
                    size_bytes,
                    origin: FindingOrigin::Filesystem,
                }
            })
            .collect();

        // Phase 3: facts from injected providers. Warnings are collected, never
        // fatal: docker being down must not hide the node findings.
        let repo_roots = collect_repo_roots(&entries);
        let facts_ctx = FactsContext { repo_roots };
        for provider in &self.providers {
            let outcome = provider.collect(&facts_ctx);
            warnings.extend(outcome.warnings);
            for fact in outcome.facts {
                for decision in self.classifier.classify_fact(&fact) {
                    findings.push(Finding {
                        path: fact.path.clone(),
                        verdict: decision.verdict,
                        category: decision.category,
                        reason: decision.reason,
                        rule_id: decision.rule_id,
                        size_bytes: fact.size_bytes,
                        origin: FindingOrigin::Facts(provider.name().to_string()),
                    });
                }
            }
        }

        // Collapse duplicates and containment across origins: the same path
        // reported by a path rule and a fact shows once (most severe verdict
        // wins), and a fact inside a swept directory is covered by the parent.
        collapse_findings(&mut findings);

        // Biggest first, then path for a stable order. Stability is what makes
        // snapshot tests and human diffing possible.
        findings.sort_by(|a, b| {
            b.size_bytes
                .unwrap_or(0)
                .cmp(&a.size_bytes.unwrap_or(0))
                .then_with(|| a.path.cmp(&b.path))
        });

        let summary = summarize(&findings);
        Ok(ScanReport {
            roots: roots.to_vec(),
            findings,
            warnings,
            entries_visited: visited,
            summary,
        })
    }

    /// Explain one path: every rule that claims it, plus context. Does not
    /// need a full walk; `why` on a path costs an ancestor check, not a scan.
    pub fn why(&self, path: &Path) -> WhyReport {
        let exists = path.symlink_metadata().is_ok();
        let context = self.classifier.context_for(path);

        let mut matches = Vec::new();
        if exists {
            let entry = WalkEntry {
                path: path.to_path_buf(),
                is_dir: path.is_dir(),
                depth: 0,
                size_bytes: None,
            };
            let ctx = context.clone();
            for decision in self.classifier.classify_path(&entry, &|| ctx.clone()) {
                matches.push(WhyMatch {
                    rule_id: decision.rule_id,
                    category: decision.category,
                    verdict: decision.verdict,
                    reason: decision.reason,
                    source: "filesystem".to_string(),
                });
            }
        }

        let mut warnings = Vec::new();
        let repo_roots = context.git_root.iter().cloned().collect();
        let facts_ctx = FactsContext { repo_roots };
        for provider in &self.providers {
            let outcome = provider.collect(&facts_ctx);
            warnings.extend(outcome.warnings);
            for fact in outcome.facts {
                if fact.path == path {
                    for decision in self.classifier.classify_fact(&fact) {
                        matches.push(WhyMatch {
                            rule_id: decision.rule_id,
                            category: decision.category,
                            verdict: decision.verdict,
                            reason: decision.reason,
                            source: format!("facts:{}", provider.name()),
                        });
                    }
                }
            }
        }

        matches.sort_by(|a, b| {
            a.verdict
                .cmp(&b.verdict)
                .then_with(|| a.rule_id.cmp(&b.rule_id))
        });
        matches.dedup_by(|a, b| a.rule_id == b.rule_id);

        WhyReport {
            path: path.to_path_buf(),
            exists,
            context,
            matches,
            warnings,
        }
    }
}

/// Several rules can claim one path (`why` shows all of them). For a scan
/// finding, the most severe verdict wins: if any rule says never, the path is
/// never. That is the safe direction to collapse ambiguity.
fn pick_decision(
    mut decisions: Vec<crate::classify::RuleDecision>,
) -> crate::classify::RuleDecision {
    decisions.sort_by_key(|d| d.verdict);
    decisions
        .pop()
        .expect("classify_path returned decisions, then none: contract violated")
}

/// One finding per path, outermost wins. Two passes:
///
/// 1. Same-path duplicates (filesystem vs fact origin) collapse to the most
///    severe verdict, preferring the filesystem origin on ties because it
///    carries a measured size.
/// 2. Containment: a finding inside another finding's directory is covered by
///    the parent, so sweeping the parent sweeps the child. Collapsing keeps
///    summary bytes honest instead of counting them twice.
fn collapse_findings(findings: &mut Vec<Finding>) {
    findings.sort_by(|a, b| {
        let depth_a = a.path.components().count();
        let depth_b = b.path.components().count();
        depth_a
            .cmp(&depth_b)
            // Most severe first within the same depth, filesystem before
            // facts on equal severity.
            .then_with(|| b.verdict.cmp(&a.verdict))
            .then_with(|| origin_rank(a).cmp(&origin_rank(b)))
            .then_with(|| a.path.cmp(&b.path))
    });
    let mut kept: Vec<Finding> = Vec::with_capacity(findings.len());
    for finding in findings.drain(..) {
        let covered = kept
            .iter()
            .any(|outer| finding.path.starts_with(&outer.path));
        if !covered {
            kept.push(finding);
        }
    }
    *findings = kept;
}

fn origin_rank(finding: &Finding) -> u8 {
    match finding.origin {
        FindingOrigin::Filesystem => 0,
        FindingOrigin::Facts(_) => 1,
    }
}

fn collect_repo_roots(entries: &[WalkEntry]) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = entries
        .iter()
        .filter(|e| e.is_dir && e.path.file_name().is_some_and(|n| n == ".git"))
        .filter_map(|e| e.path.parent().map(Path::to_path_buf))
        .collect();
    roots.sort();
    roots.dedup();
    roots
}

fn summarize(findings: &[Finding]) -> Summary {
    let mut summary = Summary {
        findings: findings.len(),
        ..Summary::default()
    };
    for finding in findings {
        match finding.verdict {
            Verdict::Safe => {
                summary.safe += 1;
                summary.reclaimable_bytes += finding.size_bytes.unwrap_or(0);
            }
            Verdict::Risky => summary.risky += 1,
            Verdict::Never => {
                summary.never += 1;
                summary.never_bytes += finding.size_bytes.unwrap_or(0);
            }
        }
    }
    summary
}
