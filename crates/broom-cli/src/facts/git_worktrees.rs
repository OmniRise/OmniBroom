//! Git facts: linked worktrees of merged branches.
//!
//! The engine finds repo roots during its walk (any directory containing
//! .git); this provider asks git which of those repos have linked worktrees
//! checked out on branches already merged into the default branch. Only the
//! repo's own subprocess work lives here, never in the engine.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use broom_core::{Fact, FactsContext, FactsProvider, ProviderOutcome, dir_size};

pub struct GitWorktreesFacts;

impl FactsProvider for GitWorktreesFacts {
    fn name(&self) -> &str {
        "git"
    }

    fn collect(&self, ctx: &FactsContext) -> ProviderOutcome {
        let mut out = ProviderOutcome::default();
        if ctx.repo_roots.is_empty() {
            return out;
        }
        // Fail once, up front, if git is missing; per-repo failures below are
        // repo-shaped warnings instead.
        if let Err(reason) = git(&ctx.repo_roots[0], &["--version"]) {
            out.warnings
                .push(format!("git worktree facts skipped: {reason}"));
            return out;
        }

        for repo in &ctx.repo_roots {
            collect_repo(repo, &mut out);
        }
        out
    }
}

fn collect_repo(repo: &Path, out: &mut ProviderOutcome) {
    let listing = match git(repo, &["worktree", "list", "--porcelain"]) {
        Ok(listing) => listing,
        Err(reason) => {
            out.warnings.push(format!(
                "worktree facts for {} skipped: {reason}",
                repo.display()
            ));
            return;
        }
    };
    let worktrees = parse_worktree_list(&listing);
    if worktrees.len() < 2 {
        // Single worktree: the main checkout is never a candidate, and there
        // is nothing linked to examine. No warning: this is the common case.
        return;
    }
    let Some(base) = default_branch(repo, out) else {
        out.warnings.push(format!(
            "worktree facts for {} skipped: no default branch resolved (origin/HEAD, origin/main, main, master all unknown)",
            repo.display()
        ));
        return;
    };
    let base_short = base.rsplit('/').next().unwrap_or(&base).to_string();

    for (index, worktree) in worktrees.iter().enumerate() {
        // git lists the main worktree first, always. The main checkout is
        // where development happens: only linked worktrees (index >= 1) are
        // removal candidates, regardless of which worktree this function was
        // called from.
        if index == 0 {
            continue;
        }
        let Some(branch) = worktree.branch.as_deref() else {
            // Detached HEAD: no branch to call merged. Skip rather than guess.
            continue;
        };
        if branch == base_short {
            // A worktree parked on the trunk itself is not "a merged branch".
            continue;
        }
        if !is_merged(repo, branch, &base) {
            continue;
        }
        out.facts.push(Fact {
            provider: "git".to_string(),
            key: "merged_worktree".to_string(),
            path: worktree.path.clone(),
            size_bytes: Some(dir_size(&worktree.path)),
        });
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub branch: Option<String>,
}

/// Parse `git worktree list --porcelain`: blocks of
/// "worktree <path>" / "HEAD <sha>" / "branch refs/heads/<name>" (or
/// "detached"), separated by blank lines.
pub fn parse_worktree_list(stdout: &str) -> Vec<WorktreeEntry> {
    let mut entries: Vec<WorktreeEntry> = Vec::new();
    for line in stdout.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            entries.push(WorktreeEntry {
                path: PathBuf::from(path.trim()),
                branch: None,
            });
        } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
            if let Some(current) = entries.last_mut() {
                current.branch = Some(branch.trim().to_string());
            }
        }
    }
    entries
}

/// The base branch merged-ness is measured against: origin/HEAD first (git's
/// own answer), then the conventional names.
fn default_branch(repo: &Path, out: &mut ProviderOutcome) -> Option<String> {
    if let Ok(stdout) = git(
        repo,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    ) {
        let target = stdout.trim();
        if !target.is_empty() {
            return Some(target.to_string());
        }
    }
    for candidate in ["origin/main", "origin/master", "main", "master"] {
        if git(repo, &["rev-parse", "--verify", "--quiet", candidate]).is_ok() {
            return Some(candidate.to_string());
        }
    }
    let _ = out;
    None
}

/// `git merge-base --is-ancestor <branch> <base>`: exit 0 means the branch's
/// tip is reachable from base, i.e. fully merged. Unknown refs are not merged
/// by definition, so errors count as false without a warning: a deleted
/// branch ref under a still-registered worktree is simply not merged.
fn is_merged(repo: &Path, branch: &str, base: &str) -> bool {
    matches!(
        git_status(repo, &["merge-base", "--is-ancestor", branch, base]),
        Ok(true)
    )
}

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = git_output(repo, args)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("git command failed")
            .trim()
            .to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn git_status(repo: &Path, args: &[&str]) -> Result<bool, String> {
    let output = git_output(repo, args)?;
    Ok(output.status.success())
}

fn git_output(repo: &Path, args: &[&str]) -> Result<Output, String> {
    Command::new("git")
        .current_dir(repo)
        // No credential prompts mid-scan: a scan must never block on a
        // terminal question from a remote helper.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .output()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                "git not found on PATH".to_string()
            } else {
                format!("could not run git: {err}")
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_worktree_output() {
        let listing = "worktree /home/u/repo\nHEAD abc123\nbranch refs/heads/main\n\nworktree /home/u/repo-feat\nHEAD def456\nbranch refs/heads/feat\n\nworktree /home/u/repo-loose\nHEAD 789aaa\ndetached\n";
        let entries = parse_worktree_list(listing);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].path, PathBuf::from("/home/u/repo"));
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert_eq!(entries[1].branch.as_deref(), Some("feat"));
        assert_eq!(entries[2].branch, None);
    }

    #[test]
    fn empty_output_yields_no_entries() {
        assert!(parse_worktree_list("").is_empty());
    }
}
