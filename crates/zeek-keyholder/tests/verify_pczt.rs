//! The keyholder's payout verification against real PCZTs built offline:
//! an honest payout for a settlement this node computed is approved and
//! signable; every tampering (redirected output, changed amount, extra or
//! missing output, missing or foreign input, needless float note, fee above
//! the collected fees, foreign vault, unknown or v1 round, already-signed
//! round) is refused with a reason naming the exact check and item.

use std::convert::Infallible;
use std::sync::OnceLock;

use orchard::circuit::{OrchardCircuitVersion, ProvingKey};
use orchard::keys::{FullViewingKey, Scope};
use orchard::note_encryption::IronwoodDomain;
use orchard::tree::{MerkleHashOrchard, MerklePath};
use orchard::{Anchor, Note};
use pczt::roles::creator::Creator;
use pczt::roles::io_finalizer::IoFinalizer;
use pczt::roles::verifier::Verifier;
use pczt::Pczt;
use rand_09::rngs::StdRng;
use rand_09::SeedableRng;
use rand_core::OsRng;
use shardtree::store::memory::MemoryShardStore;
use shardtree::ShardTree;
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::transaction::builder::{BuildConfig, Builder, BundlePadding, PcztResult};
use zcash_primitives::transaction::fees::zip317;
use zcash_protocol::consensus::TEST_NETWORK;
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::Zatoshis;
use zeek_core::digs;
use zeek_core::memo::{DigMemo, PayoutReceiver};
use zeek_core::params::{GameParams, Network};
use zeek_core::seal::generate_sealing_keypair;
use zeek_core::settle::{self, RoundOutcome, Settlement};
use zeek_core::timing::TimingVerdict;
use zeek_engine::config::GameConfig;
use zeek_engine::db::{BlockRow, Db, NoteRow, PlanStatus, Pool, SettlementRow};
use zeek_engine::processor::Processor;
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

/// A note the vault holds, witnessed in a tree checkpointed at GENESIS.
struct VaultNote {
    note: Note,
    path: MerklePath,
    cmx: [u8; 32],
}

/// Ironwood notes to the vault in one bundle (`scope`, value, memo each),
/// all witnessed under one anchor.
fn vault_notes(
    fvk: &FullViewingKey,
    specs: &[(Scope, u64, [u8; 512])],
) -> (Vec<VaultNote>, Anchor) {
    let bundle_version = orchard::bundle::BundleVersion::ironwood_v3();
    let mut builder = orchard::builder::Builder::new(
        orchard::builder::BundleType::DEFAULT,
        bundle_version,
        bundle_version.default_flags(),
        Anchor::empty_tree(),
    )
    .expect("builder");
    for (scope, value, memo) in specs {
        builder
            .add_output(
                None,
                fvk.address_at(0u32, *scope),
                orchard::value::NoteValue::from_raw(*value),
                *memo,
            )
            .expect("output");
    }
    let (bundle, meta) = builder.build::<i64>(OsRng).expect("build").expect("bundle");
    let mut tree =
        ShardTree::<_, 32, 16>::new(MemoryShardStore::<MerkleHashOrchard, u32>::empty(), 100);
    let mut decrypted = Vec::with_capacity(specs.len());
    for (i, (scope, _, _)) in specs.iter().enumerate() {
        let action = &bundle.actions()[meta.output_action_index(i).expect("index")];
        let ivk = fvk.to_ivk(*scope).prepare();
        let (note, _, _) = try_note_decryption(&IronwoodDomain::for_action(action), &ivk, action)
            .expect("decrypt");
        let cmx: orchard::note::ExtractedNoteCommitment = note.commitment().into();
        tree.append(
            MerkleHashOrchard::from_cmx(&cmx),
            incrementalmerkletree::Retention::Marked,
        )
        .expect("append");
        decrypted.push((note, cmx));
    }
    tree.checkpoint(GENESIS).expect("checkpoint");
    let mut notes = Vec::with_capacity(specs.len());
    let mut anchor = None;
    for (i, (note, cmx)) in decrypted.into_iter().enumerate() {
        let path = tree
            .witness_at_checkpoint_depth((i as u64).into(), 0)
            .expect("witness")
            .expect("exists");
        let root: Anchor = path.root(MerkleHashOrchard::from_cmx(&cmx)).into();
        anchor = Some(root);
        notes.push(VaultNote {
            note,
            path: path.into(),
            cmx: cmx.to_bytes(),
        });
    }
    (notes, anchor.expect("at least one note"))
}

/// Builds a proven payout PCZT spending `spends` to `outputs` plus change.
fn payout_pczt(
    v: &Vault,
    spends: &[&VaultNote],
    anchor: Anchor,
    outputs: &[(orchard::Address, u64)],
) -> Pczt {
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
    let mut spent = 0u64;
    for spend in spends {
        builder
            .add_ironwood_spend::<zip317::FeeError>(v.fvk.clone(), spend.note, spend.path.clone())
            .expect("spend");
        spent += spend.note.value().inner();
    }
    let mut paid = 0u64;
    for (address, value) in outputs {
        builder
            .add_ironwood_output::<zip317::FeeError>(
                Some(v.fvk.to_ovk(Scope::External)),
                *address,
                Zatoshis::const_from_u64(*value),
                MemoBytes::from_bytes(b"Zeek round 0: treasure").expect("memo"),
            )
            .expect("output");
        paid += value;
    }
    // ZIP 317: max(spends, outputs incl. change) actions, at least two.
    let actions = spends.len().max(outputs.len() + 1).max(2) as u64;
    let fee = 5_000 * actions;
    let change = spent
        .checked_sub(paid + fee)
        .filter(|c| *c > 0)
        .expect("test shapes leave positive change");
    builder
        .add_ironwood_output::<zip317::FeeError>(
            Some(v.fvk.to_ovk(Scope::Internal)),
            v.fvk.address_at(0u32, Scope::Internal),
            Zatoshis::const_from_u64(change),
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

/// The Ironwood action index whose output pays `address` (the builder
/// shuffles actions, so tests look indices up).
fn output_index_of(pczt: &Pczt, address: &orchard::Address) -> usize {
    let mut found = None;
    Verifier::new(pczt.clone())
        .with_ironwood::<Infallible, _>(|bundle| {
            for (i, action) in bundle.actions().iter().enumerate() {
                if action.output().recipient().as_ref() == Some(address) {
                    found = Some(i);
                }
            }
            Ok(())
        })
        .expect("ironwood bundle");
    found.expect("output present")
}

/// The Ironwood action index whose spend is the note with commitment `cmx`.
fn spend_index_of(pczt: &Pczt, cmx: &[u8; 32]) -> usize {
    let mut found = None;
    Verifier::new(pczt.clone())
        .with_ironwood::<Infallible, _>(|bundle| {
            for (i, action) in bundle.actions().iter().enumerate() {
                let spend = action.spend();
                if let (Some(r), Some(v), Some(rho), Some(rseed)) =
                    (spend.recipient(), spend.value(), spend.rho(), spend.rseed())
                {
                    let note: Option<Note> = Option::from(Note::from_parts(
                        *r,
                        *v,
                        *rho,
                        *rseed,
                        *spend.note_version(),
                    ));
                    if let Some(note) = note {
                        let c: orchard::note::ExtractedNoteCommitment = note.commitment().into();
                        if c.to_bytes() == *cmx {
                            found = Some(i);
                        }
                    }
                }
            }
            Ok(())
        })
        .expect("ironwood bundle");
    found.expect("spend present")
}

/// A player: an address and a sealed dig memo for `round`/`tile`.
fn player(v: &Vault, seed: u8, round: u32, tile: u8) -> (orchard::Address, DigMemo) {
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
    (address, memo)
}

/// A dig to store: value, memo, the vault note that carries it.
struct Dig<'a> {
    value: u64,
    memo: [u8; 512],
    note: &'a VaultNote,
    height: u32,
}

/// Stores `digs` as the node's notes and settles round 0 from them.
fn settle_round(
    db: &Db,
    processor: &Processor,
    digs: &[Dig<'_>],
    outcome: RoundOutcome,
    previous_cache: u64,
    rules_version: u32,
) -> Settlement {
    let params = &processor.config().params;
    db.insert_block(&BlockRow {
        height: GENESIS,
        hash: vec![1; 32],
        prev_hash: vec![0; 32],
        time: 1,
    })
    .expect("block");
    let mut received = Vec::new();
    for (i, dig) in digs.iter().enumerate() {
        let row = NoteRow {
            txid: [0xaa; 32],
            action_index: i as u16,
            pool: Pool::Ironwood,
            height: dig.height,
            block_time: 1,
            value: dig.value,
            memo: dig.memo.to_vec(),
            cmx: Some(dig.note.cmx),
        };
        assert!(db.insert_note(&row).expect("note"));
        if dig.height < GENESIS + params.blocks_per_round {
            received.push(row.to_received_note());
        }
    }
    let voided = matches!(outcome, RoundOutcome::Voided);
    let classified = digs::classify_round(params, 0, voided, received).expect("classify");
    let settlement =
        settle::settle(params, 0, &classified, previous_cache, outcome).expect("settle");
    db.insert_settlement(&SettlementRow {
        round: 0,
        settled_at_height: GENESIS,
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
        rules_version,
    })
    .expect("settlement");
    settlement
}

fn processor(v: &Vault) -> Processor {
    let receiver = PayoutReceiver {
        network: Network::Test,
        orchard: v
            .fvk
            .address_at(0u32, Scope::External)
            .to_raw_address_bytes(),
    };
    let ufvk = keys::unified_full_viewing_key(&v.fvk).expect("ufvk");
    Processor::new(GameConfig {
        params: GameParams::testnet(GENESIS),
        vault_address: receiver.to_unified_address().expect("address"),
        sealing_public_key: v.sealing_pk.clone(),
        uivk: keys::encode_uivk(&keys::unified_incoming_viewing_key(&ufvk)),
        genesis_height: GENESIS,
        initial_cache_zat: 0,
    })
}

fn refusal(result: Result<zeek_keyholder::verify::Approved, KeyholderError>) -> String {
    match result {
        Err(KeyholderError::Refused(reason)) => reason,
        Err(other) => panic!("expected a refusal, got {other}"),
        Ok(_) => panic!("expected a refusal, got approval"),
    }
}

fn drawn(tile: u8, cache_triggered: bool) -> RoundOutcome {
    RoundOutcome::Drawn {
        treasure_tile: tile,
        cache_triggered,
    }
}

#[test]
fn honest_payout_is_approved_and_tampered_ones_are_refused() {
    let v = vault(1);
    let processor = processor(&v);
    let (alice, alice_memo) = player(&v, 10, 0, 3);
    let (bob, bob_memo) = player(&v, 20, 0, 3);
    let (mallory, _) = player(&v, 30, 0, 9);
    let (_carol, carol_memo) = player(&v, 40, 1, 5);
    // Vault notes: alice's and bob's digs, carol's round-1 dig, two float notes.
    let (notes, anchor) = vault_notes(
        &v.fvk,
        &[
            (Scope::External, 500_000, alice_memo.encode()),
            (Scope::External, 300_000, bob_memo.encode()),
            (Scope::External, 150_000, carol_memo.encode()),
            (Scope::Internal, 50_000, [0xF6; 512]),
            (Scope::Internal, 1_200_000, [0xF6; 512]),
        ],
    );
    let (alice_note, bob_note, carol_note, small_float, big_float) =
        (&notes[0], &notes[1], &notes[2], &notes[3], &notes[4]);
    let chain = Db::open_in_memory().expect("db");
    let node = NodeDb::open_in_memory().expect("node db");
    let settlement = settle_round(
        &chain,
        &processor,
        &[
            Dig {
                value: 500_000,
                memo: alice_memo.encode(),
                note: alice_note,
                height: GENESIS + 1,
            },
            Dig {
                value: 300_000,
                memo: bob_memo.encode(),
                note: bob_note,
                height: GENESIS + 2,
            },
            Dig {
                value: 150_000,
                memo: carol_memo.encode(),
                note: carol_note,
                height: GENESIS + 30, // round 1
            },
        ],
        drawn(3, false),
        0,
        2,
    );
    assert_eq!(settlement.payouts.len(), 2);
    let (alice_net, bob_net) = (settlement.payouts[0].net, settlement.payouts[1].net);
    assert_eq!((alice_net, bob_net), (464_062, 276_437));
    assert_eq!(settlement.total_fees, 20_000, "2 dig fees + 2 output fees");
    let ctx = VerifyContext {
        processor: &processor,
        fvk: &v.fvk,
        sealing: &v.sealing,
        chain: &chain,
        node: &node,
    };
    let digs = [alice_note, bob_note];

    // Honest: both digs spent, both winners paid, change returns to the vault.
    let honest = payout_pczt(&v, &digs, anchor, &[(alice, alice_net), (bob, bob_net)]);
    let honest_bytes = honest.serialize().expect("serialize");
    let approved = verify_payout(&ctx, &honest_bytes, &[0]).expect("approved");
    assert_eq!(approved.payouts, 2);
    assert_eq!(approved.distinct_receivers, 2);
    assert_eq!(approved.charged_inputs, 2);
    assert_eq!(approved.float_inputs, 0);
    assert_eq!(approved.rounds, vec![0]);
    assert_eq!(
        approved.pending.len(),
        2,
        "two real Ironwood spends to sign"
    );
    assert_eq!(
        approved.fee, 15_000,
        "3 actions x 5000 zat, within the 20,000 collected"
    );

    // Redirected output: the amount still matches a payout.
    let redirected = payout_pczt(&v, &digs, anchor, &[(mallory, alice_net), (bob, bob_net)]);
    let i = output_index_of(&redirected, &mallory);
    assert_eq!(
        refusal(verify_payout(&ctx, &redirected.serialize().expect("ser"), &[0])),
        format!("output {i}: recipient does not match the sealed payout receiver for round 0 (amount matches)")
    );

    // Changed amount to the right receiver.
    let short = payout_pczt(&v, &digs, anchor, &[(alice, alice_net - 1), (bob, bob_net)]);
    let i = output_index_of(&short, &alice);
    assert_eq!(
        refusal(verify_payout(&ctx, &short.serialize().expect("ser"), &[0])),
        format!("output {i}: amount 464,061 expected 464,062")
    );

    // Extra output.
    let extra = payout_pczt(
        &v,
        &digs,
        anchor,
        &[(alice, alice_net), (bob, bob_net), (mallory, 10_000)],
    );
    let i = output_index_of(&extra, &mallory);
    assert_eq!(
        refusal(verify_payout(&ctx, &extra.serialize().expect("ser"), &[0])),
        format!("output {i}: unexpected output of 10,000 zatoshis (not a payout of rounds [0] nor vault change)")
    );

    // Missing output.
    let missing = payout_pczt(&v, &digs, anchor, &[(alice, alice_net)]);
    assert_eq!(
        refusal(verify_payout(
            &ctx,
            &missing.serialize().expect("ser"),
            &[0]
        )),
        "round 0 payout 1 (net 276,437) is not among the outputs"
    );

    // Missing input: bob's dig note is not spent (a float note pays instead).
    let no_bob = payout_pczt(
        &v,
        &[alice_note, big_float],
        anchor,
        &[(alice, alice_net), (bob, bob_net)],
    );
    assert_eq!(
        refusal(verify_payout(&ctx, &no_bob.serialize().expect("ser"), &[0])),
        format!(
            "dig note {}:1 of round 0 is not spent by the transaction",
            "aa".repeat(32)
        )
    );

    // Another round's dig note spent as if it were float.
    let foreign_dig = payout_pczt(
        &v,
        &[alice_note, bob_note, carol_note],
        anchor,
        &[(alice, alice_net), (bob, bob_net)],
    );
    let i = spend_index_of(&foreign_dig, &carol_note.cmx);
    assert_eq!(
        refusal(verify_payout(
            &ctx,
            &foreign_dig.serialize().expect("ser"),
            &[0]
        )),
        format!("input {i}: note is not a dig of rounds [0]")
    );

    // A float note that is not needed.
    let needless = payout_pczt(
        &v,
        &[alice_note, bob_note, small_float],
        anchor,
        &[(alice, alice_net), (bob, bob_net)],
    );
    let i = spend_index_of(&needless, &small_float.cmx);
    assert_eq!(
        refusal(verify_payout(&ctx, &needless.serialize().expect("ser"), &[0])),
        format!("input {i}: float note not needed: the dig notes of rounds [0] cover the outputs and the fee")
    );

    // Unknown and duplicated rounds.
    assert_eq!(
        refusal(verify_payout(&ctx, &honest_bytes, &[1])),
        "round 1 is not settled on this node"
    );
    assert_eq!(
        refusal(verify_payout(&ctx, &honest_bytes, &[0, 0])),
        "round 0 is listed twice"
    );
    assert_eq!(
        refusal(verify_payout(&ctx, &honest_bytes, &[])),
        "no rounds claimed"
    );

    // Another vault's notes cannot be spent here.
    let other = vault(2);
    let (other_notes, other_anchor) = vault_notes(
        &other.fvk,
        &[(Scope::External, 900_000, alice_memo.encode())],
    );
    let foreign = payout_pczt(
        &other,
        &[&other_notes[0]],
        other_anchor,
        &[(alice, alice_net), (bob, bob_net)],
    );
    let reason = refusal(verify_payout(
        &ctx,
        &foreign.serialize().expect("ser"),
        &[0],
    ));
    assert!(
        reason.starts_with("input ") && reason.contains(": spend is not a note of this vault ("),
        "{reason}"
    );

    // Wrong sealing key: receivers do not open.
    let wrong_key = VerifyContext {
        sealing: &other.sealing,
        ..ctx
    };
    assert!(
        refusal(verify_payout(&wrong_key, &honest_bytes, &[0])).starts_with(
            "round 0 payout 0: the sealed receiver does not open with this node's sealing key ("
        )
    );

    // Already signed.
    node.mark_signed(&[0], "earlier", 1).expect("mark");
    assert_eq!(
        refusal(verify_payout(&ctx, &honest_bytes, &[0])),
        "round 0 was already signed in session earlier"
    );

    // Garbage is refused, not a crash.
    assert!(refusal(verify_payout(&ctx, b"not a pczt", &[0])).starts_with("PCZT does not parse: "));
}

#[test]
fn cache_payout_may_use_float_notes_but_only_within_the_collected_fees() {
    let v = vault(3);
    let processor = processor(&v);
    let (alice, alice_memo) = player(&v, 11, 0, 3);
    let (notes, anchor) = vault_notes(
        &v.fvk,
        &[
            (Scope::External, 100_000, alice_memo.encode()),
            (Scope::Internal, 1_200_000, [0xF6; 512]),
            (Scope::Internal, 600_000, [0xF6; 512]),
            (Scope::Internal, 600_000, [0xF6; 512]),
        ],
    );
    let (dig, big, half_a, half_b) = (&notes[0], &notes[1], &notes[2], &notes[3]);
    let chain = Db::open_in_memory().expect("db");
    let node = NodeDb::open_in_memory().expect("node db");
    // The Cache (1,000,000 carried in) triggers for the sole winner.
    let settlement = settle_round(
        &chain,
        &processor,
        &[Dig {
            value: 100_000,
            memo: alice_memo.encode(),
            note: dig,
            height: GENESIS + 1,
        }],
        drawn(3, true),
        1_000_000,
        2,
    );
    assert_eq!(settlement.payouts.len(), 1);
    assert_eq!(settlement.payouts[0].net, 1_090_000);
    assert_eq!(settlement.total_fees, 10_000);
    let ctx = VerifyContext {
        processor: &processor,
        fvk: &v.fvk,
        sealing: &v.sealing,
        chain: &chain,
        node: &node,
    };

    // One float note: 2 spends, 2 outputs, fee 10,000 = collected. Approved.
    let one_float = payout_pczt(&v, &[dig, big], anchor, &[(alice, 1_090_000)]);
    let approved =
        verify_payout(&ctx, &one_float.serialize().expect("ser"), &[0]).expect("approved");
    assert_eq!(approved.float_inputs, 1);
    assert_eq!(approved.fee, 10_000);

    // Two float notes: 3 spends, fee 15,000 > 10,000 collected. Refused.
    let two_floats = payout_pczt(&v, &[dig, half_a, half_b], anchor, &[(alice, 1_090_000)]);
    assert_eq!(
        refusal(verify_payout(
            &ctx,
            &two_floats.serialize().expect("ser"),
            &[0]
        )),
        "fee 15,000 exceeds the fees collected by the rules for rounds [0] (10,000)"
    );
}

#[test]
fn rounds_settled_under_rules_v1_are_not_signed() {
    let v = vault(4);
    let processor = processor(&v);
    let (alice, alice_memo) = player(&v, 12, 0, 3);
    let (notes, anchor) = vault_notes(&v.fvk, &[(Scope::External, 500_000, alice_memo.encode())]);
    let chain = Db::open_in_memory().expect("db");
    let node = NodeDb::open_in_memory().expect("node db");
    let settlement = settle_round(
        &chain,
        &processor,
        &[Dig {
            value: 500_000,
            memo: alice_memo.encode(),
            note: &notes[0],
            height: GENESIS + 1,
        }],
        drawn(3, false),
        0,
        1,
    );
    let ctx = VerifyContext {
        processor: &processor,
        fvk: &v.fvk,
        sealing: &v.sealing,
        chain: &chain,
        node: &node,
    };
    let pczt = payout_pczt(
        &v,
        &[&notes[0]],
        anchor,
        &[(alice, settlement.payouts[0].net)],
    );
    assert_eq!(
        refusal(verify_payout(&ctx, &pczt.serialize().expect("ser"), &[0])),
        "round 0 was settled under rules version 1; this node only signs rules version 2 payouts"
    );
}
