//! Unit tests for schema-level behavior that fixtures cannot reach: home
//! expansion of `~` patterns (must resolve against THIS user's home, not a
//! fixture), glob compilation, fact-key matching, and condition evaluation.

use broom_core::{Classifier, Fact, PathContext, WalkEntry};
use broom_rules::{PackRegistry, PackSource, builtin_registry};

#[test]
fn tilde_patterns_expand_against_real_home() {
    let registry = builtin_registry().unwrap();
    // The npm cache rule is written as ~/.npm/_cacache and must match the
    // expanded absolute path for this user.
    let home = dirs::home_dir().expect("home dir");
    let entry = WalkEntry {
        path: home.join(".npm/_cacache"),
        is_dir: true,
        depth: 3,
        size_bytes: None,
    };
    let ctx = PathContext::default();
    let decisions = registry.classify_path(&entry, &|| ctx.clone());
    assert!(
        decisions.iter().any(|d| d.rule_id == "node.npm-cache"),
        "expected npm cache rule to fire on {}, got {:?}",
        entry.path.display(),
        decisions.iter().map(|d| &d.rule_id).collect::<Vec<_>>(),
    );
}

#[test]
fn wal_glob_matches_at_any_depth() {
    let registry = builtin_registry().unwrap();
    for path in ["/data/app.sqlite-wal", "/a/b/c/deep.wal"] {
        let entry = WalkEntry {
            path: path.into(),
            is_dir: false,
            depth: 4,
            size_bytes: Some(1),
        };
        let ctx = PathContext::default();
        let decisions = registry.classify_path(&entry, &|| ctx.clone());
        assert!(
            decisions
                .iter()
                .any(|d| d.verdict == broom_core::Verdict::Never),
            "WAL rule must claim {path}, got {:?}",
            decisions
        );
    }
}

#[test]
fn journal_fact_classifies_as_never() {
    let registry = builtin_registry().unwrap();
    let fact = Fact {
        provider: "journalctl".to_string(),
        key: "journal_usage".to_string(),
        path: "/var/log/journal".into(),
        size_bytes: Some(1),
    };
    let decisions = registry.classify_fact(&fact);
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].verdict, broom_core::Verdict::Never);
    assert_eq!(decisions[0].rule_id, "system.journal-usage");
}

#[test]
fn unknown_fact_keys_claim_nothing() {
    let registry = builtin_registry().unwrap();
    let fact = Fact {
        provider: "mystery".to_string(),
        key: "surprise".to_string(),
        path: "/x".into(),
        size_bytes: None,
    };
    assert!(registry.classify_fact(&fact).is_empty());
}

#[test]
fn invalid_glob_in_pack_reports_rule_id() {
    // TOML parses fine; the failure happens when the glob compiles, and the
    // error must name the offending rule so a pack author can find it.
    let pack = r#"
[pack]
id = "bad"
name = "Bad"
[[rules]]
id = "bad.glob"
category = "bad"
verdict = "safe"
reason = "x"
match = { kind = "path_glob", pattern = "[unclosed" }
"#;
    let sources = vec![PackSource::builtin("bad", pack)];
    let err = PackRegistry::compile(sources).expect_err("unclosed bracket must fail");
    let msg = err.to_string();
    assert!(msg.contains("bad.glob"), "error must name the rule: {msg}");
}
