//! Building, proving and signing transactions: digs and plain sends.
//!
//! Inputs are this wallet's spendable Ironwood notes, largest first, until
//! they cover the outputs and the ZIP 317 fee of the resulting shape
//! (`5000 · max(2, spends, outputs + change)`). Change always goes to the
//! internal-scope address. The transaction targets the block after the last
//! scanned one, is anchored at the checkpoint [`TRUSTED_CONFIRMATIONS`]
//! blocks below the target, and is proven, signed with the account's spend
//! authorizing key and extracted (which verifies proofs and signatures)
//! entirely here; nothing leaves the wallet but the finished transaction.
//!
//! A dig seals a **fresh** diversified address for every tile, at the next
//! unused diversifier index, which is recorded before the transaction is
//! returned so it is never handed out again.

use std::convert::Infallible;
use std::sync::OnceLock;

use orchard::circuit::{OrchardCircuitVersion, ProvingKey, VerifyingKey};
use orchard::keys::Scope;
use pczt::roles::creator::Creator;
use pczt::roles::io_finalizer::IoFinalizer;
use pczt::roles::prover::Prover;
use pczt::roles::signer::Signer;
use pczt::roles::tx_extractor::TransactionExtractor;
use pczt::roles::verifier::Verifier;
use rand_chacha_09::rand_core::SeedableRng;
use rand_chacha_09::ChaCha20Rng;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use zcash_primitives::transaction::builder::{BuildConfig, Builder, BundlePadding, PcztResult};
use zcash_primitives::transaction::fees::zip317;
use zcash_protocol::consensus::{BlockHeight, TEST_NETWORK};
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::Zatoshis;
use zeek_core::memo::{DigMemo, PayoutReceiver};
use zeek_core::params::{GameParams, Network};
use zeek_core::seal::SealingPublicKey;

use crate::error::PocketError;
use crate::keys::{encode_address, parse_address};
use crate::store::{Outgoing, OutgoingKind, OutgoingStatus, StoredNote};
use crate::wallet::{Wallet, TRUSTED_CONFIRMATIONS};

/// Blocks that must remain in the round's window for a dig to be sent (the
/// same margin as `zeek-dig`).
pub const SAFETY_MARGIN_BLOCKS: u32 = 3;
/// ZIP 317 marginal fee.
pub const MARGINAL_FEE_ZAT: u64 = 5_000;
/// ZIP 317 grace actions.
pub const GRACE_ACTIONS: u64 = 2;
/// Most tiles one dig may cover.
pub const MAX_TILES: usize = 25;

static KEYS: OnceLock<(ProvingKey, VerifyingKey)> = OnceLock::new();

/// The Orchard-protocol circuit keys (built once per session, a few seconds).
pub fn circuit_keys() -> &'static (ProvingKey, VerifyingKey) {
    KEYS.get_or_init(|| {
        (
            ProvingKey::build(OrchardCircuitVersion::PostNu6_3),
            VerifyingKey::build(OrchardCircuitVersion::PostNu6_3),
        )
    })
}

/// The subset of the engine's `/v1/config` a dig needs.
#[derive(Clone, Debug, Deserialize)]
pub struct EngineConfig {
    pub params: GameParams,
    pub vault_address: String,
    pub sealing_public_key: SealingPublicKey,
}

/// A dig request.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DigRequest {
    pub tiles: Vec<u8>,
    pub amount_each_zat: u64,
    /// The board's current round and the blocks left in its window.
    pub round: u32,
    pub blocks_remaining: u32,
    /// The chain tip the relay reports; the wallet must be scanned to it.
    pub chain_tip: u32,
}

/// A send request.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendRequest {
    pub to: String,
    pub amount_zat: u64,
    pub memo: Option<String>,
    pub chain_tip: u32,
}

/// A finished transaction, ready to broadcast.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltTransaction {
    pub txid: String,
    /// Raw transaction bytes, hex.
    pub raw_hex: String,
    pub fee_zat: u64,
    pub target_height: u32,
    pub expiry_height: u32,
    pub spends: usize,
    pub outputs: usize,
    pub proving_ms: f64,
}

/// ZIP 317 fee for `spends` spends and `outputs` outputs.
pub fn zip317_fee(spends: usize, outputs: usize) -> Result<u64, PocketError> {
    let actions = u64::try_from(spends.max(outputs))
        .map_err(|_| PocketError::Overflow("action count"))?
        .max(GRACE_ACTIONS);
    MARGINAL_FEE_ZAT
        .checked_mul(actions)
        .ok_or(PocketError::Overflow("fee"))
}

/// Chooses notes, largest first, to pay `outputs_total` over `payments`
/// payment outputs plus one change output. Returns the notes, the fee and
/// the change.
pub fn select(
    notes: &[&StoredNote],
    outputs_total: u64,
    payments: usize,
    pending_zat: u64,
) -> Result<(Vec<StoredNote>, u64, u64), PocketError> {
    let mut chosen = Vec::new();
    let mut total = 0u64;
    for note in notes {
        chosen.push((*note).clone());
        total = total
            .checked_add(note.value)
            .ok_or(PocketError::Overflow("inputs"))?;
        let fee = zip317_fee(chosen.len(), payments + 1)?;
        let needed = outputs_total
            .checked_add(fee)
            .ok_or(PocketError::Overflow("needed"))?;
        if total >= needed {
            return Ok((chosen, fee, total - needed));
        }
    }
    let fee = zip317_fee(notes.len().max(1), payments + 1)?;
    Err(PocketError::InsufficientFunds {
        needed: outputs_total
            .checked_add(fee)
            .ok_or(PocketError::Overflow("needed"))?,
        spendable: total,
        pending: pending_zat,
    })
}

struct Output {
    recipient: orchard::Address,
    value: u64,
    memo: MemoBytes,
    scope: Scope,
}

impl Wallet {
    fn require_synced(&self, chain_tip: u32) -> Result<(), PocketError> {
        if self.scanned_height() < chain_tip {
            return Err(PocketError::NotSynced {
                scanned: self.scanned_height(),
                tip: chain_tip,
            });
        }
        Ok(())
    }

    /// Builds, proves and signs a dig: one vault output per tile, each with
    /// a sealed memo naming a fresh diversified payout address of this wallet.
    pub fn build_dig(
        &mut self,
        config: &EngineConfig,
        request: &DigRequest,
    ) -> Result<BuiltTransaction, PocketError> {
        if config.params.network != Network::Test {
            return Err(PocketError::InvalidRequest(
                "this wallet plays on testnet only".into(),
            ));
        }
        if request.tiles.is_empty() || request.tiles.len() > MAX_TILES {
            return Err(PocketError::InvalidRequest(format!(
                "choose between 1 and {MAX_TILES} tiles"
            )));
        }
        let mut unique = request.tiles.clone();
        unique.sort_unstable();
        unique.dedup();
        if unique.len() != request.tiles.len() || unique.iter().any(|t| *t >= zeek_core::TILE_COUNT)
        {
            return Err(PocketError::InvalidRequest(
                "tiles must be distinct, 0 to 24".into(),
            ));
        }
        if request.amount_each_zat < config.params.min_dig_zat {
            return Err(PocketError::InvalidAmount(format!(
                "each dig must be at least {} zatoshis",
                config.params.min_dig_zat
            )));
        }
        if request.blocks_remaining < SAFETY_MARGIN_BLOCKS + 1 {
            return Err(PocketError::ClosingTooSoon {
                round: request.round,
                blocks_remaining: request.blocks_remaining,
                margin: SAFETY_MARGIN_BLOCKS,
            });
        }
        self.require_synced(request.chain_tip)?;
        let vault = parse_address(&config.vault_address)?;

        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        let mut rng = ChaCha20Rng::from_seed(seed);
        let first_index = self.state.next_receiver_index;
        let mut outputs = Vec::with_capacity(request.tiles.len());
        let mut indices = Vec::with_capacity(request.tiles.len());
        for (offset, tile) in request.tiles.iter().enumerate() {
            let index = first_index
                .checked_add(offset as u64)
                .ok_or(PocketError::Overflow("diversifier index"))?;
            let receiver = PayoutReceiver {
                network: Network::Test,
                orchard: self.keys.address(index).to_raw_address_bytes(),
            };
            let memo = DigMemo::seal(
                request.round,
                *tile,
                &receiver,
                &config.sealing_public_key,
                &mut rng,
            )
            .map_err(|e| PocketError::Build(format!("sealing the payout receiver: {e}")))?;
            outputs.push(Output {
                recipient: vault,
                value: request.amount_each_zat,
                memo: MemoBytes::from_bytes(&memo.encode_unpadded())
                    .map_err(|e| PocketError::Build(format!("memo: {e}")))?,
                scope: Scope::External,
            });
            indices.push(index);
        }
        let kind = OutgoingKind::Dig {
            round: request.round,
            tiles: request.tiles.clone(),
            amount_each_zat: request.amount_each_zat,
            receiver_indices: indices.clone(),
            memo_encoding: "binary".into(),
        };
        let built = self.build(outputs, kind)?;
        // Reserve the indices only once the transaction exists.
        self.state.next_receiver_index = first_index
            .checked_add(indices.len() as u64)
            .ok_or(PocketError::Overflow("diversifier index"))?;
        Ok(built)
    }

    /// Builds, proves and signs a payment to any address with an
    /// Orchard-protocol receiver, with an optional text memo.
    pub fn build_send(&mut self, request: &SendRequest) -> Result<BuiltTransaction, PocketError> {
        if request.amount_zat == 0 {
            return Err(PocketError::InvalidAmount("amount must be positive".into()));
        }
        self.require_synced(request.chain_tip)?;
        let recipient = parse_address(&request.to)?;
        let memo = match &request.memo {
            Some(text) if !text.is_empty() => MemoBytes::from_bytes(text.as_bytes())
                .map_err(|_| PocketError::InvalidRequest("memo is longer than 512 bytes".into()))?,
            _ => MemoBytes::empty(),
        };
        let kind = OutgoingKind::Send {
            to: request.to.clone(),
            amount_zat: request.amount_zat,
            memo: request.memo.clone().filter(|m| !m.is_empty()),
        };
        self.build(
            vec![Output {
                recipient,
                value: request.amount_zat,
                memo,
                scope: Scope::External,
            }],
            kind,
        )
    }

    fn build(
        &mut self,
        outputs: Vec<Output>,
        kind: OutgoingKind,
    ) -> Result<BuiltTransaction, PocketError> {
        let target = self.scanned_height() + 1;
        let anchor_height = target - TRUSTED_CONFIRMATIONS;
        let outputs_total = outputs
            .iter()
            .try_fold(0u64, |acc, o| acc.checked_add(o.value))
            .ok_or(PocketError::Overflow("outputs"))?;
        let pending = self.balance()?.pending_zat;
        let candidates = self.spendable_notes(target);
        let (inputs, fee, change) = select(&candidates, outputs_total, outputs.len(), pending)?;

        let anchor_id = BlockHeight::from_u32(anchor_height);
        let root = self
            .tree
            .root_at_checkpoint_id(&anchor_id)
            .map_err(|e| PocketError::Corrupt(format!("anchor: {e:?}")))?
            .ok_or_else(|| {
                PocketError::Corrupt(format!("no checkpoint at anchor {anchor_height}"))
            })?;
        let mut builder = Builder::new(
            TEST_NETWORK,
            BlockHeight::from_u32(target),
            BuildConfig::Standard {
                sapling_anchor: None,
                orchard_anchor: None,
                ironwood_anchor: Some(orchard::Anchor::from(root)),
                orchard_padding: BundlePadding::DEFAULT,
                ironwood_padding: BundlePadding::DEFAULT,
            },
        );
        let fvk = self.keys.fvk().clone();
        for note in &inputs {
            let path = self
                .tree
                .witness_at_checkpoint_id_caching(note.position.into(), &anchor_id)
                .map_err(|e| PocketError::Corrupt(format!("witness: {e:?}")))?
                .ok_or_else(|| PocketError::Corrupt("note has no witness at the anchor".into()))?;
            builder
                .add_ironwood_spend::<zip317::FeeError>(fvk.clone(), note.note()?, path.into())
                .map_err(|e| PocketError::Build(e.to_string()))?;
        }
        let output_count = outputs.len() + 1;
        for output in outputs {
            builder
                .add_ironwood_output::<zip317::FeeError>(
                    Some(fvk.to_ovk(output.scope)),
                    output.recipient,
                    Zatoshis::from_u64(output.value)
                        .map_err(|_| PocketError::Overflow("amount"))?,
                    output.memo,
                )
                .map_err(|e| PocketError::Build(e.to_string()))?;
        }
        builder
            .add_ironwood_output::<zip317::FeeError>(
                Some(fvk.to_ovk(Scope::Internal)),
                self.keys.change_address(),
                Zatoshis::from_u64(change).map_err(|_| PocketError::Overflow("change"))?,
                MemoBytes::empty(),
            )
            .map_err(|e| PocketError::Build(e.to_string()))?;
        let PcztResult { pczt_parts, .. } = builder
            .build_for_pczt(OsRng, &zip317::FeeRule::standard())
            .map_err(|e| PocketError::Build(e.to_string()))?;
        let pczt = Creator::build_from_parts(pczt_parts)
            .ok_or_else(|| PocketError::Build("PCZT creator".into()))?;
        let pczt = IoFinalizer::new(pczt)
            .finalize_io()
            .map_err(|e| PocketError::Build(format!("io finalizer: {e:?}")))?;

        let (proving_key, verifying_key) = circuit_keys();
        let started = now_ms();
        let pczt = Prover::new(pczt)
            .create_ironwood_proof(proving_key)
            .map_err(|e| PocketError::Build(format!("prove: {e:?}")))?
            .finish();
        let proving_ms = now_ms() - started;

        let mut pending_spends = Vec::new();
        Verifier::new(pczt.clone())
            .with_ironwood::<Infallible, _>(|bundle| {
                for (i, action) in bundle.actions().iter().enumerate() {
                    if action.spend().spend_auth_sig().is_none() {
                        pending_spends.push(i);
                    }
                }
                Ok(())
            })
            .map_err(|e| PocketError::Build(format!("verifier: {e:?}")))?;
        let mut signer =
            Signer::new(pczt).map_err(|e| PocketError::Build(format!("signer: {e:?}")))?;
        let ask = self.keys.ask();
        for index in pending_spends {
            signer
                .sign_ironwood(index, &ask)
                .map_err(|e| PocketError::Build(format!("sign action {index}: {e:?}")))?;
        }
        let transaction = TransactionExtractor::new(signer.finish())
            .with_orchard(verifying_key)
            .extract()
            .map_err(|e| PocketError::Build(format!("extract: {e:?}")))?;
        let mut raw = Vec::new();
        transaction
            .write(&mut raw)
            .map_err(|e| PocketError::Build(format!("serialize: {e}")))?;
        let txid = transaction.txid().to_string();
        let expiry_height = u32::from(transaction.expiry_height());
        self.state.outgoing.push(Outgoing {
            txid: txid.clone(),
            kind,
            fee_zat: fee,
            spends: inputs
                .iter()
                .map(|n| (n.txid.clone(), n.action_index))
                .collect(),
            change_zat: change,
            target_height: target,
            expiry_height,
            status: OutgoingStatus::Built,
            mined_height: None,
        });
        Ok(BuiltTransaction {
            txid,
            raw_hex: hex::encode(raw),
            fee_zat: fee,
            target_height: target,
            expiry_height,
            spends: inputs.len(),
            outputs: output_count,
            proving_ms,
        })
    }

    /// The wallet's receive address (diversifier index 0).
    pub fn receive_address(&self) -> Result<String, PocketError> {
        encode_address(&self.keys.address(0))
    }
}

#[cfg(target_arch = "wasm32")]
fn now_ms() -> f64 {
    js_sys::Date::now()
}

#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1_000.0)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{NotePool, NoteScope};

    fn note(value: u64) -> StoredNote {
        StoredNote {
            txid: format!("{value:064x}"),
            action_index: 0,
            pool: NotePool::Ironwood,
            height: 1,
            value,
            recipient: vec![],
            rho: [0; 32],
            rseed: [0; 32],
            note_version: 3,
            scope: NoteScope::External,
            diversifier_index: Some(0),
            position: value,
            nullifier: [0; 32],
            spent: None,
            memo: None,
        }
    }

    #[test]
    fn fees_follow_zip317() {
        assert_eq!(zip317_fee(1, 2).expect("fee"), 10_000);
        assert_eq!(zip317_fee(1, 3).expect("fee"), 15_000);
        assert_eq!(zip317_fee(26, 3).expect("fee"), 130_000);
    }

    #[test]
    fn selection_takes_largest_first_and_reports_shortfalls() {
        let notes = [note(1_000_000), note(300_000), note(50_000)];
        let refs: Vec<&StoredNote> = notes.iter().collect();
        // Two tiles of 100,000: 3 outputs, fee 15,000.
        let (chosen, fee, change) = select(&refs, 200_000, 2, 0).expect("select");
        assert_eq!(chosen.len(), 1);
        assert_eq!(fee, 15_000);
        assert_eq!(change, 1_000_000 - 200_000 - 15_000);
        // Needs two notes: fee stays 15,000 (max(2 spends, 3 outputs)).
        let (chosen, fee, change) = select(&refs, 1_200_000, 2, 0).expect("select");
        assert_eq!(chosen.len(), 2);
        assert_eq!(fee, 15_000);
        assert_eq!(change, 1_300_000 - 1_200_000 - 15_000);
        assert!(matches!(
            select(&refs, 2_000_000, 1, 7),
            Err(PocketError::InsufficientFunds {
                spendable: 1_350_000,
                pending: 7,
                ..
            })
        ));
    }
}
