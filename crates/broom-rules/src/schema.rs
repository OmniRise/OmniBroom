use std::collections::BTreeSet;
use std::path::Path;

use broom_core::Verdict;
use serde::{Deserialize, Serialize};

/// File extension of a rule pack inside a packs directory.
pub const PACK_FILE_EXTENSION: &str = "toml";

/// A rule pack as written in TOML, before compilation. This is the whole
/// schema: if it does not fit here, it is not v1 rule material.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RulePack {
    pub pack: PackMeta,
    pub rules: Vec<RuleSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PackMeta {
    /// Short stable id: "docker", "node", "git", "system".
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Pack format version, not the product version. Bump on schema breaks.
    #[serde(default = "default_pack_version")]
    pub version: u32,
}

fn default_pack_version() -> u32 {
    1
}

/// One classification rule. `verdict` and `reason` are mandatory: a rule that
/// cannot explain itself does not belong in a product whose pitch is reasons.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuleSpec {
    /// Stable dotted id used in `why` output, JSON, and snapshots.
    pub id: String,
    /// Category shown in output: "docker", "node", "git", "system".
    pub category: String,
    /// What happens when this rule matches.
    pub verdict: Verdict,
    /// Human-readable justification. Required, non-empty, checked at compile.
    pub reason: String,
    /// What the rule matches: a name, path globs, or a facts key.
    #[serde(rename = "match")]
    pub match_spec: MatchSpec,
    /// Optional conditions on the match site.
    #[serde(default)]
    pub when: RuleCondition,
}

/// What a rule matches. Rules are pure data: these are the only three shapes.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MatchSpec {
    /// Exact file or directory name, e.g. `node_modules`.
    Name {
        name: String,
        /// Only match directories (skip files named node_modules).
        #[serde(default = "default_true")]
        dir_only: bool,
    },
    /// Glob over the full path, e.g. `**/.npm/_acache` or `/tmp/*`.
    PathGlob {
        pattern: String,
        /// `~` expands to the user's home directory before matching.
        #[serde(default)]
        expand_home: bool,
    },
    /// A fact key from an injected provider: `provider`.`key`.
    Fact { provider: String, key: String },
}

fn default_true() -> bool {
    true
}

/// Conditions on where a rule applies. All present conditions must hold.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuleCondition {
    /// `Some(true)`: match only inside an active git worktree.
    /// `Some(false)`: match only outside one. `None`: either.
    #[serde(default)]
    pub inside_git_worktree: Option<bool>,
}

/// A pack loaded from disk or `include_str!`, before validation.
#[derive(Debug, Clone)]
pub struct PackSource {
    /// Where the bytes came from: a path or "builtin:<id>". Used in errors.
    pub origin: String,
    pub contents: String,
}

impl PackSource {
    pub fn from_path(path: &Path, contents: String) -> Self {
        Self {
            origin: path.display().to_string(),
            contents,
        }
    }

    pub fn builtin(id: &str, contents: &'static str) -> Self {
        Self {
            origin: format!("builtin:{id}"),
            contents: contents.to_string(),
        }
    }
}

impl RulePack {
    /// Structural validation: ids unique, reasons non-empty, categories sane.
    /// Runs at load time so the engine never sees a broken pack.
    pub fn validate(&self, origin: &str) -> Result<(), String> {
        if self.pack.id.is_empty() {
            return Err(format!("{origin}: pack id is empty"));
        }
        if self.rules.is_empty() {
            return Err(format!("{origin}: pack has no rules"));
        }
        let mut ids: BTreeSet<&str> = BTreeSet::new();
        for rule in &self.rules {
            if rule.id.is_empty() {
                return Err(format!("{origin}: a rule has an empty id"));
            }
            if !ids.insert(rule.id.as_str()) {
                return Err(format!("{origin}: duplicate rule id {:?}", rule.id));
            }
            if rule.reason.trim().is_empty() {
                return Err(format!(
                    "{origin}: rule {:?} has no reason. Every rule must explain itself.",
                    rule.id
                ));
            }
            if rule.category.trim().is_empty() {
                return Err(format!("{origin}: rule {:?} has no category", rule.id));
            }
            if let MatchSpec::PathGlob { pattern, .. } = &rule.match_spec {
                if pattern.trim().is_empty() {
                    return Err(format!("{origin}: rule {:?} has an empty glob", rule.id));
                }
            }
        }
        Ok(())
    }
}

/// Parse a pack from TOML text. Validation of content is a separate step so
/// callers can report origin context around both failures.
pub fn parse_pack(source: &PackSource) -> Result<RulePack, crate::RulesError> {
    let pack: RulePack =
        toml::from_str(&source.contents).map_err(|e| crate::RulesError::Parse {
            path: std::path::PathBuf::from(&source.origin),
            source: e,
        })?;
    pack.validate(&source.origin)
        .map_err(crate::RulesError::Invalid)?;
    Ok(pack)
}
