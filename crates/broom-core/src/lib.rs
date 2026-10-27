//! OmniBroom core engine: parallel filesystem walking, verdict classification,
//! and the facts-provider trait.
//!
//! This crate stays pure. No subprocess calls, no TUI dependencies, no async
//! runtime. Surfaces (CLI, FFI, future GUI) inject facts providers and a
//! classifier built from TOML rule packs, then run [`Engine::scan`] or
//! [`Engine::why`].

pub mod classify;
pub mod engine;
pub mod error;
pub mod facts;
pub mod finding;
pub mod verdict;
pub mod walk;

pub use classify::{Classifier, PathContext, RuleDecision};
pub use engine::{Engine, ScanReport, Summary, WhyMatch, WhyReport};
pub use error::CoreError;
pub use facts::{Fact, FactsContext, FactsProvider, ProviderOutcome};
pub use finding::{Finding, FindingOrigin};
pub use verdict::Verdict;
pub use walk::{ProgressSink, WalkConfig, WalkEntry, WalkOutcome, dir_size};
