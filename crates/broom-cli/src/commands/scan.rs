//! `omni broom scan`: classified findings with size, category, verdict, and
//! reason. `--json` swaps the table for the machine-readable report.

use anyhow::Result;

use crate::args::ScanArgs;
use crate::output;

pub fn run(args: ScanArgs) -> Result<()> {
    let report = super::scan_with_args(&args)?;

    if args.json {
        println!("{}", output::render_json(&report)?);
        // Warnings still go to stderr so piping JSON stays clean.
        if let Some(warnings) = output::render_warnings(&report) {
            eprint!("{warnings}");
        }
        return Ok(());
    }

    let color = output::colors_enabled();
    print!("{}", output::render_scan_report(&report, color));
    if let Some(warnings) = output::render_warnings(&report) {
        eprint!("{warnings}");
    }
    Ok(())
}
