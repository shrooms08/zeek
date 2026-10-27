//! A chain reorg near the tip never touches a settlement whose inputs it did
//! not reach, and never one whose payout transaction exists (audit B, R2).
//!
//! A settlement reads blocks up to `window.end + mtp_window - 1` and is made
//! once the tip is `SETTLE_CONFIRMATIONS` past that, so an ordinary one- or
//! two-block reorg of the tip cannot change it. It used to be deleted anyway
//! (its stored height was the tip's), and a paying round was then re-settled
//! as awaiting signatures, counted as owed again ("Short by X ZEC").

mod common;

use zeek_core::proof::{PayoutOpenings, PayoutState};
use zeek_engine::chain::ingest;
use zeek_engine::config::{GameConfig, SETTLE_CONFIRMATIONS};
use zeek_engine::db::{PayoutRecord, PlanStatus, SettlementRow};
use zeek_engine::decrypt::DigScanner;
use zeek_engine::processor::Processor;
use zeek_engine::store::Store;
use zeek_engine::synthetic::{SyntheticChain, TestVault};

use common::{fixture_beacons, scenario, start_time, GENESIS};

struct Settled {
    vault: TestVault,
    chain: SyntheticChain,
    config: GameConfig,
    scanner: DigScanner,
    store: Store,
    processor: Processor,
    now: u64,
}

impl Settled {
    async fn ingest(&mut self) -> zeek_engine::chain::IngestReport {
        ingest(&mut self.chain, &self.store, &self.scanner, &self.config)
            .await
            .expect("ingest")
    }

    async fn process(&mut self) {
        self.processor
            .process(&self.store, &mut fixture_beacons(), self.now)
            .await
            .expect("process");
    }

    fn row(&self) -> SettlementRow {
        self.store
            .read(|db| db.settlement(0))
            .expect("read")
            .expect("round 0 is settled")
    }

    /// Replaces the top `depth` blocks with new ones (and one more).
    fn reorg(&mut self, depth: u32) {
        let tip = self.chain.next_height() - 1;
        self.chain.truncate(tip + 1 - depth);
        self.chain.advance(depth + 1);
    }
}

async fn settle(chain: SyntheticChain, vault: TestVault) -> Settled {
    let config = vault.config(GENESIS);
    let scanner = DigScanner::new(config.incoming_viewing_key().expect("ivk"));
    let processor = Processor::new(config.clone());
    let now = start_time(&config.params) + 600 + 900 + 5;
    let mut s = Settled {
        vault,
        chain,
        config,
        scanner,
        store: Store::open_in_memory().expect("db"),
        processor,
        now,
    };
    s.ingest().await;
    s.process().await;
    let _ = s.row();
    s
}

/// Round 0 of the shared scenario: digs, a draw, payouts.
async fn paying_round() -> Settled {
    let sc = scenario(0);
    let s = settle(sc.chain, sc.vault).await;
    assert!(
        !s.row().settlement.payouts.is_empty(),
        "round 0 pays someone"
    );
    s
}

/// Round 0 with no digs that closed too late: voided, nothing to pay.
async fn empty_round() -> Settled {
    let vault = TestVault::new(42);
    let config = vault.config(GENESIS);
    let mut chain = SyntheticChain::new(&vault, GENESIS - 3, start_time(&config.params) - 75);
    chain.advance(3);
    chain.push_block(vec![]);
    while chain.next_height() < GENESIS + config.params.blocks_per_round {
        chain.advance(1);
    }
    chain.delay(3_600);
    chain.advance(config.params.mtp_window + SETTLE_CONFIRMATIONS);
    let s = settle(chain, vault).await;
    assert!(s.row().settlement.payouts.is_empty(), "nothing to pay");
    s
}

fn record(s: &Settled, state: PayoutState) -> PayoutRecord {
    PayoutRecord {
        round: 0,
        state,
        session: "00".repeat(32),
        txid: None,
        height: s.row().settled_at_height,
        fee: 10_000,
        signers: vec![],
    }
}

/// The coordinator signed the payout but has not broadcast it yet.
fn signing(s: &Settled) {
    let r = record(s, PayoutState::Signing);
    s.store.write(|db| db.insert_payout(&r)).expect("payout");
}

/// The coordinator broadcast the payout (and marked the round paid).
fn broadcast(s: &Settled) {
    signing(s);
    let height = s.row().settled_at_height;
    s.store
        .write(|db| {
            db.transaction(|tx| {
                tx.mark_payout_broadcast(0, &"ab".repeat(32), height, &[1, 2])?;
                tx.set_plan_status(0, PlanStatus::Paid)
            })
        })
        .expect("broadcast");
}

/// The payout was mined: its openings are recorded.
fn mined(s: &Settled) {
    broadcast(s);
    let openings = PayoutOpenings {
        txid: "ab".repeat(32),
        mined_height: s.chain.next_height() - 1,
        outputs: vec![],
    };
    s.store
        .write(|db| db.set_payout_openings(0, &openings))
        .expect("openings");
}

/// What the status page reads: nothing owed beyond the Cache, no shortfall.
fn assert_nothing_owed(s: &Settled) {
    let ledger = s
        .store
        .read(|db| {
            let cache = s.processor.cache_balance(db)?;
            zeek_engine::ledger::compute(db, &s.processor, cache, 0)
        })
        .expect("ledger");
    assert!(
        ledger.unpaid_rounds.is_empty(),
        "{:?}",
        ledger.unpaid_rounds
    );
    assert_eq!(ledger.unpaid_obligations, 0);
    assert_eq!(ledger.shortfall, 0, "the status page would say Short by");
}

#[tokio::test]
async fn a_no_payout_settlement_survives_a_one_block_reorg() {
    let mut s = empty_round().await;
    s.store
        .write(|db| db.set_plan_status(0, PlanStatus::NothingToPay))
        .expect("status");
    let before = s.row();
    s.reorg(1);
    let report = s.ingest().await;
    assert!(report.reorg_from.is_some());
    assert!(report.rolled_back_settlements.is_empty());
    s.process().await;
    assert_eq!(s.row(), before);
}

#[tokio::test]
async fn a_signed_payout_keeps_its_settlement_through_a_reorg() {
    let mut s = paying_round().await;
    signing(&s);
    let before = s.row();
    s.reorg(1);
    let report = s.ingest().await;
    assert!(report.rolled_back_settlements.is_empty());
    s.process().await;
    assert_eq!(s.row(), before);
    let payout = s
        .store
        .read(|db| db.payout(0))
        .expect("read")
        .expect("kept");
    assert_eq!(payout.state, PayoutState::Signing);
}

#[tokio::test]
async fn a_broadcast_payout_stays_paid_through_a_reorg() {
    let mut s = paying_round().await;
    broadcast(&s);
    let before = s.row();
    s.reorg(1);
    s.ingest().await;
    s.process().await;
    assert_eq!(s.row(), before);
    assert_eq!(s.row().status, PlanStatus::Paid);
    assert_nothing_owed(&s);
}

#[tokio::test]
async fn a_mined_payout_stays_paid_through_a_reorg() {
    let mut s = paying_round().await;
    mined(&s);
    let before = s.row();
    s.reorg(1);
    s.ingest().await;
    s.process().await;
    assert_eq!(s.row(), before);
    assert_nothing_owed(&s);
    assert!(s
        .store
        .read(|db| db.payout_openings(0))
        .expect("read")
        .is_some());
}

#[tokio::test]
async fn a_two_block_reorg_leaves_a_paid_round_paid() {
    let mut s = paying_round().await;
    broadcast(&s);
    let before = s.row();
    s.reorg(2);
    let report = s.ingest().await;
    assert!(report.reorg_from.is_some());
    s.process().await;
    assert_eq!(s.row(), before);
    assert_nothing_owed(&s);
}

/// Blocks a settlement read: up to `window.end + mtp_window - 1`. A reorg
/// that deep (and more) reaches them.
fn depth_reaching_inputs(s: &Settled) -> u32 {
    let tip = s.chain.next_height() - 1;
    let last_read = GENESIS + s.config.params.blocks_per_round + s.config.params.mtp_window - 1;
    tip - last_read + 1
}

#[tokio::test]
async fn a_deep_reorg_with_the_same_result_keeps_a_no_payout_settlement() {
    let mut s = empty_round().await;
    s.store
        .write(|db| db.set_plan_status(0, PlanStatus::NothingToPay))
        .expect("status");
    let before = s.row();
    let depth = depth_reaching_inputs(&s);
    s.reorg(depth);
    s.ingest().await;
    s.process().await;
    assert_eq!(s.row(), before);
    assert!(s
        .store
        .read(|db| db.settlement_rechecks())
        .expect("read")
        .is_empty());
}

#[tokio::test]
async fn a_deep_reorg_with_a_different_result_never_overwrites_the_settlement() {
    let mut s = empty_round().await;
    s.store
        .write(|db| db.set_plan_status(0, PlanStatus::NothingToPay))
        .expect("status");
    let before = s.row();
    // The new chain forks inside round 0's window, closes on time (no
    // delay) and carries a dig: recomputed, round 0 would be drawn and pay.
    let last_in_window = GENESIS + s.config.params.blocks_per_round - 1;
    s.chain.truncate(last_in_window);
    let dig = s.chain.payment(
        500_000,
        zeek_engine::db::Pool::Ironwood,
        s.vault.dig_memo(0, 3, 1).encode(),
    );
    s.chain.push_block(vec![dig]);
    s.chain
        .advance(s.config.params.mtp_window + SETTLE_CONFIRMATIONS + 1);
    s.ingest().await;
    s.process().await;
    assert_eq!(s.row(), before, "the settlement is not overwritten");
    assert_eq!(
        s.store.read(|db| db.settlement_rechecks()).expect("read"),
        vec![0],
        "it stays flagged, and settling stops there"
    );
}

#[tokio::test]
async fn a_deep_reorg_never_touches_a_paying_round() {
    let mut s = paying_round().await;
    broadcast(&s);
    let before = s.row();
    let depth = depth_reaching_inputs(&s);
    s.reorg(depth);
    s.ingest().await;
    s.process().await;
    assert_eq!(s.row(), before);
    assert!(s
        .store
        .read(|db| db.settlement_rechecks())
        .expect("read")
        .is_empty());
    assert_nothing_owed(&s);
}
