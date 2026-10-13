//! The timing rule: a round is voided unless the chain proves its window
//! closed before its beacon was published.
//!
//! # Timing safety argument
//!
//! **Threat.** A dig only counts if it is in a block inside the round's window
//! `[start, end)`. Nobody (miner or player) may be able to get a dig counted
//! after the beacon bound to the round is known, otherwise they could dig the
//! treasure tile with certainty. The beacon for drand round `n` is published
//! at `T_n = genesis_time + (n − 1)·period` (drand `common/time.go`), so what
//! must hold is: every block of the window was mined before `T_n`.
//!
//! **What block timestamps can and cannot tell us.** Zcash consensus (protocol
//! spec § 7.6) requires each block's `nTime` to be strictly greater than the
//! median-time-past of the previous `PoWMedianBlockSpan = 11` blocks and at
//! most 90·60 s above it; full validators additionally refuse blocks whose
//! `nTime` is more than two hours in the future of their clock. A single
//! miner may therefore set a timestamp well *before* the real time (anything
//! above the median) but cannot push it far into the future. A block's own
//! timestamp thus never proves the block was mined early; it can only be used
//! to show a block was mined *late*.
//!
//! **Assumptions.**
//!
//! 1. Honest-majority timestamps: among any `mtp_window = 11` consecutive
//!    blocks, at least 6 were mined by honest miners whose `nTime` is within
//!    `honest_clock_skew_secs` (δ) of real time. (An adversary controlling 6 of
//!    11 consecutive blocks can defeat this rule; that adversary could also
//!    reorganise the chain, which no timestamp rule survives.)
//! 2. The caller feeds the `nTime` values of the blocks at heights
//!    `end, end + 1, …, end + 10` of the canonical chain, in order.
//! 3. drand publishes round `n` no earlier than `T_n` (the League of Entropy's
//!    threshold network cannot sign a round early; see the drand security
//!    model).
//!
//! **Argument.** Let `L` be the last block of the window and `M` the median of
//! the 11 timestamps after the window. All 11 blocks descend from `L`, so each
//! was mined at a real time `≥ real(L)`. By assumption 1 at least 6 of them
//! carry timestamps `≥ real time − δ ≥ real(L) − δ`; the median of 11 values
//! is the 6th smallest, so `M ≥ real(L) − δ`, i.e. `real(L) ≤ M + δ`. Hence if
//! `M + δ < T_n`, every window block was mined before the beacon existed, and
//! no dig could have been placed with knowledge of it. If the inequality does
//! not hold the chain does not prove safety, and the round is **voided**:
//! every decodable dig is refunded and nothing is drawn.
//!
//! **Why this is not griefable into theft.** An adversary can lower its own
//! timestamps (down to the median) but cannot lower honest ones, so it cannot
//! make `M` small while the real close was late; the only effect of timestamp
//! games is to void rounds. An adversary can also delay the beacon-relative
//! schedule only through the start block's timestamp (bounded above by
//! median + 90 min), which merely postpones the reveal.
//!
//! The reveal delay in [`GameParams`](crate::params::GameParams) is sized so
//! that an honest chain running at up to twice the target spacing passes this
//! rule; see `GameParams::minimum_reveal_delay`.

use serde::{Deserialize, Serialize};

use crate::params::GameParams;

/// Errors from the timing rule.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TimingError {
    #[error("the timing rule needs the timestamps of {needed} blocks after the window, got {got}")]
    TooFewBlocks { needed: u32, got: usize },
    #[error("arithmetic overflow in timing check")]
    Overflow,
}

/// Outcome of the timing rule for one round.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum TimingVerdict {
    /// The window provably closed before the beacon: `median + skew < beacon_time`.
    Safe {
        /// Median of the post-close block timestamps.
        median_time_past: u64,
        /// Publication time of the bound beacon.
        beacon_time: u64,
        /// `beacon_time − (median + skew)`.
        margin_secs: u64,
    },
    /// Not provable; the round must be voided.
    Unsafe {
        /// Median of the post-close block timestamps.
        median_time_past: u64,
        /// Publication time of the bound beacon.
        beacon_time: u64,
        /// How much earlier the beacon would have had to be, in seconds.
        deficit_secs: u64,
    },
}

/// Median as the Zcash spec defines it (§ 7.7.3:
/// `median(S) := sorted(S)[ceiling((length(S)+1)/2)]`, 1-based, "the element
/// that begins the second half" for even lengths): index `len / 2` of the
/// sorted list, i.e. the 6th smallest of 11.
pub fn median_time_past(times: &[u64]) -> Option<u64> {
    if times.is_empty() {
        return None;
    }
    let mut sorted = times.to_vec();
    sorted.sort_unstable();
    sorted.get(sorted.len() / 2).copied()
}

/// Applies the timing rule to the `nTime` values of the blocks that follow the
/// window (at least `mtp_window` of them, in height order).
pub fn check_window_close(
    params: &GameParams,
    beacon_time: u64,
    post_close_block_times: &[u64],
) -> Result<TimingVerdict, TimingError> {
    let needed = params.mtp_window as usize;
    if post_close_block_times.len() < needed || needed == 0 {
        return Err(TimingError::TooFewBlocks {
            needed: params.mtp_window,
            got: post_close_block_times.len(),
        });
    }
    let median =
        median_time_past(&post_close_block_times[..needed]).ok_or(TimingError::TooFewBlocks {
            needed: params.mtp_window,
            got: 0,
        })?;
    let bound = median
        .checked_add(params.honest_clock_skew_secs)
        .ok_or(TimingError::Overflow)?;
    Ok(if bound < beacon_time {
        TimingVerdict::Safe {
            median_time_past: median,
            beacon_time,
            margin_secs: beacon_time - bound,
        }
    } else {
        TimingVerdict::Unsafe {
            median_time_past: median,
            beacon_time,
            deficit_secs: bound - beacon_time + 1,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon;

    fn params() -> GameParams {
        GameParams::testnet(1)
    }

    /// Honest timestamps: real time, blocks every `spacing` seconds after the close.
    fn honest_times(close: u64, spacing: u64, n: usize) -> Vec<u64> {
        (0..n as u64).map(|i| close + spacing * (i + 1)).collect()
    }

    fn beacon_time_for_start(p: &GameParams, start: u64) -> u64 {
        let r = beacon::beacon_round_for(p, start).expect("round");
        beacon::beacon_time(p, r).expect("time")
    }

    #[test]
    fn median_matches_bitcoin_convention() {
        assert_eq!(median_time_past(&[]), None);
        assert_eq!(median_time_past(&[5]), Some(5));
        assert_eq!(median_time_past(&[9, 1, 5]), Some(5));
        assert_eq!(median_time_past(&[4, 1, 3, 2]), Some(3));
        let eleven: Vec<u64> = (1..=11).rev().collect();
        assert_eq!(median_time_past(&eleven), Some(6));
    }

    #[test]
    fn honest_chain_on_schedule_is_safe() {
        let p = params();
        let start = p.drand.genesis_time + 100_000;
        let close = start + 600;
        let t_beacon = beacon_time_for_start(&p, start);
        let v = check_window_close(&p, t_beacon, &honest_times(close, 25, 11)).expect("verdict");
        match v {
            TimingVerdict::Safe { margin_secs, .. } => assert!(margin_secs > 0),
            other => panic!("expected safe, got {other:?}"),
        }
        // Even at twice the target spacing the reveal delay still covers it.
        assert!(matches!(
            check_window_close(&p, t_beacon, &honest_times(close, 50, 11)).expect("verdict"),
            TimingVerdict::Safe { .. }
        ));
    }

    #[test]
    fn slightly_late_honest_chain_is_voided() {
        let p = params();
        let start = p.drand.genesis_time + 100_000;
        let t_beacon = beacon_time_for_start(&p, start);
        // The chain stalls: the window closes 12 minutes late and blocks then
        // come at the normal pace. The post-close median lands inside the
        // skew bound of the beacon, so safety is not provable.
        let close = start + 600 + 720;
        let v = check_window_close(&p, t_beacon, &honest_times(close, 25, 11)).expect("verdict");
        assert!(matches!(v, TimingVerdict::Unsafe { .. }), "{v:?}");
    }

    #[test]
    fn adversarial_low_timestamps_cannot_rescue_a_late_window() {
        let p = params();
        let start = p.drand.genesis_time + 100_000;
        let t_beacon = beacon_time_for_start(&p, start);
        // Real close is after the beacon. Five adversarial blocks lie with the
        // smallest timestamps consensus allows; six honest blocks tell the truth.
        let real_close = t_beacon + 60;
        let mut times = honest_times(real_close, 25, 11);
        for t in times.iter_mut().take(5) {
            *t = start + 601; // just above the window's own timestamps
        }
        let v = check_window_close(&p, t_beacon, &times).expect("verdict");
        assert!(matches!(v, TimingVerdict::Unsafe { .. }), "{v:?}");
        // With six liars (assumption 1 broken) the rule can be fooled, which is
        // exactly the documented limit of the argument.
        for t in times.iter_mut().take(6) {
            *t = start + 601;
        }
        assert!(matches!(
            check_window_close(&p, t_beacon, &times).expect("verdict"),
            TimingVerdict::Safe { .. }
        ));
    }

    #[test]
    fn needs_a_full_window_of_timestamps() {
        let p = params();
        assert_eq!(
            check_window_close(&p, 1, &[1, 2, 3]),
            Err(TimingError::TooFewBlocks { needed: 11, got: 3 })
        );
        assert_eq!(
            check_window_close(&p, 1, &[u64::MAX; 11]),
            Err(TimingError::Overflow)
        );
    }
}
