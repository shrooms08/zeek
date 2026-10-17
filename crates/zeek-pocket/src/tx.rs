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
    /// Campfire message carried by every dig of this transaction (rules v4).
    #[serde(default)]
    pub message: Option<String>,
}

/// A shout request (rules v4).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShoutRequest {
    pub message: String,
    pub amount_zat: u64,
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

/// Smallest part a split makes: 0.02 ZEC covers a typical dig (for
/// example 3 tiles at 0.005, or 15 at the 0.001 minimum, with fees) with
/// room to spare. Below two such parts there is nothing to gain.
pub const SPLIT_PART_MIN_ZAT: u64 = 2_000_000;
/// Most parts a split makes. Under ZIP 317 a transaction costs 5,000
/// zatoshis per action and a split of one note into k parts has k actions,
/// so 4 parts cost 20,000 zatoshis (0.0002 ZEC); more would mostly make
/// later digs spend extra inputs.
pub const SPLIT_MAX_PARTS: u64 = 4;
/// A split may cost at most this share of what it splits (1 %).
pub const SPLIT_MAX_FEE_DIVISOR: u64 = 100;

/// A planned split: `parts` notes of `part_zat` (the last one also takes the
/// remainder) from `total_zat` of spendable notes, for `fee_zat`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitPlan {
    pub inputs: usize,
    pub total_zat: u64,
    pub parts: u64,
    pub part_zat: u64,
    pub fee_zat: u64,
}

/// Whether and how to split `notes` (the spendable notes, any order).
///
/// A split only pays when it lets a second dig in the same round go ahead
/// without waiting for the first one's change, so: never when two notes of
/// at least [`SPLIT_PART_MIN_ZAT`] already exist (a second dig already has
/// one), never into fewer than two such parts, and never when the fee would
/// exceed 1 % of the amount.
pub fn plan_split(notes: &[&StoredNote]) -> Result<Option<SplitPlan>, PocketError> {
    let usable = notes
        .iter()
        .filter(|n| n.value >= SPLIT_PART_MIN_ZAT)
        .count();
    if usable >= 2 || notes.is_empty() {
        return Ok(None);
    }
    let total = notes
        .iter()
        .try_fold(0u64, |acc, n| acc.checked_add(n.value))
        .ok_or(PocketError::Overflow("split total"))?;
    let mut parts = (total / SPLIT_PART_MIN_ZAT).min(SPLIT_MAX_PARTS);
    while parts >= 2 {
        let outputs = usize::try_from(parts).map_err(|_| PocketError::Overflow("parts"))?;
        let fee = zip317_fee(notes.len(), outputs)?;
        let part = total.saturating_sub(fee) / parts;
        if part >= SPLIT_PART_MIN_ZAT {
            if fee.saturating_mul(SPLIT_MAX_FEE_DIVISOR) > total {
                return Ok(None);
            }
            return Ok(Some(SplitPlan {
                inputs: notes.len(),
                total_zat: total,
                parts,
                part_zat: part,
                fee_zat: fee,
            }));
        }
        parts -= 1;
    }
    Ok(None)
}

/// Which spendable notes a transaction uses.
enum InputChoice {
    /// The fewest, largest first, that cover it (payments).
    Fewest,
    /// All of them (a split).
    All,
}

/// Spends every note in `notes` to pay `outputs_total` over `payments`
/// outputs plus one change output.
fn select_all(
    notes: &[&StoredNote],
    outputs_total: u64,
    payments: usize,
) -> Result<(Vec<StoredNote>, u64, u64), PocketError> {
    let total = notes
        .iter()
        .try_fold(0u64, |acc, n| acc.checked_add(n.value))
        .ok_or(PocketError::Overflow("inputs"))?;
    let fee = zip317_fee(notes.len(), payments + 1)?;
    let needed = outputs_total
        .checked_add(fee)
        .ok_or(PocketError::Overflow("needed"))?;
    if total < needed || notes.is_empty() {
        return Err(PocketError::InsufficientFunds {
            needed,
            spendable: total,
            pending: 0,
        });
    }
    Ok((
        notes.iter().map(|n| (*n).clone()).collect(),
        fee,
        total - needed,
    ))
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
        let message = request.message.as_deref().filter(|m| !m.is_empty());
        if let Some(message) = message {
            if !config
                .params
                .campfire_active(request.chain_tip.saturating_add(1))
            {
                return Err(PocketError::InvalidRequest(
                    "Campfire messages are not active yet in this game".into(),
                ));
            }
            zeek_core::memo::validate_message(
                message.as_bytes(),
                zeek_core::memo::MAX_DIG_MESSAGE_LEN,
            )
            .map_err(|e| PocketError::InvalidRequest(format!("message: {e}")))?;
        }

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
            let memo = match message {
                Some(m) => memo
                    .with_message(m)
                    .map_err(|e| PocketError::InvalidRequest(format!("message: {e}")))?,
                None => memo,
            };
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

    /// Builds, proves and signs a Campfire shout (rules v4): one payment to
    /// the vault carrying only a message. Its value goes to the Deep Cache.
    pub fn build_shout(
        &mut self,
        config: &EngineConfig,
        request: &ShoutRequest,
    ) -> Result<BuiltTransaction, PocketError> {
        let campfire = config
            .params
            .campfire
            .as_ref()
            .filter(|_| {
                config
                    .params
                    .campfire_active(request.chain_tip.saturating_add(1))
            })
            .ok_or_else(|| {
                PocketError::InvalidRequest("Campfire is not active yet in this game".into())
            })?;
        if request.amount_zat < campfire.min_shout_zat {
            return Err(PocketError::InvalidAmount(format!(
                "a shout must be at least {} zatoshis",
                campfire.min_shout_zat
            )));
        }
        self.require_synced(request.chain_tip)?;
        let memo = zeek_core::memo::ShoutMemo::new(&request.message)
            .map_err(|e| PocketError::InvalidRequest(format!("message: {e}")))?;
        let vault = parse_address(&config.vault_address)?;
        self.build(
            vec![Output {
                recipient: vault,
                value: request.amount_zat,
                memo: MemoBytes::from_bytes(&memo.encode_unpadded())
                    .map_err(|e| PocketError::Build(format!("memo: {e}")))?,
                scope: Scope::External,
            }],
            OutgoingKind::Shout {
                amount_zat: request.amount_zat,
                message: request.message.clone(),
            },
        )
    }

    /// Builds a win proof (see `zeek_verify::win`) for treasure payout
    /// `payout_index` of the round described by `bundle`, which must have
    /// been paid to one of this wallet's addresses.
    pub fn create_win_proof(
        &self,
        bundle: &zeek_core::proof::ProofBundle,
        payout_index: u32,
    ) -> Result<(String, Vec<u8>), PocketError> {
        let mut rng = OsRng;
        let proof = zeek_verify::win::create(
            bundle,
            payout_index,
            self.keys.fvk(),
            &self.keys.ask(),
            &circuit_keys().0,
            &mut rng,
        )
        .map_err(|e| match e {
            zeek_verify::win::WinProofError::NotOurAddress => {
                PocketError::InvalidRequest("that payout was not received by this wallet".into())
            }
            other => PocketError::Build(other.to_string()),
        })?;
        let encoded = proof
            .encode()
            .map_err(|e| PocketError::Build(e.to_string()))?;
        Ok((zeek_verify::win::WinProof::id(&encoded), encoded))
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
        self.build_from(outputs, kind, InputChoice::Fewest)
    }

    fn build_from(
        &mut self,
        outputs: Vec<Output>,
        kind: OutgoingKind,
        choice: InputChoice,
    ) -> Result<BuiltTransaction, PocketError> {
        let target = self.scanned_height() + 1;
        let anchor_height = target - TRUSTED_CONFIRMATIONS;
        let outputs_total = outputs
            .iter()
            .try_fold(0u64, |acc, o| acc.checked_add(o.value))
            .ok_or(PocketError::Overflow("outputs"))?;
        let pending = self.balance()?.pending_zat;
        let candidates = self.spendable_notes(target);
        let (inputs, fee, change) = match choice {
            InputChoice::Fewest => select(&candidates, outputs_total, outputs.len(), pending)?,
            InputChoice::All => select_all(&candidates, outputs_total, outputs.len())?,
        };

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

    /// The split the wallet would make now, if any (see [`plan_split`]).
    pub fn split_plan(&self) -> Result<Option<SplitPlan>, PocketError> {
        let target = self.scanned_height() + 1;
        plan_split(&self.spendable_notes(target))
    }

    /// Builds, proves and signs the split of [`Wallet::split_plan`]: every
    /// spendable note into `parts` notes at this wallet's own change
    /// address, so they confirm as trusted change (3 blocks).
    pub fn build_split(&mut self, chain_tip: u32) -> Result<BuiltTransaction, PocketError> {
        self.require_synced(chain_tip)?;
        let plan = self
            .split_plan()?
            .ok_or_else(|| PocketError::InvalidRequest("nothing worth splitting".into()))?;
        // parts − 1 outputs here; the change output carries the last part
        // and the remainder (all spendable notes are spent).
        let count = usize::try_from(plan.parts - 1).map_err(|_| PocketError::Overflow("parts"))?;
        let outputs = (0..count)
            .map(|_| Output {
                recipient: self.keys.change_address(),
                value: plan.part_zat,
                memo: MemoBytes::empty(),
                scope: Scope::Internal,
            })
            .collect();
        let parts = u32::try_from(plan.parts).map_err(|_| PocketError::Overflow("parts"))?;
        let built = self.build_from(
            outputs,
            OutgoingKind::Split {
                parts,
                part_zat: plan.part_zat,
            },
            InputChoice::All,
        )?;
        Ok(built)
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

    fn plan(values: &[u64]) -> Option<SplitPlan> {
        let notes: Vec<StoredNote> = values.iter().map(|v| note(*v)).collect();
        let refs: Vec<&StoredNote> = notes.iter().collect();
        plan_split(&refs).expect("plan")
    }

    #[test]
    fn a_faucet_deposit_splits_into_four() {
        // The faucet's 0.125: four parts of 0.03120000, for 20,000 zatoshis.
        let p = plan(&[12_500_000]).expect("split");
        assert_eq!(p.parts, 4);
        assert_eq!(p.fee_zat, 20_000);
        assert_eq!(p.part_zat, (12_500_000 - 20_000) / 4);
        assert!(p.part_zat >= SPLIT_PART_MIN_ZAT);
    }

    #[test]
    fn smaller_balances_split_into_fewer_parts_or_not_at_all() {
        assert_eq!(plan(&[6_000_000]).expect("split").parts, 2);
        assert_eq!(plan(&[7_000_000]).expect("split").parts, 3);
        // 0.0399: two parts would be under 0.02 after the fee.
        assert_eq!(plan(&[3_990_000]), None);
        assert_eq!(plan(&[1_000_000]), None);
        assert_eq!(plan(&[]), None);
    }

    #[test]
    fn a_digs_change_splits_again_by_the_same_rule() {
        // 0.5 split in four, three parts dug; the fourth (0.12495) paid one
        // tile of 0.001 (fee 0.0001 incl. change): its change is the last coin.
        let part = (50_000_000 - 20_000) / 4;
        let change = part - 100_000 - 10_000;
        let p = plan(&[change]).expect("re-split");
        assert_eq!((p.inputs, p.parts, p.fee_zat), (1, 4, 20_000));
        // Dust from earlier digs beside it goes along.
        assert_eq!(plan(&[change, 40_000]).expect("re-split").inputs, 2);
        // A faucet part's change after a 0.01 dig (0.0211) is too small for
        // two parts of 0.02: it stays one coin.
        assert_eq!(
            plan(&[(12_500_000 - 20_000) / 4 - 1_000_000 - 10_000]),
            None
        );
        // An untouched part beside the change: a second dig already has one.
        assert_eq!(plan(&[change, part]), None);
    }

    #[test]
    fn never_splits_when_a_second_note_is_already_there() {
        assert_eq!(plan(&[10_000_000, 2_000_000]), None);
        assert_eq!(plan(&[3_000_000, 3_000_000, 500_000]), None);
    }

    #[test]
    fn small_change_is_merged_into_the_split() {
        let p = plan(&[10_000_000, 300_000, 200_000]).expect("split");
        assert_eq!(p.inputs, 3);
        assert_eq!(p.total_zat, 10_500_000);
        assert_eq!(p.parts, 4);
        // Three inputs, four outputs: still 4 actions.
        assert_eq!(p.fee_zat, 20_000);
    }

    #[test]
    fn never_costs_more_than_one_percent() {
        // Many dust inputs push the action count (and fee) up.
        let mut values = vec![4_200_000];
        values.extend(std::iter::repeat_n(5_000, 60));
        assert_eq!(plan(&values), None);
        for total in [4_100_000u64, 8_000_000, 12_500_000, 50_000_000] {
            if let Some(p) = plan(&[total]) {
                assert!(p.fee_zat * SPLIT_MAX_FEE_DIVISOR <= p.total_zat, "{total}");
                assert!(p.part_zat * p.parts + p.fee_zat <= p.total_zat);
            }
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
