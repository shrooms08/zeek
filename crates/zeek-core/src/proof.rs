//! Proof bundles: everything an independent verifier needs to recompute a
//! settlement with nothing but `zeek-core`. The format is documented in
//! `docs/proof-bundle.md`. This module lives in `zeek-core` so verifiers,
//! including the browser one, can use it without the engine.

use crate::beacon::{self, Beacon, Randomness};
use crate::digs::{self, ReceivedNote};
use crate::params::GameParams;
use crate::random;
use crate::rounds::{self, RoundWindow};
use crate::settle::{self, RoundOutcome, Settlement};
use crate::timing::{self, TimingVerdict};
use serde::{Deserialize, Serialize};

/// Newest bundle format version.
///
/// * Versions 1 to 3: the bundle version **is** the rules version of the
///   round: 1 (no dig fee), 2 (dig fee), 3 (dig fee, and dig memos may use
///   the text encoding from `params.text_memo_activation_height`).
/// * Version 4: the rules version is stated in `rules_version` (1 to 4;
///   rules v4 is Campfire, see `params.campfire`), and the bundle may carry
///   `payout_openings`, which make the payout transaction's amounts publicly
///   checkable.
///
/// In every version the parameters must give the round the stated rules
/// version.
pub const PROOF_VERSION: u32 = 4;

/// Newest rules version this library settles.
pub const RULES_VERSION: u32 = 4;
/// Oldest bundle version `recompute` still verifies.
pub const OLDEST_SUPPORTED_PROOF_VERSION: u32 = 1;

/// A self-contained description of one settled round.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofBundle {
    /// Bundle format version (see [`PROOF_VERSION`]).
    pub version: u32,
    /// Game parameters the round was played under (including the dig fee
    /// from version 2 on).
    pub params: GameParams,
    pub round: u32,
    pub window: RoundWindow,
    /// Every note received at the vault's external address inside the
    /// window, in chain order, with its full memo.
    pub notes: Vec<ReceivedNote>,
    /// `nTime` of the window's first block (fixes the bound drand round).
    pub start_block_time: u64,
    /// `nTime` of the `mtp_window` blocks after the window, in height order.
    pub post_close_block_times: Vec<u64>,
    /// The bound drand round.
    pub drand_round: u64,
    /// Publication time of that drand round.
    pub beacon_time: u64,
    /// The beacon, when the round was drawn (absent for voided rounds).
    pub beacon: Option<Beacon>,
    /// Verified randomness of the beacon.
    pub randomness: Option<Randomness>,
    pub timing: TimingVerdict,
    pub outcome: RoundOutcome,
    /// Cache balance before this round.
    pub previous_cache: u64,
    /// The settlement as the engine computed it.
    pub settlement: Settlement,
    /// Payout transaction record, once the keyholders signed and it was broadcast.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payout: Option<PayoutRecord>,
    /// Rules version of the round (format 4 on; earlier formats use `version`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules_version: Option<u32>,
    /// Openings of the payout transaction's payout outputs (format 4 on),
    /// once the transaction is mined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payout_openings: Option<PayoutOpenings>,
    /// Settled payouts that could not be paid because their sealed receiver
    /// does not open to a valid address (a malformed dig memo). Their value
    /// stayed in the vault. Only the sealing key's holders (the keyholders,
    /// each of whom checks it before signing) can confirm this.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unclaimable_payouts: Vec<UnclaimablePayout>,
}

/// A settled payout with no payable receiver.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnclaimablePayout {
    /// Index of the payout in `settlement.payouts`.
    pub payout_index: u32,
    /// Its net amount, which stayed in the vault.
    pub net: u64,
    /// Why it cannot be paid.
    pub reason: String,
}

impl ProofBundle {
    /// The rules version the bundle claims for its round.
    pub fn claimed_rules_version(&self) -> Option<u32> {
        if self.version <= 3 {
            Some(self.version)
        } else {
            self.rules_version
        }
    }
}

/// The minimal note openings that let anyone recompute the note commitment
/// (`cmx`) of each payout output in the payout transaction, and so check its
/// amount (Zcash protocol spec § 5.4.8.4, Sinsemilla commitments; § 4.16,
/// note commitment extraction).
///
/// An Orchard-protocol note is `(d, pk_d, v, rho, rseed)`. `rho` of an
/// output note is the nullifier of the same action's spent note (§ 4.7.3),
/// which is public in the transaction, so it is not repeated here.
/// Disclosed per output: the recipient (a one-time diversified address of
/// the winner's wallet), the value (already public in the settlement) and
/// `rseed`. Nothing here lets anyone derive the note's nullifier, which
/// needs the recipient's nullifier key, so a later spend of the note stays
/// unlinkable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayoutOpenings {
    /// The payout transaction (display hex).
    pub txid: String,
    /// Height it was mined at.
    pub mined_height: u32,
    /// One opening per settlement payout, in settlement order.
    pub outputs: Vec<PayoutOpening>,
}

/// The opening of one payout output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayoutOpening {
    /// Index of the payout in `settlement.payouts`.
    pub payout_index: u32,
    /// Index of the action in the transaction's Ironwood bundle.
    pub action_index: u32,
    /// Raw 43-byte Orchard-protocol address, hex.
    pub recipient: String,
    /// Note value (the payout's net amount).
    pub value: u64,
    /// The note's `rseed`, hex.
    pub rseed: String,
    /// Note plaintext version (2 Orchard, 3 Ironwood).
    pub note_version: u8,
}

/// State of a payout transaction for one round.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayoutState {
    /// A signing session was opened; the transaction may or may not have
    /// been broadcast (inspect before retrying).
    Signing,
    /// Broadcast and recorded in the vault wallet.
    Broadcast,
}

/// The engine's record of a round's payout transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayoutRecord {
    pub round: u32,
    pub state: PayoutState,
    /// Signing session id, hex.
    pub session: String,
    /// Transaction id once broadcast (display hex).
    pub txid: Option<String>,
    /// Chain tip when the record was last updated.
    pub height: u32,
    /// Fee of the payout transaction.
    pub fee: u64,
    /// Keyholders that signed.
    pub signers: Vec<u16>,
}

/// Errors from recomputing a bundle.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProofError {
    #[error("unsupported proof bundle version {0}")]
    Version(u32),
    #[error("bundle claims rules version {bundle}, but its parameters give this round rules version {rules} (dig_fee_zat = {dig_fee})")]
    RulesVersion {
        bundle: u32,
        rules: u32,
        dig_fee: u64,
    },
    #[error("a format 4 bundle must state its rules version")]
    MissingRulesVersion,
    #[error("window does not match the parameters: expected {expected:?}, got {got:?}")]
    Window {
        expected: RoundWindow,
        got: RoundWindow,
    },
    #[error("bound drand round mismatch: expected {expected}, bundle says {got}")]
    DrandRound { expected: u64, got: u64 },
    #[error("beacon time mismatch: expected {expected}, bundle says {got}")]
    BeaconTime { expected: u64, got: u64 },
    #[error("timing verdict mismatch: recomputed {recomputed:?}, bundle says {got:?}")]
    Timing {
        recomputed: TimingVerdict,
        got: TimingVerdict,
    },
    #[error("beacon is for drand round {0}, not the bound round")]
    BeaconRound(u64),
    #[error("a drawn round needs a beacon")]
    MissingBeacon,
    #[error("randomness mismatch")]
    Randomness,
    #[error("outcome mismatch: recomputed {recomputed:?}, bundle says {got:?}")]
    Outcome {
        recomputed: RoundOutcome,
        got: RoundOutcome,
    },
    #[error("a note lies outside the window: height {0}")]
    NoteOutsideWindow(u32),
    #[error(transparent)]
    Rounds(#[from] rounds::RoundError),
    #[error(transparent)]
    Beacon(#[from] beacon::BeaconError),
    #[error(transparent)]
    Timing2(#[from] timing::TimingError),
    #[error(transparent)]
    Random(#[from] random::RandomError),
    #[error(transparent)]
    Settle(#[from] settle::SettleError),
}

/// Recomputes the settlement of `bundle` from its inputs using only
/// `zeek-core`, checking every intermediate claim the bundle makes. Returns
/// the recomputed settlement; the caller compares it with `bundle.settlement`.
pub fn recompute(bundle: &ProofBundle) -> Result<Settlement, ProofError> {
    if bundle.version < OLDEST_SUPPORTED_PROOF_VERSION || bundle.version > PROOF_VERSION {
        return Err(ProofError::Version(bundle.version));
    }
    let params = &bundle.params;
    let rules = rounds::rules_version(params, bundle.round)?;
    let claimed = bundle
        .claimed_rules_version()
        .ok_or(ProofError::MissingRulesVersion)?;
    if rules != claimed || claimed > RULES_VERSION {
        return Err(ProofError::RulesVersion {
            bundle: claimed,
            rules,
            dig_fee: params.dig_fee_zat,
        });
    }
    let window = rounds::round_window(params, bundle.round)?;
    if window != bundle.window {
        return Err(ProofError::Window {
            expected: window,
            got: bundle.window,
        });
    }
    for note in &bundle.notes {
        if !window.contains(note.height) {
            return Err(ProofError::NoteOutsideWindow(note.height));
        }
    }

    let drand_round = beacon::beacon_round_for(params, bundle.start_block_time)?;
    if drand_round != bundle.drand_round {
        return Err(ProofError::DrandRound {
            expected: drand_round,
            got: bundle.drand_round,
        });
    }
    let beacon_time = beacon::beacon_time(params, drand_round)?;
    if beacon_time != bundle.beacon_time {
        return Err(ProofError::BeaconTime {
            expected: beacon_time,
            got: bundle.beacon_time,
        });
    }

    let verdict = timing::check_window_close(params, beacon_time, &bundle.post_close_block_times)?;
    if verdict != bundle.timing {
        return Err(ProofError::Timing {
            recomputed: verdict,
            got: bundle.timing,
        });
    }

    let outcome = match verdict {
        TimingVerdict::Unsafe { .. } => RoundOutcome::Voided,
        TimingVerdict::Safe { .. } => {
            let beacon = bundle.beacon.as_ref().ok_or(ProofError::MissingBeacon)?;
            if beacon.round != drand_round {
                return Err(ProofError::BeaconRound(beacon.round));
            }
            let randomness = beacon::verify_beacon(&params.drand, beacon)?;
            if bundle.randomness != Some(randomness) {
                return Err(ProofError::Randomness);
            }
            RoundOutcome::Drawn {
                treasure_tile: random::treasure_tile(bundle.round, &randomness)?,
                cache_triggered: random::cache_triggers(params, bundle.round, &randomness)?,
            }
        }
    };
    if outcome != bundle.outcome {
        return Err(ProofError::Outcome {
            recomputed: outcome,
            got: bundle.outcome,
        });
    }

    let voided = matches!(outcome, RoundOutcome::Voided);
    let classified =
        digs::classify_round(params, bundle.round, voided, bundle.notes.iter().cloned())?;
    Ok(settle::settle(
        params,
        bundle.round,
        &classified,
        bundle.previous_cache,
        outcome,
    )?)
}
