//! Facts providers: the only place OmniBroom shells out. Each provider maps a
//! system tool into [`broom_core::Fact`]s that rule packs classify. Missing
//! tools are warnings, never failures: a scan on a machine without docker
//! still reports everything else.

mod docker;
mod git_worktrees;
mod journalctl;

pub use docker::DockerFacts;
pub use git_worktrees::GitWorktreesFacts;
pub use journalctl::JournalctlFacts;

use std::sync::Arc;

use broom_core::FactsProvider;

/// The provider set the CLI injects by default: docker, journalctl, and git
/// worktrees. Callers with `--no-facts` pass an empty slice instead.
pub fn default_providers() -> Vec<Arc<dyn FactsProvider>> {
    vec![
        Arc::new(DockerFacts),
        Arc::new(JournalctlFacts),
        Arc::new(GitWorktreesFacts),
    ]
}

/// Parse docker's human-readable sizes ("1.3GB", "5.087GB", "0B") into bytes.
/// Docker prints SI units (GB = 10^9), which matches its own reclaim math.
pub(crate) fn parse_docker_size(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let split = text
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(text.len());
    let (num, suffix) = text.split_at(split);
    let value: f64 = num.parse().ok()?;
    let multiplier = match suffix.trim().to_ascii_uppercase().as_str() {
        "" | "B" => 1.0,
        "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        "PB" => 1e15,
        // docker sometimes prints KIB-style units on newer versions.
        "KIB" => 1024.0,
        "MIB" => 1024f64.powi(2),
        "GIB" => 1024f64.powi(3),
        "TIB" => 1024f64.powi(4),
        _ => return None,
    };
    Some((value * multiplier) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_docker_sizes() {
        assert_eq!(parse_docker_size("0B"), Some(0));
        assert_eq!(parse_docker_size("370.8MB"), Some(370_800_000));
        assert_eq!(parse_docker_size("15.25GB"), Some(15_250_000_000));
        assert_eq!(parse_docker_size("5.087GB"), Some(5_087_000_000));
        assert_eq!(parse_docker_size("1.3GiB"), Some(1_395_864_371));
        assert_eq!(parse_docker_size("banana"), None);
    }
}
