//! Proof bundles: everything an independent verifier needs to recompute a
//! settlement with nothing but `zeek-core`. The format is documented in
//! `docs/proof-bundle.md`.

use serde::{Deserialize, Serialize};
use zeek_core::beacon::{self, Beacon, Randomness};
use zeek_core::digs::{self, ReceivedNote};
use zeek_core::params::GameParams;
use zeek_core::random;
use zeek_core::rounds::{self, RoundWindow};
use zeek_core::settle::{self, RoundOutcome, Settlement};
use zeek_core::timing::{self, TimingVerdict};

/// Bundle format version.
pub const PROOF_VERSION: u32 = 1;

/// A self-contained description of one settled round.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofBundle {
    pub version: u32,
    /// Game parameters the round was played under.
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
}

/// Errors from recomputing a bundle.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProofError {
    #[error("unsupported proof bundle version {0}")]
    Version(u32),
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
    if bundle.version != PROOF_VERSION {
        return Err(ProofError::Version(bundle.version));
    }
    let params = &bundle.params;
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
