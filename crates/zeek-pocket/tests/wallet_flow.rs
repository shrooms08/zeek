//! The wallet core end to end on a synthetic chain: a funding payment is
//! scanned with its memo, a two-tile dig is built and proven, the vault's own
//! keys find two dig outputs whose sealed receivers open to two distinct
//! fresh addresses of this wallet, the dig is mined and detected as a spend
//! with its change, the state survives serialization, and a reorg rewinds.

use orchard::keys::Scope;
use orchard::note_encryption::IronwoodDomain;
use prost::Message;
use zcash_client_backend::proto::service::TreeState;
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::transaction::{Transaction, TxVersion};
use zcash_protocol::consensus::BranchId;
use zeek_core::memo::DigMemo;
use zeek_core::params::Network;
use zeek_engine::chain::ChainSource;
use zeek_engine::db::Pool;
use zeek_engine::relay::encode_blocks;
use zeek_engine::synthetic::{SyntheticChain, TestVault};
use zeek_pocket::keys::Keys;
use zeek_pocket::store::{self, OutgoingKind, OutgoingStatus};
use zeek_pocket::tx::{DigRequest, EngineConfig, SendRequest};
use zeek_pocket::wallet::{HistoryEntry, Wallet};

const BIRTHDAY: u32 = 4_600_000;

fn birthday_tree_state() -> Vec<u8> {
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

async fn blocks(chain: &mut SyntheticChain, start: u32, end: u32) -> Vec<u8> {
    encode_blocks(&chain.block_range(start, end).await.expect("range")).expect("encode")
}

fn text_memo(text: &str) -> [u8; 512] {
    let mut memo = [0u8; 512];
    memo[..text.len()].copy_from_slice(text.as_bytes());
    memo
}

#[tokio::test]
async fn fund_scan_dig_mine_and_rewind() {
    let vault = TestVault::new(91);
    let config = vault.config(BIRTHDAY - 100);
    let engine_config = EngineConfig {
        params: config.params.clone(),
        vault_address: config.vault_address.clone(),
        sealing_public_key: config.sealing_public_key.clone(),
    };
    let keys = Keys::from_entropy([3; 32]).expect("keys");
    let receive = keys.address(0);
    let mut wallet = Wallet::new(keys, &birthday_tree_state()).expect("wallet");
    assert_eq!(wallet.scanned_height(), BIRTHDAY);

    // Funding: 0.01 to the receive address with a text memo, then 12 blocks.
    let mut chain = SyntheticChain::new(&vault, BIRTHDAY + 1, 1_800_000_000);
    let funding_height = chain.push_block(vec![chain.payment_to(
        receive,
        1_000_000,
        Pool::Ironwood,
        text_memo("funding from alice"),
    )]);
    chain.advance(12);
    let tip = chain.next_height() - 1;
    let report = wallet
        .scan(&blocks(&mut chain, BIRTHDAY + 1, tip).await)
        .expect("scan");
    assert_eq!(report.blocks, 13);
    assert_eq!(report.new_notes, 1);
    assert_eq!(wallet.scanned_height(), tip);
    let balance = wallet.balance().expect("balance");
    assert_eq!(balance.spendable_zat, 1_000_000);

    // Memo: fetched with decoys by the TypeScript layer; here directly.
    let requests = wallet.memo_requests();
    assert_eq!(requests.wanted.len(), 1);
    assert_eq!(requests.wanted[0].height, funding_height);
    let mut internal = hex::decode(&requests.wanted[0].txid).expect("hex");
    internal.reverse();
    let raw = chain.transaction(&internal).expect("raw");
    assert_eq!(
        wallet
            .attach_transaction(&raw, funding_height)
            .expect("attach"),
        1
    );
    assert!(wallet.memo_requests().wanted.is_empty());
    match &wallet.history()[0] {
        HistoryEntry::Received {
            memo, value_zat, ..
        } => {
            assert_eq!(memo.as_deref(), Some("funding from alice"));
            assert_eq!(*value_zat, 1_000_000);
        }
        other => panic!("unexpected history entry {other:?}"),
    }

    // A dig too close to the round's end is refused.
    let request = |blocks_remaining: u32| DigRequest {
        tiles: vec![3, 17],
        amount_each_zat: 100_000,
        round: 7,
        blocks_remaining,
        chain_tip: tip,
        message: None,
    };
    assert!(wallet.build_dig(&engine_config, &request(3)).is_err());
    // Campfire is not active in this game: a message is refused up front.
    let with_message = DigRequest {
        message: Some("hello".into()),
        ..request(10)
    };
    assert_eq!(
        wallet
            .build_dig(&engine_config, &with_message)
            .expect_err("inactive")
            .code(),
        "invalid_request"
    );

    // Two tiles, one transaction.
    let built = wallet.build_dig(&engine_config, &request(10)).expect("dig");
    assert_eq!(
        built.fee_zat, 15_000,
        "1 spend, 2 digs + change = 3 actions"
    );
    assert_eq!(built.target_height, tip + 1);
    let raw_dig = hex::decode(&built.raw_hex).expect("hex");
    let tx = Transaction::read(&raw_dig[..], BranchId::Nu6_3).expect("parse");
    assert_eq!(tx.version(), TxVersion::V6);
    assert_eq!(tx.txid().to_string(), built.txid);

    // The vault sees exactly two digs; their receivers are two distinct
    // fresh addresses of this wallet (indices 1 and 2), never index 0.
    let vault_ivk = vault.fvk.to_ivk(Scope::External).prepare();
    let mut receivers = Vec::new();
    for action in tx.ironwood_bundle().expect("bundle").actions() {
        if let Some((note, _, memo)) =
            try_note_decryption(&IronwoodDomain::for_action(action), &vault_ivk, action)
        {
            assert_eq!(note.value().inner(), 100_000);
            let dig = DigMemo::decode(&memo).expect("dig memo");
            assert_eq!(dig.round, 7);
            let receiver = dig
                .open(Network::Test, &vault.sealing_private)
                .expect("opens with the vault's sealing key");
            receivers.push((dig.tile, receiver.orchard));
        }
    }
    receivers.sort();
    assert_eq!(receivers.len(), 2);
    assert_eq!(receivers[0].0, 3);
    assert_eq!(receivers[1].0, 17);
    assert_ne!(receivers[0].1, receivers[1].1, "fresh receiver per dig");
    let keys = wallet.keys();
    let ours: Vec<[u8; 43]> = (0..4)
        .map(|i| keys.address(i).to_raw_address_bytes())
        .collect();
    assert!(receivers.iter().all(|(_, r)| ours[1..3].contains(r)));
    assert!(receivers.iter().all(|(_, r)| *r != ours[0]));

    // Reserved until mined: nothing spendable, change pending.
    wallet.mark_broadcast(&built.txid).expect("broadcast");
    let balance = wallet.balance().expect("balance");
    assert_eq!(balance.spendable_zat, 0);
    assert_eq!(balance.pending_zat, 1_000_000 - 200_000 - 15_000);
    // A second dig uses fresh indices 3 and 4, and finds no funds.
    assert!(wallet.build_dig(&engine_config, &request(10)).is_err());

    // Mined: the note is spent, the change arrives (internal scope).
    let mined_at = chain.push_raw_transaction(raw_dig).expect("mine");
    chain.advance(3);
    let tip2 = chain.next_height() - 1;
    let report = wallet
        .scan(&blocks(&mut chain, tip + 1, tip2).await)
        .expect("scan");
    assert_eq!(report.spent_notes, 1);
    assert_eq!(report.new_notes, 1, "the change note");
    let sent = wallet
        .history()
        .into_iter()
        .find_map(|e| match e {
            HistoryEntry::Sent(out) => Some(out),
            _ => None,
        })
        .expect("dig in history");
    assert_eq!(sent.status, OutgoingStatus::Mined);
    assert_eq!(sent.mined_height, Some(mined_at));
    assert!(matches!(
        sent.kind,
        OutgoingKind::Dig { ref receiver_indices, .. } if *receiver_indices == vec![1, 2]
    ));
    let balance = wallet.balance().expect("balance");
    assert_eq!(balance.spendable_zat, 785_000, "change has 3 confirmations");

    // Persisted state reopens to the same wallet.
    let bytes = wallet.state_bytes().expect("state");
    let reopened = Wallet::from_state(
        Keys::from_entropy([3; 32]).expect("keys"),
        store::from_bytes(&bytes).expect("parse"),
    )
    .expect("reopen");
    assert_eq!(
        reopened.balance().expect("balance"),
        wallet.balance().expect("balance")
    );
    assert_eq!(reopened.history(), wallet.history());

    // Reorg: rewinding below the dig's block unspends the note and drops the
    // change; rescanning the same blocks restores the state.
    wallet.rewind(mined_at - 1).expect("rewind");
    assert_eq!(wallet.scanned_height(), mined_at - 1);
    assert_eq!(
        wallet.balance().expect("balance").spendable_zat,
        0,
        "still reserved"
    );
    wallet
        .scan(&blocks(&mut chain, mined_at, tip2).await)
        .expect("rescan");
    assert_eq!(wallet.balance().expect("balance").spendable_zat, 785_000);

    // A block that does not extend the scanned chain is a reorg.
    let mut fork = SyntheticChain::new(&vault, tip2 + 1, 1_900_000_000);
    fork.advance(1);
    let err = wallet
        .scan(&blocks(&mut fork, tip2 + 1, tip2 + 1).await)
        .expect_err("prev hash mismatch");
    assert_eq!(err.code(), "reorg");

    // Sending to another wallet works from the change.
    let other = Keys::from_entropy([4; 32]).expect("keys");
    let to = zeek_pocket::keys::encode_address(&other.address(0)).expect("ua");
    let sent = wallet
        .build_send(&SendRequest {
            to,
            amount_zat: 500_000,
            memo: Some("winnings".into()),
            chain_tip: tip2,
        })
        .expect("send");
    assert_eq!(sent.fee_zat, 10_000);
    let tx = Transaction::read(
        &hex::decode(&sent.raw_hex).expect("hex")[..],
        BranchId::Nu6_3,
    )
    .expect("parse");
    let other_ivk = other.fvk().to_ivk(Scope::External).prepare();
    let received: Vec<_> = tx
        .ironwood_bundle()
        .expect("bundle")
        .actions()
        .iter()
        .filter_map(|a| try_note_decryption(&IronwoodDomain::for_action(a), &other_ivk, a))
        .collect();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].0.value().inner(), 500_000);
    assert_eq!(&received[0].2[..8], b"winnings");
}
