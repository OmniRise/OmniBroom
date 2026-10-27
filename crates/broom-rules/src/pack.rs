use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use broom_core::{Classifier, Fact, PathContext, RuleDecision, WalkEntry};
use globset::{Glob, GlobMatcher};

use crate::RulesError;
use crate::schema::{MatchSpec, PackSource, RuleCondition, RuleSpec, parse_pack};

/// A pack after compilation: precompiled globs, home paths resolved. This is
/// the immutable, thread-safe form the engine reads during a scan.
#[derive(Debug, Clone)]
pub struct CompiledPack {
    pub id: String,
    pub name: String,
    pub rules: Vec<CompiledRule>,
}

#[derive(Debug, Clone)]
pub struct CompiledRule {
    pub id: String,
    pub category: String,
    pub verdict: broom_core::Verdict,
    pub reason: String,
    pub condition: RuleCondition,
    pub kind: CompiledMatch,
}

#[derive(Debug, Clone)]
pub enum CompiledMatch {
    Name {
        name: String,
        dir_only: bool,
    },
    PathGlob {
        matcher: GlobMatcher,
        original: String,
    },
    Fact {
        provider: String,
        key: String,
    },
}

/// Compiles [`PackSource`]s into a [`Classifier`]. Holds everything the engine
/// needs to classify without touching TOML or the filesystem again.
///
/// Cross-pack rule id uniqueness is enforced here, across every pack loaded
/// together: duplicate ids in JSON output would be a lie about provenance.
#[derive(Debug)]
pub struct PackRegistry {
    packs: Vec<CompiledPack>,
    /// rule id -> (pack index, rule index) for stable `why` provenance.
    index: HashMap<String, (usize, usize)>,
    /// Fact keys that have at least one rule: providers are only consulted
    /// for facts someone actually classifies.
    fact_keys: Vec<(String, String)>,
}

impl PackRegistry {
    pub fn compile(sources: Vec<PackSource>) -> Result<Arc<Self>, RulesError> {
        let mut packs = Vec::new();
        let mut index: HashMap<String, (usize, usize)> = HashMap::new();
        let mut fact_keys: Vec<(String, String)> = Vec::new();

        for source in &sources {
            let pack = parse_pack(source)?;
            let mut rules = Vec::new();
            for rule in &pack.rules {
                if index.contains_key(&rule.id) {
                    return Err(RulesError::Invalid(format!(
                        "rule id {:?} appears in more than one loaded pack",
                        rule.id
                    )));
                }
                let kind = compile_match(rule, &pack.pack.id)?;
                if let CompiledMatch::Fact { provider, key } = &kind {
                    let pair = (provider.clone(), key.clone());
                    if !fact_keys.contains(&pair) {
                        fact_keys.push(pair);
                    }
                }
                index.insert(rule.id.clone(), (packs.len(), rules.len()));
                rules.push(CompiledRule {
                    id: rule.id.clone(),
                    category: rule.category.clone(),
                    verdict: rule.verdict,
                    reason: rule.reason.clone(),
                    condition: rule.when.clone(),
                    kind,
                });
            }
            packs.push(CompiledPack {
                id: pack.pack.id.clone(),
                name: pack.pack.name.clone(),
                rules,
            });
        }

        Ok(Arc::new(Self {
            packs,
            index,
            fact_keys,
        }))
    }

    pub fn pack_ids(&self) -> Vec<&str> {
        self.packs.iter().map(|p| p.id.as_str()).collect()
    }

    pub fn rule_count(&self) -> usize {
        self.packs.iter().map(|p| p.rules.len()).sum()
    }

    /// Rule ids in deterministic (pack, rule) order, for CLI listings.
    pub fn rule_ids(&self) -> Vec<&str> {
        self.packs
            .iter()
            .flat_map(|p| p.rules.iter().map(|r| r.id.as_str()))
            .collect()
    }

    /// Pack that owns `rule_id`, for `why` provenance.
    pub fn pack_of(&self, rule_id: &str) -> Option<&str> {
        self.index
            .get(rule_id)
            .map(|&(pack, _)| self.packs[pack].id.as_str())
    }

    fn fact_keys(&self) -> &[(String, String)] {
        &self.fact_keys
    }
}
fn compile_match(rule: &RuleSpec, pack_id: &str) -> Result<CompiledMatch, RulesError> {
    match &rule.match_spec {
        MatchSpec::Name { name, dir_only } => {
            if name.is_empty() {
                return Err(RulesError::Invalid(format!(
                    "pack {pack_id}: rule {:?} matches an empty name",
                    rule.id
                )));
            }
            Ok(CompiledMatch::Name {
                name: name.clone(),
                dir_only: *dir_only,
            })
        }
        MatchSpec::PathGlob {
            pattern,
            expand_home,
        } => {
            let resolved = if *expand_home {
                expand_home_pattern(pattern, &rule.id, pack_id)?
            } else {
                pattern.clone()
            };
            let glob = Glob::new(&resolved).map_err(|e| RulesError::Glob {
                rule_id: rule.id.clone(),
                pattern: resolved.clone(),
                source: e,
            })?;
            Ok(CompiledMatch::PathGlob {
                matcher: glob.compile_matcher(),
                original: resolved,
            })
        }
        MatchSpec::Fact { provider, key } => {
            if provider.is_empty() || key.is_empty() {
                return Err(RulesError::Invalid(format!(
                    "pack {pack_id}: rule {:?} matches a fact with an empty provider or key",
                    rule.id
                )));
            }
            Ok(CompiledMatch::Fact {
                provider: provider.clone(),
                key: key.clone(),
            })
        }
    }
}

/// `~/...` patterns need the real home directory. Missing home is an invalid
/// pack, not a silent never-matching rule: silent rule death is the bug class
/// this product exists to avoid.
fn expand_home_pattern(pattern: &str, rule_id: &str, pack_id: &str) -> Result<String, RulesError> {
    if pattern == "~" || pattern.starts_with("~/") {
        let home = dirs::home_dir().ok_or_else(|| {
            RulesError::Invalid(format!(
                "pack {pack_id}: rule {rule_id} uses ~ but no home directory is known"
            ))
        })?;
        let rest = pattern.strip_prefix('~').unwrap_or(pattern);
        return Ok(format!("{}{}", home.display(), rest));
    }
    Ok(pattern.to_string())
}

impl Classifier for PackRegistry {
    fn classify_path(&self, entry: &WalkEntry, ctx: &dyn Fn() -> PathContext) -> Vec<RuleDecision> {
        let file_name = entry
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Resolve the git context at most once per entry, and only when a
        // rule actually matched and needs it (conditions gate the call).
        let mut ctx_cache: Option<PathContext> = None;
        let mut decisions = Vec::new();
        for pack in &self.packs {
            for rule in &pack.rules {
                let pattern_matches = match &rule.kind {
                    CompiledMatch::Name { name, dir_only } => {
                        if *dir_only && !entry.is_dir {
                            false
                        } else {
                            file_name == *name
                        }
                    }
                    CompiledMatch::PathGlob { matcher, .. } => matcher.is_match(&entry.path),
                    CompiledMatch::Fact { .. } => false,
                };
                if !pattern_matches {
                    continue;
                }
                if rule.condition != RuleCondition::default() {
                    let resolved = ctx_cache.get_or_insert_with(ctx);
                    if !condition_holds(&rule.condition, resolved) {
                        continue;
                    }
                }
                decisions.push(RuleDecision {
                    rule_id: rule.id.clone(),
                    category: rule.category.clone(),
                    verdict: rule.verdict,
                    reason: rule.reason.clone(),
                });
            }
        }
        decisions
    }

    fn context_for(&self, path: &Path) -> PathContext {
        // Walk ancestors looking for .git. Cheap: an ancestor chain is dozens
        // of components at worst, and .git is one stat per level.
        let mut current = Some(path);
        while let Some(dir) = current {
            if dir.join(".git").symlink_metadata().is_ok() {
                return PathContext {
                    inside_git_worktree: true,
                    git_root: Some(dir.to_path_buf()),
                };
            }
            current = dir.parent();
        }
        PathContext::default()
    }

    fn classify_fact(&self, fact: &Fact) -> Vec<RuleDecision> {
        let wanted = self
            .fact_keys()
            .iter()
            .any(|(provider, key)| provider == &fact.provider && key == &fact.key);
        if !wanted {
            return Vec::new();
        }
        let mut decisions = Vec::new();
        for pack in &self.packs {
            for rule in &pack.rules {
                if let CompiledMatch::Fact { provider, key } = &rule.kind {
                    if provider == &fact.provider && key == &fact.key {
                        decisions.push(RuleDecision {
                            rule_id: rule.id.clone(),
                            category: rule.category.clone(),
                            verdict: rule.verdict,
                            reason: rule.reason.clone(),
                        });
                    }
                }
            }
        }
        decisions
    }
}

fn condition_holds(condition: &RuleCondition, ctx: &PathContext) -> bool {
    condition
        .inside_git_worktree
        .is_none_or(|want| want == ctx.inside_git_worktree)
}
