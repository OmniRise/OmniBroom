/// Rule packs for OmniBroom: TOML schema, compiler and the built-in packs
mod builtin;
mod pack;
mod schema;

pub use builtin::{BUILTIN_PACK_IDS, builtin_pack_sources, builtin_registry, load_packs_dir};
pub use pack::{CompiledPack, PackRegistry};
pub use schema::{
    MatchSpec, PACK_FILE_EXTENSION, PackSource, RuleCondition, RulePack, RuleSpec, parse_pack,
};

use thiserror::Error;

/// Failures loading or compiling rule packs
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
