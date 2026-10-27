//! Regression test for a real bug found during live verification: `why` on a
//! linked worktree reported "unclassified" while `scan` reported "risky" for
//! the same path.
//!
//! Root cause: the git provider skipped the candidate whose path equaled the
//! repo it was invoked on. `scan` invokes it on the main checkout, so the
//! linked worktree was examined (correct). `why` invokes it with the linked
//! worktree as the root (that is the git context for a path inside it), so
//! the linked worktree skipped itself and nothing matched. The provider now
//! treats worktree list position (first = main) as the skip rule instead of
//! path equality.
//!
//! These tests shell out to git to build a real fixture. They are skipped
//! when git is absent so a bare CI image still runs the rest of the suite.

use std::path::{Path, PathBuf};
use std::process::Command;

use broom_core::{Engine, FactsProvider, Verdict, WalkConfig};
use broom_rules::builtin_registry;
use tempfile::TempDir;

use broom_cli::facts::GitWorktreesFacts;

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run a git command, asserting success. Fixture setup only.
fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .status()
        .expect("git should run");
    assert!(
        output.success(),
        "fixture git {:?} failed in {}",
        args,
        repo.display()
    );
}

/// A repo whose `feature` branch is merged into main, plus a linked worktree
/// still parked on `feature`: the exact shape the git pack claims as risky.
struct WorktreeFixture {
    _dir: TempDir,
    main: PathBuf,
    linked: PathBuf,
}

fn merged_worktree_fixture() -> Option<WorktreeFixture> {
    if !git_available() {
        return None;
    }
    let dir = TempDir::new().ok()?;
    let main = dir.path().join("repo");
    let linked = dir.path().join("repo-feature");
    std::fs::create_dir_all(&main).ok()?;

    git(&main, &["init", "-q", "-b", "main", "."]);
    // Identity for the fixture commits only: never touch global config.
    git(&main, &["config", "user.email", "fixture@test.invalid"]);
    git(&main, &["config", "user.name", "omnibroom-fixture"]);
    std::fs::write(main.join("a.txt"), "a").ok()?;
    git(&main, &["add", "."]);
    git(&main, &["commit", "-qm", "init"]);
    git(&main, &["branch", "feature"]);
    git(&main, &["checkout", "-q", "feature"]);
    std::fs::write(main.join("b.txt"), "b").ok()?;
    git(&main, &["add", "."]);
    git(&main, &["commit", "-qm", "feature work"]);
    git(&main, &["checkout", "-q", "main"]);
    git(&main, &["merge", "-q", "feature", "-m", "merge feature"]);
    git(
        &main,
        &["worktree", "add", "-q", linked.to_str().unwrap(), "feature"],
    );

    Some(WorktreeFixture {
        _dir: dir,
        main,
        linked,
    })
}

#[test]
fn scan_and_why_agree_on_merged_worktree() {
    let Some(fx) = merged_worktree_fixture() else {
        eprintln!("git not available, skipping worktree regression test");
        return;
    };

    let providers: Vec<std::sync::Arc<dyn FactsProvider>> =
        vec![std::sync::Arc::new(GitWorktreesFacts)];
    let engine = Engine::new(
        builtin_registry().unwrap(),
        providers,
        WalkConfig::default(),
    );

    // scan: the linked worktree is a risky finding.
    let report = engine.scan(std::slice::from_ref(&fx.main), None).unwrap();
    let scanned = report
        .findings
        .iter()
        .find(|f| f.path == fx.linked)
        .expect("scan must report the merged worktree");
    assert_eq!(scanned.verdict, Verdict::Risky);

    // why: the same path, the same verdict. This is the regression.
    let why = engine.why(&fx.linked);
    assert_eq!(
        why.matches.len(),
        1,
        "why must find the worktree fact, got {:?}",
        why.matches
    );
    assert_eq!(why.matches[0].verdict, Verdict::Risky);
    assert_eq!(why.matches[0].rule_id, "git.worktree-merged-branch");
    assert_eq!(why.matches[0].source, "facts:git");
}

#[test]
fn why_on_file_inside_merged_worktree_agrees_too() {
    let Some(fx) = merged_worktree_fixture() else {
        eprintln!("git not available, skipping worktree regression test");
        return;
    };

    let providers: Vec<std::sync::Arc<dyn FactsProvider>> =
        vec![std::sync::Arc::new(GitWorktreesFacts)];
    let engine = Engine::new(
        builtin_registry().unwrap(),
        providers,
        WalkConfig::default(),
    );

    // A file inside the worktree is covered by the worktree fact: the same
    // containment rule the scan's collapse applies.
    let inside = fx.linked.join("b.txt");
    assert!(inside.exists());
    let why = engine.why(&inside);
    assert!(
        why.matches
            .iter()
            .any(|m| m.rule_id == "git.worktree-merged-branch"),
        "why on a file inside the worktree must inherit the worktree's fact, got {:?}",
        why.matches
    );
}

#[test]
fn main_checkout_is_never_a_worktree_finding() {
    let Some(fx) = merged_worktree_fixture() else {
        eprintln!("git not available, skipping worktree regression test");
        return;
    };

    let providers: Vec<std::sync::Arc<dyn FactsProvider>> =
        vec![std::sync::Arc::new(GitWorktreesFacts)];
    let engine = Engine::new(
        builtin_registry().unwrap(),
        providers,
        WalkConfig::default(),
    );
    let report = engine.scan(std::slice::from_ref(&fx.main), None).unwrap();
    assert!(
        report.findings.iter().all(|f| f.path != fx.main),
        "the main checkout must never be reported as a removable worktree"
    );
}
