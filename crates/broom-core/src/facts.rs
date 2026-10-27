use std::path::PathBuf;

/// One observed fact from the system, before any rule classifies it. Facts are
/// the vocabulary rule packs match on, so the engine never needs to know what
/// docker or journalctl is. The path doubles as the identity: filesystem
/// findings and fact findings for the same path collapse into one finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    /// Provider name, e.g. "docker". Must match the provider field in rules.
    pub provider: String,
    /// Fact key, e.g. "dangling_image". Must match the key field in rules.
    pub key: String,
    /// Path (or pseudo-path like `docker://image/sha256:abc`) this fact is about.
    pub path: PathBuf,
    pub size_bytes: Option<u64>,
}

/// Context the engine hands to providers before they run. Carries repo roots
/// the walker found so the git provider does not have to rediscover them.
#[derive(Debug, Default, Clone)]
pub struct FactsContext {
    pub repo_roots: Vec<PathBuf>,
}

/// What a provider produced: facts plus non-fatal warnings (docker daemon down,
/// journalctl missing on macOS, and so on). Providers never abort a scan.
#[derive(Debug, Default)]
pub struct ProviderOutcome {
    pub facts: Vec<Fact>,
    pub warnings: Vec<String>,
}

/// A source of system facts. Implemented on the surface (CLI shells out), never
/// inside the engine: broom-core stays free of subprocess calls.
pub trait FactsProvider: Send + Sync {
    fn name(&self) -> &str;
    fn collect(&self, ctx: &FactsContext) -> ProviderOutcome;
}
