//! The keyholder's payout verification against real PCZTs built offline:
//! an honest payout for a settlement this node computed is approved and
//! signable; every tampering (redirected output, changed amount, extra
//! output, foreign vault, unknown round, already-signed round) is refused
//! with a reason naming the check.

use std::sync::OnceLock;

use orchard::circuit::{OrchardCircuitVersion, ProvingKey};
use orchard::keys::{FullViewingKey, Scope};
use orchard::note_encryption::IronwoodDomain;
use orchard::tree::{MerkleHashOrchard, MerklePath};
use orchard::{Anchor, Note};
use pczt::roles::creator::Creator;
use pczt::roles::io_finalizer::IoFinalizer;
use pczt::Pczt;
use rand_09::rngs::StdRng;
use rand_09::SeedableRng;
use rand_core::OsRng;
use shardtree::store::memory::MemoryShardStore;
use shardtree::ShardTree;
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::transaction::builder::{BuildConfig, Builder, BundlePadding, PcztResult};
use zcash_primitives::transaction::fees::zip317;
use zcash_primitives::transaction::fees::FeeRule;
use zcash_protocol::consensus::TEST_NETWORK;
use zcash_protocol::memo::{Memo, MemoBytes};
use zcash_protocol::value::Zatoshis;
use zeek_core::digs::{ClassifiedDigs, ReceivedNote, ValidDig};
use zeek_core::memo::{DigMemo, PayoutReceiver, SealedReceiver};
use zeek_core::params::{GameParams, Network};
use zeek_core::seal::generate_sealing_keypair;
use zeek_core::settle::{self, RoundOutcome};
use zeek_core::timing::TimingVerdict;
use zeek_engine::db::{BlockRow, Db, PlanStatus, SettlementRow};
use zeek_keyholder::db::NodeDb;
use zeek_keyholder::verify::{verify_payout, VerifyContext};
use zeek_keyholder::KeyholderError;
use zeek_vault::frost;
use zeek_vault::keys;

static PROVING_KEY: OnceLock<ProvingKey> = OnceLock::new();

fn proving_key() -> &'static ProvingKey {
    PROVING_KEY.get_or_init(|| ProvingKey::build(OrchardCircuitVersion::PostNu6_3))
}

/// A post-NU7 testnet height above the stored chain tip (the genesis block),
/// so the PCZT is V6 on the NU7 branch and not expired.
const TARGET: u32 = 4_500_000;
const GENESIS: u32 = 4_499_000;
const NOTE_VALUE: u64 = 2_000_000;

/// A vault: FROST group key, viewing key, sealing key.
struct Vault {
    fvk: FullViewingKey,
    sealing: zeek_core::seal::SealingPrivateKey,
    sealing_pk: zeek_core::seal::SealingPublicKey,
}

fn vault(seed: u64) -> Vault {
    let out = frost::dealer_keygen(2, 3, OsRng).expect("keygen");
    let group = frost::group_key_bytes(&out.public_key_package).expect("gk");
    let fvk = keys::derive_full_viewing_key_from_seed(&group, [seed as u8; 32]).expect("fvk");
    let (sealing, sealing_pk) = generate_sealing_keypair(&mut StdRng::seed_from_u64(seed));
    Vault {
        fvk,
        sealing,
        sealing_pk,
    }
}

fn vault_note(fvk: &FullViewingKey) -> (Note, Anchor, MerklePath) {
    let recipient = fvk.address_at(0u32, Scope::External);
    let ivk = fvk.to_ivk(Scope::External).prepare();
    let bundle_version = orchard::bundle::BundleVersion::ironwood_v3();
    let mut builder = orchard::builder::Builder::new(
        orchard::builder::BundleType::DEFAULT,
        bundle_version,
        bundle_version.default_flags(),
        Anchor::empty_tree(),
    )
    .expect("builder");
    builder
        .add_output(
            None,
            recipient,
            orchard::value::NoteValue::from_raw(NOTE_VALUE),
            Memo::Empty.encode().into_bytes(),
        )
        .expect("output");
    let (bundle, meta) = builder.build::<i64>(OsRng).expect("build").expect("bundle");
    let action = &bundle.actions()[meta.output_action_index(0).expect("index")];
    let (note, _, _) =
        try_note_decryption(&IronwoodDomain::for_action(action), &ivk, action).expect("decrypt");
    let cmx: orchard::note::ExtractedNoteCommitment = note.commitment().into();
    let leaf = MerkleHashOrchard::from_cmx(&cmx);
    let mut tree =
        ShardTree::<_, 32, 16>::new(MemoryShardStore::<MerkleHashOrchard, u32>::empty(), 100);
    tree.append(leaf, incrementalmerkletree::Retention::Marked)
        .expect("append");
    tree.checkpoint(GENESIS).expect("checkpoint");
    let path = tree
        .witness_at_checkpoint_depth(0.into(), 0)
        .expect("witness")
        .expect("exists");
    let anchor = path.root(leaf);
    (note, anchor.into(), path.into())
}

/// Builds a proven payout PCZT from the vault note to `outputs` plus change.
fn payout_pczt(v: &Vault, outputs: &[(orchard::Address, u64)]) -> Pczt {
    let (note, anchor, path) = vault_note(&v.fvk);
    let mut builder = Builder::new(
        TEST_NETWORK,
        TARGET.into(),
        BuildConfig::Standard {
            sapling_anchor: None,
            orchard_anchor: None,
            ironwood_anchor: Some(anchor),
            orchard_padding: BundlePadding::DEFAULT,
            ironwood_padding: BundlePadding::DEFAULT,
        },
    );
    builder
        .add_ironwood_spend::<zip317::FeeRule>(v.fvk.clone(), note, path)
        .expect("spend");
    let mut paid = 0u64;
    for (address, value) in outputs {
        builder
            .add_ironwood_output::<zip317::FeeRule>(
                Some(v.fvk.to_ovk(Scope::External)),
                *address,
                Zatoshis::const_from_u64(*value),
                MemoBytes::from_bytes(b"Zeek round 0: treasure").expect("memo"),
            )
            .expect("output");
        paid += value;
    }
    // ZIP 317: one Ironwood action per output once the change is added
    // (the single spend fits in the first action), padded to two.
    let actions = (outputs.len() + 1).max(2);
    let fee = zip317::FeeRule::standard()
        .fee_required(
            &TEST_NETWORK,
            TARGET.into(),
            core::iter::empty::<zcash_primitives::transaction::fees::transparent::InputSize>(),
            core::iter::empty::<usize>(),
            0,
            0,
            0,
            actions,
        )
        .expect("fee")
        .into_u64();
    builder
        .add_ironwood_output::<zip317::FeeRule>(
            Some(v.fvk.to_ovk(Scope::Internal)),
            v.fvk.address_at(0u32, Scope::Internal),
            Zatoshis::const_from_u64(NOTE_VALUE - paid - fee),
            MemoBytes::empty(),
        )
        .expect("change");
    let PcztResult { pczt_parts, .. } = builder
        .build_for_pczt(OsRng, &zip317::FeeRule::standard())
        .expect("pczt");
    let pczt = Creator::build_from_parts(pczt_parts).expect("creator");
    let pczt = IoFinalizer::new(pczt).finalize_io().expect("io");
    zeek_vault::send::prove(pczt, proving_key()).expect("prove")
}

/// A player address and the sealed receiver a dig on `tile` would carry.
fn player(v: &Vault, seed: u8, round: u32, tile: u8) -> (orchard::Address, SealedReceiver) {
    let sk = loop {
        let mut bytes = [seed; 32];
        bytes[0] = bytes[0].wrapping_add(1);
        if let Some(sk) = Option::<orchard::keys::SpendingKey>::from(
            orchard::keys::SpendingKey::from_bytes(bytes),
        ) {
            break sk;
        }
    };
    let address = FullViewingKey::from(&sk).address_at(0u32, Scope::External);
    let receiver = PayoutReceiver {
        network: Network::Test,
        orchard: address.to_raw_address_bytes(),
    };
    let memo = DigMemo::seal(
        round,
        tile,
        &receiver,
        &v.sealing_pk,
        &mut StdRng::seed_from_u64(u64::from(seed)),
    )
    .expect("seal");
    (address, memo.sealed_receiver)
}

/// Stores a settlement for round 0 with the given digs and treasure tile.
fn settle_round(
    db: &Db,
    params: &GameParams,
    digs: Vec<(u8, u64, SealedReceiver)>,
    treasure_tile: u8,
) -> settle::Settlement {
    let classified = ClassifiedDigs {
        valid: digs
            .into_iter()
            .map(|(tile, value, sealed)| ValidDig {
                note: ReceivedNote {
                    txid: "aa".repeat(32),
                    output_index: 0,
                    height: params.genesis_height,
                    value,
                    memo: vec![],
                },
                tile,
                sealed_receiver: sealed,
            })
            .collect(),
        refunds: vec![],
        forfeits: vec![],
    };
    let outcome = RoundOutcome::Drawn {
        treasure_tile,
        cache_triggered: false,
    };
    let settlement = settle::settle(params, 0, &classified, 0, outcome).expect("settle");
    db.insert_block(&BlockRow {
        height: params.genesis_height,
        hash: vec![1; 32],
        prev_hash: vec![0; 32],
        time: 1,
    })
    .expect("block");
    db.insert_settlement(&SettlementRow {
        round: 0,
        settled_at_height: params.genesis_height,
        start_block_time: 1,
        post_close_block_times: vec![2; 11],
        drand_round: 1,
        beacon_time: 3,
        timing: TimingVerdict::Safe {
            median_time_past: 2,
            beacon_time: 3,
            margin_secs: 1,
        },
        outcome,
        settlement: settlement.clone(),
        status: PlanStatus::AwaitingSignatures,
    })
    .expect("settlement");
    settlement
}

fn refusal(result: Result<zeek_keyholder::verify::Approved, KeyholderError>) -> String {
    match result {
        Err(KeyholderError::Refused(reason)) => reason,
        Err(other) => panic!("expected a refusal, got {other}"),
        Ok(_) => panic!("expected a refusal, got approval"),
    }
}

#[test]
fn honest_payout_is_approved_and_tampered_ones_are_refused() {
    let v = vault(1);
    let params = GameParams::testnet(GENESIS);
    let (alice, alice_sealed) = player(&v, 10, 0, 3);
    let (bob, bob_sealed) = player(&v, 20, 0, 3);
    let (mallory, _) = player(&v, 30, 0, 9);
    let chain = Db::open_in_memory().expect("db");
    let node = NodeDb::open_in_memory().expect("node db");
    let settlement = settle_round(
        &chain,
        &params,
        vec![(3, 500_000, alice_sealed), (3, 300_000, bob_sealed)],
        3,
    );
    assert_eq!(settlement.payouts.len(), 2);
    let (alice_net, bob_net) = (settlement.payouts[0].net, settlement.payouts[1].net);
    let ctx = VerifyContext {
        params: &params,
        fvk: &v.fvk,
        sealing: &v.sealing,
        chain: &chain,
        node: &node,
    };

    // Honest: both winners get their net amounts, change returns to the vault.
    let honest = payout_pczt(&v, &[(alice, alice_net), (bob, bob_net)]);
    let honest_bytes = honest.serialize().expect("serialize");
    let approved = verify_payout(&ctx, &honest_bytes, &[0]).expect("approved");
    assert_eq!(approved.payouts, 2);
    assert_eq!(approved.rounds, vec![0]);
    assert_eq!(approved.pending.len(), 1, "one real Ironwood spend to sign");
    assert_eq!(approved.fee, 15_000, "3 actions x 5000 zat");

    // Redirected output.
    let redirected = payout_pczt(&v, &[(mallory, alice_net), (bob, bob_net)]);
    let reason = refusal(verify_payout(
        &ctx,
        &redirected.serialize().expect("ser"),
        &[0],
    ));
    assert!(reason.contains("payout set mismatch"), "{reason}");

    // Changed amount (shortfall stays as change).
    let short = payout_pczt(&v, &[(alice, alice_net - 1), (bob, bob_net)]);
    assert!(
        refusal(verify_payout(&ctx, &short.serialize().expect("ser"), &[0]))
            .contains("payout set mismatch")
    );

    // Extra output.
    let extra = payout_pczt(
        &v,
        &[(alice, alice_net), (bob, bob_net), (mallory, 100_000)],
    );
    assert!(
        refusal(verify_payout(&ctx, &extra.serialize().expect("ser"), &[0]))
            .contains("payout set mismatch")
    );

    // Unknown and duplicated rounds.
    assert!(refusal(verify_payout(&ctx, &honest_bytes, &[1])).contains("not settled"));
    assert!(refusal(verify_payout(&ctx, &honest_bytes, &[0, 0])).contains("listed twice"));
    assert!(refusal(verify_payout(&ctx, &honest_bytes, &[])).contains("no rounds"));

    // Another vault's note cannot be spent here.
    let other = vault(2);
    let foreign = payout_pczt(&other, &[(alice, alice_net), (bob, bob_net)]);
    let reason = refusal(verify_payout(
        &ctx,
        &foreign.serialize().expect("ser"),
        &[0],
    ));
    assert!(reason.contains("not a note of this vault"), "{reason}");

    // Wrong sealing key: receivers do not open.
    let wrong_key = VerifyContext {
        sealing: &other.sealing,
        ..ctx
    };
    assert!(refusal(verify_payout(&wrong_key, &honest_bytes, &[0])).contains("does not open"));

    // Already signed.
    node.mark_signed(&[0], "earlier", 1).expect("mark");
    assert!(refusal(verify_payout(&ctx, &honest_bytes, &[0])).contains("already signed"));

    // Garbage is refused, not a crash.
    assert!(refusal(verify_payout(&ctx, b"not a pczt", &[0])).contains("does not parse"));
}
