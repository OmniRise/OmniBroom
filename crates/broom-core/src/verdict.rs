use std::fmt;

use serde::{Deserialize, Serialize};

/// How safe a finding is to delete. Ordering is severity order: `Safe` is the
/// least severe, `Never` the most, so `max()` on a set of verdicts gives the
/// verdict a path deserves overall.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// Deleting costs nothing of value: caches and rebuildable artifacts.
    Safe,
    /// Deleting may cost something: verify before you sweep.
    Risky,
    /// Never delete by hand. Data loss or corruption is the likely outcome.
    Never,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Safe => "safe",
            Verdict::Risky => "risky",
            Verdict::Never => "never",
        }
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_orders_by_severity() {
        assert!(Verdict::Safe < Verdict::Risky);
        assert!(Verdict::Risky < Verdict::Never);
        let verdicts = [Verdict::Safe, Verdict::Never, Verdict::Risky];
        assert_eq!(verdicts.into_iter().max(), Some(Verdict::Never));
    }

    #[test]
    fn verdict_json_is_lowercase() {
        assert_eq!(serde_json::to_string(&Verdict::Risky).unwrap(), "\"risky\"");
        let back: Verdict = serde_json::from_str("\"never\"").unwrap();
        assert_eq!(back, Verdict::Never);
    }
}
