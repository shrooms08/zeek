//! Public board views.
//!
//! While a round is open only *relative* depths are shown: each tile's total
//! as a fraction of the deepest tile, scaled to `0..=100` with integer
//! arithmetic. Exact per-tile amounts appear only in the post-settlement view.

use serde::{Deserialize, Serialize};

use crate::digs::ClassifiedDigs;
use crate::params::GameParams;
use crate::rounds::{round_window, RoundError, RoundStatus};
use crate::settle::Settlement;
use crate::TILE_COUNT;

/// Errors from building board views.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BoardError {
    #[error("a dig names tile {0}, which is out of range")]
    TileOutOfRange(u8),
    #[error("arithmetic overflow in board totals")]
    Overflow,
    #[error(transparent)]
    Round(#[from] RoundError),
}

/// Per-tile totals of a round.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileTotals(pub [u64; TILE_COUNT as usize]);

impl TileTotals {
    /// Sums the valid digs per tile.
    pub fn from_digs(digs: &ClassifiedDigs) -> Result<Self, BoardError> {
        let mut totals = [0u64; TILE_COUNT as usize];
        for dig in &digs.valid {
            let slot = totals
                .get_mut(usize::from(dig.tile))
                .ok_or(BoardError::TileOutOfRange(dig.tile))?;
            *slot = slot
                .checked_add(dig.note.value)
                .ok_or(BoardError::Overflow)?;
        }
        Ok(TileTotals(totals))
    }

    /// Relative depth of each tile: `tile_total · 100 / max_tile_total`.
    pub fn relative_depths(&self) -> [u8; TILE_COUNT as usize] {
        let max = self.0.iter().copied().max().unwrap_or(0);
        let mut depths = [0u8; TILE_COUNT as usize];
        if max == 0 {
            return depths;
        }
        for (depth, total) in depths.iter_mut().zip(self.0.iter()) {
            let scaled = u128::from(*total) * 100 / u128::from(max);
            *depth = u8::try_from(scaled).unwrap_or(100);
        }
        depths
    }

    /// Depth stage 0 to 5 of every tile: its share of the round's total,
    /// bucketed at `0`, `> 0`, `≥ 4 %`, `≥ 8 %`, `≥ 14 %`, `≥ 22 %` (integer
    /// math in basis points). Exact amounts are never shown during a round.
    pub fn depth_stages(&self) -> [u8; TILE_COUNT as usize] {
        const BOUNDS_BPS: [u128; 4] = [400, 800, 1_400, 2_200];
        let total: u128 = self.0.iter().map(|v| u128::from(*v)).sum();
        let mut stages = [0u8; TILE_COUNT as usize];
        if total == 0 {
            return stages;
        }
        for (stage, value) in stages.iter_mut().zip(self.0.iter()) {
            if *value == 0 {
                continue;
            }
            let bps = u128::from(*value) * 10_000 / total;
            *stage = 1 + BOUNDS_BPS.iter().filter(|b| bps >= **b).count() as u8;
        }
        stages
    }

    /// Sum of all tiles.
    pub fn pot(&self) -> Option<u64> {
        self.0.iter().try_fold(0u64, |acc, v| acc.checked_add(*v))
    }
}

/// What players see while a round is in progress.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardView {
    pub round: u32,
    pub status: RoundStatus,
    /// Blocks until the window closes (0 once closed).
    pub blocks_remaining: u32,
    /// What winners would split right now: valid digs minus dig fees, minus
    /// the rake.
    pub prize_pool_zat: u64,
    /// Gross total dug so far: the plain sum of the valid digs, before dig
    /// fees and rake. Not what is paid out; see `prize_pool_zat`.
    pub gross_dug_zat: u64,
    /// Current Deep Cache balance.
    pub cache: u64,
    /// Relative depth per tile, `0..=100`.
    pub depths: [u8; TILE_COUNT as usize],
    /// Depth stage 0 to 5 per tile (see [`TileTotals::depth_stages`]).
    pub stages: [u8; TILE_COUNT as usize],
}

impl BoardView {
    /// Builds the in-round view.
    pub fn new(
        params: &GameParams,
        round: u32,
        status: RoundStatus,
        tip_height: u32,
        digs: &ClassifiedDigs,
        cache: u64,
    ) -> Result<Self, BoardError> {
        let totals = TileTotals::from_digs(digs)?;
        let window = round_window(params, round)?;
        let blocks_remaining = window.end.saturating_sub(tip_height.saturating_add(1));
        Ok(BoardView {
            round,
            status,
            blocks_remaining,
            prize_pool_zat: crate::settle::prize_pool(params, digs)
                .map_err(|_| BoardError::Overflow)?,
            gross_dug_zat: totals.pot().ok_or(BoardError::Overflow)?,
            cache,
            depths: totals.relative_depths(),
            stages: totals.depth_stages(),
        })
    }
}

/// What players see after settlement: exact amounts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettledView {
    pub round: u32,
    pub status: RoundStatus,
    /// Exact value dug per tile.
    pub tile_totals: TileTotals,
    /// Treasure tile, if the round was drawn.
    pub treasure_tile: Option<u8>,
    pub cache_triggered: bool,
    pub pot: u64,
    pub rake: u64,
    pub winners: usize,
    pub total_paid: u64,
    pub cache_paid: u64,
    pub previous_cache: u64,
    pub new_cache: u64,
}

impl SettledView {
    /// Builds the post-settlement view.
    pub fn new(totals: TileTotals, settlement: &Settlement) -> Self {
        let (treasure_tile, cache_triggered, status) = match settlement.outcome {
            crate::settle::RoundOutcome::Voided => (None, false, RoundStatus::Voided),
            crate::settle::RoundOutcome::Drawn {
                treasure_tile,
                cache_triggered,
            } => (Some(treasure_tile), cache_triggered, RoundStatus::Settled),
        };
        SettledView {
            round: settlement.round,
            status,
            tile_totals: totals,
            treasure_tile,
            cache_triggered,
            pot: settlement.pot,
            rake: settlement.rake,
            winners: settlement.winners,
            total_paid: settlement.total_paid,
            cache_paid: settlement.cache_paid,
            previous_cache: settlement.previous_cache,
            new_cache: settlement.new_cache,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_depths_use_integer_math() {
        let mut totals = TileTotals([0; 25]);
        assert_eq!(totals.relative_depths(), [0; 25]);
        totals.0[0] = 1_000;
        totals.0[1] = 333;
        totals.0[2] = 1;
        let depths = totals.relative_depths();
        assert_eq!(depths[0], 100);
        assert_eq!(depths[1], 33);
        assert_eq!(depths[2], 0);
        assert_eq!(totals.pot(), Some(1_334));
    }

    #[test]
    fn depth_stages_follow_the_design_buckets() {
        let mut totals = TileTotals([0; 25]);
        assert_eq!(totals.depth_stages(), [0; 25]);
        // Shares: 3/1000 (>0), 40 (4 %), 80 (8 %), 140 (14 %), 220 (22 %), rest.
        for (i, v) in [3u64, 40, 80, 140, 220, 517].iter().enumerate() {
            totals.0[i] = *v;
        }
        let stages = totals.depth_stages();
        assert_eq!(&stages[..7], &[1, 2, 3, 4, 5, 5, 0]);
        totals.0 = [0; 25];
        totals.0[3] = 39;
        totals.0[4] = 961;
        assert_eq!(totals.depth_stages()[3], 1, "3.9 % is below the 4 % bucket");
    }

    #[test]
    fn totals_come_from_valid_digs_only() {
        use crate::digs::{ReceivedNote, ValidDig};
        use crate::memo::{SealedReceiver, SEALED_RECEIVER_LEN};
        let dig = |tile: u8, value: u64| ValidDig {
            note: ReceivedNote {
                txid: String::new(),
                output_index: 0,
                height: 0,
                value,
                memo: vec![],
            },
            tile,
            sealed_receiver: SealedReceiver::from_bytes(&[0; SEALED_RECEIVER_LEN]).expect("sealed"),
        };
        let digs = ClassifiedDigs {
            valid: vec![dig(2, 10), dig(2, 5), dig(24, 1)],
            refunds: vec![],
            forfeits: vec![],
            shouts: vec![],
        };
        let totals = TileTotals::from_digs(&digs).expect("totals");
        assert_eq!(totals.0[2], 15);
        assert_eq!(totals.0[24], 1);
        let bad = ClassifiedDigs {
            valid: vec![dig(25, 1)],
            refunds: vec![],
            forfeits: vec![],
            shouts: vec![],
        };
        assert_eq!(
            TileTotals::from_digs(&bad),
            Err(BoardError::TileOutOfRange(25))
        );
    }

    #[test]
    fn board_view_counts_remaining_blocks() {
        use crate::digs::{ReceivedNote, ValidDig};
        use crate::memo::{SealedReceiver, SEALED_RECEIVER_LEN};
        let p = GameParams::testnet(1_000);
        let empty = ClassifiedDigs::default();
        let v = BoardView::new(&p, 0, RoundStatus::Open, 1_000, &empty, 5).expect("view");
        assert_eq!(v.blocks_remaining, 23);
        assert_eq!((v.prize_pool_zat, v.gross_dug_zat), (0, 0));
        let v = BoardView::new(&p, 0, RoundStatus::Closing, 1_030, &empty, 5).expect("view");
        assert_eq!(v.blocks_remaining, 0);
        // Two digs of 100,000: gross 200,000; prize pool (200,000 - 2 x 5,000) x 95%.
        let dig = |tile: u8| ValidDig {
            note: ReceivedNote {
                txid: String::new(),
                output_index: 0,
                height: 1_000,
                value: 100_000,
                memo: vec![],
            },
            tile,
            sealed_receiver: SealedReceiver::from_bytes(&[0; SEALED_RECEIVER_LEN]).expect("sealed"),
        };
        let digs = ClassifiedDigs {
            valid: vec![dig(1), dig(2)],
            refunds: vec![],
            forfeits: vec![],
            shouts: vec![],
        };
        let v = BoardView::new(&p, 0, RoundStatus::Open, 1_000, &digs, 5).expect("view");
        assert_eq!(v.gross_dug_zat, 200_000);
        assert_eq!(v.prize_pool_zat, 180_500);
        assert_eq!(v.depths[1], 100);
        let json = serde_json::to_string(&v).expect("json");
        assert!(json.contains("\"status\":\"open\""));
    }
}
