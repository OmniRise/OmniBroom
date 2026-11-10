//! `omnibroom scan`: classified findings with size, category, verdict, and
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

    let view = super::scan_view(args.top, args.all, args.sort, args.group_by);
    print!("{}", output::render_scan_report(&report, &view));
    if let Some(warnings) = output::render_warnings(&report) {
        eprint!("{warnings}");
    }
    Ok(())
}
