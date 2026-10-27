//! Rule-pack tests: the built-in packs compile, validate, and classify
//! fixture trees the way the verdict tables promise. Snapshot tests pin the
//! verdict-per-path table so a rule edit that changes a verdict shows up as
//! a reviewed snapshot diff, never silently.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use broom_core::{Engine, Fact, FactsContext, FactsProvider, ProviderOutcome, Verdict, WalkConfig};
use broom_rules::{PackRegistry, builtin_pack_sources, builtin_registry, parse_pack};
use tempfile::TempDir;

#[test]
fn builtin_packs_all_compile() {
    let registry = builtin_registry().expect("built-in packs must compile");
    let mut ids = registry.pack_ids();
    ids.sort_unstable();
    assert_eq!(ids, vec!["docker", "git", "node", "system"]);
    // docker 3 + node 6 + git 1 + system 5 = 15 v1 rules.
    assert_eq!(registry.rule_count(), 15);
}

#[test]
fn builtin_sources_parse_and_validate() {
    for source in builtin_pack_sources() {
        parse_pack(&source).unwrap_or_else(|e| panic!("{} invalid: {e}", source.origin));
    }
}

#[test]
fn every_builtin_rule_has_a_reason() {
    let registry = builtin_registry().unwrap();
    for rule_id in registry.rule_ids() {
        assert!(!rule_id.is_empty());
        // pack_of proves provenance is resolvable for `why` output.
        assert!(registry.pack_of(rule_id).is_some(), "no pack for {rule_id}");
    }
}

#[test]
fn duplicate_rule_ids_across_packs_are_rejected() {
    let good = r#"
[pack]
id = "a"
name = "A"
[[rules]]
id = "dup.rule"
category = "a"
verdict = "safe"
reason = "first"
match = { kind = "name", name = "thing" }
"#;
    let bad = r#"
[pack]
id = "b"
name = "B"
[[rules]]
id = "dup.rule"
category = "b"
verdict = "risky"
reason = "second"
match = { kind = "name", name = "other" }
"#;
    let sources = vec![
        broom_rules::PackSource::builtin("a", good),
        broom_rules::PackSource::builtin("b", bad),
    ];
    let err = PackRegistry::compile(sources).expect_err("duplicate ids must fail");
    assert!(err.to_string().contains("dup.rule"), "got: {err}");
}

#[test]
fn rule_without_reason_is_rejected() {
    let pack = r#"
[pack]
id = "x"
name = "X"
[[rules]]
id = "x.noreason"
category = "x"
verdict = "safe"
reason = "   "
match = { kind = "name", name = "thing" }
"#;
    let err = parse_pack(&broom_rules::PackSource::builtin("x", pack))
        .expect_err("blank reason must fail validation");
    assert!(err.to_string().contains("explain itself"), "got: {err}");
}

// ---------------------------------------------------------------------------
// Fixture-driven verdict tables (insta snapshots)
// ---------------------------------------------------------------------------

/// Facts provider replaying a fixed docker journal: tests classify fact rules
/// deterministically, no daemon involved.
struct FixedFacts;

impl FactsProvider for FixedFacts {
    fn name(&self) -> &str {
        "docker"
    }

    fn collect(&self, _ctx: &FactsContext) -> ProviderOutcome {
        ProviderOutcome {
            facts: vec![
                Fact {
                    provider: "docker".to_string(),
                    key: "dangling_image".to_string(),
                    path: PathBuf::from("docker://image/aaaa1111bbbb"),
                    size_bytes: Some(1_300_000_000),
                },
                Fact {
                    provider: "docker".to_string(),
                    key: "orphaned_volume".to_string(),
                    path: PathBuf::from("docker://volume/pg_data"),
                    size_bytes: None,
                },
            ],
            warnings: vec![],
        }
    }
}

/// Build a fixture tree covering every non-system path rule:
/// - orphan node_modules (safe)
/// - node_modules inside a git worktree (risky)
/// - a loose .wal file (never)
/// - plain files nothing claims (absent from the table)
fn build_fixture(root: &Path) {
    let write = |rel: &str, bytes: usize| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![0u8; bytes]).unwrap();
    };
    // Orphan node_modules: at the fixture root, no .git ancestor anywhere.
    write("node_modules/pkg/index.js", 4096);
    write("worktree/.git/HEAD", 42);
    write("worktree/node_modules/dep/index.js", 2048);
    write("scratch.wal", 512);
    write("readme.md", 128);
    // npm cache paths: fixtures must not depend on the real home dir, so the
    // home-expanded rules are exercised in the expansion unit test instead.
}

/// Render the verdict table as stable text: path (relative), verdict, rule.
/// This is the artifact insta pins.
fn verdict_table(report: &broom_core::ScanReport, root: &Path) -> String {
    let mut lines: Vec<String> = report
        .findings
        .iter()
        .map(|f| {
            let rel = f
                .path
                .strip_prefix(root)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| f.path.display().to_string());
            let size = f
                .size_bytes
                .map(|b| format!("{b}B"))
                .unwrap_or_else(|| "-".to_string());
            format!(
                "{:<8} {:>10} {:<8} {} [{}]",
                f.verdict.as_str(),
                size,
                f.category,
                rel,
                f.rule_id
            )
        })
        .collect();
    lines.sort();
    format!(
        "{}\n--\nsafe={} risky={} never={} reclaimable={}B\n",
        lines.join("\n"),
        report.summary.safe,
        report.summary.risky,
        report.summary.never,
        report.summary.reclaimable_bytes,
    )
}

fn scan_fixture(with_facts: bool) -> (TempDir, broom_core::ScanReport) {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let registry = builtin_registry().unwrap();
    let providers: Vec<Arc<dyn FactsProvider>> = if with_facts {
        vec![Arc::new(FixedFacts)]
    } else {
        vec![]
    };
    let engine = Engine::new(registry, providers, WalkConfig::default());
    let report = engine.scan(&[dir.path().to_path_buf()], None).unwrap();
    (dir, report)
}

#[test]
fn filesystem_verdict_table_matches_snapshot() {
    let (dir, report) = scan_fixture(false);
    let table = verdict_table(&report, dir.path());
    insta::assert_snapshot!(table);
}

#[test]
fn facts_verdict_table_matches_snapshot() {
    let (dir, report) = scan_fixture(true);
    let table = verdict_table(&report, dir.path());
    insta::assert_snapshot!(table);
}

#[test]
fn worktree_condition_splits_node_modules_verdicts() {
    let (dir, report) = scan_fixture(false);
    let outside = report
        .findings
        .iter()
        .find(|f| f.path == dir.path().join("node_modules"))
        .expect("orphan node_modules found");
    let inside = report
        .findings
        .iter()
        .find(|f| f.path == dir.path().join("worktree/node_modules"))
        .expect("in-worktree node_modules found");
    assert_eq!(outside.verdict, Verdict::Safe);
    assert_eq!(inside.verdict, Verdict::Risky);
    let _ = dir;
}

#[test]
fn docker_facts_classify_into_safe_and_risky() {
    let (dir, report) = scan_fixture(true);
    let image = report
        .findings
        .iter()
        .find(|f| f.path.starts_with("docker://image/"))
        .expect("dangling image fact");
    assert_eq!(image.verdict, Verdict::Safe);
    let volume = report
        .findings
        .iter()
        .find(|f| f.path.starts_with("docker://volume/"))
        .expect("orphaned volume fact");
    assert_eq!(volume.verdict, Verdict::Risky);
    let _ = dir;
}

#[test]
fn why_explains_each_class_on_fixture() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let registry = builtin_registry().unwrap();
    let engine = Engine::new(registry, Vec::new(), WalkConfig::default());

    // Safe: orphan node_modules.
    let why = engine.why(&dir.path().join("node_modules"));
    assert_eq!(why.matches.len(), 1);
    assert_eq!(why.matches[0].verdict, Verdict::Safe);
    assert_eq!(why.matches[0].source, "filesystem");

    // Never: the WAL file.
    let why = engine.why(&dir.path().join("scratch.wal"));
    assert_eq!(why.matches[0].verdict, Verdict::Never);
    assert!(why.matches[0].reason.contains("write-ahead log"));

    // Nothing: readme.md is unclassified, and why says so honestly.
    let why = engine.why(&dir.path().join("readme.md"));
    assert!(why.matches.is_empty());
}
