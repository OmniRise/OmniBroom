//! `omni broom why <path>`: explain any verdict. Runs the same rule packs
//! and facts providers as scan, scoped to one path.

use std::path::PathBuf;

use anyhow::Result;

use crate::output;

pub fn run(path: PathBuf) -> Result<()> {
    let engine = super::build_engine(false, None)?;
    let report = engine.why(&path);
    let color = output::colors_enabled();
    print!("{}", output::render_why(&report, color));
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}
