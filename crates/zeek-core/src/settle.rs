//! Settlement of a round: who is paid what, and what the Deep Cache becomes.
//!
//! `settle` is a pure function of the parameters, the classified digs, the
//! previous Cache balance and the round's outcome. Its result satisfies the
//! conservation invariant, which is checked before returning:
//!
//! ```text
//! valid digs + refunded digs + forfeits + shouts + previous Cache
//!     == paid amounts + fees + shout fees + new Cache
//! ```
//!
//! where `fees` are the dig fees charged to the notes the payout transaction
//! spends plus the output fees charged to the outputs it creates, and `shout
//! fees` are the dig fees reserved for spending shout notes later (rules v4).
//!
//! Rules (all integer arithmetic, `u128` only for the pro-rata products):
//!
//! 1. Every valid dig is charged `dig_fee_zat` for the transaction action
//!    that will spend its note; `pot` is the sum of the remainders.
//!    `rake = floor(pot · rake_bps / 10000)` goes to the Cache;
//!    `distributable = pot − rake`.
//! 2. If the treasure tile has digs, each digger on it gets
//!    `floor(distributable · dig / tile_total)` weighted by the dig's note
//!    value; the rounding dust goes to the Cache. Otherwise `distributable`
//!    rolls into the Cache.
//! 3. If the Cache triggers this round and the treasure tile has winners, the
//!    whole Cache after this round's additions is split the same way and its
//!    dust stays in the Cache. Without winners it carries over untouched.
//! 4. Every payout and refund is one shielded output per dig, charged one
//!    `marginal_fee_zat`. A winner's treasure share and Cache share are paid
//!    in the same output. A winning output whose gross amount is at most the
//!    output fee is not paid: the amount goes to the Cache instead.
//! 5. A refunded dig (wrong round, outside the window, voided round) pays
//!    `dig_fee_zat` for its input and `marginal_fee_zat` for its output and
//!    gets the rest back. If its value is at most the two fees combined it is
//!    forfeited to the Cache whole and its note is not spent.
//!
//! 6. A shout (rules v4) is charged `dig_fee_zat` for the action that will
//!    spend its note some day; the rest goes to the Cache. Shout notes are
//!    not payout inputs (the vault keeps them, like forfeits), so the dig
//!    fee is reserved rather than collected: it is reported as `shout_fees`
//!    and stays with the operator float, which pays for every input the
//!    rules do not charge. Shouts are never refunded.
//!
//! The notes a payout transaction must spend are exactly the charged ones:
//! [`payout_inputs`] lists them in a deterministic order. Rules v1 (proof
//! bundles of version 1) had no dig fee; with `dig_fee_zat = 0` every formula
//! above reduces to v1, which is how old rounds stay verifiable.
//!
//! # Fee coverage
//!
//! ZIP 317 charges `marginal_fee · max(grace, actions)` with `grace = 2`,
//! where an Orchard-shaped bundle has `max(spends, outputs)` actions. A
//! payout transaction spends the `n` charged notes and creates `p` payout
//! outputs plus one change output, so its fee is
//! `marginal · max(2, n, p + 1)`, while the rules collected
//! `marginal · (n + p)` (one dig fee per charged note, one output fee per
//! payout, both equal to the marginal fee). Whenever the transaction exists,
//! `p ≥ 1` and every payout came from a charged note so `n ≥ 1`; then
//! `n ≤ n + p`, `p + 1 ≤ n + p` and `2 ≤ n + p`, so the fee never exceeds
//! the fees collected. [`zip317_fee`] and [`fees_cover`] state this for the
//! keyholders and the coordinator, which also handle the operator-float
//! inputs a Cache payout may need (see the coordinator's documentation).

use serde::{Deserialize, Serialize};

use crate::digs::{ClassifiedDigs, ReceivedNote, RefundReason};
use crate::memo::{DigMemo, SealedReceiver};
use crate::params::GameParams;
use crate::TILE_COUNT;

/// ZIP 317 grace actions: a transaction always pays for at least this many.
pub const ZIP317_GRACE_ACTIONS: u64 = 2;

/// Errors from settlement.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SettleError {
    #[error("arithmetic overflow in settlement")]
    Overflow,
    #[error("treasure tile {0} is out of range")]
    TileOutOfRange(u8),
    #[error("a valid dig names tile {0}, which is out of range")]
    DigTileOutOfRange(u8),
    #[error("a valid dig of {value} zatoshis cannot pay the dig fee of {dig_fee}")]
    DigBelowFee { value: u64, dig_fee: u64 },
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
    /// Amount before the output fee.
    pub gross: u64,
    /// Output fee charged to this output.
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
    /// Sum of the valid digs after their dig fees (rules v1: the sum itself).
    pub pot: u64,
    /// Rake sent to the Cache.
    pub rake: u64,
    /// Pot after rake.
    pub distributable: u64,
    /// Value dug on the treasure tile (note values).
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
    /// Sum of fees charged: dig fees plus output fees.
    pub total_fees: u64,
    /// Sum of net amounts sent.
    pub total_paid: u64,
    /// Sum of forfeited notes.
    pub forfeited: u64,
    /// Dig fees charged to the notes the payout transaction spends (zero
    /// under rules v1, whose bundles omit the field).
    #[serde(default)]
    pub dig_fees: u64,
    /// Sum of the shouts (rules v4; absent and zero before).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub shouts: u64,
    /// Dig fees reserved for spending the shout notes later (rules v4).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub shout_fees: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
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

/// The ZIP 317 fee of an Orchard-shaped transaction with `spends` spends and
/// `outputs` outputs: `marginal_fee · max(grace, spends, outputs)`.
pub fn zip317_fee(params: &GameParams, spends: u64, outputs: u64) -> Result<u64, SettleError> {
    let actions = ZIP317_GRACE_ACTIONS.max(spends).max(outputs);
    params
        .marginal_fee_zat
        .checked_mul(actions)
        .ok_or(SettleError::Overflow)
}

/// Whether the fees a settlement collected (`dig_fees + output fees`) cover
/// the ZIP 317 fee of a transaction with the given shape: `charged_spends`
/// charged notes plus `float_spends` operator-float notes, `payouts` payout
/// outputs and one change output.
pub fn fees_cover(
    params: &GameParams,
    charged_spends: u64,
    float_spends: u64,
    payouts: u64,
) -> Result<bool, SettleError> {
    let collected = params
        .dig_fee_zat
        .checked_mul(charged_spends)
        .and_then(|d| {
            params
                .marginal_fee_zat
                .checked_mul(payouts)
                .and_then(|o| d.checked_add(o))
        })
        .ok_or(SettleError::Overflow)?;
    let spends = add(charged_spends, float_spends)?;
    let outputs = add(payouts, 1)?;
    Ok(zip317_fee(params, spends, outputs)? <= collected)
}

/// Whether a refund-type output of a note worth `value` is paid at all: it
/// must cover its dig fee and its output fee with something left over.
fn refund_is_paid(params: &GameParams, value: u64) -> Result<bool, SettleError> {
    let fees = add(params.dig_fee_zat, params.marginal_fee_zat)?;
    Ok(value > fees)
}

/// The notes the payout transaction of `round` must spend: every valid dig
/// of a drawn round, plus every refunded dig (including the valid digs of a
/// voided round) whose value exceeds its dig fee and output fee. Ordered by
/// height, txid and output index so that every node derives the same list.
pub fn payout_inputs(
    params: &GameParams,
    digs: &ClassifiedDigs,
    outcome: RoundOutcome,
) -> Result<Vec<ReceivedNote>, SettleError> {
    let mut inputs = Vec::new();
    for dig in &digs.valid {
        let charged = match outcome {
            RoundOutcome::Drawn { .. } => true,
            RoundOutcome::Voided => refund_is_paid(params, dig.note.value)?,
        };
        if charged {
            inputs.push(dig.note.clone());
        }
    }
    for refund in &digs.refunds {
        if refund_is_paid(params, refund.note.value)? {
            inputs.push(refund.note.clone());
        }
    }
    inputs.sort_by(|a, b| {
        (a.height, &a.txid, a.output_index).cmp(&(b.height, &b.txid, b.output_index))
    });
    Ok(inputs)
}

/// What the winners of a drawn round would split right now: the valid digs
/// minus their dig fees, minus the rake. This is the settlement's
/// `distributable` computed without a draw (the Cache is separate).
pub fn prize_pool(params: &GameParams, digs: &ClassifiedDigs) -> Result<u64, SettleError> {
    let mut pot = 0u64;
    for dig in &digs.valid {
        let contribution =
            dig.note
                .value
                .checked_sub(params.dig_fee_zat)
                .ok_or(SettleError::DigBelowFee {
                    value: dig.note.value,
                    dig_fee: params.dig_fee_zat,
                })?;
        pot = add(pot, contribution)?;
    }
    sub(pot, pro_rata(pot, params.rake_bps, 10_000)?)
}

/// An output before its fee is charged: sealed receiver, memo round and
/// tile, gross amount, reason.
type GrossOutput = (SealedReceiver, u32, u8, u64, PayoutReason);

/// Running Cache balance and the outputs accumulated while settling.
struct Flows<'a> {
    params: &'a GameParams,
    cache: u64,
    cache_in: u64,
    outputs: Vec<GrossOutput>,
}

impl Flows<'_> {
    fn credit_cache(&mut self, amount: u64) -> Result<(), SettleError> {
        self.cache = add(self.cache, amount)?;
        self.cache_in = add(self.cache_in, amount)?;
        Ok(())
    }

    /// A refund-type output: the note minus its dig fee if it is paid at all,
    /// otherwise the whole note goes to the Cache and nothing is charged.
    fn refund(
        &mut self,
        sealed: SealedReceiver,
        memo_round: u32,
        tile: u8,
        value: u64,
        reason: RefundReason,
    ) -> Result<(), SettleError> {
        if refund_is_paid(self.params, value)? {
            self.outputs.push((
                sealed,
                memo_round,
                tile,
                sub(value, self.params.dig_fee_zat)?,
                PayoutReason::Refund(reason),
            ));
            Ok(())
        } else {
            self.credit_cache(value)
        }
    }
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
    let dig_fee = params.dig_fee_zat;
    let gross_digs = digs.pot().ok_or(SettleError::Overflow)?;
    let refunded = digs.refunded_value().ok_or(SettleError::Overflow)?;
    let forfeited = digs.forfeited_value().ok_or(SettleError::Overflow)?;
    for dig in &digs.valid {
        if dig.tile >= TILE_COUNT {
            return Err(SettleError::DigTileOutOfRange(dig.tile));
        }
    }
    let inputs = payout_inputs(params, digs, outcome)?;
    let dig_fees = dig_fee
        .checked_mul(u64::try_from(inputs.len()).map_err(|_| SettleError::Overflow)?)
        .ok_or(SettleError::Overflow)?;

    let mut flows = Flows {
        params,
        cache: previous_cache,
        cache_in: 0,
        outputs: Vec::new(),
    };

    // Forfeits always feed the Cache.
    flows.credit_cache(forfeited)?;

    // Shouts feed the Cache minus the dig fee reserved for their notes.
    let shouts = digs.shouted_value().ok_or(SettleError::Overflow)?;
    let mut shout_fees = 0u64;
    for shout in &digs.shouts {
        let to_cache = shout
            .note
            .value
            .checked_sub(dig_fee)
            .ok_or(SettleError::DigBelowFee {
                value: shout.note.value,
                dig_fee,
            })?;
        flows.credit_cache(to_cache)?;
        shout_fees = add(shout_fees, dig_fee)?;
    }

    let (pot, rake, distributable, treasure_tile_total, winners, cache_paid) = match outcome {
        RoundOutcome::Voided => {
            // Nothing is drawn: valid digs are refunded like any other refund.
            for dig in &digs.valid {
                flows.refund(
                    dig.sealed_receiver.clone(),
                    round,
                    dig.tile,
                    dig.note.value,
                    RefundReason::RoundVoided,
                )?;
            }
            (0, 0, 0, 0, 0, 0)
        }
        RoundOutcome::Drawn {
            treasure_tile,
            cache_triggered,
        } => {
            if treasure_tile >= TILE_COUNT {
                return Err(SettleError::TileOutOfRange(treasure_tile));
            }
            let mut pot = 0u64;
            for dig in &digs.valid {
                let contribution =
                    dig.note
                        .value
                        .checked_sub(dig_fee)
                        .ok_or(SettleError::DigBelowFee {
                            value: dig.note.value,
                            dig_fee,
                        })?;
                pot = add(pot, contribution)?;
            }
            let rake = pro_rata(pot, params.rake_bps, 10_000)?;
            let distributable = sub(pot, rake)?;
            flows.credit_cache(rake)?;

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
                flows.credit_cache(distributable)?;
            } else {
                let (treasure_shares, dust) = split(distributable, &values, tile_total)?;
                flows.credit_cache(dust)?;
                let cache_shares = if cache_triggered {
                    let (shares, _dust_stays) = split(flows.cache, &values, tile_total)?;
                    let paid = shares
                        .iter()
                        .try_fold(0u64, |acc, v| acc.checked_add(*v))
                        .ok_or(SettleError::Overflow)?;
                    flows.cache = sub(flows.cache, paid)?;
                    cache_paid = paid;
                    shares
                } else {
                    vec![0; winners.len()]
                };
                for ((dig, treasure_share), cache_share) in
                    winners.iter().zip(treasure_shares).zip(cache_shares)
                {
                    flows.outputs.push((
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
            (
                pot,
                rake,
                distributable,
                tile_total,
                winners.len(),
                cache_paid,
            )
        }
    };

    for refund in &digs.refunds {
        let DigMemo {
            round: memo_round,
            tile,
            ..
        } = refund.memo;
        flows.refund(
            refund.memo.sealed_receiver.clone(),
            memo_round,
            tile,
            refund.note.value,
            refund.reason,
        )?;
    }

    // Charge output fees; winning amounts at or below the fee are not worth an
    // output (refunds were already filtered above and always exceed it).
    let Flows {
        mut cache,
        mut cache_in,
        outputs,
        ..
    } = flows;
    let mut payouts = Vec::with_capacity(outputs.len());
    let mut total_fees = dig_fees;
    let mut total_paid = 0u64;
    for (sealed_receiver, memo_round, memo_tile, gross, reason) in outputs {
        if gross <= fee {
            cache = add(cache, gross)?;
            cache_in = add(cache_in, gross)?;
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

    let inputs_total = add(
        add(add(add(gross_digs, refunded)?, forfeited)?, shouts)?,
        previous_cache,
    )?;
    let outputs_total = add(add(add(total_paid, total_fees)?, shout_fees)?, cache)?;
    if inputs_total != outputs_total {
        return Err(SettleError::Conservation {
            inputs: inputs_total,
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
        dig_fees,
        shouts,
        shout_fees,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digs::{Forfeit, ForfeitReason, ReceivedNote, RefundDig, ValidDig};
    use crate::memo::SEALED_RECEIVER_LEN;

    fn note(value: u64) -> ReceivedNote {
        note_at(1, "00", 0, value)
    }

    fn note_at(height: u32, txid: &str, output_index: u16, value: u64) -> ReceivedNote {
        ReceivedNote {
            txid: txid.repeat(32),
            output_index,
            height,
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

    fn refund(value: u64, memo_round: u32, seed: u8, reason: RefundReason) -> RefundDig {
        RefundDig {
            note: note(value),
            memo: DigMemo {
                round: memo_round,
                tile: 2,
                sealed_receiver: sealed(seed),
                message: None,
            },
            reason,
        }
    }

    fn digs(valid: Vec<ValidDig>) -> ClassifiedDigs {
        ClassifiedDigs {
            valid,
            refunds: vec![],
            forfeits: vec![],
            shouts: vec![],
        }
    }

    fn drawn(tile: u8, cache: bool) -> RoundOutcome {
        RoundOutcome::Drawn {
            treasure_tile: tile,
            cache_triggered: cache,
        }
    }

    fn v1() -> GameParams {
        let mut p = GameParams::testnet(1);
        p.dig_fee_zat = 0;
        p
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
        // Three dig fees come off the 6,000,000 before the rake.
        assert_eq!(s.dig_fees, 15_000);
        assert_eq!(s.pot, 5_985_000);
        assert_eq!(s.rake, 299_250);
        assert_eq!(s.distributable, 5_685_750);
        assert_eq!(prize_pool(&p, &d).expect("pool"), s.distributable);
        assert_eq!(s.treasure_tile_total, 3_000_000);
        assert_eq!(s.winners, 2);
        assert_eq!(s.payouts.len(), 2);
        assert_eq!(s.payouts[0].gross, 1_895_250);
        assert_eq!(s.payouts[1].gross, 3_790_500);
        assert_eq!(s.payouts[0].net, 1_890_250);
        assert_eq!(s.total_fees, 25_000, "3 dig fees + 2 output fees");
        assert_eq!(s.new_cache, 10_000 + 299_250);
        assert_eq!(s.cache_paid, 0);
        assert!(matches!(
            s.payouts[0].reason,
            PayoutReason::Win {
                treasure_share: 1_895_250,
                cache_share: 0
            }
        ));
        assert_eq!(
            payout_inputs(&p, &d, drawn(3, false))
                .expect("inputs")
                .len(),
            3,
            "every valid dig of a drawn round is spent"
        );
    }

    #[test]
    fn rules_v1_have_no_dig_fee_and_reproduce_the_old_numbers() {
        let p = v1();
        assert_eq!(p.rules_version(), 1);
        let d = digs(vec![
            valid(3, 1_000_000, 1),
            valid(3, 2_000_000, 2),
            valid(7, 3_000_000, 3),
        ]);
        let s = settle(&p, 5, &d, 10_000, drawn(3, false)).expect("settle");
        assert_eq!(s.dig_fees, 0);
        assert_eq!(s.pot, 6_000_000);
        assert_eq!(s.rake, 300_000);
        assert_eq!(s.payouts[0].gross, 1_900_000);
        assert_eq!(s.payouts[1].gross, 3_800_000);
        assert_eq!(s.total_fees, 10_000);
        assert_eq!(s.new_cache, 310_000);
        // A v1 settlement serialized without `dig_fees` parses back equal.
        let mut json = serde_json::to_value(&s).expect("json");
        json.as_object_mut()
            .expect("object")
            .remove("dig_fees")
            .expect("present");
        let back: Settlement = serde_json::from_value(json).expect("parse");
        assert_eq!(back, s);
    }

    #[test]
    fn rounding_dust_goes_to_the_cache() {
        let p = GameParams::testnet(1);
        // Three near-equal diggers on a pot that does not divide evenly.
        let d = digs(vec![
            valid(0, 100_000, 1),
            valid(0, 100_000, 2),
            valid(0, 100_001, 3),
        ]);
        let s = settle(&p, 1, &d, 0, drawn(0, false)).expect("settle");
        assert_eq!(s.pot, 285_001);
        assert_eq!(s.rake, 14_250);
        assert_eq!(s.distributable, 270_751);
        for payout in &s.payouts {
            assert_eq!(payout.gross, 90_250);
        }
        assert_eq!(s.new_cache, 14_250 + 1);
    }

    #[test]
    fn no_winners_rolls_the_pot_into_the_cache() {
        let p = GameParams::testnet(1);
        let d = digs(vec![valid(1, 500_000, 1)]);
        let s = settle(&p, 2, &d, 1_000, drawn(2, true)).expect("settle");
        assert!(s.payouts.is_empty());
        assert_eq!(s.dig_fees, 5_000, "the dig is still charged for its action");
        assert_eq!(s.total_fees, 5_000);
        assert_eq!(s.new_cache, 1_000 + 24_750 + 470_250);
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
        // Pot 390,000 after two dig fees; rake 19,500; Cache 1,019,503 split 1:3.
        assert_eq!(s.cache_paid, 254_875 + 764_627);
        assert_eq!(s.new_cache, 1);
        assert!(matches!(
            s.payouts[0].reason,
            PayoutReason::Win {
                treasure_share: 92_625,
                cache_share: 254_875
            }
        ));
        assert_eq!(s.payouts[0].gross, 347_500);
        assert_eq!(s.payouts[0].net, 342_500);
    }

    #[test]
    fn shouts_feed_the_cache_minus_a_reserved_dig_fee() {
        use crate::digs::Shout;
        let p = GameParams::testnet(1);
        let d = ClassifiedDigs {
            valid: vec![valid(3, 100_000, 1)],
            shouts: vec![
                Shout {
                    note: note(50_000),
                    message: Some("hi".into()),
                },
                Shout {
                    note: note(70_000),
                    message: None,
                },
            ],
            ..ClassifiedDigs::default()
        };
        let drawn = RoundOutcome::Drawn {
            treasure_tile: 7,
            cache_triggered: false,
        };
        let s = settle(&p, 0, &d, 1_000, drawn).expect("settle");
        assert_eq!(s.shouts, 120_000);
        assert_eq!(s.shout_fees, 10_000);
        // Cache: previous 1,000 + shouts 110,000 + the whole pot (95,000,
        // nobody on tile 7: rake and distributable both roll over).
        assert_eq!(s.new_cache, 1_000 + 110_000 + 95_000);
        assert!(s.payouts.is_empty());
        // Shout notes are not payout inputs.
        assert_eq!(payout_inputs(&p, &d, drawn).expect("inputs").len(), 1);
        let json = serde_json::to_string(&s).expect("json");
        assert!(json.contains("\"shout_fees\":10000"));
        // Without shouts the fields are omitted, so older bundles round-trip.
        let plain = settle(&p, 0, &ClassifiedDigs::default(), 0, drawn).expect("settle");
        assert!(!serde_json::to_string(&plain)
            .expect("json")
            .contains("shout"));
    }

    #[test]
    fn refunds_forfeits_and_dust_outputs() {
        let p = GameParams::testnet(1);
        let d = ClassifiedDigs {
            valid: vec![valid(0, 100_000, 1)],
            refunds: vec![
                refund(200_000, 9, 5, RefundReason::WrongRound { memo_round: 9 }),
                refund(10_000, 1, 6, RefundReason::OutOfWindow),
            ],
            forfeits: vec![Forfeit {
                note: note(7_000),
                reason: ForfeitReason::BelowMinimum { minimum: 100_000 },
            }],
            shouts: vec![],
        };
        let s = settle(&p, 1, &d, 0, drawn(24, false)).expect("settle");
        // No winner: distributable (90,250) rolls into the Cache, plus rake
        // 4,750, plus the forfeit, plus the refund too small to pay (exactly
        // the two fees: forfeited whole, no fee charged).
        assert_eq!(s.payouts.len(), 1);
        assert_eq!(s.payouts[0].memo_round, 9);
        assert_eq!(s.payouts[0].gross, 195_000);
        assert_eq!(s.payouts[0].net, 190_000);
        assert_eq!(s.new_cache, 90_250 + 4_750 + 7_000 + 10_000);
        assert_eq!(s.forfeited, 7_000);
        assert_eq!(s.dig_fees, 10_000, "the valid dig and the paid refund");
        assert_eq!(s.total_fees, 15_000);
        let inputs = payout_inputs(&p, &d, drawn(24, false)).expect("inputs");
        assert_eq!(inputs.len(), 2, "the unpaid refund is not spent");
        assert!(inputs.iter().all(|n| n.value != 10_000));
    }

    #[test]
    fn voided_round_refunds_every_decodable_dig() {
        let p = GameParams::testnet(1);
        let d = digs(vec![valid(0, 100_000, 1), valid(3, 400_000, 2)]);
        let s = settle(&p, 1, &d, 777, RoundOutcome::Voided).expect("settle");
        assert_eq!(s.payouts.len(), 2);
        assert_eq!(s.rake, 0);
        assert_eq!(s.pot, 0);
        assert_eq!(s.new_cache, 777);
        assert!(s
            .payouts
            .iter()
            .all(|o| o.reason == PayoutReason::Refund(RefundReason::RoundVoided)));
        assert_eq!(s.dig_fees, 10_000);
        assert_eq!(s.total_fees, 20_000);
        assert_eq!(s.total_paid, 500_000 - 20_000);
    }

    #[test]
    fn payout_inputs_are_ordered_by_height_txid_and_index() {
        let p = GameParams::testnet(1);
        let d = ClassifiedDigs {
            valid: vec![
                ValidDig {
                    note: note_at(7, "bb", 1, 100_000),
                    tile: 1,
                    sealed_receiver: sealed(1),
                },
                ValidDig {
                    note: note_at(5, "ff", 0, 100_000),
                    tile: 2,
                    sealed_receiver: sealed(2),
                },
                ValidDig {
                    note: note_at(7, "bb", 0, 100_000),
                    tile: 3,
                    sealed_receiver: sealed(3),
                },
            ],
            refunds: vec![RefundDig {
                note: note_at(7, "aa", 3, 50_000),
                memo: DigMemo {
                    round: 9,
                    tile: 0,
                    sealed_receiver: sealed(4),
                    message: None,
                },
                reason: RefundReason::WrongRound { memo_round: 9 },
            }],
            forfeits: vec![],
            shouts: vec![],
        };
        let inputs = payout_inputs(&p, &d, drawn(0, false)).expect("inputs");
        let keys: Vec<(u32, String, u16)> = inputs
            .iter()
            .map(|n| (n.height, n.txid[..2].to_owned(), n.output_index))
            .collect();
        assert_eq!(
            keys,
            vec![
                (5, "ff".into(), 0),
                (7, "aa".into(), 3),
                (7, "bb".into(), 0),
                (7, "bb".into(), 1)
            ]
        );
        let s = settle(&p, 1, &d, 0, drawn(0, false)).expect("settle");
        assert_eq!(s.dig_fees, 4 * 5_000);
    }

    #[test]
    fn zip317_fee_and_coverage() {
        let p = GameParams::testnet(1);
        assert_eq!(zip317_fee(&p, 1, 2).expect("fee"), 10_000, "grace actions");
        assert_eq!(zip317_fee(&p, 25, 2).expect("fee"), 125_000);
        assert_eq!(zip317_fee(&p, 3, 7).expect("fee"), 35_000);
        // One charged note, one payout, one change output: exactly covered.
        assert!(fees_cover(&p, 1, 0, 1).expect("cover"));
        // Round 7's shape: 25 digs, one payout.
        assert!(fees_cover(&p, 25, 0, 1).expect("cover"));
        // Float notes are covered up to the number of payouts.
        assert!(fees_cover(&p, 3, 2, 2).expect("cover"));
        assert!(!fees_cover(&p, 3, 3, 2).expect("cover"));
        // Under rules v1 nothing covers the inputs.
        assert!(!fees_cover(&v1(), 25, 0, 1).expect("cover"));
    }

    #[test]
    fn rejects_out_of_range_tiles_and_digs_below_the_fee() {
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
        assert_eq!(
            settle(&p, 1, &digs(vec![valid(1, 4_000, 1)]), 0, drawn(1, false)),
            Err(SettleError::DigBelowFee {
                value: 4_000,
                dig_fee: 5_000
            })
        );
    }
}
