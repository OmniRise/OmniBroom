//! Docker facts: dangling images, buildx cache per builder, orphaned volumes.
//!
//! All three come from plain-text docker output: `docker system df --json`
//! does not exist in every docker build (verified: 29.8.1 rejects the flag),
//! so this provider parses the human formats and keeps the parsing honest
//! with fixtures from real output.

use std::path::Path;
use std::process::Command;

use broom_core::{Fact, FactsContext, FactsProvider, ProviderOutcome};

use super::parse_docker_size;

pub struct DockerFacts;

impl FactsProvider for DockerFacts {
    fn name(&self) -> &str {
        "docker"
    }

    fn collect(&self, _ctx: &FactsContext) -> ProviderOutcome {
        let mut out = ProviderOutcome::default();

        // One daemon probe up front: if docker is absent or the daemon is
        // down, a single warning replaces three confusing ones.
        match docker(&["version", "--format", "{{.Server.Version}}"]) {
            Ok(_) => {}
            Err(reason) => {
                out.warnings
                    .push(format!("docker facts skipped: {reason}. Re-run without --no-facts after starting the daemon to include them."));
                return out;
            }
        }

        dangling_images(&mut out);
        buildx_cache(&mut out);
        orphaned_volumes(&mut out);
        out
    }
}

/// Images with no tag that no container references. Unreferenced is the load-
/// bearing half: `docker images -f dangling=true` also lists untagged images
/// that running containers use (verified on this machine: homepage and
/// code-server run on untagged images), and those are never safe to prune.
fn dangling_images(out: &mut ProviderOutcome) {
    let images = match docker(&[
        "images",
        "--filter",
        "dangling=true",
        "--format",
        "{{.ID}} {{.Size}}",
    ]) {
        Ok(stdout) => stdout,
        Err(reason) => {
            out.warnings
                .push(format!("dangling image facts skipped: {reason}"));
            return;
        }
    };
    if images.trim().is_empty() {
        return;
    }

    // Container references, so in-use untagged images are excluded. Containers
    // may reference images by tag or by id prefix; matching on the short id
    // covers both (a 12-hex sequence colliding with a tag is negligible).
    let referenced = match docker(&["ps", "-a", "--format", "{{.Image}}"]) {
        Ok(stdout) => stdout,
        Err(reason) => {
            out.warnings.push(format!(
                "dangling image facts skipped (could not list containers): {reason}"
            ));
            return;
        }
    };

    for line in images.lines() {
        let mut parts = line.split_whitespace();
        let (Some(id), size) = (parts.next(), parts.next()) else {
            continue;
        };
        if referenced.contains(id) {
            continue;
        }
        let size_bytes = size.and_then(parse_docker_size);
        out.facts.push(Fact {
            provider: "docker".to_string(),
            key: "dangling_image".to_string(),
            path: Path::new(&format!("docker://image/{id}")).to_path_buf(),
            size_bytes,
        });
    }
}

/// Build cache total per builder. `docker buildx du` prints a per-builder
/// record list ending in `Total:`; the builder list comes from `docker buildx ls`.
fn buildx_cache(out: &mut ProviderOutcome) {
    let listing = match docker(&["buildx", "ls"]) {
        Ok(stdout) => stdout,
        Err(reason) => {
            // Buildx is a plugin: absence is common and not worth alarm.
            out.warnings
                .push(format!("buildx cache facts skipped: {reason}"));
            return;
        }
    };
    let builders: Vec<String> = listing
        .lines()
        // Builder rows start at column 0; endpoint rows are indented.
        // The default builder carries a trailing asterisk (e.g. "multiarch*").
        .filter(|line| !line.starts_with(char::is_whitespace))
        .filter(|line| !line.starts_with("NAME/NODE"))
        .filter_map(|line| line.split_whitespace().next())
        .map(|name| name.trim_end_matches('*').to_string())
        .collect();

    for builder in builders {
        let du = match docker(&["buildx", "du", "--builder", &builder]) {
            Ok(stdout) => stdout,
            Err(reason) => {
                out.warnings.push(format!(
                    "buildx cache facts for builder {builder} skipped: {reason}"
                ));
                continue;
            }
        };
        let Some(total_line) = du.lines().find(|l| l.trim_start().starts_with("Total:")) else {
            continue;
        };
        let Some((_, size_text)) = total_line.split_once(':') else {
            continue;
        };
        let Some(size_bytes) = parse_docker_size(size_text) else {
            continue;
        };
        if size_bytes == 0 {
            continue;
        }
        out.facts.push(Fact {
            provider: "docker".to_string(),
            key: "buildx_cache".to_string(),
            path: Path::new(&format!("docker://buildx/{builder}")).to_path_buf(),
            size_bytes: Some(size_bytes),
        });
    }
}

/// Volumes no container references (docker's own dangling filter for volumes).
/// Sizes stay `None`: the volume data lives under /var/lib/docker and reading
/// it needs privileges the CLI does not demand for a scan.
fn orphaned_volumes(out: &mut ProviderOutcome) {
    let volumes = match docker(&[
        "volume",
        "ls",
        "--filter",
        "dangling=true",
        "--format",
        "{{.Name}}",
    ]) {
        Ok(stdout) => stdout,
        Err(reason) => {
            out.warnings
                .push(format!("orphaned volume facts skipped: {reason}"));
            return;
        }
    };
    for name in volumes.lines().map(str::trim).filter(|n| !n.is_empty()) {
        out.facts.push(Fact {
            provider: "docker".to_string(),
            key: "orphaned_volume".to_string(),
            path: Path::new(&format!("docker://volume/{name}")).to_path_buf(),
            size_bytes: None,
        });
    }
}

/// Run one docker subcommand and return trimmed stdout, or a human reason.
fn docker(args: &[&str]) -> Result<String, String> {
    let output = Command::new("docker").args(args).output().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            "docker CLI not found on PATH".to_string()
        } else {
            format!("could not run docker: {err}")
        }
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let first_line = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        return Err(format!(
            "docker {} failed: {}",
            args.first().unwrap_or(&""),
            first_line.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
