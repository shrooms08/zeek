//! A crafted payment to the vault never halts the chain follower (audit B, R1).
//!
//! Compact blocks carry only the first 52 bytes of an output's ciphertext,
//! enough to detect a payment and read its value, but not its authentication
//! tag. Consensus does not check that a non-coinbase output's ciphertext
//! decrypts, so anyone can pay the public vault address with an output whose
//! ciphertext is valid for 52 bytes and corrupt after them. The engine and the
//! keyholders (which run the same `chain::ingest`) used to fail on every pass
//! on that block. Such an output is now skipped: it is not a game note. Real
//! digs (whole, matching outputs) are never skipped, and a source that serves
//! a different transaction than the block describes is still an error.

mod common;

use std::collections::BTreeMap;

use async_trait::async_trait;
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::BranchId;
use zeek_core::proof::ProofBundle;
use zeek_core::settle::Settlement;
use zeek_engine::chain::{ingest, ChainSource};
use zeek_engine::config::GameConfig;
use zeek_engine::db::Pool;
use zeek_engine::decrypt::DigScanner;
use zeek_engine::processor::Processor;
use zeek_engine::store::Store;
use zeek_engine::synthetic::{SyntheticChain, TestVault};
use zeek_engine::EngineError;
use zeek_verify::chain::{Blocks, MinedTx};
use zeek_verify::checks::{verify_round, PublicConfig};
use zeek_verify::{CheckId, CheckResult, CheckStatus, RoundReport};

use common::{fixture_beacons, scenario, scenario_with, start_time, Scenario, GENESIS};

/// Where the corruption goes in the 580-byte ciphertext (the compact form
/// covers bytes 0..52; the memo follows; the last 16 bytes are the tag).
#[derive(Clone, Copy, Debug)]
enum Corrupt {
    FirstByteAfterCompact,
    Memo,
    Tag,
}

/// A real payment to the vault (carrying a valid dig memo) whose first
/// Ironwood action's ciphertext is corrupted past the compact part.
fn crafted(chain: &mut SyntheticChain, vault: &TestVault, value: u64, at: Corrupt) -> Vec<u8> {
    let spec = chain.payment(value, Pool::Ironwood, vault.dig_memo(0, 3, 9).encode());
    let raw = chain.unmined_transaction(&spec);
    corrupt(raw, at)
}

/// `raw` with every Ironwood action's ciphertext corrupted at `at` (one of
/// them pays the vault; the others are change or dummies).
fn corrupt(raw: Vec<u8>, at: Corrupt) -> Vec<u8> {
    let actions = Transaction::read(&raw[..], BranchId::Nu6_3)
        .expect("parse")
        .ironwood_bundle()
        .expect("an Ironwood bundle")
        .actions()
        .len();
    (0..actions).fold(raw, |raw, index| corrupt_one(raw, index, at))
}

/// `raw` with Ironwood action `index`'s ciphertext corrupted at `at`.
fn corrupt_one(mut raw: Vec<u8>, index: usize, at: Corrupt) -> Vec<u8> {
    let tx = Transaction::read(&raw[..], BranchId::Nu6_3).expect("parse");
    let bundle = tx.ironwood_bundle().expect("an Ironwood bundle");
    let ciphertext = bundle.actions()[index]
        .encrypted_note()
        .enc_ciphertext
        .to_vec();
    let start = raw
        .windows(ciphertext.len())
        .position(|w| w == ciphertext.as_slice())
        .expect("the ciphertext is in the serialization");
    let offset = match at {
        Corrupt::FirstByteAfterCompact => 52,
        Corrupt::Memo => 300,
        Corrupt::Tag => ciphertext.len() - 1,
    };
    raw[start + offset] ^= 0x01;
    raw
}

/// The shared scenario's round 0 (alice and bob on tile 3, carol on 17, a
/// wrong-round dig, dust and a non-dig), with `extra` transactions mined
/// inside its window.
fn round_with(extra: impl FnOnce(&mut SyntheticChain, &TestVault)) -> (SyntheticChain, TestVault) {
    let Scenario { chain, vault } = scenario_with(0, extra);
    (chain, vault)
}

/// Follows `chain` and settles round 0, as the engine does and as each
/// keyholder does (its follower runs the same `chain::ingest` and
/// `Processor::process`), into a store of its own.
async fn follow_and_settle<S: ChainSource>(
    source: &mut S,
    config: &GameConfig,
) -> Result<(Store, Processor), EngineError> {
    let store = Store::open_in_memory()?;
    let scanner = DigScanner::new(config.incoming_viewing_key()?);
    ingest(source, &store, &scanner, config).await?;
    let mut processor = Processor::new(config.clone());
    let now = start_time(&config.params) + 600 + 900 + 5;
    processor
        .process(&store, &mut fixture_beacons(), now)
        .await?;
    Ok((store, processor))
}

fn settlement(store: &Store) -> Settlement {
    store
        .read(|db| db.settlement(0))
        .expect("read")
        .expect("round 0 settled")
        .settlement
}

#[tokio::test]
async fn a_crafted_payment_does_not_halt_the_follower() {
    for at in [Corrupt::FirstByteAfterCompact, Corrupt::Memo, Corrupt::Tag] {
        let (mut chain, vault) = round_with(|chain, vault| {
            let raw = crafted(chain, vault, 250_000, at);
            chain.push_raw_transaction(raw).expect("mined");
        });
        let config = vault.config(GENESIS);
        let (store, _) = follow_and_settle(&mut chain, &config)
            .await
            .unwrap_or_else(|e| panic!("{at:?}: the follower halted: {e}"));
        // The crafted output is not a game note; the six real ones are kept.
        assert_eq!(
            store.read(|db| db.note_count()).expect("count"),
            6,
            "{at:?}"
        );
    }
}

#[tokio::test]
async fn real_digs_alongside_a_crafted_payment_settle_exactly_as_without_it() {
    let Scenario {
        chain: mut plain_chain,
        vault,
    } = scenario(0);
    let config = vault.config(GENESIS);
    let (plain, _) = follow_and_settle(&mut plain_chain, &config)
        .await
        .expect("plain");

    let (mut chain, vault) = round_with(|chain, vault| {
        let raw = crafted(chain, vault, 250_000, Corrupt::Tag);
        chain.push_raw_transaction(raw).expect("mined");
    });
    let config = vault.config(GENESIS);
    let (with_crafted, _) = follow_and_settle(&mut chain, &config)
        .await
        .expect("crafted");
    assert_eq!(settlement(&with_crafted), settlement(&plain));
}

/// A source that serves a different transaction than the compact block
/// describes for one txid (a faulty or hostile lightwalletd).
struct Swapped {
    chain: SyntheticChain,
    txid: [u8; 32],
    raw: Vec<u8>,
}

#[async_trait]
impl ChainSource for Swapped {
    async fn latest_height(&mut self) -> Result<u32, EngineError> {
        ChainSource::latest_height(&mut self.chain).await
    }

    async fn block_range(
        &mut self,
        start: u32,
        end: u32,
    ) -> Result<Vec<CompactBlock>, EngineError> {
        ChainSource::block_range(&mut self.chain, start, end).await
    }

    async fn transaction(&mut self, txid: &[u8; 32]) -> Result<Vec<u8>, EngineError> {
        if *txid == self.txid {
            return Ok(self.raw.clone());
        }
        ChainSource::transaction(&mut self.chain, txid).await
    }

    async fn transaction_height(&mut self, txid: &[u8; 32]) -> Result<u32, EngineError> {
        ChainSource::transaction_height(&mut self.chain, txid).await
    }
}

#[tokio::test]
async fn a_source_serving_another_transaction_for_a_real_dig_is_an_error_not_a_skip() {
    let (chain, vault) = round_with(|_, _| {});
    let config = vault.config(GENESIS);
    // Alice's dig, and in its place a different (crafted) transaction.
    let alice = chain
        .block(GENESIS + 5)
        .expect("block")
        .vtx
        .first()
        .expect("tx")
        .txid
        .clone();
    let mut other = SyntheticChain::new(&vault, GENESIS, start_time(&config.params));
    let raw = crafted(&mut other, &vault, 500_000, Corrupt::Tag);
    let mut source = Swapped {
        chain,
        txid: alice.as_slice().try_into().expect("32 bytes"),
        raw,
    };
    let result = follow_and_settle(&mut source, &config).await;
    assert!(
        result.is_err(),
        "a mismatched transaction must stop ingestion (to retry another source), never drop a dig"
    );
}

#[tokio::test]
async fn engine_keyholder_and_verifier_classify_the_crafted_payment_alike() {
    let (mut chain, vault) = round_with(|chain, vault| {
        let raw = crafted(chain, vault, 250_000, Corrupt::Memo);
        chain.push_raw_transaction(raw).expect("mined");
    });
    let config = vault.config(GENESIS);
    // The engine and a keyholder, each from its own copy of the chain.
    let (engine, processor) = follow_and_settle(&mut chain, &config)
        .await
        .expect("engine");
    let (keyholder, _) = follow_and_settle(&mut chain, &config)
        .await
        .expect("keyholder");
    assert_eq!(settlement(&engine), settlement(&keyholder));

    // The verifier, from the engine's proof bundle and the chain itself.
    let row = engine
        .read(|db| db.settlement(0))
        .expect("read")
        .expect("settled");
    let bundle = engine
        .read(|db| processor.proof_bundle(db, &row))
        .expect("bundle");
    let report = verify(&config, &bundle, &chain, chain_txs(&chain));
    for id in [CheckId::Digs, CheckId::Settlement] {
        let check = find(&report, id);
        assert_eq!(check.status, CheckStatus::Pass, "{id:?}: {check:?}");
    }
}

/// Every mined transaction of `chain`, by display txid, as a verifier
/// fetches them.
fn chain_txs(chain: &SyntheticChain) -> BTreeMap<String, MinedTx> {
    let mut txs = BTreeMap::new();
    for h in GENESIS..chain.next_height() {
        for tx in &chain.block(h).expect("block").vtx {
            let txid: [u8; 32] = tx.txid.as_slice().try_into().expect("32 bytes");
            let mut display = txid;
            display.reverse();
            if let Some(raw) = SyntheticChain::transaction(chain, &txid) {
                txs.insert(hex::encode(display), MinedTx { raw, height: h });
            }
        }
    }
    txs
}

/// The verifier's report on `bundle`, from `chain`'s blocks and `txs`.
fn verify(
    config: &GameConfig,
    bundle: &ProofBundle,
    chain: &SyntheticChain,
    txs: BTreeMap<String, MinedTx>,
) -> RoundReport {
    let blocks =
        Blocks::new((GENESIS..chain.next_height()).filter_map(|h| chain.block(h).cloned()));
    let public = PublicConfig {
        params: config.params.clone(),
        vault_address: config.vault_address.clone(),
        uivk: config.uivk.clone(),
    };
    let ivk = public.ivk().expect("ivk");
    verify_round(&zeek_verify::checks::RoundInput {
        config: &public,
        bundle,
        blocks: &blocks,
        txs: &txs,
        ivk: &ivk,
    })
}

fn find(report: &RoundReport, id: CheckId) -> &CheckResult {
    report
        .checks
        .iter()
        .find(|c| c.id == id)
        .expect("check present")
}

/// Alice's dig (a real one) and its mined transaction.
fn alice(chain: &SyntheticChain) -> ([u8; 32], Vec<u8>) {
    let txid: [u8; 32] = chain
        .block(GENESIS + 5)
        .expect("block")
        .vtx
        .first()
        .expect("tx")
        .txid
        .as_slice()
        .try_into()
        .expect("32 bytes");
    let raw = SyntheticChain::transaction(chain, &txid).expect("raw");
    (txid, raw)
}

#[tokio::test]
async fn a_source_flipping_a_byte_of_a_real_dig_is_an_error_not_a_skip() {
    // The same transaction, one memo byte changed: same note commitment, so
    // only its txid shows it is not the transaction the block holds.
    let (chain, vault) = round_with(|_, _| {});
    let config = vault.config(GENESIS);
    let (txid, honest) = alice(&chain);
    let mut source = Swapped {
        chain,
        txid,
        raw: corrupt(honest, Corrupt::Memo),
    };
    let result = follow_and_settle(&mut source, &config).await;
    assert!(
        result.is_err(),
        "a real dig must never be ignored because a source altered its transaction"
    );
}

#[tokio::test]
async fn the_verifier_rejects_a_transaction_that_does_not_hash_to_its_txid() {
    // A bundle missing alice's dig (an engine fooled by such a source) and
    // the same altered transaction: the verifier must not agree with it.
    let (mut chain, vault) = round_with(|_, _| {});
    let config = vault.config(GENESIS);
    let (engine, processor) = follow_and_settle(&mut chain, &config)
        .await
        .expect("engine");
    let row = engine
        .read(|db| db.settlement(0))
        .expect("read")
        .expect("settled");
    let mut bundle = engine
        .read(|db| processor.proof_bundle(db, &row))
        .expect("bundle");
    let (txid, honest) = alice(&chain);
    let mut display = txid;
    display.reverse();
    let alice_hex = hex::encode(display);
    bundle.notes.retain(|n| n.txid != alice_hex);
    let mut txs = chain_txs(&chain);
    let altered = corrupt(honest, Corrupt::Memo);
    let height = txs.get(&alice_hex).expect("alice").height;
    txs.insert(
        alice_hex,
        MinedTx {
            raw: altered,
            height,
        },
    );
    let report = verify(&config, &bundle, &chain, txs);
    let digs = find(&report, CheckId::Digs);
    assert_eq!(digs.status, CheckStatus::Fail, "{digs:?}");
    assert!(
        digs.summary.contains("does not hash to its txid"),
        "{digs:?}"
    );
}

#[tokio::test]
async fn a_real_dig_sharing_a_transaction_with_a_crafted_output_still_counts() {
    // One transaction, two outputs to the vault (two digs on tile 5); one
    // of them corrupted after its compact part. Either order of actions.
    for crafted_index in 0..2 {
        let mut txid = [0u8; 32];
        let (mut chain, vault) = round_with(|chain, vault| {
            let a = chain.payment(250_000, Pool::Ironwood, vault.dig_memo(0, 5, 20).encode());
            let b = chain.payment(250_000, Pool::Ironwood, vault.dig_memo(0, 5, 21).encode());
            let raw = chain.unmined_transaction_with(&[a, b]);
            let raw = corrupt_one(raw, crafted_index, Corrupt::Memo);
            txid = *Transaction::read(&raw[..], BranchId::Nu6_3)
                .expect("parse")
                .txid()
                .as_ref();
            chain.push_raw_transaction(raw).expect("mined");
        });
        let config = vault.config(GENESIS);
        let (engine, processor) = follow_and_settle(&mut chain, &config)
            .await
            .unwrap_or_else(|e| panic!("action {crafted_index}: the follower halted: {e}"));
        let kept: Vec<_> = engine
            .read(|db| db.notes_in_range(GENESIS, chain.next_height()))
            .expect("notes")
            .into_iter()
            .filter(|n| n.txid == txid)
            .collect();
        assert_eq!(
            kept.len(),
            1,
            "action {crafted_index}: the real dig is kept"
        );
        assert_ne!(usize::from(kept[0].action_index), crafted_index);
        assert_eq!(kept[0].value, 250_000);
        let digs = [20, 21].map(|p| vault.dig_memo(0, 5, p).encode().to_vec());
        assert!(digs.contains(&kept[0].memo), "it is one of the two digs");

        // A keyholder settles it the same, and the verifier agrees.
        let (keyholder, _) = follow_and_settle(&mut chain, &config)
            .await
            .expect("keyholder");
        assert_eq!(settlement(&engine), settlement(&keyholder));
        let row = engine
            .read(|db| db.settlement(0))
            .expect("read")
            .expect("settled");
        let bundle = engine
            .read(|db| processor.proof_bundle(db, &row))
            .expect("bundle");
        let report = verify(&config, &bundle, &chain, chain_txs(&chain));
        for id in [CheckId::Digs, CheckId::Settlement] {
            let check = find(&report, id);
            assert_eq!(check.status, CheckStatus::Pass, "{id:?}: {check:?}");
        }
    }
}
