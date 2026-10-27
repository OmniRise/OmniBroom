//! UniFFI surface for OmniBroom: the stub that exists from day one so the
//! CLI, future Tauri GUI, and phone apps share one engine.
//!
//! What this crate does today:
//! - compiles as cdylib + rlib with UniFFI scaffolding wired up,
//! - exports `scan_json`, a synchronous JSON-in/JSON-out entry point that
//!   phone shells can call without knowing Rust types.
//!
//! What it deliberately does not do yet: generate language bindings. That is
//! a `uniffi-bindgen` step (Kotlin, Swift) run when the phone rung needs it;
//! wiring it into CI before there is a consumer would be build cost without
//! a user. No facts providers here either: subprocess facts are the CLI's
//! job, and shells get pure filesystem classification until their own
//! provider story exists.
//!
//! Async stays out: the engine is sync (no tokio in core), so the FFI is
//! sync too. Callers thread it themselves.

use std::path::PathBuf;
use std::sync::Arc;

use broom_core::{Engine, WalkConfig};
use broom_rules::builtin_registry;

/// Errors crossing the FFI boundary. UniFFI turns this into the platform's
/// error type when bindings are generated.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum BroomFfiError {
    #[error("rule packs failed to load: {0}")]
    RulePacks(String),
    #[error("scan failed: {0}")]
    Scan(String),
    #[error("invalid root path: {0}")]
    InvalidRoot(String),
}

/// Scan the given roots with built-in rules and no facts providers, and
/// return the full report as pretty JSON. The stable shape: roots, findings
/// (path, verdict, category, reason, rule_id, size_bytes, origin), warnings,
/// entries_visited, summary.
#[uniffi::export]
pub fn scan_json(roots: Vec<String>) -> Result<String, BroomFfiError> {
    let registry = builtin_registry().map_err(|e| BroomFfiError::RulePacks(e.to_string()))?;
    let engine = Engine::new(registry, Vec::new(), WalkConfig::default());
    let paths: Vec<PathBuf> = roots.into_iter().map(PathBuf::from).collect();
    if paths.is_empty() {
        return Err(BroomFfiError::InvalidRoot(
            "at least one root path is required".to_string(),
        ));
    }
    let report = engine
        .scan(&paths, None)
        .map_err(|e| BroomFfiError::Scan(e.to_string()))?;
    serde_json::to_string_pretty(&report).map_err(|e| BroomFfiError::Scan(e.to_string()))
}

/// Rule ids compiled into this build, JSON array. Shells use it to render
/// provenance in `why`-style views without shipping their own rule list.
#[uniffi::export]
pub fn rule_ids_json() -> Result<String, BroomFfiError> {
    let registry = builtin_registry().map_err(|e| BroomFfiError::RulePacks(e.to_string()))?;
    serde_json::to_string_pretty(&registry.rule_ids())
        .map_err(|e| BroomFfiError::Scan(e.to_string()))
}

uniffi::setup_scaffolding!();

// Keep the engine types referenced so the crate graph stays honest about
// what the FFI surface depends on.
#[allow(dead_code)]
fn _type_anchor(engine: &Engine, provider: Arc<dyn broom_core::FactsProvider>) {
    let _ = (engine, provider);
}
