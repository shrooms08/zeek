//! Round arithmetic and status.
//!
//! Round `r` covers block heights
//! `[genesis_height + r·blocks_per_round, genesis_height + (r+1)·blocks_per_round)`.

use serde::{Deserialize, Serialize};

use crate::beacon::{self, BeaconError};
use crate::params::GameParams;
use crate::timing::{self, TimingError, TimingVerdict};

/// Errors from round arithmetic.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RoundError {
    #[error("round {0} overflows the block height range")]
    Overflow(u32),
    #[error(transparent)]
    Beacon(#[from] BeaconError),
    #[error(transparent)]
    Timing(#[from] TimingError),
}

/// Half-open block height window of a round: `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundWindow {
    /// First block of the round.
    pub start: u32,
    /// First block after the round.
    pub end: u32,
}

impl RoundWindow {
    /// Whether `height` is inside the window.
    pub fn contains(&self, height: u32) -> bool {
        height >= self.start && height < self.end
    }

    /// Last block of the round.
    pub fn last(&self) -> u32 {
        self.end - 1
    }
}

/// The round that contains `height`, or `None` before the game's genesis.
pub fn round_for_height(params: &GameParams, height: u32) -> Option<u32> {
    if params.blocks_per_round == 0 {
        return None;
    }
    height
        .checked_sub(params.genesis_height)
        .map(|offset| offset / params.blocks_per_round)
}

/// The rules version `round` is played under: 4 once any block of its window
/// is at or above the Campfire activation height, 3 once any block is at or
/// above the text-memo activation height, otherwise the fee rules version of
/// the parameters (1 or 2). Earlier rounds keep their versions.
pub fn rules_version(params: &GameParams, round: u32) -> Result<u32, RoundError> {
    let window = round_window(params, round)?;
    Ok(if params.campfire_active(window.last()) {
        4
    } else if params.text_memo_active(window.last()) {
        3
    } else {
        params.rules_version()
    })
}

/// The block window of round `round`.
pub fn round_window(params: &GameParams, round: u32) -> Result<RoundWindow, RoundError> {
    let overflow = || RoundError::Overflow(round);
    let start = round
        .checked_mul(params.blocks_per_round)
        .and_then(|o| o.checked_add(params.genesis_height))
        .ok_or_else(overflow)?;
    let end = start
        .checked_add(params.blocks_per_round)
        .ok_or_else(overflow)?;
    Ok(RoundWindow { start, end })
}

/// Lifecycle of a round.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundStatus {
    /// Digs are being accepted: the chain tip is inside the window.
    Open,
    /// The window has closed but the `mtp_window` blocks after it, whose
    /// timestamps decide the timing rule, are not all mined yet.
    Closing,
    /// The timing rule passed: the beacon bound to this round, once
    /// published, reveals the treasure.
    Revealable,
    /// The round was settled (payouts computed).
    Settled,
    /// The timing rule failed: every decodable dig is refunded, nothing is
    /// drawn. This is final; a voided round is never `Settled`.
    Voided,
}

/// What the caller knows about a round's surroundings on chain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundContext {
    /// Current chain tip.
    pub tip_height: u32,
    /// `nTime` of the round's first block, once mined.
    pub start_block_time: Option<u64>,
    /// `nTime` of the blocks at heights `end, end+1, …`, in height order, as
    /// many as are mined.
    pub post_close_block_times: Vec<u64>,
    /// Whether this round's settlement has been applied.
    pub settled: bool,
}

/// Computes the round's status from its on-chain context.
pub fn round_status(
    params: &GameParams,
    round: u32,
    ctx: &RoundContext,
) -> Result<RoundStatus, RoundError> {
    let window = round_window(params, round)?;
    if ctx.tip_height < window.end {
        return Ok(RoundStatus::Open);
    }
    let needed = params.mtp_window as usize;
    let (Some(start_time), true) = (
        ctx.start_block_time,
        ctx.post_close_block_times.len() >= needed,
    ) else {
        return Ok(RoundStatus::Closing);
    };
    let drand_round = beacon::beacon_round_for(params, start_time)?;
    let beacon_time = beacon::beacon_time(params, drand_round)?;
    match timing::check_window_close(params, beacon_time, &ctx.post_close_block_times)? {
        TimingVerdict::Unsafe { .. } => Ok(RoundStatus::Voided),
        TimingVerdict::Safe { .. } if ctx.settled => Ok(RoundStatus::Settled),
        TimingVerdict::Safe { .. } => Ok(RoundStatus::Revealable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> GameParams {
        GameParams::testnet(1_000)
    }

    #[test]
    fn rules_version_switches_at_the_activation_round() {
        let mut p = params();
        assert_eq!(rules_version(&p, 0).expect("v"), 2);
        let w1 = round_window(&p, 1).expect("window");
        // Activation in the middle of round 1: rounds 1 and later are v3.
        p.text_memo_activation_height = Some(w1.start + 5);
        assert_eq!(rules_version(&p, 0).expect("v"), 2);
        assert_eq!(rules_version(&p, 1).expect("v"), 3);
        assert_eq!(rules_version(&p, 9).expect("v"), 3);
        p.dig_fee_zat = 0;
        p.text_memo_activation_height = None;
        assert_eq!(rules_version(&p, 9).expect("v"), 1);
    }

    #[test]
    fn windows_tile_the_chain_from_genesis() {
        let p = params();
        assert_eq!(round_for_height(&p, 999), None);
        assert_eq!(round_for_height(&p, 1_000), Some(0));
        assert_eq!(round_for_height(&p, 1_023), Some(0));
        assert_eq!(round_for_height(&p, 1_024), Some(1));
        let w = round_window(&p, 3).expect("window");
        assert_eq!(
            w,
            RoundWindow {
                start: 1_072,
                end: 1_096
            }
        );
        assert!(w.contains(1_072) && w.contains(1_095) && !w.contains(1_096));
        assert_eq!(w.last(), 1_095);
        for h in w.start..w.end {
            assert_eq!(round_for_height(&p, h), Some(3));
        }
        assert_eq!(
            round_window(&p, u32::MAX),
            Err(RoundError::Overflow(u32::MAX))
        );
    }

    #[test]
    fn status_follows_the_tip_and_the_timing_rule() {
        let p = params();
        let window = round_window(&p, 0).expect("window");
        let start_time = 1_700_000_000u64;
        let mut ctx = RoundContext {
            tip_height: window.end - 1,
            start_block_time: Some(start_time),
            post_close_block_times: vec![],
            settled: false,
        };
        assert_eq!(
            round_status(&p, 0, &ctx).expect("status"),
            RoundStatus::Open
        );
        ctx.tip_height = window.end + 3;
        assert_eq!(
            round_status(&p, 0, &ctx).expect("status"),
            RoundStatus::Closing
        );

        // Honest chain: blocks keep pace with the target spacing.
        let close = start_time + 24 * 25;
        ctx.post_close_block_times = (0..11).map(|i| close + 25 * (i + 1)).collect();
        ctx.tip_height = window.end + 11;
        assert_eq!(
            round_status(&p, 0, &ctx).expect("status"),
            RoundStatus::Revealable
        );
        ctx.settled = true;
        assert_eq!(
            round_status(&p, 0, &ctx).expect("status"),
            RoundStatus::Settled
        );

        // Blocks arrive long after the beacon: voided, even if "settled".
        ctx.post_close_block_times = (0..11).map(|i| close + 10_000 + 25 * (i + 1)).collect();
        assert_eq!(
            round_status(&p, 0, &ctx).expect("status"),
            RoundStatus::Voided
        );
    }

    #[test]
    fn status_round_trips_through_serde() {
        let json = serde_json::to_string(&RoundStatus::Revealable).expect("json");
        assert_eq!(json, "\"revealable\"");
    }
}
