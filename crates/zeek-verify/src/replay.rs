//! A round's replay for the web app's Replay mode (shown while the engine
//! is offline): how the field filled block by block, then the draw and the
//! payout, derived from the round's proof bundle.
//!
//! It carries only what the live field showed: depth stages per block
//! (`TileTotals::depth_stages`, the same buckets as the engine's board) and
//! which tiles took a dig in that block. Never amounts per tile, never
//! anything about who dug.

use serde::Serialize;
use zeek_core::board::{BoardError, TileTotals};
use zeek_core::digs::{classify_round, ClassifiedDigs};
use zeek_core::proof::ProofBundle;
use zeek_core::rounds::RoundError;
use zeek_core::settle::RoundOutcome;
use zeek_core::TILE_COUNT;

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("the round's digs cannot be classified: {0}")]
    Classify(#[from] RoundError),
    #[error("the round's tile totals cannot be computed: {0}")]
    Board(#[from] BoardError),
}

/// The field after one block that carried digs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Frame {
    pub height: u32,
    /// Tiles that took a dig in this block (each once, ascending).
    pub landed: Vec<u8>,
    /// Depth stage 0–5 of every tile after this block.
    pub stages: [u8; TILE_COUNT as usize],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ReplayOutcome {
    Drawn {
        treasure_tile: u8,
        cache_triggered: bool,
    },
    Voided,
}

/// One round, ready to play back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Replay {
    pub round: u32,
    pub window_start: u32,
    pub window_end: u32,
    pub start_block_time: u64,
    pub frames: Vec<Frame>,
    pub outcome: ReplayOutcome,
    /// The prize as shown live (the pot after dig fees).
    pub pot: u64,
    pub winners: usize,
    pub total_paid: u64,
    pub previous_cache: u64,
    pub new_cache: u64,
    pub cache_paid: u64,
    pub drand_round: u64,
    pub payout_txid: Option<String>,
}

/// Derives the replay of `bundle`'s round.
pub fn replay(bundle: &ProofBundle) -> Result<Replay, ReplayError> {
    // As the live board showed it during the round: every dig counted (a
    // voided round is only refunded after it closes).
    let classified = classify_round(&bundle.params, bundle.round, false, bundle.notes.clone())?;
    let mut frames: Vec<Frame> = Vec::new();
    let mut so_far = ClassifiedDigs::default();
    let mut digs = classified.valid.into_iter().peekable();
    while let Some(first) = digs.next() {
        let height = first.note.height;
        let mut landed = vec![first.tile];
        so_far.valid.push(first);
        while let Some(next) = digs.next_if(|d| d.note.height == height) {
            landed.push(next.tile);
            so_far.valid.push(next);
        }
        landed.sort_unstable();
        landed.dedup();
        frames.push(Frame {
            height,
            landed,
            stages: TileTotals::from_digs(&so_far)?.depth_stages(),
        });
    }
    let s = &bundle.settlement;
    Ok(Replay {
        round: bundle.round,
        window_start: bundle.window.start,
        window_end: bundle.window.end,
        start_block_time: bundle.start_block_time,
        frames,
        outcome: match bundle.outcome {
            RoundOutcome::Drawn {
                treasure_tile,
                cache_triggered,
            } => ReplayOutcome::Drawn {
                treasure_tile,
                cache_triggered,
            },
            RoundOutcome::Voided => ReplayOutcome::Voided,
        },
        pot: s.pot,
        winners: s.winners,
        total_paid: s.total_paid,
        previous_cache: s.previous_cache,
        new_cache: s.new_cache,
        cache_paid: s.cache_paid,
        drand_round: bundle.drand_round,
        payout_txid: bundle.payout.as_ref().and_then(|p| p.txid.clone()),
    })
}
