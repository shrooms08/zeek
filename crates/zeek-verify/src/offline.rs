//! What can be checked about a round with the Zeek engine and relay off:
//! from the proof bundle alone, plus the beacon as a public drand relay
//! serves it (fetched by the caller, the browser's Replay mode).
//!
//! Each part says exactly what it did. A part that needs chain data (block
//! times, the window's transactions, the mined payout transaction) is
//! `NeedsChain` and was not performed: there is no tick for it. No public
//! source of testnet chain data can be read from a browser on another site
//! (lightwalletd speaks gRPC without CORS), so in the browser those parts
//! need the live engine's relay.
//!
//! | Check      | Offline                                                   |
//! |------------|-----------------------------------------------------------|
//! | Beacon     | rules match, BLS signature, same as drand's public copy,  |
//! |            | the draw follows; the bound round needs the chain         |
//! | Digs       | needs the chain                                           |
//! | Timing     | needs the chain                                           |
//! | Settlement | recomputed in full (same as online)                       |
//! | Payout     | openings agree with the settlement; the transaction needs |
//! |            | the chain                                                 |

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use zeek_core::beacon::{self, Beacon};
use zeek_core::proof::ProofBundle;
use zeek_core::random;
use zeek_core::settle::RoundOutcome;

use crate::checks::{check_settlement, params_agree, unclaimable_payouts, PublicConfig};
use crate::report::{CheckId, CheckStatus};

/// Outcome of one offline part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartStatus {
    Pass,
    Fail,
    /// Cannot apply to this round (for example no payout to open).
    NotAvailable,
    /// Not performed: it needs chain data.
    NeedsChain,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfflinePart {
    pub check: CheckId,
    /// Short name of the part (stable, for tests and the UI).
    pub part: String,
    pub status: PartStatus,
    pub summary: String,
    pub detail: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfflineReport {
    pub round: u32,
    pub parts: Vec<OfflinePart>,
}

/// The beacon as a public drand relay served it, or why there is none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublicBeacon {
    Fetched { beacon: Beacon, source: String },
    Unreachable { reason: String },
}

fn part(check: CheckId, name: &str, status: PartStatus, summary: impl Into<String>) -> OfflinePart {
    OfflinePart {
        check,
        part: name.to_owned(),
        status,
        summary: summary.into(),
        detail: Vec::new(),
    }
}

impl OfflinePart {
    fn with(mut self, line: impl Into<String>) -> Self {
        self.detail.push(line.into());
        self
    }
}

/// Runs every part that needs no chain data. `public` is the beacon of the
/// bundle's drand round as a public relay served it (None for a voided
/// round, which has no beacon).
pub fn verify_offline(
    bundle: &ProofBundle,
    config: &PublicConfig,
    public: Option<&PublicBeacon>,
) -> OfflineReport {
    let mut parts = beacon_parts(bundle, config, public);
    parts.push(part(
        CheckId::Digs,
        "chain",
        PartStatus::NeedsChain,
        "Needs the chain: every payment in the round's blocks must be found with the public viewing key.",
    ));
    parts.push(part(
        CheckId::Timing,
        "chain",
        PartStatus::NeedsChain,
        "Needs the chain: the times of the blocks after the round decide whether it closed in time.",
    ));
    let settlement = check_settlement(bundle);
    let status = match settlement.status {
        CheckStatus::Pass | CheckStatus::Warn => PartStatus::Pass,
        CheckStatus::Fail => PartStatus::Fail,
        CheckStatus::NotAvailable => PartStatus::NotAvailable,
    };
    let mut s = part(CheckId::Settlement, "recompute", status, settlement.summary);
    s.detail = settlement.detail;
    parts.push(s);
    parts.push(payout_openings(bundle));
    parts.push(part(
        CheckId::Payout,
        "chain",
        PartStatus::NeedsChain,
        "Needs the chain: that the payout transaction is mined, and its proof and signatures.",
    ));
    OfflineReport {
        round: bundle.round,
        parts,
    }
}

fn beacon_parts(
    bundle: &ProofBundle,
    config: &PublicConfig,
    public: Option<&PublicBeacon>,
) -> Vec<OfflinePart> {
    let id = CheckId::Beacon;
    let params = &bundle.params;
    let mut out = Vec::new();
    match params_agree(params, &config.params) {
        Ok(()) => out.push(part(
            id,
            "rules",
            PartStatus::Pass,
            "The bundle's game rules match the published configuration.",
        )),
        Err(e) => {
            out.push(
                part(
                    id,
                    "rules",
                    PartStatus::Fail,
                    "The bundle's game rules differ from the published configuration.",
                )
                .with(e),
            );
            return out;
        }
    }
    let binding = part(
        id,
        "binding",
        PartStatus::NeedsChain,
        "Needs the chain: which drand round the rules bind follows from the time of the round's first block.",
    );
    let RoundOutcome::Drawn {
        treasure_tile,
        cache_triggered,
    } = bundle.outcome
    else {
        out.push(part(
            id,
            "signature",
            PartStatus::NotAvailable,
            "The round was voided, so no beacon decides it.",
        ));
        out.push(binding);
        return out;
    };
    let Some(b) = &bundle.beacon else {
        out.push(part(
            id,
            "signature",
            PartStatus::Fail,
            "A drawn round must include its beacon.",
        ));
        return out;
    };
    if b.round != bundle.drand_round {
        out.push(
            part(
                id,
                "signature",
                PartStatus::Fail,
                "The beacon is for a different drand round than the bundle names.",
            )
            .with(format!(
                "beacon round {}, bundle round {}",
                b.round, bundle.drand_round
            )),
        );
        return out;
    }
    let randomness = match beacon::verify_beacon(&params.drand, b) {
        Ok(r) if bundle.randomness == Some(r) => {
            out.push(
                part(
                    id,
                    "signature",
                    PartStatus::Pass,
                    format!("drand quicknet round {} is genuine: its BLS signature verifies against the quicknet public key.", b.round),
                )
                .with("verified in this browser, from the bundle alone"),
            );
            r
        }
        Ok(_) => {
            out.push(part(
                id,
                "signature",
                PartStatus::Fail,
                "The bundle's randomness does not come from this beacon.",
            ));
            return out;
        }
        Err(e) => {
            out.push(
                part(
                    id,
                    "signature",
                    PartStatus::Fail,
                    "The beacon's BLS signature does not verify against drand quicknet.",
                )
                .with(e.to_string()),
            );
            return out;
        }
    };
    out.push(match public {
        Some(PublicBeacon::Fetched { beacon, source }) if beacon == b => part(
            id,
            "public",
            PartStatus::Pass,
            "drand's own public copy of this beacon is the same.",
        )
        .with(format!("{source}: round {}", beacon.round)),
        Some(PublicBeacon::Fetched { beacon, source }) => part(
            id,
            "public",
            PartStatus::Fail,
            "drand's public copy of this round differs from the bundle's beacon.",
        )
        .with(format!(
            "{source}: round {} signature {}",
            beacon.round, beacon.signature
        )),
        Some(PublicBeacon::Unreachable { reason }) => part(
            id,
            "public",
            PartStatus::NotAvailable,
            "drand's public relays could not be reached, so its copy was not compared.",
        )
        .with(reason.clone()),
        None => part(
            id,
            "public",
            PartStatus::NotAvailable,
            "drand's public copy was not fetched.",
        ),
    });
    let tile = random::treasure_tile(bundle.round, &randomness);
    let cache = random::cache_triggers(params, bundle.round, &randomness);
    out.push(match (tile, cache) {
        (Ok(t), Ok(c)) if t == treasure_tile && c == cache_triggered => part(
            id,
            "draw",
            PartStatus::Pass,
            format!(
                "The beacon puts the treasure on tile {} and the Deep Cache {}, as published.",
                t + 1,
                if c { "breaks open" } else { "stays shut" }
            ),
        ),
        (Ok(t), Ok(c)) => part(
            id,
            "draw",
            PartStatus::Fail,
            "The published draw does not follow from the beacon.",
        )
        .with(format!(
            "recomputed tile {t} cache {c}; bundle tile {treasure_tile} cache {cache_triggered}"
        )),
        (Err(e), _) | (_, Err(e)) => part(
            id,
            "draw",
            PartStatus::Fail,
            "The draw cannot be recomputed.",
        )
        .with(e.to_string()),
    });
    out.push(binding);
    out
}

/// The payout openings against the settlement (no transaction needed):
/// one opening per payable payout, with the settled amount, none twice.
fn payout_openings(bundle: &ProofBundle) -> OfflinePart {
    let id = CheckId::Payout;
    let settlement = &bundle.settlement;
    let (unclaimable, notes) = match unclaimable_payouts(bundle) {
        Ok(v) => v,
        Err(why) => return part(id, "openings", PartStatus::Fail, why),
    };
    let Some(openings) = &bundle.payout_openings else {
        let why = if settlement.payouts.is_empty() {
            "Nothing to pay this round, so there is nothing to open."
        } else if bundle.version < 4 {
            "This bundle format has no payout openings."
        } else {
            "The payout transaction was not mined yet when this round was saved."
        };
        return part(id, "openings", PartStatus::NotAvailable, why);
    };
    if let Some(record) = &bundle.payout {
        if record.txid.as_deref() != Some(openings.txid.as_str()) {
            return part(
                id,
                "openings",
                PartStatus::Fail,
                "The openings name a different transaction than the payout record.",
            );
        }
    }
    let mut covered = BTreeSet::new();
    for opening in &openings.outputs {
        let Some(payout) = settlement.payouts.get(opening.payout_index as usize) else {
            return part(
                id,
                "openings",
                PartStatus::Fail,
                "An opening names a payout the settlement does not have.",
            )
            .with(format!("payout index {}", opening.payout_index));
        };
        if !covered.insert(opening.payout_index) || unclaimable.contains(&opening.payout_index) {
            return part(
                id,
                "openings",
                PartStatus::Fail,
                "Two openings claim the same payout.",
            );
        }
        if opening.value != payout.net {
            return part(
                id,
                "openings",
                PartStatus::Fail,
                "An opening's amount differs from the settled amount.",
            )
            .with(format!(
                "payout {}: opening {}, settlement {}",
                opening.payout_index, opening.value, payout.net
            ));
        }
    }
    if covered.len() + unclaimable.len() != settlement.payouts.len() {
        return part(
            id,
            "openings",
            PartStatus::Fail,
            "Some settled payouts have no opening.",
        )
        .with(format!(
            "{} payouts settled, {} opened",
            settlement.payouts.len(),
            covered.len()
        ));
    }
    let mut result = part(
        id,
        "openings",
        PartStatus::Pass,
        format!(
            "Every payable settled amount has an opening with exactly that amount ({} output(s)).",
            covered.len()
        ),
    )
    .with(format!(
        "transaction {} at height {}",
        openings.txid, openings.mined_height
    ));
    for note in notes {
        result = result.with(note);
    }
    result
}
