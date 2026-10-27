//! Engine tests on real fixture trees: classification, collapsing, summaries.
//! The classifier here is a hand-rolled mock so broom-core stays tested
//! without depending on broom-rules (dependency direction: rules -> core).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use broom_core::{
    Classifier, Engine, Fact, FactsContext, FactsProvider, PathContext, ProviderOutcome,
    RuleDecision, Verdict, WalkConfig, WalkEntry,
};
use tempfile::TempDir;

/// Rule table: (suffix_name, verdict, rule_id) matched on file_name, plus a
/// fact rule table keyed by "provider.key". Enough to exercise the engine.
struct MockClassifier {
    names: HashMap<String, (Verdict, String)>,
    facts: HashMap<String, (Verdict, String)>,
}

impl MockClassifier {
    fn new() -> Self {
        let mut names = HashMap::new();
        names.insert(
            "node_modules".to_string(),
            (Verdict::Safe, "node.modules".to_string()),
        );
        names.insert(
            "cache.wal".to_string(),
            (Verdict::Never, "system.wal".to_string()),
        );
        let mut facts = HashMap::new();
        facts.insert(
            "docker.dangling_image".to_string(),
            (Verdict::Safe, "docker.dangling".to_string()),
        );
        Self { names, facts }
    }
}

impl Classifier for MockClassifier {
    fn classify_path(
        &self,
        entry: &WalkEntry,
        _ctx: &dyn Fn() -> PathContext,
    ) -> Vec<RuleDecision> {
        let Some(name) = entry
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
        else {
            return Vec::new();
        };
        match self.names.get(&name) {
            Some((verdict, rule_id)) => vec![RuleDecision {
                rule_id: rule_id.clone(),
                category: "mock".to_string(),
                verdict: *verdict,
                reason: format!("mock rule {rule_id} fired on {name}"),
            }],
            None => Vec::new(),
        }
    }

    fn context_for(&self, path: &Path) -> PathContext {
        let _ = path;
        PathContext::default()
    }

    fn classify_fact(&self, fact: &Fact) -> Vec<RuleDecision> {
        let key = format!("{}.{}", fact.provider, fact.key);
        match self.facts.get(&key) {
            Some((verdict, rule_id)) => vec![RuleDecision {
                rule_id: rule_id.clone(),
                category: "mock".to_string(),
                verdict: *verdict,
                reason: format!("mock fact rule {rule_id}"),
            }],
            None => Vec::new(),
        }
    }
}

/// Provider emitting one dangling-image fact and one warning: proves facts
/// join the report and warnings ride along without failing the scan.
struct MockFacts;

impl FactsProvider for MockFacts {
    fn name(&self) -> &str {
        "docker"
    }

    fn collect(&self, _ctx: &FactsContext) -> ProviderOutcome {
        ProviderOutcome {
            facts: vec![Fact {
                provider: "docker".to_string(),
                key: "dangling_image".to_string(),
                path: PathBuf::from("docker://image/deadbeef"),
                size_bytes: Some(1024),
            }],
            warnings: vec!["docker daemon restarted mid-scan".to_string()],
        }
    }
}

fn write_file(path: &Path, bytes: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, vec![0u8; bytes]).unwrap();
}

fn scan(dir: &TempDir, with_facts: bool) -> broom_core::ScanReport {
    let providers: Vec<Arc<dyn FactsProvider>> = if with_facts {
        vec![Arc::new(MockFacts)]
    } else {
        vec![]
    };
    let engine = Engine::new(
        Arc::new(MockClassifier::new()),
        providers,
        WalkConfig::default(),
    );
    engine
        .scan(&[dir.path().to_path_buf()], None)
        .expect("scan should not error")
}

#[test]
fn classifies_files_and_directories_with_sizes() {
    let dir = TempDir::new().unwrap();
    write_file(&dir.path().join("node_modules/pkg/index.js"), 2048);
    write_file(&dir.path().join("cache.wal"), 512);
    write_file(&dir.path().join("notes.txt"), 100);

    let report = scan(&dir, false);

    // notes.txt matches nothing: exactly two findings.
    assert_eq!(report.findings.len(), 2);

    let wal = report
        .findings
        .iter()
        .find(|f| f.path.ends_with("cache.wal"))
        .expect("wal finding");
    assert_eq!(wal.verdict, Verdict::Never);
    assert_eq!(wal.size_bytes, Some(512));

    let nm = report
        .findings
        .iter()
        .find(|f| f.path.ends_with("node_modules"))
        .expect("node_modules finding");
    assert_eq!(nm.verdict, Verdict::Safe);
    // Directory size is measured lazily from contents.
    assert_eq!(nm.size_bytes, Some(2048));
}

#[test]
fn nested_matches_collapse_to_outermost() {
    let dir = TempDir::new().unwrap();
    // node_modules inside node_modules: both match, parent must win.
    write_file(
        &dir.path()
            .join("node_modules/outer/node_modules/inner/x.js"),
        64,
    );

    let report = scan(&dir, false);
    assert_eq!(report.findings.len(), 1);
    assert!(report.findings[0].path.ends_with("node_modules"));
    assert_eq!(report.findings[0].size_bytes, Some(64));
}

#[test]
fn facts_join_the_report_with_warnings() {
    let dir = TempDir::new().unwrap();
    write_file(&dir.path().join("keep.txt"), 1);

    let report = scan(&dir, true);

    let fact_finding = report
        .findings
        .iter()
        .find(|f| f.path.starts_with("docker://image/"))
        .expect("fact finding present");
    assert_eq!(fact_finding.verdict, Verdict::Safe);
    assert_eq!(fact_finding.size_bytes, Some(1024));
    assert_eq!(
        report.warnings,
        vec!["docker daemon restarted mid-scan".to_string()]
    );
}

#[test]
fn summary_counts_by_verdict() {
    let dir = TempDir::new().unwrap();
    write_file(&dir.path().join("node_modules/a.js"), 1000);
    write_file(&dir.path().join("cache.wal"), 200);

    let report = scan(&dir, false);
    assert_eq!(report.summary.findings, 2);
    assert_eq!(report.summary.safe, 1);
    assert_eq!(report.summary.never, 1);
    assert_eq!(report.summary.reclaimable_bytes, 1000);
    assert_eq!(report.summary.never_bytes, 200);
}

#[test]
fn findings_sort_biggest_first_then_path() {
    let dir = TempDir::new().unwrap();
    write_file(&dir.path().join("cache.wal"), 10);
    write_file(&dir.path().join("node_modules/big.js"), 5000);
    write_file(&dir.path().join("node_modules/small.js"), 100);

    let report = scan(&dir, false);
    // node_modules (5100) beats cache.wal (10).
    assert!(report.findings[0].path.ends_with("node_modules"));
    assert!(report.findings[1].path.ends_with("cache.wal"));
}

#[test]
fn scan_of_empty_dir_has_no_findings() {
    let dir = TempDir::new().unwrap();
    write_file(&dir.path().join("nothing.txt"), 3);
    let report = scan(&dir, false);
    assert!(report.findings.is_empty());
    assert_eq!(report.summary.findings, 0);
}

#[test]
fn missing_root_yields_warning_not_failure() {
    let engine = Engine::new(
        Arc::new(MockClassifier::new()),
        Vec::new(),
        WalkConfig::default(),
    );
    let ghost = PathBuf::from("/definitely/not/a/real/path/omnibroom-test");
    let report = engine
        .scan(std::slice::from_ref(&ghost), None)
        .expect("no hard error");
    assert!(report.findings.is_empty());
    // jwalk reports the missing root as a walk error warning.
    assert!(
        report.warnings.iter().any(|w| w.contains("walk error")),
        "expected a walk warning, got {:?}",
        report.warnings
    );
}
