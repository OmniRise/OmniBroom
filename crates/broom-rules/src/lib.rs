//! Rule packs for OmniBroom: TOML schema, compiler, and the built-in v1 packs.
//!
//! Rules stay data. Nothing in this crate hardcodes verdict logic for docker,
//! node, git, or system paths: every verdict comes from a TOML file, compiled
//! once into precompiled glob matchers behind [`broom_core::Classifier`].
//!
//! ```text
//! [[rules]]
//! id = "node.node_modules-outside-worktree"
//! category = "node"
//! verdict = "safe"
//! reason = "node_modules outside a worktree is pure build output..."
//! match = { kind = "name", name = "node_modules", dir_only = true }
//! when = { inside_git_worktree = false }
//! ```

mod builtin;
mod pack;
mod schema;

pub use builtin::{BUILTIN_PACK_IDS, builtin_pack_sources, builtin_registry, load_packs_dir};
pub use pack::{CompiledPack, PackRegistry};
pub use schema::{
    MatchSpec, PACK_FILE_EXTENSION, PackSource, RuleCondition, RulePack, RuleSpec, parse_pack,
};

use thiserror::Error;

/// Failures loading or compiling rule packs. Surfaced by the CLI, never by
/// the engine mid-scan: packs load once at startup.
#[derive(Debug, Error)]
pub enum RulesError {
    #[error("could not read rule pack {path}: {source}")]
    Read {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("rule pack {path} is not valid TOML: {source}")]
    Parse {
        path: std::path::PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("rule pack invalid: {0}")]
    Invalid(String),
    #[error("glob pattern {pattern:?} in rule {rule_id} does not compile: {source}")]
    Glob {
        rule_id: String,
        pattern: String,
        #[source]
        source: globset::Error,
    },
}
