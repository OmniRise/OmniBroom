//! Output rendering: human tables (comfy-table) and JSON. The `--json` mode
//! serializes the engine's report directly; the table is the human skin.
//!
//! Visual contract: tables may get prettier, JSON never changes shape, and
//! piped (non-tty) output loses only color, never information.

use std::io::IsTerminal;
use std::path::Path;

use broom_core::{Finding, ScanReport, Verdict, WhyReport};
use comfy_table::presets;
use comfy_table::{Attribute, Cell, CellAlignment, Color, ContentArrangement, Table, modifiers};

/// Brand tokens from the OmniBroom pack: safe #3D9674, risky #B67B28,
/// never #C34D50. Truecolor on terminals that support it; comfy-table
/// degrades to the nearest ANSI color on the rest.
const SAFE_RGB: (u8, u8, u8) = (0x3D, 0x96, 0x74);
const RISKY_RGB: (u8, u8, u8) = (0xB6, 0x7B, 0x28);
const NEVER_RGB: (u8, u8, u8) = (0xC3, 0x4D, 0x50);

/// How the findings table is sorted. `Size` is the engine order (biggest
/// first); `Verdict` puts danger first (never, then risky, then safe).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum SortKey {
    #[default]
    Size,
    Path,
    Verdict,
}

/// Optional section grouping of the table.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum GroupKey {
    #[default]
    None,
    Verdict,
    Category,
}

/// Everything the human table needs to know. JSON ignores this completely.
#[derive(Debug, Clone)]
pub struct TableView {
    pub color: bool,
    /// Show only the first N rows after sorting (the `--top` flag).
    pub top: Option<usize>,
    pub sort: SortKey,
    pub group: GroupKey,
}

impl TableView {
    /// Deterministic, full, uncolored: what tests and pipes should use.
    pub fn plain() -> Self {
        Self {
            color: false,
            top: None,
            sort: SortKey::Size,
            group: GroupKey::None,
        }
    }
}

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

fn verdict_rgb(verdict: Verdict) -> (u8, u8, u8) {
    match verdict {
        Verdict::Safe => SAFE_RGB,
        Verdict::Risky => RISKY_RGB,
        Verdict::Never => NEVER_RGB,
    }
}

fn verdict_cell(verdict: Verdict, color: bool) -> Cell {
    // The dot carries the verdict even where color does not survive
    // (color-blind terminals, logs, grayscale screenshots).
    let cell = Cell::new(format!("{} {}", "●", verdict.as_str()));
    if color {
        let (r, g, b) = verdict_rgb(verdict);
        cell.fg(Color::Rgb { r, g, b })
    } else {
        cell
    }
}

/// Should this run colorize? TTY and no NO_COLOR, per the convention every
/// terminal tool follows.
pub fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

/// Abbreviate the user's home directory to `~` for display. Render-only:
/// JSON keeps full paths so machines get exact values.
pub fn shorten_path(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home {
        if let Ok(rest) = path.strip_prefix(home) {
            let rest = rest.display().to_string();
            if rest.is_empty() {
                return "~".to_string();
            }
            return format!("~/{rest}");
        }
    }
    path.to_string_lossy().into_owned()
}

/// Long reasons wreck the table width; the full text always lives in `why`
/// and `--json`. Cut at a word boundary and mark it.
fn cap_reason(reason: &str) -> String {
    const CAP: usize = 72;
    if reason.chars().count() <= CAP {
        return reason.to_string();
    }
    let cut: String = reason.chars().take(CAP + 1).collect();
    match cut.rfind(' ') {
        Some(0) | None => format!("{}…", reason.chars().take(CAP).collect::<String>()),
        Some(i) => format!("{}…", &cut[..i]),
    }
}

fn sort_findings(findings: &[Finding], sort: SortKey) -> Vec<Finding> {
    let mut rows: Vec<Finding> = findings.to_vec();
    match sort {
        SortKey::Size => {
            // Engine order: biggest first, then path for stability.
            rows.sort_by(|a, b| {
                b.size_bytes
                    .unwrap_or(0)
                    .cmp(&a.size_bytes.unwrap_or(0))
                    .then_with(|| a.path.cmp(&b.path))
            });
        }
        SortKey::Path => {
            rows.sort_by(|a, b| a.path.cmp(&b.path));
        }
        SortKey::Verdict => {
            // Danger first: never, then risky, then safe; size breaks ties.
            rows.sort_by(|a, b| {
                b.verdict
                    .cmp(&a.verdict)
                    .then_with(|| b.size_bytes.unwrap_or(0).cmp(&a.size_bytes.unwrap_or(0)))
                    .then_with(|| a.path.cmp(&b.path))
            });
        }
    }
    rows
}

fn findings_table(rows: &[Finding], view: &TableView) -> Table {
    let home = dirs::home_dir();
    let mut table = Table::new();
    table
        .load_preset(presets::UTF8_FULL)
        .apply_modifier(modifiers::UTF8_ROUND_CORNERS);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header([
        Cell::new("verdict").add_attribute(Attribute::Bold),
        Cell::new("size")
            .add_attribute(Attribute::Bold)
            .set_alignment(CellAlignment::Right),
        Cell::new("category").add_attribute(Attribute::Bold),
        Cell::new("path").add_attribute(Attribute::Bold),
        Cell::new("reason").add_attribute(Attribute::Bold),
    ]);
    for finding in rows {
        let size = finding
            .size_bytes
            .map(format_size)
            .unwrap_or_else(|| "-".to_string());
        table.add_row([
            verdict_cell(finding.verdict, view.color),
            Cell::new(size).set_alignment(CellAlignment::Right),
            Cell::new(&finding.category),
            Cell::new(shorten_path(&finding.path, home.as_deref())),
            Cell::new(cap_reason(&finding.reason)),
        ]);
    }
    table
}

/// The one and only wording of the truncation note, shared by scan and
/// sweep so both commands disclose limits identically.
pub fn more_line(hidden_count: usize, hidden_bytes: u64) -> String {
    format!(
        "… and {hidden_count} more finding(s) ({}) not shown. Re-run with --all to see everything.\n",
        format_size(hidden_bytes)
    )
}

/// Render findings with the view's sort, grouping, and row limit. Returns
/// the rendered block plus how many rows (and bytes) a `--top` limit hid,
/// so the caller can add the "more" line.
pub fn render_findings_view(findings: &[Finding], view: &TableView) -> (String, usize, u64) {
    let sorted = sort_findings(findings, view.sort);
    let (shown, hidden) = match view.top {
        Some(top) if sorted.len() > top => {
            let hidden_rows = &sorted[top..];
            let hidden_bytes = hidden_rows.iter().filter_map(|f| f.size_bytes).sum();
            (
                sorted[..top].to_vec(),
                Some((hidden_rows.len(), hidden_bytes)),
            )
        }
        _ => (sorted, None),
    };

    let mut out = String::new();
    match view.group {
        GroupKey::None => {
            out.push_str(&findings_table(&shown, view).to_string());
            out.push('\n');
        }
        GroupKey::Verdict => {
            for verdict in [Verdict::Never, Verdict::Risky, Verdict::Safe] {
                let rows: Vec<Finding> = shown
                    .iter()
                    .filter(|f| f.verdict == verdict)
                    .cloned()
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                out.push_str(&format!(
                    "── {} · {} finding(s) ──\n",
                    verdict.as_str(),
                    rows.len()
                ));
                out.push_str(&findings_table(&rows, view).to_string());
                out.push('\n');
            }
        }
        GroupKey::Category => {
            let mut categories: Vec<String> = shown.iter().map(|f| f.category.clone()).collect();
            categories.sort();
            categories.dedup();
            for category in categories {
                let rows: Vec<Finding> = shown
                    .iter()
                    .filter(|f| f.category == category)
                    .cloned()
                    .collect();
                out.push_str(&format!("── {} · {} finding(s) ──\n", category, rows.len()));
                out.push_str(&findings_table(&rows, view).to_string());
                out.push('\n');
            }
        }
    }

    // The "more" line is the caller's: sweep and scan word it differently.
    let hidden_count = hidden.map(|(c, _)| c).unwrap_or(0);
    let hidden_bytes = hidden.map(|(_, b)| b).unwrap_or(0);
    (out, hidden_count, hidden_bytes)
}

/// Full scan output: table(s) with limit note, summary line, and the verdict
/// byte-share bar.
pub fn render_scan_report(report: &ScanReport, view: &TableView) -> String {
    let mut out = String::new();
    if report.findings.is_empty() {
        out.push_str("No findings: nothing under the given paths matches any rule.\n");
    } else {
        let (table, hidden_count, hidden_bytes) = render_findings_view(&report.findings, view);
        out.push_str(&table);
        if hidden_count > 0 {
            out.push_str(&more_line(hidden_count, hidden_bytes));
        }
    }
    out.push_str(&render_summary(report));
    out.push_str(&render_verdict_bar(report, view.color));
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

/// One bar per verdict, sized by byte share (largest-remainder rounding, so
/// the blocks always add up to the full width). A bucket with measured bytes
/// always shows at least one block; buckets without any show none.
fn render_verdict_bar(report: &ScanReport, color: bool) -> String {
    const WIDTH: usize = 24;
    let s = &report.summary;
    let rows = [
        (
            "safe",
            s.safe,
            s.reclaimable_bytes,
            format_size(s.reclaimable_bytes),
        ),
        ("risky", s.risky, s.risky_bytes, format_size(s.risky_bytes)),
        (
            "never",
            s.never,
            s.never_bytes,
            format!("{} locked", format_size(s.never_bytes)),
        ),
    ];

    let total_bytes: u64 = s.reclaimable_bytes + s.risky_bytes + s.never_bytes;
    let mut blocks = [0usize; 3];
    let mut order: Vec<(u64, usize)> = rows
        .iter()
        .enumerate()
        .map(|(i, (_, _, b, _))| (*b, i))
        .collect();
    for (bytes, i) in &order {
        if total_bytes > 0 {
            blocks[*i] = (*bytes as f64 / total_bytes as f64 * WIDTH as f64).floor() as usize;
        }
    }
    // Largest remainder gets the leftover blocks, ties in row order.
    order.sort_by(|a, b| {
        let frac = |x: (u64, usize)| {
            if total_bytes == 0 {
                0.0
            } else {
                x.0 as f64 / total_bytes as f64 * WIDTH as f64
            }
        };
        let (fa, fb) = (frac(*a), frac(*b));
        let da = fa - fa.floor();
        let db = fb - fb.floor();
        db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut used: usize = blocks.iter().sum();
    let mut pick = 0usize;
    while used < WIDTH {
        let (_, i) = order[pick % order.len()];
        blocks[i] += 1;
        used += 1;
        pick += 1;
    }

    let mut out = String::from("\n");
    for (i, (label, count, bytes, size_text)) in rows.iter().enumerate() {
        let filled = blocks[i];
        let bar_plain = format!("{}{}", "█".repeat(filled), "░".repeat(WIDTH - filled));
        let bar = if color && *bytes > 0 {
            let (r, g, b) = verdict_rgb(match i {
                0 => Verdict::Safe,
                1 => Verdict::Risky,
                _ => Verdict::Never,
            });
            format!("\x1b[38;2;{r};{g};{b}m{bar_plain}\x1b[0m")
        } else {
            bar_plain
        };
        // Color the bar only; counts stay plain for copy-paste.
        out.push_str(&format!("  {label:<5} {bar} {count:>3} · {size_text}\n"));
    }
    out
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
    table
        .load_preset(presets::UTF8_FULL)
        .apply_modifier(modifiers::UTF8_ROUND_CORNERS);
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
            Cell::new(cap_reason(&m.reason)),
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

    #[test]
    fn shortens_home_paths() {
        let home = Path::new("/home/manak");
        assert_eq!(
            shorten_path(Path::new("/home/manak/.npm/_cacache"), Some(home)),
            "~/.npm/_cacache"
        );
        assert_eq!(shorten_path(Path::new("/home/manak"), Some(home)), "~");
        assert_eq!(
            shorten_path(Path::new("/etc/passwd"), Some(home)),
            "/etc/passwd"
        );
        assert_eq!(
            shorten_path(Path::new("/home/manak2/x"), Some(home)),
            "/home/manak2/x",
            "prefix must not match a different user's home"
        );
    }

    #[test]
    fn caps_long_reasons_at_word_boundary() {
        let short = "short reason";
        assert_eq!(cap_reason(short), short);
        let long = "word ".repeat(40);
        let capped = cap_reason(long.trim());
        assert!(capped.ends_with('…'));
        assert!(capped.chars().count() <= 73);
        assert!(!capped.ends_with("wor…"), "cut must land on a space");
    }
}
