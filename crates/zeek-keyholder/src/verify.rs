//! Independent verification of a payout PCZT before signing.
//!
//! A keyholder signs only if **all** of the following hold:
//!
//! 1. the PCZT spends only notes of this vault (every non-padding spend's
//!    nullifier and `rk` recompute from this vault's full viewing key and the
//!    spend's own fields, and every pending spend's `rk` equals the vault's
//!    `ak` randomized by the spend's `alpha`);
//! 2. every output's note commitment recomputes from its stated recipient,
//!    value and `rseed` (missing fields are a refusal);
//! 3. the multiset of (recipient, amount) of the non-change, non-padding
//!    outputs equals exactly the payouts and refunds this node's **own**
//!    settlement computed for the claimed rounds, after opening each sealed
//!    receiver with the sealing key;
//! 4. every other value-carrying output goes to the vault's internal-scope
//!    change address (follows from 3: anything else would be an unexpected
//!    payout);
//! 5. the fee equals the ZIP 317 fee for this transaction's action counts;
//! 6. none of the claimed rounds was signed by this node before;
//! 7. the transaction is V6 on the consensus branch of its expiry height,
//!    and not expired.
//!
//! Refusal reasons name the check that failed and never include an opened
//! receiver.

use std::collections::BTreeMap;
use std::convert::Infallible;

use orchard::keys::{FullViewingKey, Scope, SpendValidatingKey};
use orchard::ValuePool;
use pczt::roles::signer::Signer;
use pczt::roles::verifier::Verifier;
use pczt::Pczt;
use zcash_primitives::transaction::fees::zip317;
use zcash_primitives::transaction::fees::FeeRule;
use zcash_primitives::transaction::TxVersion;
use zcash_protocol::consensus::{BranchId, MAIN_NETWORK, TEST_NETWORK};
use zeek_core::memo::{DigMemo, ORCHARD_RECEIVER_LEN};
use zeek_core::params::{GameParams, Network};
use zeek_core::seal::SealingPrivateKey;
use zeek_engine::db::Db;
use zeek_vault::send::{self, PendingSpend};

use crate::db::NodeDb;
use crate::error::KeyholderError;

/// What verification needs.
pub struct VerifyContext<'a> {
    pub params: &'a GameParams,
    pub fvk: &'a FullViewingKey,
    pub sealing: &'a SealingPrivateKey,
    /// This node's own chain database (its settlements).
    pub chain: &'a Db,
    /// This node's signing records.
    pub node: &'a NodeDb,
}

/// A PCZT this node is willing to sign.
pub struct Approved {
    pub pending: Vec<PendingSpend>,
    pub sighash: [u8; 32],
    pub rounds: Vec<u32>,
    pub payouts: usize,
    pub fee: u64,
}

fn refuse(reason: impl Into<String>) -> KeyholderError {
    KeyholderError::Refused(reason.into())
}

/// Runs every check; returns the pending spends to sign on success.
pub fn verify_payout(
    ctx: &VerifyContext<'_>,
    pczt_bytes: &[u8],
    rounds: &[u32],
) -> Result<Approved, KeyholderError> {
    let pczt =
        Pczt::parse(pczt_bytes).map_err(|e| refuse(format!("PCZT does not parse: {e:?}")))?;

    // 7. Version and branch.
    let effects = pczt
        .clone()
        .into_effects()
        .map_err(|e| refuse(format!("PCZT effects cannot be extracted: {e:?}")))?;
    if effects.version() != TxVersion::V6 {
        return Err(refuse(format!(
            "transaction version is {:?}, not V6",
            effects.version()
        )));
    }
    let expiry = effects.expiry_height();
    let expected_branch = match ctx.params.network {
        Network::Main => BranchId::for_height(&MAIN_NETWORK, expiry),
        Network::Test => BranchId::for_height(&TEST_NETWORK, expiry),
    };
    if effects.consensus_branch_id() != expected_branch {
        return Err(refuse(format!(
            "consensus branch {:?} differs from {expected_branch:?} expected at height {expiry}",
            effects.consensus_branch_id()
        )));
    }
    let tip = ctx.chain.tip()?.map(|b| b.height).unwrap_or(0);
    if u32::from(expiry) <= tip {
        return Err(refuse(format!(
            "transaction expires at {expiry}, chain tip is already {tip}"
        )));
    }
    if effects.transparent_bundle().is_some() || effects.sapling_bundle().is_some() {
        return Err(refuse(
            "transaction carries transparent or Sapling components",
        ));
    }

    // 6. Rounds: settled here, distinct, not signed before.
    let mut seen = std::collections::BTreeSet::new();
    let mut expected: Vec<([u8; ORCHARD_RECEIVER_LEN], u64)> = Vec::new();
    for round in rounds {
        if !seen.insert(*round) {
            return Err(refuse(format!("round {round} is listed twice")));
        }
        if let Some(session) = ctx.node.signed_session(*round)? {
            return Err(refuse(format!(
                "round {round} was already signed in session {session}"
            )));
        }
        let row = ctx
            .chain
            .settlement(*round)?
            .ok_or_else(|| refuse(format!("round {round} is not settled on this node")))?;
        for payout in &row.settlement.payouts {
            let memo = DigMemo {
                round: payout.memo_round,
                tile: payout.memo_tile,
                sealed_receiver: payout.sealed_receiver.clone(),
            };
            let receiver = memo
                .open(ctx.params.network, ctx.sealing)
                .map_err(|e| refuse(format!("round {round}: a sealed receiver does not open with this node's sealing key ({e})")))?;
            expected.push((receiver.orchard, payout.net));
        }
    }
    if rounds.is_empty() {
        return Err(refuse("no rounds claimed"));
    }

    // 1 + 2 + 3: walk every action of both bundles.
    let change_address = ctx
        .fvk
        .address_at(0u32, Scope::Internal)
        .to_raw_address_bytes();
    let ak = SpendValidatingKey::from(ctx.fvk.clone());
    let mut spent = 0u64;
    let mut out_total = 0u64;
    let mut found: Vec<([u8; ORCHARD_RECEIVER_LEN], u64)> = Vec::new();
    let mut change_outputs = 0usize;
    let mut action_counts: BTreeMap<ValuePool, usize> = BTreeMap::new();
    let mut failure: Option<String> = None;
    let mut walk = |pool: ValuePool, bundle: &orchard::pczt::Bundle| {
        action_counts.insert(pool, bundle.actions().len());
        for (index, action) in bundle.actions().iter().enumerate() {
            let spend = action.spend();
            let output = action.output();
            let padding_spend =
                spend.value().map(|v| v.inner()) == Some(0) && spend.spend_auth_sig().is_some();
            if !padding_spend {
                if let Err(e) = spend.verify_nullifier(Some(ctx.fvk)) {
                    failure = Some(format!(
                        "{pool:?} action {index}: spend is not a note of this vault ({e:?})"
                    ));
                    return;
                }
                if let Err(e) = spend.verify_rk(Some(ctx.fvk)) {
                    failure = Some(format!(
                        "{pool:?} action {index}: spend rk is inconsistent with this vault ({e:?})"
                    ));
                    return;
                }
                match spend.value() {
                    Some(v) => spent = spent.saturating_add(v.inner()),
                    None => {
                        failure = Some(format!("{pool:?} action {index}: spend value is missing"));
                        return;
                    }
                }
            }
            if let Err(e) = output.verify_note_commitment(spend) {
                failure = Some(format!(
                    "{pool:?} action {index}: output commitment does not recompute ({e:?})"
                ));
                return;
            }
            let (Some(recipient), Some(value)) = (output.recipient(), output.value()) else {
                failure = Some(format!(
                    "{pool:?} action {index}: output recipient or value missing"
                ));
                return;
            };
            let value = value.inner();
            out_total = out_total.saturating_add(value);
            if value == 0 {
                continue;
            }
            let raw = recipient.to_raw_address_bytes();
            if raw == change_address {
                change_outputs += 1;
            } else {
                found.push((raw, value));
            }
        }
    };
    Verifier::new(pczt.clone())
        .with_orchard::<Infallible, _>(|bundle| {
            walk(ValuePool::Orchard, bundle);
            Ok(())
        })
        .map_err(|e| refuse(format!("Orchard bundle does not parse: {e:?}")))?
        .with_ironwood::<Infallible, _>(|bundle| {
            walk(ValuePool::Ironwood, bundle);
            Ok(())
        })
        .map_err(|e| refuse(format!("Ironwood bundle does not parse: {e:?}")))?;
    if let Some(reason) = failure {
        return Err(refuse(reason));
    }

    // 3. Payout multiset.
    let mut expected_sorted = expected.clone();
    expected_sorted.sort();
    let mut found_sorted = found.clone();
    found_sorted.sort();
    if expected_sorted != found_sorted {
        let sum = |v: &[([u8; 43], u64)]| v.iter().map(|(_, a)| *a).sum::<u64>();
        return Err(refuse(format!(
            "payout set mismatch: this node expects {} output(s) totalling {} zatoshis for rounds {:?}, the PCZT pays {} output(s) totalling {}",
            expected.len(),
            sum(&expected),
            rounds,
            found.len(),
            sum(&found)
        )));
    }

    // 5. Fee.
    let fee = spent
        .checked_sub(out_total)
        .ok_or_else(|| refuse("outputs exceed spent value"))?;
    let expected_fee = zip317::FeeRule::standard()
        .fee_required(
            &TEST_NETWORK,
            expiry,
            core::iter::empty::<zcash_primitives::transaction::fees::transparent::InputSize>(),
            core::iter::empty::<usize>(),
            0,
            0,
            action_counts.get(&ValuePool::Orchard).copied().unwrap_or(0),
            action_counts
                .get(&ValuePool::Ironwood)
                .copied()
                .unwrap_or(0),
        )
        .map_err(|e| refuse(format!("fee rule error: {e:?}")))?
        .into_u64();
    if fee != expected_fee {
        return Err(refuse(format!(
            "fee {fee} differs from the ZIP 317 fee {expected_fee}"
        )));
    }

    // Pending spends and their randomized keys.
    let pending =
        send::pending_spends(&pczt).map_err(|e| refuse(format!("no signable spends: {e}")))?;
    for spend in &pending {
        let expected_rk: [u8; 32] = (&ak.randomize(&spend.alpha)).into();
        if expected_rk != spend.rk {
            return Err(refuse(format!(
                "{} action {}: rk is not this vault's ak randomized by the action's alpha",
                zeek_vault::wallet::pool_name(spend.pool),
                spend.index
            )));
        }
    }
    let sighash = Signer::new(pczt)
        .map_err(|e| refuse(format!("signer cannot parse the PCZT: {e:?}")))?
        .shielded_sighash();
    tracing::info!(
        rounds = ?rounds,
        payouts = found.len(),
        change_outputs,
        fee,
        "payout PCZT verified against this node's own settlement"
    );
    Ok(Approved {
        pending,
        sighash,
        rounds: rounds.to_vec(),
        payouts: found.len(),
        fee,
    })
}
