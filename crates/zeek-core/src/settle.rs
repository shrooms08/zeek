//! Settlement of a round: who is paid what, and what the Deep Cache becomes.
//!
//! `settle` is a pure function of the parameters, the classified digs, the
//! previous Cache balance and the round's outcome. Its result satisfies the
//! conservation invariant, which is checked before returning:
//!
//! ```text
//! valid digs + refunded digs + forfeits + previous Cache
//!     == paid amounts + fees + new Cache
//! ```
//!
//! Rules (all integer arithmetic, `u128` only for the pro-rata products):
//!
//! 1. `pot` = sum of valid digs; `rake = floor(pot · rake_bps / 10000)` goes to
//!    the Cache; `distributable = pot − rake`.
//! 2. If the treasure tile has digs, each digger on it gets
//!    `floor(distributable · dig / tile_total)`; the rounding dust goes to the
//!    Cache. Otherwise `distributable` rolls into the Cache.
//! 3. If the Cache triggers this round and the treasure tile has winners, the
//!    whole Cache after this round's additions is split the same way and its
//!    dust stays in the Cache. Without winners it carries over untouched.
//! 4. Every payout and refund is one shielded output per dig, charged one
//!    `marginal_fee_zat`. A winner's treasure share and Cache share are paid
//!    in the same output. An output whose gross amount is at most the fee is
//!    not paid: the amount goes to the Cache instead.

use serde::{Deserialize, Serialize};

use crate::digs::{ClassifiedDigs, RefundReason};
use crate::memo::{DigMemo, SealedReceiver};
use crate::params::GameParams;
use crate::TILE_COUNT;

/// Errors from settlement.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SettleError {
    #[error("arithmetic overflow in settlement")]
    Overflow,
    #[error("treasure tile {0} is out of range")]
    TileOutOfRange(u8),
    #[error("a valid dig names tile {0}, which is out of range")]
    DigTileOutOfRange(u8),
    #[error("conservation violated: inputs {inputs} != outputs {outputs}")]
    Conservation { inputs: u64, outputs: u64 },
}

/// How the round ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RoundOutcome {
    /// The timing rule failed: refunds only.
    Voided,
    /// The beacon was verified and the draws made.
    Drawn {
        /// Treasure tile, `0..=24`.
        treasure_tile: u8,
        /// Whether the Deep Cache broke open.
        cache_triggered: bool,
    },
}

/// Why an output is paid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum PayoutReason {
    /// A dig on the treasure tile.
    Win {
        /// Share of the distributable pot.
        treasure_share: u64,
        /// Share of the Deep Cache (0 unless it triggered).
        cache_share: u64,
    },
    /// A refunded dig.
    Refund(RefundReason),
}

/// One shielded output the vault must send.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payout {
    /// Sealed receiver of the dig being paid.
    pub sealed_receiver: SealedReceiver,
    /// Round and tile the sealed receiver was bound to (needed to open it).
    pub memo_round: u32,
    /// See `memo_round`.
    pub memo_tile: u8,
    /// Amount before the fee.
    pub gross: u64,
    /// Fee charged to this output.
    pub fee: u64,
    /// Amount actually sent.
    pub net: u64,
    /// Why it is paid.
    pub reason: PayoutReason,
}

/// Result of settling a round.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settlement {
    pub round: u32,
    pub outcome: RoundOutcome,
    /// Sum of valid digs.
    pub pot: u64,
    /// Rake sent to the Cache.
    pub rake: u64,
    /// Pot after rake.
    pub distributable: u64,
    /// Value dug on the treasure tile.
    pub treasure_tile_total: u64,
    /// Number of winning digs.
    pub winners: usize,
    /// Cache paid out to winners (gross).
    pub cache_paid: u64,
    /// Outputs to send.
    pub payouts: Vec<Payout>,
    /// Cache balance before this round.
    pub previous_cache: u64,
    /// Cache balance after this round.
    pub new_cache: u64,
    /// Everything that flowed into the Cache this round (rake, dust,
    /// rollover, forfeits, unpaid dust outputs).
    pub cache_in: u64,
    /// Sum of fees charged.
    pub total_fees: u64,
    /// Sum of net amounts sent.
    pub total_paid: u64,
    /// Sum of forfeited notes.
    pub forfeited: u64,
}

fn add(a: u64, b: u64) -> Result<u64, SettleError> {
    a.checked_add(b).ok_or(SettleError::Overflow)
}

fn sub(a: u64, b: u64) -> Result<u64, SettleError> {
    a.checked_sub(b).ok_or(SettleError::Overflow)
}

/// `floor(total · part / whole)` without overflow; `whole > 0`.
fn pro_rata(total: u64, part: u64, whole: u64) -> Result<u64, SettleError> {
    if whole == 0 {
        return Err(SettleError::Overflow);
    }
    let value = u128::from(total) * u128::from(part) / u128::from(whole);
    u64::try_from(value).map_err(|_| SettleError::Overflow)
}

/// Splits `pool` across the winning digs pro rata; returns the shares and the
/// dust left over.
fn split(pool: u64, dig_values: &[u64], tile_total: u64) -> Result<(Vec<u64>, u64), SettleError> {
    let mut shares = Vec::with_capacity(dig_values.len());
    let mut given = 0u64;
    for value in dig_values {
        let share = pro_rata(pool, *value, tile_total)?;
        shares.push(share);
        given = add(given, share)?;
    }
    Ok((shares, sub(pool, given)?))
}

/// Settles `round`.
pub fn settle(
    params: &GameParams,
    round: u32,
    digs: &ClassifiedDigs,
    previous_cache: u64,
    outcome: RoundOutcome,
) -> Result<Settlement, SettleError> {
    let fee = params.marginal_fee_zat;
    let pot = digs.pot().ok_or(SettleError::Overflow)?;
    let refunded = digs.refunded_value().ok_or(SettleError::Overflow)?;
    let forfeited = digs.forfeited_value().ok_or(SettleError::Overflow)?;
    for dig in &digs.valid {
        if dig.tile >= TILE_COUNT {
            return Err(SettleError::DigTileOutOfRange(dig.tile));
        }
    }

    let mut cache = previous_cache;
    let mut cache_in = 0u64;
    let into_cache =
        |cache: &mut u64, cache_in: &mut u64, amount: u64| -> Result<(), SettleError> {
            *cache = add(*cache, amount)?;
            *cache_in = add(*cache_in, amount)?;
            Ok(())
        };

    // Forfeits always feed the Cache.
    into_cache(&mut cache, &mut cache_in, forfeited)?;

    let mut outputs: Vec<(SealedReceiver, u32, u8, u64, PayoutReason)> = Vec::new();

    let (rake, distributable, treasure_tile_total, winners, cache_paid) = match outcome {
        RoundOutcome::Voided => {
            // Nothing is drawn: valid digs are refunded as if the round were voided.
            for dig in &digs.valid {
                outputs.push((
                    dig.sealed_receiver.clone(),
                    round,
                    dig.tile,
                    dig.note.value,
                    PayoutReason::Refund(RefundReason::RoundVoided),
                ));
            }
            (0, 0, 0, 0, 0)
        }
        RoundOutcome::Drawn {
            treasure_tile,
            cache_triggered,
        } => {
            if treasure_tile >= TILE_COUNT {
                return Err(SettleError::TileOutOfRange(treasure_tile));
            }
            let rake = pro_rata(pot, params.rake_bps, 10_000)?;
            let distributable = sub(pot, rake)?;
            into_cache(&mut cache, &mut cache_in, rake)?;

            let winners: Vec<_> = digs
                .valid
                .iter()
                .filter(|d| d.tile == treasure_tile)
                .collect();
            let values: Vec<u64> = winners.iter().map(|d| d.note.value).collect();
            let tile_total = values
                .iter()
                .try_fold(0u64, |acc, v| acc.checked_add(*v))
                .ok_or(SettleError::Overflow)?;

            let mut cache_paid = 0u64;
            if winners.is_empty() {
                into_cache(&mut cache, &mut cache_in, distributable)?;
            } else {
                let (treasure_shares, dust) = split(distributable, &values, tile_total)?;
                into_cache(&mut cache, &mut cache_in, dust)?;
                let cache_shares = if cache_triggered {
                    let (shares, _dust_stays) = split(cache, &values, tile_total)?;
                    let paid = shares
                        .iter()
                        .try_fold(0u64, |acc, v| acc.checked_add(*v))
                        .ok_or(SettleError::Overflow)?;
                    cache = sub(cache, paid)?;
                    cache_paid = paid;
                    shares
                } else {
                    vec![0; winners.len()]
                };
                for ((dig, treasure_share), cache_share) in
                    winners.iter().zip(treasure_shares).zip(cache_shares)
                {
                    outputs.push((
                        dig.sealed_receiver.clone(),
                        round,
                        dig.tile,
                        add(treasure_share, cache_share)?,
                        PayoutReason::Win {
                            treasure_share,
                            cache_share,
                        },
                    ));
                }
            }
            (rake, distributable, tile_total, winners.len(), cache_paid)
        }
    };

    for refund in &digs.refunds {
        let DigMemo {
            round: memo_round,
            tile,
            ..
        } = refund.memo;
        outputs.push((
            refund.memo.sealed_receiver.clone(),
            memo_round,
            tile,
            refund.note.value,
            PayoutReason::Refund(refund.reason),
        ));
    }

    // Charge fees; amounts at or below the fee are not worth an output.
    let mut payouts = Vec::with_capacity(outputs.len());
    let mut total_fees = 0u64;
    let mut total_paid = 0u64;
    for (sealed_receiver, memo_round, memo_tile, gross, reason) in outputs {
        if gross <= fee {
            into_cache(&mut cache, &mut cache_in, gross)?;
            continue;
        }
        let net = sub(gross, fee)?;
        total_fees = add(total_fees, fee)?;
        total_paid = add(total_paid, net)?;
        payouts.push(Payout {
            sealed_receiver,
            memo_round,
            memo_tile,
            gross,
            fee,
            net,
            reason,
        });
    }

    let inputs = add(add(add(pot, refunded)?, forfeited)?, previous_cache)?;
    let outputs_total = add(add(total_paid, total_fees)?, cache)?;
    if inputs != outputs_total {
        return Err(SettleError::Conservation {
            inputs,
            outputs: outputs_total,
        });
    }

    Ok(Settlement {
        round,
        outcome,
        pot,
        rake,
        distributable,
        treasure_tile_total,
        winners,
        cache_paid,
        payouts,
        previous_cache,
        new_cache: cache,
        cache_in,
        total_fees,
        total_paid,
        forfeited,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digs::{Forfeit, ForfeitReason, ReceivedNote, RefundDig, ValidDig};
    use crate::memo::SEALED_RECEIVER_LEN;

    fn note(value: u64) -> ReceivedNote {
        ReceivedNote {
            txid: "00".repeat(32),
            output_index: 0,
            height: 1,
            value,
            memo: vec![],
        }
    }

    fn sealed(seed: u8) -> SealedReceiver {
        SealedReceiver::from_bytes(&[seed; SEALED_RECEIVER_LEN]).expect("sealed")
    }

    fn valid(tile: u8, value: u64, seed: u8) -> ValidDig {
        ValidDig {
            note: note(value),
            tile,
            sealed_receiver: sealed(seed),
        }
    }

    fn digs(valid: Vec<ValidDig>) -> ClassifiedDigs {
        ClassifiedDigs {
            valid,
            refunds: vec![],
            forfeits: vec![],
        }
    }

    fn drawn(tile: u8, cache: bool) -> RoundOutcome {
        RoundOutcome::Drawn {
            treasure_tile: tile,
            cache_triggered: cache,
        }
    }

    #[test]
    fn treasure_is_split_pro_rata_with_rake_and_dust_to_cache() {
        let p = GameParams::testnet(1);
        let d = digs(vec![
            valid(3, 1_000_000, 1),
            valid(3, 2_000_000, 2),
            valid(7, 3_000_000, 3),
        ]);
        let s = settle(&p, 5, &d, 10_000, drawn(3, false)).expect("settle");
        assert_eq!(s.pot, 6_000_000);
        assert_eq!(s.rake, 300_000);
        assert_eq!(s.distributable, 5_700_000);
        assert_eq!(s.winners, 2);
        assert_eq!(s.payouts.len(), 2);
        assert_eq!(s.payouts[0].gross, 1_900_000);
        assert_eq!(s.payouts[1].gross, 3_800_000);
        assert_eq!(s.payouts[0].net, 1_895_000);
        assert_eq!(s.total_fees, 10_000);
        assert_eq!(s.new_cache, 10_000 + 300_000);
        assert_eq!(s.cache_paid, 0);
        assert!(matches!(
            s.payouts[0].reason,
            PayoutReason::Win {
                treasure_share: 1_900_000,
                cache_share: 0
            }
        ));
    }

    #[test]
    fn rounding_dust_goes_to_the_cache() {
        let p = GameParams::testnet(1);
        // Three equal diggers on a pot that does not divide evenly.
        let d = digs(vec![
            valid(0, 100_000, 1),
            valid(0, 100_000, 2),
            valid(0, 100_000, 3),
        ]);
        let s = settle(&p, 1, &d, 0, drawn(0, false)).expect("settle");
        assert_eq!(s.rake, 15_000);
        assert_eq!(s.distributable, 285_000);
        for payout in &s.payouts {
            assert_eq!(payout.gross, 95_000);
        }
        assert_eq!(s.new_cache, 15_000);
    }

    #[test]
    fn no_winners_rolls_the_pot_into_the_cache() {
        let p = GameParams::testnet(1);
        let d = digs(vec![valid(1, 500_000, 1)]);
        let s = settle(&p, 2, &d, 1_000, drawn(2, true)).expect("settle");
        assert!(s.payouts.is_empty());
        assert_eq!(s.new_cache, 1_000 + 500_000);
        assert_eq!(
            s.cache_paid, 0,
            "cache carries over untouched without winners"
        );
    }

    #[test]
    fn cache_trigger_pays_the_whole_cache_pro_rata() {
        let p = GameParams::testnet(1);
        let d = digs(vec![valid(4, 100_000, 1), valid(4, 300_000, 2)]);
        let s = settle(&p, 3, &d, 1_000_003, drawn(4, true)).expect("settle");
        // Cache after additions: 1_000_003 + rake 20_000 = 1_020_003; split 1:3.
        assert_eq!(s.cache_paid, 255_000 + 765_002);
        assert_eq!(s.new_cache, 1);
        assert!(matches!(
            s.payouts[0].reason,
            PayoutReason::Win {
                treasure_share: 95_000,
                cache_share: 255_000
            }
        ));
        assert_eq!(s.payouts[0].gross, 350_000);
        assert_eq!(s.payouts[0].net, 345_000);
    }

    #[test]
    fn refunds_forfeits_and_dust_outputs() {
        let p = GameParams::testnet(1);
        let d = ClassifiedDigs {
            valid: vec![valid(0, 100_000, 1)],
            refunds: vec![
                RefundDig {
                    note: note(200_000),
                    memo: DigMemo {
                        round: 9,
                        tile: 2,
                        sealed_receiver: sealed(5),
                    },
                    reason: RefundReason::WrongRound { memo_round: 9 },
                },
                RefundDig {
                    note: note(5_000),
                    memo: DigMemo {
                        round: 1,
                        tile: 2,
                        sealed_receiver: sealed(6),
                    },
                    reason: RefundReason::OutOfWindow,
                },
            ],
            forfeits: vec![Forfeit {
                note: note(7_000),
                reason: ForfeitReason::BelowMinimum { minimum: 100_000 },
            }],
        };
        let s = settle(&p, 1, &d, 0, drawn(24, false)).expect("settle");
        // No winner: distributable (95_000) rolls into the Cache, plus rake 5_000,
        // plus the forfeit, plus the refund too small to pay.
        assert_eq!(s.payouts.len(), 1);
        assert_eq!(s.payouts[0].memo_round, 9);
        assert_eq!(s.payouts[0].net, 195_000);
        assert_eq!(s.new_cache, 95_000 + 5_000 + 7_000 + 5_000);
        assert_eq!(s.forfeited, 7_000);
    }

    #[test]
    fn voided_round_refunds_every_decodable_dig() {
        let p = GameParams::testnet(1);
        let d = digs(vec![valid(0, 100_000, 1), valid(3, 400_000, 2)]);
        let s = settle(&p, 1, &d, 777, RoundOutcome::Voided).expect("settle");
        assert_eq!(s.payouts.len(), 2);
        assert_eq!(s.rake, 0);
        assert_eq!(s.new_cache, 777);
        assert!(s
            .payouts
            .iter()
            .all(|o| o.reason == PayoutReason::Refund(RefundReason::RoundVoided)));
        assert_eq!(s.total_paid, 500_000 - 10_000);
    }

    #[test]
    fn rejects_out_of_range_tiles() {
        let p = GameParams::testnet(1);
        assert_eq!(
            settle(&p, 1, &digs(vec![]), 0, drawn(25, false)),
            Err(SettleError::TileOutOfRange(25))
        );
        assert_eq!(
            settle(
                &p,
                1,
                &digs(vec![valid(25, 100_000, 1)]),
                0,
                drawn(1, false)
            ),
            Err(SettleError::DigTileOutOfRange(25))
        );
    }
}
