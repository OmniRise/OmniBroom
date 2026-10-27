use std::path::PathBuf;
use std::sync::Arc;

use crate::RulesError;
use crate::schema::PackSource;

/// Ids of the packs compiled into the binary, in load order.
pub const BUILTIN_PACK_IDS: [&str; 4] = ["docker", "node", "git", "system"];

/// The built-in v1 packs, read from TOML files at compile time. `include_str!`
/// keeps the shipped rule set byte-identical to the files in `packs/`: what the
/// repo says is what the binary classifies with.
pub fn builtin_pack_sources() -> Vec<PackSource> {
    let docker = PackSource::builtin("docker", include_str!("../packs/docker.toml"));
    let node = PackSource::builtin("node", include_str!("../packs/node.toml"));
    let git = PackSource::builtin("git", include_str!("../packs/git.toml"));
    let system = PackSource::builtin("system", include_str!("../packs/system.toml"));
    vec![docker, node, git, system]
}

/// Compile the built-in packs. The entry point every surface starts from.
pub fn builtin_registry() -> Result<Arc<crate::PackRegistry>, RulesError> {
    crate::PackRegistry::compile(builtin_pack_sources())
}

/// Load every `*.toml` file in `dir`, sorted by file name for determinism.
/// Lets users add packs without a release; same schema, same validation.
pub fn load_packs_dir(dir: &PathBuf) -> Result<Vec<PackSource>, RulesError> {
    let mut sources = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| RulesError::Read {
        path: dir.clone(),
        source: e,
    })?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext == crate::schema::PACK_FILE_EXTENSION)
        })
        .collect();
    paths.sort();
    for path in paths {
        let contents = std::fs::read_to_string(&path).map_err(|e| RulesError::Read {
            path: path.clone(),
            source: e,
        })?;
        sources.push(PackSource::from_path(&path, contents));
    }
    Ok(sources)
}
