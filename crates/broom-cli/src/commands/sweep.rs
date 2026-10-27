//! `omni broom sweep`: dry run by default, real deletion only with `--yes`.
//!
//! Deletion policy, enforced here and nowhere else in the codebase:
//! - Only `Verdict::Safe` filesystem findings are ever deleted.
//! - `Risky` and `Never` findings are never deleted, `--yes` or not.
//! - Facts findings (docker images, buildx cache, volumes, worktrees) are
//!   never deleted by this v1: deleting those needs provider-specific
//!   commands with their own safety rails, which is future work. The dry run
//!   says so explicitly instead of pretending they are covered.

use std::path::Path;

use anyhow::Result;

use broom_core::{Finding, FindingOrigin, ScanReport, Verdict};

use crate::args::SweepArgs;
use crate::output;

/// What a sweep would do, split by policy.
#[derive(Debug, Default)]
pub struct SweepPlan {
    /// Safe filesystem findings: the only things ever deleted.
    pub deletable: Vec<Finding>,
    /// Safe findings that need a provider command (docker etc.): reported,
    /// not deleted.
    pub provider_managed: Vec<Finding>,
    /// Risky findings: shown, never touched.
    pub risky: usize,
    /// Never findings: shown, never touched.
    pub never: usize,
    pub risky_bytes: u64,
    pub never_bytes: u64,
}

pub fn plan(report: &ScanReport) -> SweepPlan {
    let mut plan = SweepPlan::default();
    for finding in &report.findings {
        match finding.verdict {
            Verdict::Safe => match finding.origin {
                FindingOrigin::Filesystem => plan.deletable.push(finding.clone()),
                FindingOrigin::Facts(_) => plan.provider_managed.push(finding.clone()),
            },
            Verdict::Risky => {
                plan.risky += 1;
                plan.risky_bytes += finding.size_bytes.unwrap_or(0);
            }
            Verdict::Never => {
                plan.never += 1;
                plan.never_bytes += finding.size_bytes.unwrap_or(0);
            }
        }
    }
    plan
}

#[derive(Debug)]
pub struct DeletionOutcome {
    pub path: std::path::PathBuf,
    pub result: std::io::Result<()>,
}

/// Execute the plan: delete every deletable finding. Refuses files that
/// vanished between scan and sweep (reported as a no-op error line).
pub fn execute(plan: &SweepPlan) -> Vec<DeletionOutcome> {
    plan.deletable
        .iter()
        .map(|finding| {
            let result = delete_path(&finding.path);
            DeletionOutcome {
                path: finding.path.clone(),
                result,
            }
        })
        .collect()
}

fn delete_path(path: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

pub fn run(args: SweepArgs) -> Result<()> {
    let report = super::scan_for_sweep(&args)?;
    let plan = plan(&report);
    let color = output::colors_enabled();

    print_plan_header(&plan, color);

    if !args.yes {
        let dry_run_bytes: u64 = plan.deletable.iter().filter_map(|f| f.size_bytes).sum();
        println!(
            "DRY RUN: nothing was deleted. Re-run with --yes to delete the {} safe finding(s) above ({}).",
            plan.deletable.len(),
            output::format_size(dry_run_bytes),
        );
        if !plan.provider_managed.is_empty() {
            println!(
                "{} safe finding(s) are managed by their provider (docker, git) and are reported, not deleted, in v1.",
                plan.provider_managed.len()
            );
        }
        return Ok(());
    }

    if plan.deletable.is_empty() {
        println!("Nothing to delete: no safe filesystem findings.");
        return Ok(());
    }

    let progress = indicatif::ProgressBar::new(plan.deletable.len() as u64);
    progress.set_style(
        indicatif::ProgressStyle::with_template("{spinner} {bar:40.cyan/blue} {pos}/{len} {msg}")
            .unwrap_or_else(|_| indicatif::ProgressStyle::default_bar()),
    );
    let mut deleted = 0u64;
    let mut failures = 0usize;
    let mut removed_bytes = 0u64;
    for outcome in execute(&plan) {
        match outcome.result {
            Ok(()) => {
                deleted += 1;
                if let Some(finding) = plan.deletable.iter().find(|f| f.path == outcome.path) {
                    removed_bytes += finding.size_bytes.unwrap_or(0);
                }
                progress.set_message(outcome.path.display().to_string());
            }
            Err(err) => {
                failures += 1;
                eprintln!("failed to delete {}: {err}", outcome.path.display());
            }
        }
        progress.inc(1);
    }
    progress.finish_and_clear();

    println!(
        "Deleted {} finding(s), reclaimed {}. {} risky ({}) and {} never ({}) finding(s) were not touched.",
        deleted,
        output::format_size(removed_bytes),
        plan.risky,
        output::format_size(plan.risky_bytes),
        plan.never,
        output::format_size(plan.never_bytes),
    );
    if failures > 0 {
        println!("{failures} deletion(s) failed; see errors above.");
    }
    if !plan.provider_managed.is_empty() {
        println!(
            "{} provider-managed safe finding(s) (docker, git) were not deleted: v1 sweep does not run provider commands.",
            plan.provider_managed.len()
        );
    }
    Ok(())
}

/// Header: table of deletable items, then counts for everything held back.
fn print_plan_header(plan: &SweepPlan, color: bool) {
    if plan.deletable.is_empty() && plan.provider_managed.is_empty() {
        println!("No safe findings under the given paths.");
    } else if !plan.deletable.is_empty() {
        println!("Safe to delete:");
        print!("{}", output::render_findings_table(&plan.deletable, color));
        println!();
    }
    if !plan.provider_managed.is_empty() {
        println!("Safe but provider-managed (reported, not deleted in v1):");
        print!(
            "{}",
            output::render_findings_table(&plan.provider_managed, color)
        );
        println!();
    }
}
