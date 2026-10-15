//! Round processing: status, timing rule, beacon, classification and
//! settlement, every step delegated to `zeek-core`.
//!
//! Rounds settle strictly in order so that the Deep Cache balance carries
//! from each settlement to the next. A round is settled only once the
//! `mtp_window` blocks after its window are stored and the last of them has
//! [`SETTLE_CONFIRMATIONS`] confirmations, so a reorg can no longer change
//! the inputs the settlement was computed from.

use serde::{Deserialize, Serialize};
use zeek_core::beacon;
use zeek_core::board::BoardView;
use zeek_core::digs::{self, ClassifiedDigs};
use zeek_core::rounds::{self, RoundContext, RoundStatus, RoundWindow};
use zeek_core::settle::{self, RoundOutcome};
use zeek_core::timing::TimingVerdict;

use crate::beacon::{Backoff, BeaconFetcher};
use crate::config::{GameConfig, SETTLE_CONFIRMATIONS};
use crate::db::{BeaconRow, Db, PlanStatus, SettlementRow};
use crate::error::EngineError;
use crate::proof::{ProofBundle, PROOF_VERSION};
use crate::store::Store;

/// Something that happened, pushed to WebSocket clients.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The board changed (new block or new dig).
    Board(BoardState),
    /// A round's window closed.
    RoundClosed {
        round: u32,
        window: RoundWindow,
        drand_round: u64,
        beacon_time: u64,
    },
    /// A beacon was verified and the treasure drawn.
    Reveal {
        round: u32,
        drand_round: u64,
        treasure_tile: u8,
        cache_triggered: bool,
    },
    /// A settlement plan was stored.
    Settled {
        round: u32,
        outcome: RoundOutcome,
        new_cache: u64,
    },
    /// A payout transaction was broadcast.
    Paid {
        round: u32,
        txid: String,
        signers: Vec<u16>,
    },
    /// A new Campfire message was mined (filtered ones arrive hidden, with
    /// their text omitted).
    Campfire(crate::campfire::CampfireMessage),
    /// The set of digs seen in the mempool but not mined yet changed (see
    /// [`crate::mempool`]). Always the complete current set; never counted
    /// on the board.
    Landing {
        digs: Vec<crate::mempool::LandingDig>,
    },
}

/// The public board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardState {
    /// Chain tip the engine has scanned (0 before the game's genesis block).
    pub tip_height: u32,
    /// `nTime` of that tip (0 before genesis).
    pub tip_time: u64,
    /// Chain tip reported by lightwalletd, which may be below genesis.
    pub chain_tip: u32,
    /// Round the tip belongs to, or the first round if the game has not
    /// started.
    pub round: u32,
    pub status: RoundStatus,
    pub window: RoundWindow,
    /// Blocks until the window closes (counting from the tip).
    pub blocks_remaining: u32,
    /// `tip_time + blocks_remaining · target_spacing` (0 before genesis).
    pub estimated_close_time: u64,
    /// Before genesis: `chain_tip_time + (genesis − chain_tip) · target_spacing`;
    /// `None` once the game has started.
    pub estimated_start_time: Option<u64>,
    /// What winners would split right now: valid digs minus dig fees, minus
    /// the rake.
    pub prize_pool_zat: u64,
    /// Gross total dug so far (plain sum of the valid digs, before dig fees
    /// and rake). Not what is paid out.
    pub gross_dug_zat: u64,
    /// Rules version of this round (3 once text memos are active).
    pub rules_version: u32,
    /// Deep Cache balance carried into this round.
    pub cache: u64,
    /// Relative tile depths, `0..=100`.
    pub depths: [u8; 25],
    /// Depth stage 0 to 5 per tile (share of the round's total, bucketed).
    pub stages: [u8; 25],
    /// Number of valid digs so far.
    pub digs: usize,
}

/// A closed round waiting for its reveal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRound {
    pub round: u32,
    pub window: RoundWindow,
    pub status: RoundStatus,
    pub drand_round: u64,
    /// Expected publication time of the bound beacon.
    pub beacon_time: u64,
}

/// Drives rounds forward.
pub struct Processor {
    config: GameConfig,
    backoff: Backoff,
}

impl Processor {
    /// A processor for `config`.
    pub fn new(config: GameConfig) -> Self {
        Processor {
            config,
            backoff: Backoff::default(),
        }
    }

    /// The game config.
    pub fn config(&self) -> &GameConfig {
        &self.config
    }

    /// Cache balance after the last settled round.
    pub fn cache_balance(&self, db: &Db) -> Result<u64, EngineError> {
        Ok(match db.last_settled_round()? {
            Some(round) => {
                db.settlement(round)?
                    .ok_or_else(|| EngineError::Corrupt(format!("settlement {round} vanished")))?
                    .settlement
                    .new_cache
            }
            None => self.config.initial_cache_zat,
        })
    }

    /// The on-chain context of `round` as currently stored.
    pub fn round_context(
        &self,
        db: &Db,
        round: u32,
    ) -> Result<(RoundWindow, RoundContext), EngineError> {
        let params = &self.config.params;
        let window = rounds::round_window(params, round)?;
        let tip_height = db.tip()?.map(|b| b.height).unwrap_or(0);
        let start_block_time = db.block(window.start)?.map(|b| b.time);
        let post_close_block_times = db.block_times(
            window.end,
            window
                .end
                .checked_add(params.mtp_window)
                .ok_or(EngineError::Overflow("post-close window"))?,
        )?;
        Ok((
            window,
            RoundContext {
                tip_height,
                start_block_time,
                post_close_block_times,
                settled: db.settlement(round)?.is_some(),
            },
        ))
    }

    /// Status of `round`.
    pub fn status(
        &self,
        db: &Db,
        round: u32,
    ) -> Result<(RoundWindow, RoundStatus, RoundContext), EngineError> {
        let (window, ctx) = self.round_context(db, round)?;
        let status = rounds::round_status(&self.config.params, round, &ctx)?;
        Ok((window, status, ctx))
    }

    /// The round the scanned tip belongs to (round 0 before the game starts).
    pub fn current_round(&self, db: &Db) -> Result<u32, EngineError> {
        let tip = db.tip()?.map(|b| b.height).unwrap_or(0);
        Ok(rounds::round_for_height(&self.config.params, tip).unwrap_or(0))
    }

    /// Every note of `round` classified under the game rules.
    pub fn classified(
        &self,
        db: &Db,
        round: u32,
        voided: bool,
    ) -> Result<ClassifiedDigs, EngineError> {
        let window = rounds::round_window(&self.config.params, round)?;
        let notes = db
            .notes_in_range(window.start, window.end)?
            .iter()
            .map(|n| n.to_received_note())
            .collect::<Vec<_>>();
        Ok(digs::classify_round(
            &self.config.params,
            round,
            voided,
            notes,
        )?)
    }

    /// The notes the payout transaction of a settled round must spend, in the
    /// deterministic order of [`settle::payout_inputs`].
    pub fn payout_inputs(
        &self,
        db: &Db,
        row: &SettlementRow,
    ) -> Result<Vec<zeek_core::digs::ReceivedNote>, EngineError> {
        let voided = matches!(row.outcome, RoundOutcome::Voided);
        let classified = self.classified(db, row.round, voided)?;
        Ok(settle::payout_inputs(
            &self.config.params,
            &classified,
            row.outcome,
        )?)
    }

    /// The public board for the current round.
    pub fn board(&self, db: &Db) -> Result<BoardState, EngineError> {
        let params = &self.config.params;
        let tip = db.tip()?;
        let (tip_height, tip_time) = tip.map(|b| (b.height, b.time)).unwrap_or((0, 0));
        // Before genesis nothing is stored; the chain tip lightwalletd reported
        // still says how far away round 0 is.
        let chain_tip = db.chain_tip()?.unwrap_or(tip_height).max(tip_height);
        let round = self.current_round(db)?;
        let (window, status, _) = self.status(db, round)?;
        let classified = self.classified(db, round, status == RoundStatus::Voided)?;
        let view = BoardView::new(
            params,
            round,
            status,
            chain_tip,
            &classified,
            self.cache_balance(db)?,
        )
        .map_err(|e| EngineError::Corrupt(e.to_string()))?;
        let estimated_start_time = match (tip_time, db.chain_tip_time()?) {
            (0, Some(chain_tip_time)) if chain_tip < params.genesis_height => Some(
                chain_tip_time
                    .checked_add(
                        u64::from(params.genesis_height - chain_tip)
                            .saturating_mul(params.target_spacing_secs),
                    )
                    .ok_or(EngineError::Overflow("estimated start time"))?,
            ),
            _ => None,
        };
        // Unknown (0) until the first game block is stored.
        let estimated_close_time = if tip_time == 0 {
            0
        } else {
            tip_time
                .checked_add(
                    u64::from(view.blocks_remaining).saturating_mul(params.target_spacing_secs),
                )
                .ok_or(EngineError::Overflow("estimated close time"))?
        };
        Ok(BoardState {
            tip_height,
            tip_time,
            chain_tip,
            round,
            status,
            window,
            blocks_remaining: view.blocks_remaining,
            estimated_close_time,
            estimated_start_time,
            prize_pool_zat: view.prize_pool_zat,
            gross_dug_zat: view.gross_dug_zat,
            rules_version: rounds::rules_version(params, round)?,
            cache: view.cache,
            depths: view.depths,
            stages: view.stages,
            digs: classified.valid.len(),
        })
    }

    /// Closed, unsettled rounds in order.
    pub fn pending_rounds(&self, db: &Db) -> Result<Vec<PendingRound>, EngineError> {
        let params = &self.config.params;
        let tip = db.tip()?.map(|b| b.height).unwrap_or(0);
        let first = db.last_settled_round()?.map(|r| r + 1).unwrap_or(0);
        let current = self.current_round(db)?;
        let mut pending = Vec::new();
        for round in first..=current {
            let (window, status, ctx) = self.status(db, round)?;
            if tip < window.end || status == RoundStatus::Settled {
                continue;
            }
            let Some(start_time) = ctx.start_block_time else {
                continue;
            };
            let drand_round = beacon::beacon_round_for(params, start_time)?;
            pending.push(PendingRound {
                round,
                window,
                status,
                drand_round,
                beacon_time: beacon::beacon_time(params, drand_round)?,
            });
        }
        Ok(pending)
    }

    /// The proof bundle of a settled round.
    pub fn proof_bundle(&self, db: &Db, row: &SettlementRow) -> Result<ProofBundle, EngineError> {
        let window = rounds::round_window(&self.config.params, row.round)?;
        let notes = db
            .notes_in_range(window.start, window.end)?
            .iter()
            .map(|n| n.to_received_note())
            .collect();
        let beacon_row = db.beacon(row.drand_round)?;
        let (beacon, randomness) = match row.outcome {
            RoundOutcome::Voided => (None, None),
            RoundOutcome::Drawn { .. } => {
                let b = beacon_row.ok_or_else(|| {
                    EngineError::Corrupt(format!(
                        "beacon {} missing for drawn round {}",
                        row.drand_round, row.round
                    ))
                })?;
                (Some(b.beacon), Some(b.randomness))
            }
        };
        // The bundle states the parameters the round was settled under: rounds
        // of earlier rules versions predate the dig fee (v1) or the text-memo
        // activation (v1, v2), whatever the current config says.
        let mut params = self.config.params.clone();
        if row.rules_version < 2 {
            params.dig_fee_zat = 0;
        }
        if row.rules_version < 3 {
            params.text_memo_activation_height = None;
        }
        if row.rules_version < 4 {
            params.campfire = None;
        }
        Ok(ProofBundle {
            version: PROOF_VERSION,
            rules_version: Some(row.rules_version),
            payout_openings: db.payout_openings(row.round)?,
            unclaimable_payouts: db.unclaimable(row.round)?,
            params,
            round: row.round,
            window,
            notes,
            start_block_time: row.start_block_time,
            post_close_block_times: row.post_close_block_times.clone(),
            drand_round: row.drand_round,
            beacon_time: row.beacon_time,
            beacon,
            randomness,
            timing: row.timing,
            outcome: row.outcome,
            previous_cache: row.settlement.previous_cache,
            settlement: row.settlement.clone(),
            payout: db.payout(row.round)?,
        })
    }

    /// Settles every round that is ready, in order. `now` is the current Unix
    /// time (beacons are only fetched once published).
    ///
    /// The beacon is fetched with no database connection held; each round's
    /// beacon and settlement are then written in one short transaction.
    pub async fn process(
        &mut self,
        store: &Store,
        fetcher: &mut dyn BeaconFetcher,
        now: u64,
    ) -> Result<Vec<Event>, EngineError> {
        let params = self.config.params.clone();
        let mut events = Vec::new();
        let Some(tip) = store.read(|db| db.tip())? else {
            return Ok(events);
        };
        let mut round = store
            .read(|db| db.last_settled_round())?
            .map(|r| r + 1)
            .unwrap_or(0);
        loop {
            let (window, status, ctx) = store.read(|db| self.status(db, round))?;
            let required_tip = window
                .end
                .checked_add(params.mtp_window - 1)
                .and_then(|h| h.checked_add(SETTLE_CONFIRMATIONS))
                .ok_or(EngineError::Overflow("settlement height"))?;
            if tip.height < required_tip
                || ctx.post_close_block_times.len() < params.mtp_window as usize
            {
                break;
            }
            let start_time = ctx.start_block_time.ok_or_else(|| {
                EngineError::Corrupt(format!("start block of round {round} missing"))
            })?;
            let drand_round = beacon::beacon_round_for(&params, start_time)?;
            let beacon_time = beacon::beacon_time(&params, drand_round)?;
            let timing = zeek_core::timing::check_window_close(
                &params,
                beacon_time,
                &ctx.post_close_block_times,
            )?;

            let mut new_beacon = None;
            let outcome = match (status, timing) {
                (RoundStatus::Voided, _) | (_, TimingVerdict::Unsafe { .. }) => {
                    RoundOutcome::Voided
                }
                (RoundStatus::Revealable, TimingVerdict::Safe { .. }) => {
                    let fetched = match store.read(|db| db.beacon(drand_round))? {
                        Some(row) => row,
                        None => {
                            if now < beacon_time || !self.backoff.ready(drand_round, now) {
                                break;
                            }
                            match fetcher.fetch(&params.drand, drand_round).await {
                                Ok(f) => {
                                    self.backoff.succeeded(drand_round);
                                    let row = BeaconRow {
                                        beacon: f.beacon,
                                        randomness: f.randomness,
                                        relay: f.relay,
                                    };
                                    new_beacon = Some(row.clone());
                                    row
                                }
                                Err(e) => {
                                    let delay = self.backoff.failed(drand_round, now);
                                    tracing::warn!(round, drand_round, retry_in_secs = delay, error = %e, "beacon not available yet");
                                    break;
                                }
                            }
                        }
                    };
                    let treasure_tile =
                        zeek_core::random::treasure_tile(round, &fetched.randomness)?;
                    let cache_triggered =
                        zeek_core::random::cache_triggers(&params, round, &fetched.randomness)?;
                    events.push(Event::Reveal {
                        round,
                        drand_round,
                        treasure_tile,
                        cache_triggered,
                    });
                    RoundOutcome::Drawn {
                        treasure_tile,
                        cache_triggered,
                    }
                }
                (other, _) => {
                    return Err(EngineError::Corrupt(format!(
                        "round {round} has status {other:?} although its blocks are confirmed"
                    )));
                }
            };

            let voided = matches!(outcome, RoundOutcome::Voided);
            let (classified, previous_cache) = store.read(|db| {
                Ok::<_, EngineError>((self.classified(db, round, voided)?, self.cache_balance(db)?))
            })?;
            let settlement = settle::settle(&params, round, &classified, previous_cache, outcome)?;
            let row = SettlementRow {
                round,
                settled_at_height: tip.height,
                start_block_time: start_time,
                post_close_block_times: ctx.post_close_block_times[..params.mtp_window as usize]
                    .to_vec(),
                drand_round,
                beacon_time,
                timing,
                outcome,
                settlement: settlement.clone(),
                status: PlanStatus::AwaitingSignatures,
                rules_version: rounds::rules_version(&params, round)?,
            };
            store.write(|db| {
                db.transaction(|tx| {
                    if let Some(beacon) = &new_beacon {
                        tx.insert_beacon(beacon)?;
                    }
                    tx.insert_settlement(&row)
                })
            })?;
            tracing::info!(
                round,
                ?outcome,
                pot = settlement.pot,
                payouts = settlement.payouts.len(),
                new_cache = settlement.new_cache,
                "settlement plan stored (awaiting signatures)"
            );
            events.push(Event::Settled {
                round,
                outcome,
                new_cache: settlement.new_cache,
            });
            round += 1;
        }
        Ok(events)
    }
}
