//! The vault ledger: `vault balance = Cache + unpaid obligations + reserved
//! fees + unclaimable payouts + operator float`.
//!
//! Obligations are the net amounts of settlements that have no payout
//! transaction yet; reserved fees are the per-output fees those settlements
//! already deducted from players (they pay the payout transaction's fee).
//! When a transaction's real ZIP 317 fee exceeds the fees collected, the
//! difference comes out of the operator float, never the Cache, and a payout
//! whose deficit the float cannot cover is refused.

use serde::{Deserialize, Serialize};

use crate::db::{Db, PayoutState, PlanStatus};
use crate::error::EngineError;
use crate::processor::Processor;

/// `GET /v1/vault/ledger`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    /// Total Ironwood value in the vault wallet (spendable or not).
    pub vault_balance: u64,
    /// Deep Cache balance.
    pub cache: u64,
    /// Net payouts and refunds owed but not yet broadcast.
    pub unpaid_obligations: u64,
    /// Per-output fees already deducted from those payouts, reserved for
    /// their transaction fees.
    pub reserved_fees: u64,
    /// Payouts that could not be paid (their digger's sealed receiver does
    /// not open); held in the vault, never counted as operator float.
    pub unclaimable: u64,
    /// `vault_balance − cache − unpaid_obligations − reserved_fees −
    /// unclaimable`, floored at 0.
    pub operator_float: u64,
    /// How far the vault is short of covering everything above (0 when sound).
    pub shortfall: u64,
    /// Rounds whose payouts are still owed.
    pub unpaid_rounds: Vec<u32>,
    /// Rounds with a payout in progress (signing opened, not broadcast).
    pub in_progress_rounds: Vec<u32>,
    /// Chain tip of the vault wallet when the balance was read.
    pub last_reconciliation_height: u32,
}

/// Computes the ledger from the engine database and the vault's balance.
pub fn compute(
    db: &Db,
    processor: &Processor,
    vault_balance: u64,
    last_reconciliation_height: u32,
) -> Result<Ledger, EngineError> {
    let mut unpaid_obligations = 0u64;
    let mut reserved_fees = 0u64;
    let mut unpaid_rounds = Vec::new();
    let mut in_progress_rounds = Vec::new();
    for round in db.settled_rounds()? {
        let row = db
            .settlement(round)?
            .ok_or_else(|| EngineError::Corrupt(format!("settlement {round} vanished")))?;
        match row.status {
            PlanStatus::Paid | PlanStatus::NothingToPay => continue,
            PlanStatus::AwaitingSignatures => {}
        }
        if row.settlement.payouts.is_empty() {
            continue;
        }
        if let Some(record) = db.payout(round)? {
            if record.state == PayoutState::Signing {
                in_progress_rounds.push(round);
            }
        }
        unpaid_obligations = unpaid_obligations
            .checked_add(row.settlement.total_paid)
            .ok_or(EngineError::Overflow("obligations"))?;
        reserved_fees = reserved_fees
            .checked_add(row.settlement.total_fees)
            .ok_or(EngineError::Overflow("reserved fees"))?;
        unpaid_rounds.push(round);
    }
    let cache = processor.cache_balance(db)?;
    let unclaimable = db.unclaimable_total()?;
    let owed = cache
        .checked_add(unpaid_obligations)
        .and_then(|v| v.checked_add(reserved_fees))
        .and_then(|v| v.checked_add(unclaimable))
        .ok_or(EngineError::Overflow("owed"))?;
    let (operator_float, shortfall) = match vault_balance.checked_sub(owed) {
        Some(float) => (float, 0),
        None => (0, owed - vault_balance),
    };
    Ok(Ledger {
        vault_balance,
        cache,
        unpaid_obligations,
        reserved_fees,
        unclaimable,
        operator_float,
        shortfall,
        unpaid_rounds,
        in_progress_rounds,
        last_reconciliation_height,
    })
}
