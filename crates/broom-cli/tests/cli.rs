//! CLI-layer tests: sweep policy (the safety core), output rendering
//! snapshots, and the facts-provider contract. Deletion only ever touches
//! safe filesystem findings; this file pins that against regressions.

use std::path::PathBuf;
use std::sync::Arc;

use broom_core::{
    Engine, FactsContext, FactsProvider, Finding, FindingOrigin, ScanReport, Summary, Verdict,
    WalkConfig,
};
use broom_rules::builtin_registry;
use tempfile::TempDir;

use broom_cli::commands::sweep::{execute, plan};
use broom_cli::output;

fn finding(path: &str, verdict: Verdict, origin: FindingOrigin, size: Option<u64>) -> Finding {
    Finding {
        path: PathBuf::from(path),
        verdict,
        category: "test".to_string(),
        reason: "test reason".to_string(),
        rule_id: "test.rule".to_string(),
        size_bytes: size,
        origin,
    }
}

fn report_with(findings: Vec<Finding>) -> ScanReport {
    let mut report = ScanReport {
        roots: vec![PathBuf::from("/fixture")],
        findings,
        warnings: vec![],
        entries_visited: 0,
        summary: Summary::default(),
    };
    report.summary.findings = report.findings.len();
    report
}

#[test]
fn dry_run_plan_splits_by_policy() {
    let report = report_with(vec![
        finding(
            "/fixture/node_modules",
            Verdict::Safe,
            FindingOrigin::Filesystem,
            Some(100),
        ),
        finding(
            "docker://image/x",
            Verdict::Safe,
            FindingOrigin::Facts("docker".into()),
            Some(50),
        ),
        finding(
            "/fixture/volume-data",
            Verdict::Risky,
            FindingOrigin::Filesystem,
            Some(900),
        ),
        finding(
            "/var/log/journal",
            Verdict::Never,
            FindingOrigin::Facts("journalctl".into()),
            Some(700),
        ),
    ]);
    let plan = plan(&report);

    // Only safe filesystem findings are deletable. Ever.
    assert_eq!(plan.deletable.len(), 1);
    assert_eq!(
        plan.deletable[0].path,
        PathBuf::from("/fixture/node_modules")
    );
    assert_eq!(plan.provider_managed.len(), 1);
    assert_eq!(plan.risky, 1);
    assert_eq!(plan.never, 1);
    assert_eq!(plan.risky_bytes, 900);
    assert_eq!(plan.never_bytes, 700);
}

#[test]
fn execution_deletes_only_deletable_paths() {
    let dir = TempDir::new().unwrap();
    let victim = dir.path().join("node_modules");
    std::fs::create_dir_all(&victim).unwrap();
    std::fs::write(victim.join("a.js"), vec![0u8; 64]).unwrap();
    let survivor_wal = dir.path().join("keep.wal");
    std::fs::write(&survivor_wal, b"wal").unwrap();

    let report = report_with(vec![
        finding(
            victim.to_str().unwrap(),
            Verdict::Safe,
            FindingOrigin::Filesystem,
            Some(64),
        ),
        finding(
            survivor_wal.to_str().unwrap(),
            Verdict::Never,
            FindingOrigin::Filesystem,
            Some(3),
        ),
    ]);
    let plan = plan(&report);
    assert_eq!(plan.deletable.len(), 1, "never findings are not deletable");

    let outcomes = execute(&plan);
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].result.is_ok());
    assert!(!victim.exists(), "safe directory was deleted");
    assert!(survivor_wal.exists(), "never file untouched");
}

#[test]
fn sweeping_with_no_safe_findings_deletes_nothing() {
    let report = report_with(vec![
        finding(
            "/fixture/a",
            Verdict::Risky,
            FindingOrigin::Filesystem,
            Some(1),
        ),
        finding(
            "/fixture/b",
            Verdict::Never,
            FindingOrigin::Filesystem,
            Some(1),
        ),
    ]);
    let plan = plan(&report);
    let outcomes = execute(&plan);
    assert!(outcomes.is_empty());
}

// ---------------------------------------------------------------------------
// Output snapshots: the verdict table and JSON shape users see.
// ---------------------------------------------------------------------------

fn sample_report() -> ScanReport {
    let mut report = report_with(vec![
        finding(
            "/home/u/.npm/_cacache",
            Verdict::Safe,
            FindingOrigin::Filesystem,
            Some(2_100_000),
        ),
        finding(
            "/home/u/proj/node_modules",
            Verdict::Risky,
            FindingOrigin::Filesystem,
            Some(350_000),
        ),
        finding(
            "/var/log/journal",
            Verdict::Never,
            FindingOrigin::Facts("journalctl".into()),
            Some(217_040_896),
        ),
        finding(
            "docker://volume/pg_data",
            Verdict::Risky,
            FindingOrigin::Facts("docker".into()),
            None,
        ),
    ]);
    report
        .warnings
        .push("docker CLI not found on PATH".to_string());
    report.entries_visited = 4321;
    // Mirror what the engine's summarize() would compute for these findings.
    report.summary.safe = 1;
    report.summary.reclaimable_bytes = 2_100_000;
    report.summary.risky = 2;
    report.summary.never = 1;
    report.summary.never_bytes = 217_040_896;
    report
}

#[test]
fn scan_table_snapshot() {
    let rendered = output::render_scan_report(&sample_report(), false);
    insta::assert_snapshot!(rendered);
}

#[test]
fn scan_json_snapshot() {
    let json = output::render_json(&sample_report()).expect("json renders");
    insta::assert_snapshot!(json);
}

#[test]
fn why_snapshot() {
    let engine = Engine::new(
        builtin_registry().unwrap(),
        Vec::new(),
        WalkConfig::default(),
    );
    let dir = TempDir::new().unwrap();
    let nm = dir.path().join("node_modules");
    std::fs::create_dir_all(&nm).unwrap();
    let report = engine.why(&nm);
    let rendered = output::render_why(&report, false);
    // The tempdir path differs per run: normalize before snapshotting.
    let normalized = rendered.replace(&dir.path().display().to_string(), "<TMP>");
    insta::assert_snapshot!(normalized);
}

#[test]
fn why_unclassified_snapshot() {
    let engine = Engine::new(
        builtin_registry().unwrap(),
        Vec::new(),
        WalkConfig::default(),
    );
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("readme.txt");
    std::fs::write(&file, b"hi").unwrap();
    let report = engine.why(&file);
    let rendered = output::render_why(&report, false);
    let normalized = rendered.replace(&dir.path().display().to_string(), "<TMP>");
    insta::assert_snapshot!(normalized);
}

// ---------------------------------------------------------------------------
// Engine wiring with real rules, no facts: scan through the same entry point
// the CLI uses.
// ---------------------------------------------------------------------------

#[test]
fn cli_engine_scan_counts_safe_findings() {
    let dir = TempDir::new().unwrap();
    let nm = dir.path().join("orphan_nm");
    std::fs::create_dir_all(nm.join("node_modules")).unwrap();
    std::fs::write(nm.join("node_modules/x.js"), vec![0u8; 10]).unwrap();
    let registry = builtin_registry().unwrap();
    let engine = Engine::new(registry, vec![], WalkConfig::default());
    let report = engine.scan(&[dir.path().to_path_buf()], None).unwrap();
    assert_eq!(report.summary.safe, 1);
    assert_eq!(report.summary.reclaimable_bytes, 10);
}

// ---------------------------------------------------------------------------
// Facts providers: the object-safety and no-panic contract. On a machine
// with docker absent or down, providers warn instead of failing; either way
// the trait boundary holds.
// ---------------------------------------------------------------------------

#[test]
fn facts_providers_are_send_sync_and_never_panic() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<broom_cli::facts::DockerFacts>();
    assert_send_sync::<broom_cli::facts::JournalctlFacts>();
    assert_send_sync::<broom_cli::facts::GitWorktreesFacts>();

    let providers: Vec<Arc<dyn FactsProvider>> = vec![
        Arc::new(broom_cli::facts::DockerFacts),
        Arc::new(broom_cli::facts::JournalctlFacts),
        Arc::new(broom_cli::facts::GitWorktreesFacts),
    ];
    let ctx = FactsContext::default();
    for provider in providers {
        let outcome = provider.collect(&ctx);
        // Contract: collect() returns, it never panics or aborts. Warnings
        // carry absence; facts carry observations.
        let _ = outcome.facts.len();
        let _ = outcome.warnings.len();
    }
}
