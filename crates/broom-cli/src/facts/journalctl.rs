//! Journal facts: total journal disk usage from `journalctl --disk-usage`.
//!
//! Finding journal usage as a fact (instead of only walking /var/log/journal)
//! matters because reading that directory needs privileges a normal scan does
//! not have, and journalctl reports the number authoritatively either way.

use std::path::Path;
use std::process::Command;

use broom_core::{Fact, FactsContext, FactsProvider, ProviderOutcome};

pub struct JournalctlFacts;

impl FactsProvider for JournalctlFacts {
    fn name(&self) -> &str {
        "journalctl"
    }

    fn collect(&self, _ctx: &FactsContext) -> ProviderOutcome {
        let mut out = ProviderOutcome::default();
        let output = match Command::new("journalctl")
            // Force English so the size line parses regardless of locale.
            .env("LC_ALL", "C")
            .args(["--disk-usage", "--no-pager"])
            .output()
        {
            Ok(output) => output,
            Err(err) => {
                let reason = if err.kind() == std::io::ErrorKind::NotFound {
                    "journalctl not found on PATH (not a systemd host?)".to_string()
                } else {
                    format!("could not run journalctl: {err}")
                };
                out.warnings
                    .push(format!("journal facts skipped: {reason}"));
                return out;
            }
        };
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            out.warnings.push(format!(
                "journal facts skipped: journalctl --disk-usage failed: {}",
                stderr.trim()
            ));
            return out;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let Some((size_bytes, source)) = parse_disk_usage(&stdout) else {
            out.warnings.push(format!(
                "journal facts skipped: could not parse journalctl output {:?}",
                stdout.trim()
            ));
            return out;
        };
        if size_bytes == 0 {
            return out;
        }
        out.facts.push(Fact {
            provider: "journalctl".to_string(),
            key: "journal_usage".to_string(),
            path: Path::new(source).to_path_buf(),
            size_bytes: Some(size_bytes),
        });
        out
    }
}

/// Parse `journalctl --disk-usage` output: the line reads like
/// "Archived and active journals take up 207M in the file system." or
/// "Archived journals take up 1.5G in /var/log/journal/abc." Older versions
/// print two lines (archived and system), so sum every size found.
/// Returns total bytes and the location to attach the finding to.
pub fn parse_disk_usage(stdout: &str) -> Option<(u64, &'static str)> {
    let mut total: u64 = 0;
    let mut found = false;
    let mut source: &'static str = "/var/log/journal";
    for line in stdout.lines() {
        // Modern: "Archived and active journals take up 207M ...".
        // Old: "System journal takes up 300M ..." (note the "s").
        let after_take = if let Some(rest) = line.split_once("take up") {
            rest.1
        } else if let Some(rest) = line.split_once("takes up") {
            rest.1
        } else {
            continue;
        };
        let size_token = after_take.split_whitespace().next().unwrap_or("");
        let Some(size_bytes) = parse_journal_size(size_token) else {
            continue;
        };
        total += size_bytes;
        found = true;
        // Prefer an explicit path over the generic "in the file system" form.
        if let Some(pos) = after_take.find("/var/log/journal") {
            let _ = pos;
            source = "/var/log/journal";
        }
    }
    if found { Some((total, source)) } else { None }
}

/// systemd sizes are 1024-based with bare K/M/G/T/P suffixes ("207M").
fn parse_journal_size(token: &str) -> Option<u64> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    let split = token
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(token.len());
    let (num, suffix) = token.split_at(split);
    let value: f64 = num.parse().ok()?;
    let multiplier = match suffix.trim().to_ascii_uppercase().as_str() {
        "" | "B" => 1.0,
        "K" => 1024.0,
        "M" => 1024f64.powi(2),
        "G" => 1024f64.powi(3),
        "T" => 1024f64.powi(4),
        "P" => 1024f64.powi(5),
        _ => return None,
    };
    Some((value * multiplier) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modern_single_line_output() {
        let (bytes, source) =
            parse_disk_usage("Archived and active journals take up 207M in the file system.\n")
                .expect("should parse");
        assert_eq!(bytes, 207 * 1024 * 1024);
        assert_eq!(source, "/var/log/journal");
    }

    #[test]
    fn parses_old_two_line_output_and_sums() {
        let (bytes, _) = parse_disk_usage(
            "Archived journals take up 1.5G in /var/log/journal/abc.\nSystem journal takes up 300M in /var/log/journal/abc.\n",
        )
        .expect("should parse");
        assert_eq!(bytes, 1024u64.pow(3) * 3 / 2 + 300 * 1024 * 1024);
    }

    #[test]
    fn rejects_unparsable_output() {
        assert!(parse_disk_usage("Hint: you are not in the journal group\n").is_none());
        assert!(parse_disk_usage("").is_none());
    }

    #[test]
    fn journal_size_units() {
        assert_eq!(parse_journal_size("0"), Some(0));
        assert_eq!(parse_journal_size("207M"), Some(207 * 1024 * 1024));
        assert_eq!(parse_journal_size("1.5G"), Some(1024u64.pow(3) * 3 / 2));
        assert_eq!(parse_journal_size("banana"), None);
    }
}
