//! Prove-a-win: "the treasure payout of round N, output k, was received at
//! an address I control", without revealing anything else about the
//! prover's wallet.
//!
//! # Construction
//!
//! The winner's payout output is already opened in the round's proof bundle
//! (format 4, [`zeek_core::proof::PayoutOpening`]), so anyone can check that
//! output `k` of the payout transaction pays the settled amount to address
//! `A = (d, pk_d)`. What remains is a proof of **control** of `A`, in the
//! style of ZIP 304 (Sapling address signatures), adapted to the
//! Orchard-protocol circuit the Ironwood pool uses:
//!
//! 1. The prover makes up a note `n* = (A, v = 1, rho*, rseed*)` with random
//!    `rho*` and `rseed*`. It never exists on chain.
//! 2. The anchor is the root of a depth-32 Orchard note commitment tree
//!    whose only leaf is `cmx(n*)` at position 0 (every other node is the
//!    empty root). The verifier recomputes this anchor from `A`, `rho*` and
//!    `rseed*` itself.
//! 3. The prover builds a one-action Ironwood bundle that spends `n*` and
//!    creates no output (the action's output side is a fabricated
//!    zero-valued note to `A`), so its value balance is **+1**. It proves the
//!    bundle with the Orchard-protocol circuit and signs it, spend
//!    authorization and binding signature, over the message
//!    `BLAKE2b-256("ZeekWinProofV1__", "zeek-win-proof/v1" ‖ network ‖ round ‖
//!    payout txid ‖ k ‖ recipient ‖ nonce)` instead of a transaction sighash.
//!
//! # Why not a zero-value note
//!
//! The circuit only checks the spent note's Merkle path when its value is
//! non-zero: "Either v_old = 0, or root = anchor" (orchard 0.15.5
//! `src/circuit.rs`, gate "Orchard circuit checks"; protocol spec § 4.17.4,
//! Action statement, Merkle path validity). The spent note's address is not
//! a public input; it is bound to the statement only through its commitment
//! and that Merkle check. A zero-value spend would therefore prove control
//! of *some* key, not of `A`. Spending a 1-zatoshi made-up note keeps the
//! Merkle check on, and the +1 value balance, enforced by the binding
//! signature, guarantees that a non-zero spend exists.
//!
//! # What verification establishes
//!
//! With the Halo2 proof valid for the instance (anchor, `cv_net`, `nf`,
//! `rk`, `cmx`, flags), the binding signature valid for value balance +1 and
//! the spend authorization signature valid under `rk`, all over the message:
//!
//! * some spent note has `v_old ≥ 1`, so its commitment is in the tree with
//!   root `anchor`, whose only leaf is `cmx(n*)`: the spent note is `n*`
//!   (collision resistance of Sinsemilla note commitments and the Merkle
//!   hash, spec § 5.4.1.9, § 5.4.8.4);
//! * address integrity: `pk_d = [ivk] g_d` with `ivk = Commit^ivk_rivk(ak,
//!   nk)` (spec § 4.17.4), so the prover knows `ak, nk, rivk` of a full
//!   viewing key that `A` belongs to;
//! * spend authority: `rk = ak + [α] G` is proven, and the signature under
//!   `rk` proves knowledge of `rsk = ask + α`, hence of `ask` (spec § 4.15);
//! * the message binds the round, the payout transaction, the output index,
//!   the recipient and a fresh nonce, so the proof cannot be moved to
//!   another payout, and the same claim made twice gives unlinkable proofs.
//!
//! Together with the bundle's opening of output `k`, which recomputes the
//! output's `cmx` in the mined payout transaction, and the settlement saying
//! payout `k` is the treasure share of round N, this proves: *whoever made
//! this proof can spend notes sent to the address that received round N's
//! treasure payout k*.
//!
//! It does **not** prove that the prover still holds the payout, that the
//! proof was made by the original winner rather than someone the winner
//! gave the spending key to, or anything about other addresses: the proof
//! reveals only `A` (already in the bundle), a random nullifier of the
//! made-up note, a randomized `rk` and a value commitment.

use blake2b_simd::Params;
use incrementalmerkletree::{Hashable, Level};
use nonempty::NonEmpty;
use orchard::builder::{Builder, BundleType};
use orchard::bundle::{Authorized, BundleVersion, Flags};
use orchard::circuit::ProvingKey;
use orchard::keys::{FullViewingKey, SpendAuthorizingKey};
use orchard::note::{
    ExtractedNoteCommitment, NoteVersion, Nullifier, RandomSeed, Rho, TransmittedNoteCiphertext,
};
use orchard::primitives::redpallas::{self, Binding, SpendAuth};
use orchard::tree::{MerkleHashOrchard, MerklePath};
use orchard::value::{NoteValue, ValueCommitment};
use orchard::{Action, Address, Anchor, Bundle, Note, Proof, NOTE_COMMITMENT_TREE_DEPTH};
use pasta_curves::group::ff::{Field, PrimeField};
use pasta_curves::pallas;
use rand_core::{CryptoRng, RngCore};
use serde::{Deserialize, Serialize};
use zeek_core::params::Network;
use zeek_core::proof::{PayoutOpening, ProofBundle};
use zeek_core::settle::PayoutReason;

use crate::chain::{parse_tx, txid_bytes, MinedTx};
use crate::checks::{opening_cmx, verifying_key};

/// Domain string bound into every win proof message.
pub const DOMAIN: &[u8] = b"zeek-win-proof/v1";
/// BLAKE2b personalization of the message hash (16 bytes).
pub const PERSONALIZATION: &[u8; 16] = b"ZeekWinProofV1__";
/// Format version of [`WinProof`].
pub const WIN_PROOF_VERSION: u8 = 1;
/// Largest encoded proof accepted (a one-action proof is about 7.4 kB).
pub const MAX_ENCODED_LEN: usize = 16 * 1024;

/// Errors of creating or decoding a win proof.
#[derive(Debug, thiserror::Error)]
pub enum WinProofError {
    #[error("the payout address does not belong to this wallet's keys")]
    NotOurAddress,
    #[error("payout {0} of the round is not a treasure payout")]
    NotATreasurePayout(u32),
    #[error("the round's proof bundle has no payout openings yet (payout not mined?)")]
    NoOpenings,
    #[error("payout {0} has no opening in the bundle")]
    NoOpening(u32),
    #[error("round {round} has no payout {index} (it made {count} payout(s))")]
    NoPayout {
        round: u32,
        index: u32,
        count: usize,
    },
    #[error("the opening is malformed: {0}")]
    BadOpening(String),
    #[error("building the proof failed: {0}")]
    Build(String),
    #[error("the encoded proof is malformed: {0}")]
    Encoding(String),
}

/// The action of the proof, as bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionBytes {
    pub nf: [u8; 32],
    pub rk: [u8; 32],
    pub cmx: [u8; 32],
    pub cv_net: [u8; 32],
    pub epk: [u8; 32],
    pub enc_ciphertext: Vec<u8>,
    pub out_ciphertext: Vec<u8>,
    pub spend_auth_sig: Vec<u8>,
}

/// A win proof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WinProof {
    pub version: u8,
    pub network: Network,
    pub round: u32,
    /// The payout transaction (display hex).
    pub payout_txid: String,
    /// Index of the payout in the round's settlement.
    pub payout_index: u32,
    /// The payout output's opening (as in the round's proof bundle).
    pub opening: PayoutOpening,
    pub nonce: [u8; 32],
    /// `rho` and `rseed` of the made-up 1-zatoshi note at the payout address.
    pub note_rho: [u8; 32],
    pub note_rseed: [u8; 32],
    pub flags: u8,
    pub anchor: [u8; 32],
    pub action: ActionBytes,
    pub proof: Vec<u8>,
    pub binding_sig: Vec<u8>,
}

/// The message both signatures sign.
pub fn message(
    network: Network,
    round: u32,
    payout_txid: &[u8; 32],
    payout_index: u32,
    recipient: &[u8; 43],
    nonce: &[u8; 32],
) -> [u8; 32] {
    let mut state = Params::new()
        .hash_length(32)
        .personal(PERSONALIZATION)
        .to_state();
    state.update(DOMAIN);
    state.update(&[network.receiver_tag()]);
    state.update(&round.to_be_bytes());
    state.update(payout_txid);
    state.update(&payout_index.to_be_bytes());
    state.update(recipient);
    state.update(nonce);
    let mut out = [0u8; 32];
    out.copy_from_slice(state.finalize().as_bytes());
    out
}

/// The Merkle path of position 0 in a tree that is empty elsewhere.
fn lone_leaf_path() -> MerklePath {
    let auth_path: [MerkleHashOrchard; NOTE_COMMITMENT_TREE_DEPTH] =
        core::array::from_fn(|level| MerkleHashOrchard::empty_root(Level::from(level as u8)));
    MerklePath::from_parts(0, auth_path)
}

fn made_up_note(address: Address, rho: &[u8; 32], rseed: &[u8; 32]) -> Option<Note> {
    let rho = Option::<Rho>::from(Rho::from_bytes(rho))?;
    let rseed = Option::<RandomSeed>::from(RandomSeed::from_bytes(*rseed, &rho))?;
    Option::from(Note::from_parts(
        address,
        NoteValue::from_raw(1),
        rho,
        rseed,
        NoteVersion::V3,
    ))
}

/// The anchor of the one-leaf tree holding the made-up note.
fn lone_leaf_anchor(note: &Note) -> Anchor {
    lone_leaf_path().root(ExtractedNoteCommitment::from(note.commitment()))
}

fn opening_address(opening: &PayoutOpening) -> Result<([u8; 43], Address), WinProofError> {
    let raw: [u8; 43] = hex::decode(&opening.recipient)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| WinProofError::BadOpening("recipient is not 43 bytes".into()))?;
    let address = Option::<Address>::from(Address::from_raw_address_bytes(&raw))
        .ok_or_else(|| WinProofError::BadOpening("recipient is not an address".into()))?;
    Ok((raw, address))
}

/// The opening of treasure payout `payout_index` from a round's bundle.
pub fn treasure_opening(
    bundle: &ProofBundle,
    payout_index: u32,
) -> Result<PayoutOpening, WinProofError> {
    let payout =
        bundle
            .settlement
            .payouts
            .get(payout_index as usize)
            .ok_or(WinProofError::NoPayout {
                round: bundle.round,
                index: payout_index,
                count: bundle.settlement.payouts.len(),
            })?;
    if !matches!(payout.reason, PayoutReason::Win { .. }) {
        return Err(WinProofError::NotATreasurePayout(payout_index));
    }
    let openings = bundle
        .payout_openings
        .as_ref()
        .ok_or(WinProofError::NoOpenings)?;
    openings
        .outputs
        .iter()
        .find(|o| o.payout_index == payout_index)
        .cloned()
        .ok_or(WinProofError::NoOpening(payout_index))
}

/// Builds a win proof for treasure payout `payout_index` of `bundle`'s round,
/// with the keys of the wallet that received it.
pub fn create<R: RngCore + CryptoRng>(
    bundle: &ProofBundle,
    payout_index: u32,
    fvk: &FullViewingKey,
    ask: &SpendAuthorizingKey,
    proving_key: &ProvingKey,
    rng: &mut R,
) -> Result<WinProof, WinProofError> {
    let opening = treasure_opening(bundle, payout_index)?;
    let payout_txid = bundle
        .payout_openings
        .as_ref()
        .map(|o| o.txid.clone())
        .ok_or(WinProofError::NoOpenings)?;
    let txid = txid_bytes(&payout_txid).ok_or_else(|| WinProofError::BadOpening("txid".into()))?;
    let (recipient_raw, address) = opening_address(&opening)?;
    if fvk.scope_for_address(&address).is_none() {
        return Err(WinProofError::NotOurAddress);
    }
    let (note, note_rho, note_rseed) = loop {
        let rho = pallas::Base::random(&mut *rng).to_repr();
        let mut rseed = [0u8; 32];
        rng.fill_bytes(&mut rseed);
        if let Some(note) = made_up_note(address, &rho, &rseed) {
            break (note, rho, rseed);
        }
    };
    let anchor = lone_leaf_anchor(&note);
    let version = BundleVersion::ironwood_v3();
    let flags = version.default_flags();
    let mut builder = Builder::new(BundleType::UNPADDED, version, flags, anchor)
        .map_err(|e| WinProofError::Build(format!("{e:?}")))?;
    builder
        .add_spend(fvk.clone(), note, lone_leaf_path())
        .map_err(|e| WinProofError::Build(format!("{e:?}")))?;
    let (unauthorized, _) = builder
        .build::<i64>(&mut *rng)
        .map_err(|e| WinProofError::Build(format!("{e:?}")))?
        .ok_or_else(|| WinProofError::Build("empty bundle".into()))?;
    let mut nonce = [0u8; 32];
    rng.fill_bytes(&mut nonce);
    let msg = message(
        bundle.params.network,
        bundle.round,
        &txid,
        payout_index,
        &recipient_raw,
        &nonce,
    );
    let authorized: Bundle<Authorized, i64> = unauthorized
        .create_proof(proving_key, &mut *rng)
        .map_err(|e| WinProofError::Build(format!("prove: {e:?}")))?
        .apply_signatures(&mut *rng, msg, std::slice::from_ref(ask))
        .map_err(|e| WinProofError::Build(format!("sign: {e:?}")))?;
    let actions = authorized.actions();
    if actions.len() != 1 || *authorized.value_balance() != 1 {
        return Err(WinProofError::Build("unexpected bundle shape".into()));
    }
    let action = actions.first();
    let enc = action.encrypted_note();
    Ok(WinProof {
        version: WIN_PROOF_VERSION,
        network: bundle.params.network,
        round: bundle.round,
        payout_txid,
        payout_index,
        opening,
        nonce,
        note_rho,
        note_rseed,
        flags: authorized
            .flags()
            .to_byte(version)
            .ok_or_else(|| WinProofError::Build("flags".into()))?,
        anchor: authorized.anchor().to_bytes(),
        action: ActionBytes {
            nf: action.nullifier().to_bytes(),
            rk: <[u8; 32]>::from(action.rk()),
            cmx: action.cmx().to_bytes(),
            cv_net: action.cv_net().to_bytes(),
            epk: enc.epk_bytes,
            enc_ciphertext: enc.enc_ciphertext.to_vec(),
            out_ciphertext: enc.out_ciphertext.to_vec(),
            spend_auth_sig: <[u8; 64]>::from(action.authorization()).to_vec(),
        },
        proof: authorized.authorization().proof().as_ref().to_vec(),
        binding_sig: <[u8; 64]>::from(authorized.authorization().binding_signature()).to_vec(),
    })
}

impl WinProof {
    /// Compact binary encoding (postcard).
    pub fn encode(&self) -> Result<Vec<u8>, WinProofError> {
        postcard::to_allocvec(self).map_err(|e| WinProofError::Encoding(e.to_string()))
    }

    /// Parses [`WinProof::encode`] output, refusing oversized input.
    pub fn decode(bytes: &[u8]) -> Result<Self, WinProofError> {
        if bytes.len() > MAX_ENCODED_LEN {
            return Err(WinProofError::Encoding(format!(
                "{} bytes, more than {MAX_ENCODED_LEN}",
                bytes.len()
            )));
        }
        let proof: WinProof =
            postcard::from_bytes(bytes).map_err(|e| WinProofError::Encoding(e.to_string()))?;
        if proof.version != WIN_PROOF_VERSION {
            return Err(WinProofError::Encoding(format!(
                "unknown version {}",
                proof.version
            )));
        }
        Ok(proof)
    }

    /// Stable identifier: the first 16 bytes of BLAKE2b-256 of the encoding.
    pub fn id(encoded: &[u8]) -> String {
        let hash = Params::new()
            .hash_length(32)
            .personal(b"ZeekWinProofId__")
            .hash(encoded);
        hex::encode(&hash.as_bytes()[..16])
    }
}

/// One step of verification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WinStep {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// The verdict on a win proof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WinVerdict {
    pub valid: bool,
    /// The claim in words, when valid.
    pub claim: String,
    /// The first failure, in words.
    pub reason: Option<String>,
    pub steps: Vec<WinStep>,
}

struct Steps(Vec<WinStep>);

impl Steps {
    fn pass(&mut self, name: &str, detail: impl Into<String>) {
        self.0.push(WinStep {
            name: name.into(),
            passed: true,
            detail: detail.into(),
        });
    }

    fn fail(mut self, name: &str, detail: impl Into<String>) -> WinVerdict {
        let detail = detail.into();
        self.0.push(WinStep {
            name: name.into(),
            passed: false,
            detail: detail.clone(),
        });
        WinVerdict {
            valid: false,
            claim: String::new(),
            reason: Some(format!("{name}: {detail}")),
            steps: self.0,
        }
    }
}

/// Rebuilds the proof's bundle from bytes.
fn rebuild_bundle(proof: &WinProof) -> Result<Bundle<Authorized, i64>, String> {
    let version = BundleVersion::ironwood_v3();
    let a = &proof.action;
    let nf = Option::<Nullifier>::from(Nullifier::from_bytes(&a.nf))
        .ok_or("nullifier is not canonical")?;
    let rk = redpallas::VerificationKey::<SpendAuth>::try_from(a.rk)
        .map_err(|_| "rk is not a valid key")?;
    let cmx = Option::<ExtractedNoteCommitment>::from(ExtractedNoteCommitment::from_bytes(&a.cmx))
        .ok_or("cmx is not canonical")?;
    let cv_net = Option::<ValueCommitment>::from(ValueCommitment::from_bytes(&a.cv_net))
        .ok_or("value commitment is not canonical")?;
    let enc_ciphertext: [u8; 580] = a
        .enc_ciphertext
        .as_slice()
        .try_into()
        .map_err(|_| "note ciphertext length")?;
    let out_ciphertext: [u8; 80] = a
        .out_ciphertext
        .as_slice()
        .try_into()
        .map_err(|_| "out ciphertext length")?;
    let sig: [u8; 64] = a
        .spend_auth_sig
        .as_slice()
        .try_into()
        .map_err(|_| "signature length")?;
    let action = Action::from_parts(
        nf,
        rk,
        cmx,
        TransmittedNoteCiphertext {
            epk_bytes: a.epk,
            enc_ciphertext,
            out_ciphertext,
        },
        cv_net,
        redpallas::Signature::<SpendAuth>::from(sig),
    )
    .map_err(|e| format!("action: {e:?}"))?;
    let flags = Flags::from_byte(proof.flags, version)
        .ok_or("flags are not valid for an Ironwood bundle")?;
    let anchor = Option::<Anchor>::from(Anchor::from_bytes(proof.anchor))
        .ok_or("anchor is not canonical")?;
    let binding: [u8; 64] = proof
        .binding_sig
        .as_slice()
        .try_into()
        .map_err(|_| "binding signature length")?;
    Bundle::try_from_parts(
        NonEmpty::new(action),
        flags,
        1i64,
        anchor,
        Authorized::from_parts(
            Proof::new(proof.proof.clone()),
            redpallas::Signature::<Binding>::from(binding),
        ),
        version,
    )
    .map_err(|e| format!("bundle: {e:?}"))
}

/// Verifies a win proof against the round's proof bundle and the mined
/// payout transaction (`payout_tx`, whose inclusion the caller confirmed in
/// the block at `payout_tx.height`, `included`).
pub fn verify(
    proof: &WinProof,
    bundle: &ProofBundle,
    payout_tx: &MinedTx,
    included: bool,
) -> WinVerdict {
    let mut steps = Steps(Vec::new());
    // 1. The claim matches the round.
    if proof.network != bundle.params.network || proof.round != bundle.round {
        return steps.fail(
            "round",
            format!(
                "the proof is for round {} on {:?}, the bundle is round {} on {:?}",
                proof.round, proof.network, bundle.round, bundle.params.network
            ),
        );
    }
    let opening = match treasure_opening(bundle, proof.payout_index) {
        Ok(o) => o,
        Err(e) => return steps.fail("payout", e.to_string()),
    };
    if opening != proof.opening {
        return steps.fail(
            "payout",
            format!(
                "the proof's opening is not the bundle's opening of payout {}",
                proof.payout_index
            ),
        );
    }
    let bundle_txid = bundle
        .payout_openings
        .as_ref()
        .map(|o| o.txid.as_str())
        .unwrap_or_default();
    if proof.payout_txid != bundle_txid {
        return steps.fail(
            "payout",
            "the proof names a different payout transaction than the round's",
        );
    }
    let payout = &bundle.settlement.payouts[proof.payout_index as usize];
    steps.pass(
        "payout",
        format!(
            "payout {} of round {} is a treasure payout of {} zatoshis",
            proof.payout_index, proof.round, payout.net
        ),
    );
    // 2. The output exists in the mined payout transaction.
    if !included {
        return steps.fail(
            "payout transaction",
            "the payout transaction is not in the block it claims",
        );
    }
    let tx = match parse_tx(bundle.params.network, &proof.payout_txid, payout_tx) {
        Ok(tx) => tx,
        Err(e) => return steps.fail("payout transaction", e.to_string()),
    };
    if tx.txid().to_string() != proof.payout_txid {
        return steps.fail(
            "payout transaction",
            "the transaction does not have the claimed txid",
        );
    }
    let action = tx
        .ironwood_bundle()
        .and_then(|b| b.actions().get(opening.action_index as usize));
    let Some(action) = action else {
        return steps.fail("payout transaction", "the opened action is missing");
    };
    match opening_cmx(&opening, action.nullifier().to_bytes()) {
        Ok(cmx) if cmx == action.cmx().to_bytes() => steps.pass(
            "payout transaction",
            format!(
                "output {} of {} (mined at {}) pays {} zatoshis to the opened address",
                opening.action_index, proof.payout_txid, payout_tx.height, opening.value
            ),
        ),
        Ok(_) => {
            return steps.fail(
                "payout transaction",
                "the opening does not match the output's note commitment",
            )
        }
        Err(e) => return steps.fail("payout transaction", e),
    }
    // 3. The made-up note is at the payout address and fixes the anchor.
    let (recipient_raw, address) = match opening_address(&opening) {
        Ok(a) => a,
        Err(e) => return steps.fail("address binding", e.to_string()),
    };
    let Some(note) = made_up_note(address, &proof.note_rho, &proof.note_rseed) else {
        return steps.fail(
            "address binding",
            "the made-up note's rho or rseed is invalid",
        );
    };
    if lone_leaf_anchor(&note).to_bytes() != proof.anchor {
        return steps.fail(
            "address binding",
            "the anchor is not the one-leaf tree of a 1-zatoshi note at the payout address",
        );
    }
    steps.pass(
        "address binding",
        "the anchor commits to a 1-zatoshi note at the payout address",
    );
    // 4. The bundle: shape, proof, signatures.
    let rebuilt = match rebuild_bundle(proof) {
        Ok(b) => b,
        Err(e) => return steps.fail("proof", e),
    };
    if !rebuilt.flags().spends_enabled() {
        return steps.fail("proof", "spends are disabled in the proof's flags");
    }
    if let Err(e) = rebuilt.verify_proof(verifying_key()) {
        return steps.fail("proof", format!("the Halo2 proof does not verify: {e:?}"));
    }
    steps.pass("proof", "the Orchard-protocol Halo2 proof verifies (address integrity, spend authority, Merkle path)");
    let Some(txid) = txid_bytes(&proof.payout_txid) else {
        return steps.fail("signature", "malformed txid");
    };
    let msg = message(
        proof.network,
        proof.round,
        &txid,
        proof.payout_index,
        &recipient_raw,
        &proof.nonce,
    );
    let action = rebuilt.actions().first();
    if action.rk().verify(&msg, action.authorization()).is_err() {
        return steps.fail("signature", "the spend authorization signature does not verify over the bound message (round, transaction, output, recipient, nonce)");
    }
    if rebuilt
        .binding_validating_key()
        .verify(&msg, rebuilt.authorization().binding_signature())
        .is_err()
    {
        return steps.fail(
            "signature",
            "the binding signature does not verify (value balance must be +1)",
        );
    }
    steps.pass(
        "signature",
        "spend authorization and binding signatures verify over the bound message",
    );
    WinVerdict {
        valid: true,
        claim: format!(
            "The holder of this proof controls the address that received round {}'s treasure payout {} ({} zatoshis, transaction {}).",
            proof.round, proof.payout_index, opening.value, proof.payout_txid
        ),
        reason: None,
        steps: steps.0,
    }
}
