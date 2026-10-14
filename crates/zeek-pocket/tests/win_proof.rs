//! Prove-a-win end to end on a synthetic chain, with real proofs and
//! signatures: a payout-shaped transaction pays a fresh address of a second
//! wallet, that wallet proves control of it, and tampered proofs fail with
//! the precise step that catches them.

use orchard::keys::Scope;
use orchard::note_encryption::IronwoodDomain;
use prost::Message;
use rand_core::OsRng;
use zcash_client_backend::proto::service::TreeState;
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::BranchId;
use zeek_core::proof::{PayoutOpening, PayoutOpenings, ProofBundle};
use zeek_engine::chain::ChainSource;
use zeek_engine::db::Pool;
use zeek_engine::relay::encode_blocks;
use zeek_engine::synthetic::{SyntheticChain, TestVault};
use zeek_pocket::keys::{encode_address, Keys};
use zeek_pocket::tx::{circuit_keys, SendRequest};
use zeek_pocket::wallet::Wallet;
use zeek_verify::chain::MinedTx;
use zeek_verify::win::{self, WinProof};

const BIRTHDAY: u32 = 4_600_000;
/// Round 74's real treasure payout amount.
const PAYOUT: u64 = 2_251_250;

fn tree_state() -> Vec<u8> {
    TreeState {
        network: "test".into(),
        height: u64::from(BIRTHDAY),
        hash: "00".repeat(32),
        time: 0,
        sapling_tree: String::new(),
        orchard_tree: String::new(),
        ironwood_tree: String::new(),
    }
    .encode_to_vec()
}

fn steps_failed_at(verdict: &win::WinVerdict) -> &str {
    verdict
        .steps
        .iter()
        .find(|s| !s.passed)
        .map(|s| s.name.as_str())
        .unwrap_or("none")
}

#[tokio::test]
async fn a_winner_proves_control_and_tampering_fails_precisely() {
    // A payer wallet funds itself, then sends the "payout" to a fresh
    // address (index 5) of the winner's wallet.
    let vault = TestVault::new(7);
    let payer_keys = Keys::from_entropy([11; 32]).expect("keys");
    let funding = payer_keys.address(0);
    let mut payer = Wallet::new(payer_keys, &tree_state()).expect("wallet");
    let winner = Keys::from_entropy([12; 32]).expect("keys");
    let fresh = winner.address(5);
    let mut chain = SyntheticChain::new(&vault, BIRTHDAY + 1, 1_800_000_000);
    chain.push_block(vec![chain.payment_to(
        funding,
        3_000_000,
        Pool::Ironwood,
        [0xF6; 512],
    )]);
    chain.advance(11);
    let tip = chain.next_height() - 1;
    let blocks =
        encode_blocks(&chain.block_range(BIRTHDAY + 1, tip).await.expect("range")).expect("encode");
    payer.scan(&blocks).expect("scan");
    let built = payer
        .build_send(&SendRequest {
            to: encode_address(&fresh).expect("ua"),
            amount_zat: PAYOUT,
            memo: Some("Zeek round 74: treasure".into()),
            chain_tip: tip,
        })
        .expect("send");
    let raw = hex::decode(&built.raw_hex).expect("hex");
    let height = chain.push_raw_transaction(raw.clone()).expect("mine");
    let tx = Transaction::read(&raw[..], BranchId::Nu6_3).expect("parse");
    let txid = tx.txid().to_string();

    // The real V6 transaction verifies under the verifier's rules (proof,
    // spend authorization and binding signatures with sighash = txid).
    assert_eq!(
        zeek_verify::checks::verify_ironwood_bundle(&tx),
        Ok(tx.ironwood_bundle().map_or(0, |b| b.actions().len()))
    );

    // The opening, as the engine records it.
    let ivk = winner.fvk().to_ivk(Scope::External).prepare();
    let (action_index, note) = tx
        .ironwood_bundle()
        .expect("bundle")
        .actions()
        .iter()
        .enumerate()
        .find_map(|(i, a)| {
            try_note_decryption(&IronwoodDomain::for_action(a), &ivk, a).map(|(n, _, _)| (i, n))
        })
        .expect("the winner's output");
    assert_eq!(note.value().inner(), PAYOUT);
    let opening = PayoutOpening {
        payout_index: 0,
        action_index: action_index as u32,
        recipient: hex::encode(note.recipient().to_raw_address_bytes()),
        value: PAYOUT,
        rseed: hex::encode(note.rseed().as_bytes()),
        note_version: 3,
    };
    let rho = tx.ironwood_bundle().expect("bundle").actions()[action_index]
        .nullifier()
        .to_bytes();
    assert_eq!(
        zeek_verify::checks::opening_cmx(&opening, rho).expect("cmx"),
        tx.ironwood_bundle().expect("bundle").actions()[action_index]
            .cmx()
            .to_bytes()
    );

    // Round 74's real bundle, extended (format 4) with this opening.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/examples/phase5/round-74.proof.json"
    );
    let mut bundle: ProofBundle =
        serde_json::from_str(&std::fs::read_to_string(path).expect("bundle")).expect("json");
    assert_eq!(bundle.settlement.payouts[0].net, PAYOUT);
    bundle.rules_version = bundle.claimed_rules_version();
    bundle.version = 4;
    bundle.payout_openings = Some(PayoutOpenings {
        txid: txid.clone(),
        mined_height: height,
        outputs: vec![opening],
    });
    let mined = MinedTx { raw, height };

    // The winner proves control.
    let proof = win::create(
        &bundle,
        0,
        winner.fvk(),
        &winner.ask(),
        &circuit_keys().0,
        &mut OsRng,
    )
    .expect("proof");
    let encoded = proof.encode().expect("encode");
    assert!(encoded.len() < 10_000, "compact: {} bytes", encoded.len());
    let decoded = WinProof::decode(&encoded).expect("decode");
    let verdict = win::verify(&decoded, &bundle, &mined, true);
    assert!(verdict.valid, "{verdict:?}");
    assert!(verdict.claim.contains("round 74"));

    // Another wallet cannot even build one.
    let stranger = Keys::from_entropy([13; 32]).expect("keys");
    assert!(matches!(
        win::create(
            &bundle,
            0,
            stranger.fvk(),
            &stranger.ask(),
            &circuit_keys().0,
            &mut OsRng
        ),
        Err(win::WinProofError::NotOurAddress)
    ));

    // Tampering.
    let mut wrong_round = decoded.clone();
    wrong_round.round += 1;
    let v = win::verify(&wrong_round, &bundle, &mined, true);
    assert_eq!(steps_failed_at(&v), "round");
    let mut other_round_bundle = bundle.clone();
    other_round_bundle.round += 1;
    let mut relabelled = decoded.clone();
    relabelled.round += 1;
    let v = win::verify(&relabelled, &other_round_bundle, &mined, true);
    assert_eq!(
        steps_failed_at(&v),
        "signature",
        "the signature binds the round: {v:?}"
    );

    let mut wrong_output = decoded.clone();
    wrong_output.payout_index = 1;
    let v = win::verify(&wrong_output, &bundle, &mined, true);
    assert_eq!(steps_failed_at(&v), "payout");

    let mut altered = decoded.clone();
    altered.nonce[0] ^= 1;
    let v = win::verify(&altered, &bundle, &mined, true);
    assert_eq!(steps_failed_at(&v), "signature");
    assert!(v.reason.as_deref().unwrap_or("").contains("bound message"));

    let mut swapped_note = decoded.clone();
    swapped_note.note_rseed[0] ^= 1;
    let v = win::verify(&swapped_note, &bundle, &mined, true);
    assert!(["address binding"].contains(&steps_failed_at(&v)), "{v:?}");

    let v = win::verify(&decoded, &bundle, &mined, false);
    assert_eq!(steps_failed_at(&v), "payout transaction");

    // A proof made for one opening cannot be pointed at another address.
    let mut moved = decoded.clone();
    moved.opening.recipient = hex::encode(winner.address(6).to_raw_address_bytes());
    let v = win::verify(&moved, &bundle, &mined, true);
    assert_eq!(steps_failed_at(&v), "payout");
}
