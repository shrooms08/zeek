//! Offline end-to-end test: build a PCZT that spends an Orchard note owned by
//! the FROST vault, prove it, FROST-sign it with two of three keyholders, and
//! extract a fully authorized transaction. No network is involved: the note is
//! created locally and its commitment is placed in a single-leaf tree whose
//! root serves as the anchor.
//!
//! The consensus parameters are a local network with NU6.2 active and NU6.3
//! not yet active, so the Orchard v2 bundle and the `FixedPostNu6_2` circuit
//! are used. Testnet itself is past NU6.3 (`PostNu6_3`); the signing path is
//! identical, only the proving key differs and the CLI picks it from the
//! branch at the target height.

use std::sync::OnceLock;

use orchard::circuit::{OrchardCircuitVersion, ProvingKey, VerifyingKey};
use orchard::tree::MerkleHashOrchard;
use pczt::roles::creator::Creator;
use pczt::roles::io_finalizer::IoFinalizer;
use pczt::roles::prover::Prover;
use pczt::roles::tx_extractor::TransactionExtractor;
use pczt::Pczt;
use rand_core::OsRng;
use shardtree::store::memory::MemoryShardStore;
use shardtree::ShardTree;
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::transaction::builder::{BuildConfig, Builder, BundlePadding, PcztResult};
use zcash_primitives::transaction::fees::zip317;
use zcash_protocol::consensus::BlockHeight;
use zcash_protocol::local_consensus::LocalNetwork;
use zcash_protocol::memo::{Memo, MemoBytes};
use zcash_protocol::value::Zatoshis;
use zeek_vault::frost::{self, KeyPackage, KeygenOutput};
use zeek_vault::{keys, send, VaultError};

static PROVING_KEY: OnceLock<ProvingKey> = OnceLock::new();
static VERIFYING_KEY: OnceLock<VerifyingKey> = OnceLock::new();

fn proving_key() -> &'static ProvingKey {
    PROVING_KEY.get_or_init(|| ProvingKey::build(OrchardCircuitVersion::FixedPostNu6_2))
}

fn verifying_key() -> &'static VerifyingKey {
    VERIFYING_KEY.get_or_init(|| VerifyingKey::build(OrchardCircuitVersion::FixedPostNu6_2))
}

fn pre_nu6_3_network() -> LocalNetwork {
    LocalNetwork {
        overwinter: Some(BlockHeight::from_u32(1)),
        sapling: Some(BlockHeight::from_u32(2)),
        blossom: Some(BlockHeight::from_u32(3)),
        heartwood: Some(BlockHeight::from_u32(4)),
        canopy: Some(BlockHeight::from_u32(5)),
        nu5: Some(BlockHeight::from_u32(6)),
        nu6: Some(BlockHeight::from_u32(7)),
        nu6_1: Some(BlockHeight::from_u32(8)),
        nu6_2: Some(BlockHeight::from_u32(9)),
        nu6_3: None,
    }
}

fn keyholders<'a>(out: &'a KeygenOutput, numbers: &[u16]) -> Vec<&'a KeyPackage> {
    numbers
        .iter()
        .map(|n| &out.key_packages[&frost::identifier(*n).expect("identifier")])
        .collect()
}

/// Builds, finalizes and proves a PCZT that spends one vault note.
///
/// Returns the PCZT and the index of the real spend action.
fn proven_vault_pczt(fvk: &orchard::keys::FullViewingKey) -> (Pczt, usize) {
    let recipient = fvk.address_at(0u32, orchard::keys::Scope::External);
    let ivk = fvk.to_ivk(orchard::keys::Scope::External);
    let value = orchard::value::NoteValue::from_raw(1_000_000);

    // Create a note to the vault by building a throw-away bundle and decrypting
    // its output with the vault's incoming viewing key.
    let note = {
        let mut builder = orchard::builder::Builder::new(
            orchard::builder::BundleType::DEFAULT,
            orchard::bundle::BundleVersion::orchard_v2(),
            orchard::bundle::BundleVersion::orchard_v2().default_flags(),
            orchard::Anchor::empty_tree(),
        )
        .expect("orchard builder");
        builder
            .add_output(None, recipient, value, Memo::Empty.encode().into_bytes())
            .expect("add output");
        let (bundle, meta) = builder
            .build::<i64>(OsRng)
            .expect("build")
            .expect("non-empty bundle");
        let action = &bundle.actions()[meta.output_action_index(0).expect("output index")];
        let domain = orchard::note_encryption::OrchardDomain::for_action(action);
        let (note, _, _) = try_note_decryption(&domain, &ivk.prepare(), action).expect("decrypt");
        note
    };

    // Put the note commitment into a one-leaf tree and witness it.
    let (anchor, merkle_path) = {
        let cmx: orchard::note::ExtractedNoteCommitment = note.commitment().into();
        let leaf = MerkleHashOrchard::from_cmx(&cmx);
        let mut tree =
            ShardTree::<_, 32, 16>::new(MemoryShardStore::<MerkleHashOrchard, u32>::empty(), 100);
        tree.append(leaf, incrementalmerkletree::Retention::Marked)
            .expect("append");
        tree.checkpoint(9_999_999).expect("checkpoint");
        let merkle_path = tree
            .witness_at_checkpoint_depth(0.into(), 0)
            .expect("witness")
            .expect("witness exists");
        let anchor = merkle_path.root(leaf);
        (anchor.into(), merkle_path.into())
    };

    let mut builder = Builder::new(
        pre_nu6_3_network(),
        10_000_000.into(),
        BuildConfig::Standard {
            sapling_anchor: None,
            orchard_anchor: Some(anchor),
            ironwood_anchor: None,
            orchard_padding: BundlePadding::DEFAULT,
            ironwood_padding: BundlePadding::DEFAULT,
        },
    );
    builder
        .add_orchard_spend::<zip317::FeeRule>(fvk.clone(), note, merkle_path)
        .expect("add spend");
    builder
        .add_orchard_output::<zip317::FeeRule>(
            Some(fvk.to_ovk(orchard::keys::Scope::External)),
            recipient,
            Zatoshis::const_from_u64(100_000),
            MemoBytes::empty(),
        )
        .expect("add payment");
    builder
        .add_orchard_output::<zip317::FeeRule>(
            Some(fvk.to_ovk(orchard::keys::Scope::Internal)),
            fvk.address_at(0u32, orchard::keys::Scope::Internal),
            Zatoshis::const_from_u64(890_000),
            MemoBytes::empty(),
        )
        .expect("add change");
    let PcztResult {
        pczt_parts,
        orchard_meta,
        ..
    } = builder
        .build_for_pczt(OsRng, &zip317::FeeRule::standard())
        .expect("build for pczt");

    let pczt = Creator::build_from_parts(pczt_parts).expect("creator");
    let pczt = IoFinalizer::new(pczt).finalize_io().expect("io finalizer");
    let pczt = Prover::new(pczt)
        .create_orchard_proof(proving_key())
        .expect("prove")
        .finish();
    let spend_index = orchard_meta.spend_action_index(0).expect("spend index");
    (pczt, spend_index)
}

#[test]
fn two_of_three_frost_signs_a_vault_pczt_offline() {
    let vault = frost::dealer_keygen(2, 3, OsRng).expect("keygen");
    let group_key = frost::group_key_bytes(&vault.public_key_package).expect("group key");
    let fvk = keys::derive_full_viewing_key(&group_key, OsRng).expect("fvk");

    let (pczt, spend_index) = proven_vault_pczt(&fvk);

    // Exactly one real spend is waiting for a signature, and it is ours.
    let pending = send::pending_spends(&pczt).expect("pending spends");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].index, spend_index);
    let ak = orchard::primitives::redpallas::VerificationKey::<
        orchard::primitives::redpallas::SpendAuth,
    >::try_from(group_key)
    .expect("ak");
    let expected_rk: [u8; 32] = (&ak.randomize(&pending[0].alpha)).into();
    assert_eq!(
        expected_rk, pending[0].rk,
        "rk must be ak randomized by alpha"
    );

    // Too few signers, or signers from another vault, must be rejected before
    // anything is applied to the PCZT.
    let err = send::sign_pczt(
        pczt.clone(),
        &keyholders(&vault, &[2]),
        &vault.public_key_package,
    )
    .expect_err("one signer");
    assert!(
        matches!(
            err,
            VaultError::NotEnoughSigners {
                given: 1,
                threshold: 2
            }
        ),
        "{err}"
    );

    let other = frost::dealer_keygen(2, 3, OsRng).expect("other keygen");
    let err = send::sign_pczt(
        pczt.clone(),
        &keyholders(&other, &[1, 2]),
        &vault.public_key_package,
    )
    .expect_err("foreign signers");
    assert!(matches!(err, VaultError::UnknownSigner(_)), "{err}");

    // Keyholders 1 and 3 sign.
    let (signed, count) = send::sign_pczt(
        pczt,
        &keyholders(&vault, &[1, 3]),
        &vault.public_key_package,
    )
    .expect("sign with keyholders 1 and 3");
    assert_eq!(count, 1);
    assert!(
        matches!(
            send::pending_spends(&signed),
            Err(VaultError::NothingToSign)
        ),
        "no spend should remain unsigned"
    );

    // The extractor re-verifies the proof, every spend-auth signature and the
    // binding signature before producing a transaction.
    let tx = TransactionExtractor::new(signed)
        .with_orchard(verifying_key())
        .extract()
        .expect("extract fully authorized transaction");
    let bundle = tx.orchard_bundle().expect("orchard bundle");
    assert_eq!(bundle.actions().len(), 2);
    assert_eq!(u32::from(tx.expiry_height()), 10_000_040);
}
