//! Offline end-to-end tests: build a version 6 PCZT that spends a note owned by
//! the FROST vault, prove it, FROST-sign it with two of three keyholders, and
//! extract a fully authorized transaction. No network is involved: each note is
//! created locally and its commitment is placed in a single-leaf tree whose
//! root serves as the anchor.
//!
//! The consensus parameters are a local network with NU6.3 active (NU7 not
//! active), so transactions are V6 and both Orchard-protocol bundles use the
//! `PostNu6_3` circuit. Testnet is past NU7, which keeps the same transaction
//! format and circuit; the CLI picks both from the branch at the target height.
//!
//! Two shapes are covered:
//!
//! * [`ironwood_spend`]: the vault's normal transaction. One Ironwood note is
//!   spent to a payment plus change in the Ironwood bundle. The padding dummy
//!   is pre-signed by the IO finalizer; FROST signs the one real spend.
//! * [`orchard_pool_spend_crossing_into_ironwood`]: a legacy Orchard-pool note
//!   is spent with Orchard change and an Ironwood payment. Under the NU6.3
//!   cross-address rule the change output is paired with a wallet-controlled
//!   zero-value spend keyed by the vault's `ak`, which has no `dummy_sk` and
//!   therefore must be FROST-signed like the real spend. The vault CLI never
//!   builds this shape (it refuses to spend the closed pool) but the signer
//!   must handle it, and it is the only place such a spend arises.

use std::sync::OnceLock;

use orchard::bundle::BundleVersion;
use orchard::circuit::{OrchardCircuitVersion, ProvingKey, VerifyingKey};
use orchard::keys::{FullViewingKey, Scope};
use orchard::note_encryption::{IronwoodDomain, OrchardDomain};
use orchard::tree::{MerkleHashOrchard, MerklePath};
use orchard::{Anchor, Note, ValuePool};
use pczt::roles::creator::Creator;
use pczt::roles::io_finalizer::IoFinalizer;
use pczt::roles::tx_extractor::TransactionExtractor;
use pczt::Pczt;
use rand_core::OsRng;
use shardtree::store::memory::MemoryShardStore;
use shardtree::ShardTree;
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::transaction::builder::{BuildConfig, Builder, BundlePadding, PcztResult};
use zcash_primitives::transaction::fees::zip317;
use zcash_primitives::transaction::{Transaction, TxVersion};
use zcash_protocol::consensus::{BlockHeight, BranchId, Parameters};
use zcash_protocol::local_consensus::LocalNetwork;
use zcash_protocol::memo::{Memo, MemoBytes};
use zcash_protocol::value::Zatoshis;
use zeek_vault::frost::{self, KeyPackage, KeygenOutput};
use zeek_vault::send::PendingSpend;
use zeek_vault::{keys, send, VaultError};

static PROVING_KEY: OnceLock<ProvingKey> = OnceLock::new();
static VERIFYING_KEY: OnceLock<VerifyingKey> = OnceLock::new();

const TARGET_HEIGHT: u32 = 10_000_000;
const NOTE_VALUE: u64 = 1_000_000;
const PAYMENT: u64 = 100_000;

fn proving_key() -> &'static ProvingKey {
    PROVING_KEY.get_or_init(|| ProvingKey::build(OrchardCircuitVersion::PostNu6_3))
}

fn verifying_key() -> &'static VerifyingKey {
    VERIFYING_KEY.get_or_init(|| VerifyingKey::build(OrchardCircuitVersion::PostNu6_3))
}

fn post_nu6_3_network() -> LocalNetwork {
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
        nu6_3: Some(BlockHeight::from_u32(10)),
        #[cfg(zcash_unstable = "nu7")]
        nu7: None,
    }
}

fn keyholders<'a>(out: &'a KeygenOutput, numbers: &[u16]) -> Vec<&'a KeyPackage> {
    numbers
        .iter()
        .map(|n| &out.key_packages[&frost::identifier(*n).expect("identifier")])
        .collect()
}

/// Creates a note to the vault in the given pool by building a throw-away
/// bundle and decrypting its output with the vault's incoming viewing key.
fn note_to_vault(fvk: &FullViewingKey, pool: ValuePool) -> Note {
    let recipient = fvk.address_at(0u32, Scope::External);
    let ivk = fvk.to_ivk(Scope::External).prepare();
    let value = orchard::value::NoteValue::from_raw(NOTE_VALUE);
    let bundle_version = match pool {
        ValuePool::Orchard => BundleVersion::orchard_v2(),
        ValuePool::Ironwood => BundleVersion::ironwood_v3(),
    };
    let mut builder = orchard::builder::Builder::new(
        orchard::builder::BundleType::DEFAULT,
        bundle_version,
        bundle_version.default_flags(),
        Anchor::empty_tree(),
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
    let (note, _, _) = match pool {
        ValuePool::Orchard => try_note_decryption(&OrchardDomain::for_action(action), &ivk, action)
            .expect("decrypt Orchard note"),
        ValuePool::Ironwood => {
            try_note_decryption(&IronwoodDomain::for_action(action), &ivk, action)
                .expect("decrypt Ironwood note")
        }
    };
    assert_eq!(note.version(), bundle_version.note_version());
    note
}

/// Puts the note commitment into a one-leaf tree and witnesses it.
fn witness(note: &Note) -> (Anchor, MerklePath) {
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
}

/// Builds, finalizes and proves a V6 PCZT that spends one Ironwood vault note.
///
/// Returns the PCZT and the index of the real spend action.
fn proven_ironwood_pczt<P: Parameters>(
    params: P,
    height: u32,
    fvk: &FullViewingKey,
) -> (Pczt, usize) {
    let recipient = fvk.address_at(0u32, Scope::External);
    let note = note_to_vault(fvk, ValuePool::Ironwood);
    let (anchor, merkle_path) = witness(&note);

    let mut builder = Builder::new(
        params,
        height.into(),
        BuildConfig::Standard {
            sapling_anchor: None,
            orchard_anchor: None,
            ironwood_anchor: Some(anchor),
            orchard_padding: BundlePadding::DEFAULT,
            ironwood_padding: BundlePadding::DEFAULT,
        },
    );
    builder
        .add_ironwood_spend::<zip317::FeeRule>(fvk.clone(), note, merkle_path)
        .expect("add Ironwood spend");
    builder
        .add_ironwood_output::<zip317::FeeRule>(
            Some(fvk.to_ovk(Scope::External)),
            recipient,
            Zatoshis::const_from_u64(PAYMENT),
            MemoBytes::empty(),
        )
        .expect("add payment");
    let fee = builder
        .get_fee(&zip317::FeeRule::standard())
        .expect("fee")
        .into_u64();
    builder
        .add_ironwood_output::<zip317::FeeRule>(
            Some(fvk.to_ovk(Scope::Internal)),
            fvk.address_at(0u32, Scope::Internal),
            Zatoshis::const_from_u64(NOTE_VALUE - PAYMENT - fee),
            MemoBytes::empty(),
        )
        .expect("add change");
    let PcztResult {
        pczt_parts,
        ironwood_meta,
        ..
    } = builder
        .build_for_pczt(OsRng, &zip317::FeeRule::standard())
        .expect("build for pczt");
    assert_eq!(pczt_parts.version, TxVersion::V6);
    assert_eq!(pczt_parts.consensus_branch_id, BranchId::Nu6_3);

    let pczt = Creator::build_from_parts(pczt_parts).expect("creator");
    let pczt = IoFinalizer::new(pczt).finalize_io().expect("io finalizer");
    let pczt = send::prove(pczt, proving_key()).expect("prove");
    let spend_index = ironwood_meta.spend_action_index(0).expect("spend index");
    (pczt, spend_index)
}

/// Builds, finalizes and proves a V6 PCZT that spends one legacy Orchard-pool
/// vault note into an Ironwood payment, keeping Orchard change.
///
/// Returns the PCZT and the index of the real Orchard spend action.
fn proven_orchard_pool_pczt(fvk: &FullViewingKey) -> (Pczt, usize) {
    let recipient = fvk.address_at(0u32, Scope::External);
    let note = note_to_vault(fvk, ValuePool::Orchard);
    let (anchor, merkle_path) = witness(&note);

    let mut builder = Builder::new(
        post_nu6_3_network(),
        TARGET_HEIGHT.into(),
        BuildConfig::Standard {
            sapling_anchor: None,
            orchard_anchor: Some(anchor),
            // Output-only Ironwood bundle: no spend is witnessed against it.
            ironwood_anchor: Some(Anchor::empty_tree()),
            orchard_padding: BundlePadding::DEFAULT,
            ironwood_padding: BundlePadding::DEFAULT,
        },
    );
    builder
        .add_orchard_spend::<zip317::FeeRule>(fvk.clone(), note, merkle_path)
        .expect("add Orchard spend");
    builder
        .add_ironwood_output::<zip317::FeeRule>(
            Some(fvk.to_ovk(Scope::External)),
            recipient,
            Zatoshis::const_from_u64(PAYMENT),
            MemoBytes::empty(),
        )
        .expect("add Ironwood payment");
    let fee = builder
        .get_fee(&zip317::FeeRule::standard())
        .expect("fee")
        .into_u64();
    // A plain output is impossible in a post-NU6.3 Orchard-pool bundle (the
    // cross-address rule); change must go through `add_orchard_change_output`,
    // which pairs it with a wallet-controlled zero-value spend.
    builder
        .add_orchard_change_output::<zip317::FeeRule>(
            fvk.clone(),
            Some(fvk.to_ovk(Scope::Internal)),
            fvk.address_at(0u32, Scope::Internal),
            Zatoshis::const_from_u64(NOTE_VALUE - PAYMENT - fee),
            MemoBytes::empty(),
        )
        .expect("add Orchard change");
    let PcztResult {
        pczt_parts,
        orchard_meta,
        ..
    } = builder
        .build_for_pczt(OsRng, &zip317::FeeRule::standard())
        .expect("build for pczt");
    assert_eq!(pczt_parts.version, TxVersion::V6);

    let pczt = Creator::build_from_parts(pczt_parts).expect("creator");
    let pczt = IoFinalizer::new(pczt).finalize_io().expect("io finalizer");
    let pczt = send::prove(pczt, proving_key()).expect("prove");
    let spend_index = orchard_meta.spend_action_index(0).expect("spend index");
    (pczt, spend_index)
}

fn vault() -> (KeygenOutput, [u8; 32], FullViewingKey) {
    let out = frost::dealer_keygen(2, 3, OsRng).expect("keygen");
    let group_key = frost::group_key_bytes(&out.public_key_package).expect("group key");
    let fvk = keys::derive_full_viewing_key(&group_key, OsRng).expect("fvk");
    (out, group_key, fvk)
}

/// Every pending spend must be keyed by the vault: `rk == ak + [alpha] G`.
fn assert_rks_are_ours(group_key: [u8; 32], pending: &[PendingSpend]) {
    let ak = orchard::primitives::redpallas::VerificationKey::<
        orchard::primitives::redpallas::SpendAuth,
    >::try_from(group_key)
    .expect("ak");
    for spend in pending {
        let expected_rk: [u8; 32] = (&ak.randomize(&spend.alpha)).into();
        assert_eq!(
            expected_rk, spend.rk,
            "rk of {:?} action {} must be ak randomized by its alpha",
            spend.pool, spend.index
        );
    }
}

/// Signer-set checks that must fail before anything is applied to the PCZT,
/// then the real 2-of-3 signing and extraction.
fn sign_and_extract(
    vault: &KeygenOutput,
    pczt: Pczt,
    height: u32,
) -> (Transaction, Vec<send::SignedSpend>) {
    let err = send::sign_pczt(
        pczt.clone(),
        &keyholders(vault, &[2]),
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
    let (signed, spends) =
        send::sign_pczt(pczt, &keyholders(vault, &[1, 3]), &vault.public_key_package)
            .expect("sign with keyholders 1 and 3");
    assert!(
        matches!(
            send::pending_spends(&signed),
            Err(VaultError::NothingToSign)
        ),
        "no spend should remain unsigned"
    );

    // The extractor re-verifies the proofs, every spend-auth signature and the
    // binding signatures of both bundles before producing a transaction.
    let tx = TransactionExtractor::new(signed)
        .with_orchard(verifying_key())
        .extract()
        .expect("extract fully authorized transaction");
    assert_eq!(tx.version(), TxVersion::V6);
    assert_eq!(tx.consensus_branch_id(), BranchId::Nu6_3);
    assert_eq!(u32::from(tx.expiry_height()), height + 40);
    (tx, spends)
}

#[test]
fn ironwood_spend() {
    let (vault, group_key, fvk) = vault();
    let (pczt, spend_index) = proven_ironwood_pczt(post_nu6_3_network(), TARGET_HEIGHT, &fvk);

    // Exactly one spend is waiting for a signature: the real Ironwood spend.
    // The padding dummy was signed by the IO finalizer with its own key.
    let pending = send::pending_spends(&pczt).expect("pending spends");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].pool, ValuePool::Ironwood);
    assert_eq!(pending[0].index, spend_index);
    assert_eq!(pending[0].value, Some(NOTE_VALUE));
    assert_rks_are_ours(group_key, &pending);

    let (tx, spends) = sign_and_extract(&vault, pczt, TARGET_HEIGHT);
    assert_eq!(
        spends,
        vec![send::SignedSpend {
            pool: ValuePool::Ironwood,
            index: spend_index,
            value: Some(NOTE_VALUE),
        }]
    );
    assert!(tx.orchard_bundle().is_none(), "no Orchard-pool bundle");
    let ironwood = tx.ironwood_bundle().expect("ironwood bundle");
    assert_eq!(ironwood.actions().len(), 2);
}

#[test]
fn orchard_pool_spend_crossing_into_ironwood() {
    let (vault, group_key, fvk) = vault();
    let (pczt, spend_index) = proven_orchard_pool_pczt(&fvk);

    // Two Orchard-pool spends await signatures: the real spend and the
    // wallet-controlled zero-value spend paired with the change output. The
    // Ironwood bundle holds only pre-signed padding dummies.
    let mut pending = send::pending_spends(&pczt).expect("pending spends");
    pending.sort_by_key(|s| s.index);
    assert_eq!(pending.len(), 2, "{pending:?}");
    assert!(pending.iter().all(|s| s.pool == ValuePool::Orchard));
    let real = pending
        .iter()
        .find(|s| s.index == spend_index)
        .expect("real spend pending");
    assert_eq!(real.value, Some(NOTE_VALUE));
    let paired = pending
        .iter()
        .find(|s| s.index != spend_index)
        .expect("paired zero-value spend pending");
    assert_eq!(paired.value, Some(0));
    assert_rks_are_ours(group_key, &pending);

    let (tx, spends) = sign_and_extract(&vault, pczt, TARGET_HEIGHT);
    assert_eq!(spends.len(), 2);
    assert!(spends.iter().any(|s| s.value == Some(0)));
    assert_eq!(
        tx.orchard_bundle().expect("orchard bundle").actions().len(),
        2
    );
    assert_eq!(
        tx.ironwood_bundle()
            .expect("ironwood bundle")
            .actions()
            .len(),
        2
    );
}

/// Mainnet as it is today (NU6.3 since height 3,428,143; NU7 has no mainnet
/// height yet, ZIP 259): the vault's pipeline, with the vendored
/// `zcash_protocol`, builds, proves and FROST-signs a version 6 transaction
/// on branch `0x37a5165b` that the mainnet version check accepts. Nothing is
/// broadcast.
#[test]
fn mainnet_ironwood_spend_is_v6_on_the_nu6_3_branch() {
    const MAINNET_TIP: u32 = 3_518_600;
    let mainnet = zeek_vault::Params::MainNetwork;
    assert_eq!(
        send::circuit_version_for(&mainnet, MAINNET_TIP.into()).expect("circuit"),
        OrchardCircuitVersion::PostNu6_3
    );
    let (vault, group_key, fvk) = vault();
    let (pczt, _) = proven_ironwood_pczt(mainnet, MAINNET_TIP, &fvk);
    assert_rks_are_ours(group_key, &send::pending_spends(&pczt).expect("pending"));
    let (tx, _) = sign_and_extract(&vault, pczt, MAINNET_TIP);
    assert_eq!(u32::from(tx.consensus_branch_id()), 0x37a5_165b);
    send::check_version(&mainnet, &tx, MAINNET_TIP.into()).expect("mainnet branch and version");
    assert!(
        send::check_version(&zeek_vault::Params::TestNetwork, &tx, 4_500_000.into()).is_err(),
        "the testnet NU7 branch differs"
    );
    // Before NU6.3 mainnet was on NU6.2, whose transactions are not V6 Ironwood.
    assert_eq!(
        BranchId::for_height(&mainnet, BlockHeight::from_u32(3_428_142)),
        BranchId::Nu6_2
    );
}
