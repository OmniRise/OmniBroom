//! Output rendering: human tables (comfy-table) and JSON. The `--json` mode
//! serializes the engine's report directly; the table is the human skin.

use std::io::IsTerminal;

use broom_core::{Finding, ScanReport, Verdict, WhyReport};
use comfy_table::{Attribute, Cell, Color, ContentArrangement, Table};

/// Human-readable byte size with binary units: 1024 B, 1.0 KiB, 370.8 MiB.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        // One decimal everywhere above bytes: predictable in snapshots and
        // close enough for the eye (1025 B and 1 KiB differ by a hair).
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn verdict_cell(verdict: Verdict, color: bool) -> Cell {
    let cell = Cell::new(verdict.as_str());
    if color {
        match verdict {
            Verdict::Safe => cell.fg(Color::Green),
            Verdict::Risky => cell.fg(Color::Yellow),
            Verdict::Never => cell.fg(Color::Red),
        }
    } else {
        cell
    }
}

/// Should this run colorize? TTY and no NO_COLOR, per the convention every
/// terminal tool follows.
pub fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

/// The findings table: verdict, size, category, path, reason. Sorted biggest
/// first (engine order), stable across runs.
pub fn render_findings_table(findings: &[Finding], color: bool) -> String {
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header([
        Cell::new("verdict").add_attribute(Attribute::Bold),
        Cell::new("size").add_attribute(Attribute::Bold),
        Cell::new("category").add_attribute(Attribute::Bold),
        Cell::new("path").add_attribute(Attribute::Bold),
        Cell::new("reason").add_attribute(Attribute::Bold),
    ]);
    for finding in findings {
        let size = finding
            .size_bytes
            .map(format_size)
            .unwrap_or_else(|| "-".to_string());
        let path = finding.path.to_string_lossy().into_owned();
        table.add_row([
            verdict_cell(finding.verdict, color),
            Cell::new(size),
            Cell::new(&finding.category),
            Cell::new(path),
            Cell::new(&finding.reason),
        ]);
    }
    table.to_string()
}

/// Full scan output: table plus summary line.
pub fn render_scan_report(report: &ScanReport, color: bool) -> String {
    let mut out = String::new();
    if report.findings.is_empty() {
        out.push_str("No findings: nothing under the given paths matches any rule.\n");
    } else {
        out.push_str(&render_findings_table(&report.findings, color));
        out.push('\n');
    }
    out.push_str(&render_summary(report));
    out
}

pub fn render_summary(report: &ScanReport) -> String {
    let s = &report.summary;
    format!(
        "\n{} findings: {} safe ({} reclaimable), {} risky, {} never ({} locked). {} entries scanned.\n",
        s.findings,
        s.safe,
        format_size(s.reclaimable_bytes),
        s.risky,
        s.never,
        format_size(s.never_bytes),
        report.entries_visited,
    )
}

/// Warnings go to stderr as a block so table/JSON on stdout stays clean.
pub fn render_warnings(report: &ScanReport) -> Option<String> {
    if report.warnings.is_empty() {
        return None;
    }
    let mut out = String::from("warnings:\n");
    for warning in &report.warnings {
        out.push_str(&format!("  - {warning}\n"));
    }
    Some(out)
}

/// `why` output: verdict of record, context, then every rule that claims the
/// path with its reason.
pub fn render_why(report: &WhyReport, color: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!("path: {}\n", report.path.display()));
    out.push_str(&format!(
        "exists: {}\n",
        if report.exists {
            "yes"
        } else {
            "no (reasoning from rules only)"
        }
    ));
    out.push_str(&format!(
        "inside git worktree: {}\n",
        match (&report.context.git_root, report.context.inside_git_worktree) {
            (Some(root), true) => format!("yes ({})", root.display()),
            (_, true) => "yes".to_string(),
            (_, false) => "no".to_string(),
        }
    ));

    if report.matches.is_empty() {
        out.push_str("verdict: unclassified\n");
        out.push_str("No rule claims this path. OmniBroom has no opinion on it, so it was not counted in any sweep.\n");
        return out;
    }

    // The verdict of record is the most severe claim: if one rule says never,
    // the answer is never, no matter what safer rules also match.
    let worst = report
        .matches
        .iter()
        .map(|m| m.verdict)
        .max()
        .expect("matches is non-empty");
    out.push_str(&format!("verdict: {}\n", worst.as_str()));

    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header([
        Cell::new("verdict").add_attribute(Attribute::Bold),
        Cell::new("rule").add_attribute(Attribute::Bold),
        Cell::new("category").add_attribute(Attribute::Bold),
        Cell::new("via").add_attribute(Attribute::Bold),
        Cell::new("reason").add_attribute(Attribute::Bold),
    ]);
    for m in &report.matches {
        table.add_row([
            verdict_cell(m.verdict, color),
            Cell::new(&m.rule_id),
            Cell::new(&m.category),
            Cell::new(&m.source),
            Cell::new(&m.reason),
        ]);
    }
    out.push_str(&table.to_string());
    out.push('\n');
    out
}

/// JSON output for `scan --json`: the whole report, machine-readable.
pub fn render_json(report: &ScanReport) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes_binary() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1024), "1.0 KiB");
        assert_eq!(format_size(1025), "1.0 KiB");
        assert_eq!(format_size(370_800_000), "353.6 MiB");
        assert_eq!(format_size(15_250_000_000), "14.2 GiB");
    }
}
