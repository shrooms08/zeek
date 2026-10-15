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
//! 4. there is exactly one other value-carrying output and it goes to the
//!    vault's internal-scope change address;
//! 5. the inputs are exactly the dig notes the rules charge for the claimed
//!    rounds ([`zeek_core::settle::payout_inputs`]), plus operator-float notes
//!    (vault notes that are not digs) only when those dig notes cannot pay the
//!    outputs and the fee; the fee equals the ZIP 317 fee for the
//!    transaction's action counts and does not exceed the fees the rules
//!    collected for those rounds;
//! 6. none of the claimed rounds was signed by this node before, and each was
//!    settled under the rules version this node signs for;
//! 7. the transaction is V6 on the consensus branch of its expiry height,
//!    and not expired.
//!
//! Refusal reasons name the check that failed and the exact item (output or
//! input index, round, amounts) and never include an opened receiver or any
//! secret.

use std::collections::BTreeMap;
use std::convert::Infallible;

use orchard::keys::{FullViewingKey, Scope, SpendValidatingKey};
use orchard::note::ExtractedNoteCommitment;
use orchard::ValuePool;
use pczt::roles::signer::Signer;
use pczt::roles::verifier::Verifier;
use pczt::Pczt;
use zcash_primitives::transaction::TxVersion;
use zcash_protocol::consensus::{BranchId, MAIN_NETWORK, TEST_NETWORK};
use zeek_core::memo::ORCHARD_RECEIVER_LEN;
use zeek_core::params::Network;
use zeek_core::seal::SealingPrivateKey;
use zeek_core::settle;
use zeek_engine::coordinator::payable_receiver;
use zeek_engine::db::Db;
use zeek_engine::processor::Processor;
use zeek_vault::send::{self, PendingSpend};

use crate::db::NodeDb;
use crate::error::KeyholderError;

/// Oldest rules version this node signs payouts for: rules v2 introduced the
/// dig fee and the exact input set the checks below rely on. Rules v3 only
/// adds the text encoding of dig memos, which this node's own classification
/// already applied when it settled the round.
pub const MIN_SIGNED_RULES_VERSION: u32 = 2;

/// What verification needs.
pub struct VerifyContext<'a> {
    /// The game, for the rules and for classifying this node's notes.
    pub processor: &'a Processor,
    pub fvk: &'a FullViewingKey,
    pub sealing: &'a SealingPrivateKey,
    /// This node's own chain database (its notes and settlements).
    pub chain: &'a Db,
    /// This node's signing records.
    pub node: &'a NodeDb,
    /// Unix seconds now (for the freshness of this node's chain view).
    pub now: u64,
}

/// How many target block spacings this node's newest block may be old
/// before it refuses to sign (its chain view may have stalled).
pub const MAX_TIP_AGE_SPACINGS: u64 = 40;
/// How far this node's stored tip may trail the chain tip its servers last
/// reported.
pub const MAX_TIP_LAG_BLOCKS: u32 = 3;

/// A PCZT this node is willing to sign.
pub struct Approved {
    pub pending: Vec<PendingSpend>,
    pub sighash: [u8; 32],
    pub rounds: Vec<u32>,
    pub payouts: usize,
    /// Distinct payout receivers among the payouts (never the receivers).
    pub distinct_receivers: usize,
    pub charged_inputs: usize,
    pub float_inputs: usize,
    pub fee: u64,
}

fn refuse(reason: impl Into<String>) -> KeyholderError {
    KeyholderError::Refused(reason.into())
}

/// Formats zatoshis with thousands separators, as refusal reasons print them.
pub fn zat(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A payout this node expects, by its position in a round's settlement.
struct ExpectedPayout {
    round: u32,
    index: usize,
    receiver: [u8; ORCHARD_RECEIVER_LEN],
    net: u64,
}

/// A dig note this node expects to be spent.
struct ExpectedInput {
    round: u32,
    txid: String,
    output_index: u16,
    value: u64,
    cmx: [u8; 32],
}

/// One spend of the PCZT, as the walk saw it.
struct SeenSpend {
    index: usize,
    cmx: [u8; 32],
}

/// Runs every check; returns the pending spends to sign on success.
pub fn verify_payout(
    ctx: &VerifyContext<'_>,
    pczt_bytes: &[u8],
    rounds: &[u32],
) -> Result<Approved, KeyholderError> {
    let params = &ctx.processor.config().params;
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
    let expected_branch = match params.network {
        Network::Main => BranchId::for_height(&MAIN_NETWORK, expiry),
        Network::Test => BranchId::for_height(&TEST_NETWORK, expiry),
    };
    if effects.consensus_branch_id() != expected_branch {
        return Err(refuse(format!(
            "consensus branch {:?} differs from {expected_branch:?} expected at height {expiry}",
            effects.consensus_branch_id()
        )));
    }
    // A node whose chain view stalled could miss digs and take an unsettled
    // dig for operator float (security review L3): sign only when current.
    let stored = ctx
        .chain
        .tip()?
        .ok_or_else(|| refuse("this node has not followed the chain yet"))?;
    let max_age = params
        .target_spacing_secs
        .saturating_mul(MAX_TIP_AGE_SPACINGS);
    if ctx.now.saturating_sub(stored.time) > max_age {
        return Err(refuse(format!(
            "this node's newest block ({}) is older than {max_age} s; its chain view may have stalled",
            stored.height
        )));
    }
    if let Some(chain_tip) = ctx.chain.chain_tip()? {
        if chain_tip > stored.height.saturating_add(MAX_TIP_LAG_BLOCKS) {
            return Err(refuse(format!(
                "this node is {} blocks behind the chain tip {chain_tip}",
                chain_tip - stored.height
            )));
        }
    }
    let tip = stored.height;
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

    // 6. Rounds: distinct, settled here under the signed rules, not signed before.
    if rounds.is_empty() {
        return Err(refuse("no rounds claimed"));
    }
    let mut seen_rounds = std::collections::BTreeSet::new();
    let mut expected_payouts: Vec<ExpectedPayout> = Vec::new();
    let mut expected_inputs: Vec<ExpectedInput> = Vec::new();
    let mut collected_fees = 0u64;
    for round in rounds {
        if !seen_rounds.insert(*round) {
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
        if row.rules_version < MIN_SIGNED_RULES_VERSION {
            return Err(refuse(format!(
                "round {round} was settled under rules version {}; this node only signs payouts of rules version {MIN_SIGNED_RULES_VERSION} or later",
                row.rules_version
            )));
        }
        for (index, payout) in row.settlement.payouts.iter().enumerate() {
            // The coordinator's own rule: a payout with no payable receiver
            // is not paid and its value stays in the change output.
            match payable_receiver(params.network, ctx.sealing, payout) {
                Ok(address) => expected_payouts.push(ExpectedPayout {
                    round: *round,
                    index,
                    receiver: address.to_raw_address_bytes(),
                    net: payout.net,
                }),
                Err(reason) => {
                    tracing::info!(round, payout = index, %reason, "unclaimable payout: expecting its value in the change");
                }
            }
        }
        collected_fees = collected_fees
            .checked_add(row.settlement.total_fees)
            .ok_or_else(|| refuse("collected fees overflow"))?;
        for note in ctx.processor.payout_inputs(ctx.chain, &row)? {
            let stored = ctx
                .chain
                .notes_in_range(note.height, note.height + 1)?
                .into_iter()
                .find(|n| n.txid_hex() == note.txid && n.action_index == note.output_index)
                .ok_or_else(|| {
                    refuse(format!(
                        "dig note {}:{} of round {round} is not in this node's database",
                        note.txid, note.output_index
                    ))
                })?;
            let cmx = stored.cmx.ok_or_else(|| {
                refuse(format!(
                    "dig note {}:{} of round {round} has no recorded note commitment yet (backfill pending)",
                    note.txid, note.output_index
                ))
            })?;
            expected_inputs.push(ExpectedInput {
                round: *round,
                txid: note.txid,
                output_index: note.output_index,
                value: note.value,
                cmx,
            });
        }
    }

    // 1 + 2: walk every action of both bundles.
    let change_address = ctx
        .fvk
        .address_at(0u32, Scope::Internal)
        .to_raw_address_bytes();
    let ak = SpendValidatingKey::from(ctx.fvk.clone());
    let mut spent = 0u64;
    let mut out_total = 0u64;
    let mut spends: Vec<SeenSpend> = Vec::new();
    let mut found: Vec<(usize, [u8; ORCHARD_RECEIVER_LEN], u64)> = Vec::new();
    let mut change_outputs: Vec<(usize, u64)> = Vec::new();
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
                        "input {index}: spend is not a note of this vault ({e:?})"
                    ));
                    return;
                }
                if let Err(e) = spend.verify_rk(Some(ctx.fvk)) {
                    failure = Some(format!(
                        "input {index}: spend rk is inconsistent with this vault ({e:?})"
                    ));
                    return;
                }
                let (Some(recipient), Some(value), Some(rho), Some(rseed)) =
                    (spend.recipient(), spend.value(), spend.rho(), spend.rseed())
                else {
                    failure = Some(format!("input {index}: spent note fields are missing"));
                    return;
                };
                let note = Option::from(orchard::Note::from_parts(
                    *recipient,
                    *value,
                    *rho,
                    *rseed,
                    *spend.note_version(),
                ));
                let Some(note) = note else {
                    failure = Some(format!(
                        "input {index}: spent note fields do not form a valid note"
                    ));
                    return;
                };
                let note: orchard::Note = note;
                let cmx = ExtractedNoteCommitment::from(note.commitment()).to_bytes();
                spent = spent.saturating_add(value.inner());
                spends.push(SeenSpend { index, cmx });
            }
            if let Err(e) = output.verify_note_commitment(spend) {
                failure = Some(format!(
                    "output {index}: note commitment does not recompute ({e:?})"
                ));
                return;
            }
            let (Some(recipient), Some(value)) = (output.recipient(), output.value()) else {
                failure = Some(format!("output {index}: recipient or value missing"));
                return;
            };
            let value = value.inner();
            out_total = out_total.saturating_add(value);
            if value == 0 {
                continue;
            }
            let raw = recipient.to_raw_address_bytes();
            if raw == change_address {
                change_outputs.push((index, value));
            } else {
                found.push((index, raw, value));
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

    // 3. Payouts: each output must be one expected (receiver, amount).
    let mut unmatched: Vec<&ExpectedPayout> = expected_payouts.iter().collect();
    for (index, raw, value) in &found {
        if let Some(pos) = unmatched
            .iter()
            .position(|e| e.receiver == *raw && e.net == *value)
        {
            unmatched.swap_remove(pos);
            continue;
        }
        if let Some(e) = unmatched.iter().find(|e| e.net == *value) {
            return Err(refuse(format!(
                "output {index}: recipient does not match the sealed payout receiver for round {} (amount matches)",
                e.round
            )));
        }
        if let Some(e) = unmatched.iter().find(|e| e.receiver == *raw) {
            return Err(refuse(format!(
                "output {index}: amount {} expected {}",
                zat(*value),
                zat(e.net)
            )));
        }
        return Err(refuse(format!(
            "output {index}: unexpected output of {} zatoshis (not a payout of rounds {rounds:?} nor vault change)",
            zat(*value)
        )));
    }
    if let Some(e) = unmatched.first() {
        return Err(refuse(format!(
            "round {} payout {} (net {}) is not among the outputs",
            e.round,
            e.index,
            zat(e.net)
        )));
    }

    // 4. Exactly one change output.
    if change_outputs.len() != 1 {
        return Err(refuse(format!(
            "the transaction has {} outputs to the vault's change address; exactly one is expected",
            change_outputs.len()
        )));
    }

    // 5. Inputs: the charged dig notes, plus float notes only when needed.
    let mut float_inputs: Vec<&SeenSpend> = Vec::new();
    let mut matched_inputs = vec![false; expected_inputs.len()];
    for spend in &spends {
        if let Some(pos) = expected_inputs.iter().position(|e| e.cmx == spend.cmx) {
            if matched_inputs[pos] {
                return Err(refuse(format!(
                    "input {}: dig note spent twice",
                    spend.index
                )));
            }
            matched_inputs[pos] = true;
            continue;
        }
        if ctx.chain.note_by_cmx(&spend.cmx)?.is_some() {
            return Err(refuse(format!(
                "input {}: note is not a dig of rounds {rounds:?}",
                spend.index
            )));
        }
        float_inputs.push(spend);
    }
    if let Some(pos) = matched_inputs.iter().position(|m| !m) {
        let e = &expected_inputs[pos];
        return Err(refuse(format!(
            "dig note {}:{} of round {} is not spent by the transaction",
            e.txid, e.output_index, e.round
        )));
    }
    let charged_value = expected_inputs
        .iter()
        .try_fold(0u64, |acc, e| acc.checked_add(e.value))
        .ok_or_else(|| refuse("charged value overflow"))?;
    let payouts_value = expected_payouts
        .iter()
        .try_fold(0u64, |acc, e| acc.checked_add(e.net))
        .ok_or_else(|| refuse("payout value overflow"))?;
    let charged_count = u64::try_from(expected_inputs.len()).map_err(|_| refuse("count"))?;
    let payout_count = u64::try_from(expected_payouts.len()).map_err(|_| refuse("count"))?;
    if let Some(first_float) = float_inputs.first() {
        let fee_without_float = settle::zip317_fee(params, charged_count, payout_count + 1)
            .map_err(|e| refuse(e.to_string()))?;
        let needed = payouts_value
            .checked_add(fee_without_float)
            .ok_or_else(|| refuse("needed value overflow"))?;
        if charged_value > needed {
            return Err(refuse(format!(
                "input {}: float note not needed: the dig notes of rounds {rounds:?} cover the outputs and the fee",
                first_float.index
            )));
        }
    }

    // Fee: exact for the shape, and within what the rules collected.
    let fee = spent
        .checked_sub(out_total)
        .ok_or_else(|| refuse("outputs exceed spent value"))?;
    let actions: usize = action_counts.values().sum();
    let expected_fee = settle::zip317_fee(
        params,
        u64::try_from(actions).map_err(|_| refuse("count"))?,
        0,
    )
    .map_err(|e| refuse(e.to_string()))?;
    if fee != expected_fee {
        return Err(refuse(format!(
            "fee {} differs from the ZIP 317 fee {} for {actions} actions",
            zat(fee),
            zat(expected_fee)
        )));
    }
    if fee > collected_fees {
        return Err(refuse(format!(
            "fee {} exceeds the fees collected by the rules for rounds {rounds:?} ({})",
            zat(fee),
            zat(collected_fees)
        )));
    }

    // Pending spends and their randomized keys.
    let pending =
        send::pending_spends(&pczt).map_err(|e| refuse(format!("no signable spends: {e}")))?;
    for spend in &pending {
        let expected_rk: [u8; 32] = (&ak.randomize(&spend.alpha)).into();
        if expected_rk != spend.rk {
            return Err(refuse(format!(
                "input {}: rk is not this vault's ak randomized by the action's alpha",
                spend.index
            )));
        }
    }
    let sighash = Signer::new(pczt)
        .map_err(|e| refuse(format!("signer cannot parse the PCZT: {e:?}")))?
        .shielded_sighash();
    let mut receivers: Vec<[u8; ORCHARD_RECEIVER_LEN]> =
        expected_payouts.iter().map(|e| e.receiver).collect();
    receivers.sort_unstable();
    receivers.dedup();
    tracing::info!(
        rounds = ?rounds,
        payouts = found.len(),
        distinct_receivers = receivers.len(),
        charged_inputs = expected_inputs.len(),
        float_inputs = float_inputs.len(),
        fee,
        collected = collected_fees,
        "payout PCZT verified against this node's own settlement"
    );
    Ok(Approved {
        pending,
        sighash,
        rounds: rounds.to_vec(),
        payouts: found.len(),
        distinct_receivers: receivers.len(),
        charged_inputs: expected_inputs.len(),
        float_inputs: float_inputs.len(),
        fee,
    })
}

#[cfg(test)]
mod tests {
    use super::zat;

    #[test]
    fn thousands_separators() {
        assert_eq!(zat(0), "0");
        assert_eq!(zat(999), "999");
        assert_eq!(zat(1_000), "1,000");
        assert_eq!(zat(2_370_000), "2,370,000");
        assert_eq!(zat(1_234_567_890), "1,234,567,890");
    }
}
