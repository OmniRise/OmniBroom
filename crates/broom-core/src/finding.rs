use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::verdict::Verdict;

/// Where a finding came from. Filesystem findings are paths the walker saw;
/// facts findings come from an injected provider (docker, journalctl, git).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindingOrigin {
    Filesystem,
    Facts(String),
}

impl Serialize for FindingOrigin {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            FindingOrigin::Filesystem => serializer.serialize_str("filesystem"),
            FindingOrigin::Facts(provider) => {
                serializer.serialize_str(&format!("facts:{provider}"))
            }
        }
    }
}

/// One classified item: a path (or facts pseudo-path), what to do with it, and
/// the human-readable reason why. Every finding must carry a reason; that is
/// the whole product.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    #[serde(serialize_with = "ser_path")]
    pub path: PathBuf,
    pub verdict: Verdict,
    pub category: String,
    pub reason: String,
    pub rule_id: String,
    pub size_bytes: Option<u64>,
    pub origin: FindingOrigin,
}

fn ser_path<S: serde::Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&path.to_string_lossy())
}
