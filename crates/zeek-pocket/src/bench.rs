//! The proving benchmark behind the Phase 5 gate: one Ironwood spend, two
//! outputs, built with the same PCZT roles the wallet uses, proven, signed,
//! extracted and verified. Nothing is broadcast.

use incrementalmerkletree::Retention;
use orchard::circuit::{OrchardCircuitVersion, ProvingKey, VerifyingKey};
use orchard::keys::{FullViewingKey, Scope, SpendAuthorizingKey, SpendingKey};
use orchard::note_encryption::IronwoodDomain;
use orchard::tree::MerkleHashOrchard;
use orchard::{Anchor, Note};
use pczt::roles::creator::Creator;
use pczt::roles::io_finalizer::IoFinalizer;
use pczt::roles::prover::Prover;
use pczt::roles::signer::Signer;
use pczt::roles::tx_extractor::TransactionExtractor;
use pczt::roles::verifier::Verifier;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use shardtree::store::memory::MemoryShardStore;
use shardtree::ShardTree;
use zcash_primitives::transaction::builder::{BuildConfig, Builder, BundlePadding, PcztResult};
use zcash_primitives::transaction::fees::zip317;
use zcash_protocol::consensus::{BlockHeight, TEST_NETWORK};
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::Zatoshis;

/// A post-NU7 testnet height, so the transaction is V6 on the live branch.
pub const TARGET_HEIGHT: u32 = 4_500_000;
const NOTE_VALUE: u64 = 1_000_000;
const PAYMENT: u64 = 400_000;

/// Timings in milliseconds and the sizes the gate asks for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchReport {
    pub proving_key_ms: f64,
    pub verifying_key_ms: f64,
    pub build_ms: f64,
    pub prove_ms: f64,
    pub sign_ms: f64,
    pub extract_verify_ms: f64,
    pub tx_bytes: usize,
    pub actions: usize,
    pub version: String,
}

/// Errors of the benchmark.
#[derive(Debug, thiserror::Error)]
pub enum BenchError {
    #[error("key derivation produced an invalid spending key")]
    SpendingKey,
    #[error("note decryption of the funding output failed")]
    Decrypt,
    #[error("commitment tree: {0}")]
    Tree(String),
    #[error("transaction builder: {0}")]
    Builder(String),
    #[error("PCZT: {0}")]
    Pczt(String),
    #[error("transaction serialization: {0}")]
    Serialize(String),
}

fn random_spending_key(rng: &mut impl RngCore) -> Result<SpendingKey, BenchError> {
    for _ in 0..64 {
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        if let Some(sk) = Option::<SpendingKey>::from(SpendingKey::from_bytes(bytes)) {
            return Ok(sk);
        }
    }
    Err(BenchError::SpendingKey)
}

/// A funding note to `fvk`'s external address, witnessed in a tree of one.
fn funding_note(
    fvk: &FullViewingKey,
    rng: &mut impl RngCore,
) -> Result<(Note, orchard::tree::MerklePath, Anchor), BenchError> {
    let version = orchard::bundle::BundleVersion::ironwood_v3();
    let mut builder = orchard::builder::Builder::new(
        orchard::builder::BundleType::DEFAULT,
        version,
        version.default_flags(),
        Anchor::empty_tree(),
    )
    .map_err(|e| BenchError::Builder(format!("{e:?}")))?;
    builder
        .add_output(
            None,
            fvk.address_at(0u32, Scope::External),
            orchard::value::NoteValue::from_raw(NOTE_VALUE),
            MemoBytes::empty().into_bytes(),
        )
        .map_err(|e| BenchError::Builder(format!("{e:?}")))?;
    let (bundle, meta) = builder
        .build::<i64>(&mut *rng)
        .map_err(|e| BenchError::Builder(format!("{e:?}")))?
        .ok_or_else(|| BenchError::Builder("empty bundle".into()))?;
    let index = meta
        .output_action_index(0)
        .ok_or_else(|| BenchError::Builder("output index".into()))?;
    let action = &bundle.actions()[index];
    let ivk = fvk.to_ivk(Scope::External).prepare();
    let (note, _, _) = zcash_note_encryption::try_note_decryption(
        &IronwoodDomain::for_action(action),
        &ivk,
        action,
    )
    .ok_or(BenchError::Decrypt)?;
    let cmx: orchard::note::ExtractedNoteCommitment = note.commitment().into();
    let leaf = MerkleHashOrchard::from_cmx(&cmx);
    let mut tree =
        ShardTree::<_, 32, 16>::new(MemoryShardStore::<MerkleHashOrchard, u32>::empty(), 8);
    tree.append(leaf, Retention::Marked)
        .map_err(|e| BenchError::Tree(format!("{e:?}")))?;
    tree.checkpoint(TARGET_HEIGHT - 10)
        .map_err(|e| BenchError::Tree(format!("{e:?}")))?;
    let path = tree
        .witness_at_checkpoint_depth(0.into(), 0)
        .map_err(|e| BenchError::Tree(format!("{e:?}")))?
        .ok_or_else(|| BenchError::Tree("no witness".into()))?;
    let anchor: Anchor = path.root(leaf).into();
    Ok((note, path.into(), anchor))
}

/// Runs the benchmark with one spend, `payments` payment outputs and one
/// change output; `now_ms` is the clock (JavaScript `Date.now()` in the
/// browser, `Instant` natively).
pub fn run(now_ms: impl Fn() -> f64, payments: u32) -> Result<BenchReport, BenchError> {
    let payments = payments.clamp(1, 30);
    let each = PAYMENT / u64::from(payments);
    let fee = 5_000 * u64::from(payments + 1).max(2);
    let mut rng = OsRng;
    let t0 = now_ms();
    let proving_key = ProvingKey::build(OrchardCircuitVersion::PostNu6_3);
    let t1 = now_ms();
    let verifying_key = VerifyingKey::build(OrchardCircuitVersion::PostNu6_3);
    let t2 = now_ms();

    let sk = random_spending_key(&mut rng)?;
    let fvk = FullViewingKey::from(&sk);
    let recipient =
        FullViewingKey::from(&random_spending_key(&mut rng)?).address_at(0u32, Scope::External);
    let (note, path, anchor) = funding_note(&fvk, &mut rng)?;
    let target = BlockHeight::from_u32(TARGET_HEIGHT);
    let mut builder = Builder::new(
        TEST_NETWORK,
        target,
        BuildConfig::Standard {
            sapling_anchor: None,
            orchard_anchor: None,
            ironwood_anchor: Some(anchor),
            orchard_padding: BundlePadding::DEFAULT,
            ironwood_padding: BundlePadding::DEFAULT,
        },
    );
    builder
        .add_ironwood_spend::<zip317::FeeError>(fvk.clone(), note, path)
        .map_err(|e| BenchError::Builder(e.to_string()))?;
    for _ in 0..payments {
        builder
            .add_ironwood_output::<zip317::FeeError>(
                Some(fvk.to_ovk(Scope::External)),
                recipient,
                Zatoshis::const_from_u64(each),
                MemoBytes::from_bytes(b"zeek pocket benchmark")
                    .map_err(|e| BenchError::Builder(format!("{e:?}")))?,
            )
            .map_err(|e| BenchError::Builder(e.to_string()))?;
    }
    builder
        .add_ironwood_output::<zip317::FeeError>(
            Some(fvk.to_ovk(Scope::Internal)),
            fvk.address_at(0u32, Scope::Internal),
            Zatoshis::const_from_u64(NOTE_VALUE - each * u64::from(payments) - fee),
            MemoBytes::empty(),
        )
        .map_err(|e| BenchError::Builder(e.to_string()))?;
    let PcztResult { pczt_parts, .. } = builder
        .build_for_pczt(rng, &zip317::FeeRule::standard())
        .map_err(|e| BenchError::Builder(e.to_string()))?;
    let pczt =
        Creator::build_from_parts(pczt_parts).ok_or_else(|| BenchError::Pczt("creator".into()))?;
    let pczt = IoFinalizer::new(pczt)
        .finalize_io()
        .map_err(|e| BenchError::Pczt(format!("io finalizer: {e:?}")))?;
    let t3 = now_ms();

    let pczt = Prover::new(pczt)
        .create_ironwood_proof(&proving_key)
        .map_err(|e| BenchError::Pczt(format!("prove: {e:?}")))?
        .finish();
    let t4 = now_ms();

    // Sign every Ironwood spend that still lacks a signature.
    let mut pending = Vec::new();
    Verifier::new(pczt.clone())
        .with_ironwood::<core::convert::Infallible, _>(|bundle| {
            for (i, action) in bundle.actions().iter().enumerate() {
                if action.spend().spend_auth_sig().is_none() {
                    pending.push(i);
                }
            }
            Ok(())
        })
        .map_err(|e| BenchError::Pczt(format!("verifier: {e:?}")))?;
    let mut signer = Signer::new(pczt).map_err(|e| BenchError::Pczt(format!("signer: {e:?}")))?;
    let ask = SpendAuthorizingKey::from(&sk);
    for index in pending {
        signer
            .sign_ironwood(index, &ask)
            .map_err(|e| BenchError::Pczt(format!("sign {index}: {e:?}")))?;
    }
    let pczt = signer.finish();
    let t5 = now_ms();

    let transaction = TransactionExtractor::new(pczt)
        .with_orchard(&verifying_key)
        .extract()
        .map_err(|e| BenchError::Pczt(format!("extract: {e:?}")))?;
    let t6 = now_ms();
    let mut bytes = Vec::new();
    transaction
        .write(&mut bytes)
        .map_err(|e| BenchError::Serialize(e.to_string()))?;
    let actions = transaction
        .ironwood_bundle()
        .map(|b| b.actions().len())
        .unwrap_or(0);
    Ok(BenchReport {
        proving_key_ms: t1 - t0,
        verifying_key_ms: t2 - t1,
        build_ms: t3 - t2,
        prove_ms: t4 - t3,
        sign_ms: t5 - t4,
        extract_verify_ms: t6 - t5,
        tx_bytes: bytes.len(),
        actions,
        version: format!("{:?}", transaction.version()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zcash_primitives::transaction::TxVersion;

    #[test]
    fn benchmark_produces_a_valid_v6_transaction() {
        let start = std::time::Instant::now();
        let report = run(|| start.elapsed().as_secs_f64() * 1000.0, 1).expect("bench");
        assert_eq!(report.version, format!("{:?}", TxVersion::V6));
        assert_eq!(report.actions, 2, "one spend, two outputs: two actions");
        assert!(report.tx_bytes > 1_000);
        assert!(report.prove_ms > 0.0);
    }
}
