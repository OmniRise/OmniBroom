use std::path::PathBuf;

use thiserror::Error;

/// Hard failures of the engine itself. Per-entry walk problems and facts
/// provider hiccups become warnings on the report instead of aborting a scan.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("failed to walk {path}: {source}")]
    Walk {
        path: PathBuf,
        #[source]
        source: jwalk::Error,
    },
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
