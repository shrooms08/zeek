//! Verdicts.

use serde::{Deserialize, Serialize};

/// The five round checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckId {
    Beacon,
    Digs,
    Timing,
    Settlement,
    Payout,
}

impl CheckId {
    /// Short human title.
    pub fn title(self) -> &'static str {
        match self {
            CheckId::Beacon => "Randomness beacon",
            CheckId::Digs => "Digs complete and correct",
            CheckId::Timing => "Timing rule",
            CheckId::Settlement => "Settlement",
            CheckId::Payout => "Payout transaction",
        }
    }
}

/// Outcome of one check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    /// Passed, but part of the result rests on a claim the verifier cannot
    /// check itself (for example a payout left unpaid as unclaimable).
    Warn,
    Fail,
    /// The check cannot run for this bundle (for example check 5 for a
    /// bundle format without payout openings, or a payout not mined yet).
    NotAvailable,
}

/// One check's verdict: a sentence for people and the technical detail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub id: CheckId,
    pub title: String,
    pub status: CheckStatus,
    /// Plain-language explanation.
    pub summary: String,
    /// Technical detail (values compared, heights, hashes).
    pub detail: Vec<String>,
}

impl CheckResult {
    pub(crate) fn new(id: CheckId, status: CheckStatus, summary: impl Into<String>) -> Self {
        CheckResult {
            id,
            title: id.title().to_owned(),
            status,
            summary: summary.into(),
            detail: Vec::new(),
        }
    }

    pub(crate) fn with(mut self, line: impl Into<String>) -> Self {
        self.detail.push(line.into());
        self
    }
}

/// Every check of one round.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundReport {
    pub round: u32,
    pub bundle_version: u32,
    pub rules_version: Option<u32>,
    pub checks: Vec<CheckResult>,
}

impl RoundReport {
    /// True when no check failed (checks that are not available do not fail).
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.status != CheckStatus::Fail)
    }
}
