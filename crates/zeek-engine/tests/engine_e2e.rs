//! End-to-end: a recorded (synthetic) chain with digs flows through
//! ingestion, timing rule, beacon verification and settlement without any
//! network, and the result is reproducible, verifiable and private.

use std::collections::HashMap;

use zeek_core::beacon::Beacon;
use zeek_core::digs::RefundReason;
use zeek_core::params::GameParams;
use zeek_core::rounds::RoundStatus;
use zeek_core::settle::{PayoutReason, RoundOutcome};
use zeek_engine::api::{config_response, proof_response, round_response};
use zeek_engine::beacon::FixedBeacons;
use zeek_engine::chain::ingest;
use zeek_engine::config::SETTLE_CONFIRMATIONS;
use zeek_engine::db::{Db, Pool};
use zeek_engine::decrypt::DigScanner;
use zeek_engine::processor::{Event, Processor};
use zeek_engine::proof;
use zeek_engine::synthetic::{SyntheticChain, TestVault};

/// A real quicknet beacon (tests/fixtures/quicknet.json in zeek-core, from
/// https://api.drand.sh/v2/beacons/quicknet/rounds/10000000).
const FIXTURE_ROUND: u64 = 10_000_000;
const FIXTURE_SIGNATURE: &str =
    "a187c9f5800521e986b5658924696682071ce7ae965486e8fd08bc2c4758bdbb9ed9ecb7beee392a38214e321ecd434f";

const GENESIS: u32 = 5_000;

/// Start time of round 0 such that its bound drand round is the fixture round.
fn start_time(params: &GameParams) -> u64 {
    let beacon_time = params.drand.genesis_time + (FIXTURE_ROUND - 1) * params.drand.period_secs;
    beacon_time - params.round_duration_secs().expect("duration") - params.reveal_delay_secs
}

fn fixture_beacons() -> FixedBeacons {
    let mut beacons = HashMap::new();
    beacons.insert(
        FIXTURE_ROUND,
        Beacon {
            round: FIXTURE_ROUND,
            signature: FIXTURE_SIGNATURE.to_owned(),
        },
    );
    FixedBeacons { beacons }
}

struct Scenario {
    vault: TestVault,
    chain: SyntheticChain,
}

/// Round 0 with three valid digs (alice 0.005 on tile 3, bob 0.003 on tile
/// 3, carol 0.004 on tile 17), one wrong-round dig, one dust dig and one
/// text-memo note, followed by the blocks the timing rule and the
/// confirmation policy need.
fn scenario(delay_close_secs: u64) -> Scenario {
    let vault = TestVault::new(42);
    let config = vault.config(GENESIS);
    let mut chain = SyntheticChain::new(&vault, GENESIS - 3, start_time(&config.params) - 75);
    chain.advance(3); // GENESIS-3 .. GENESIS-1 (operator float era)
    assert_eq!(chain.next_height(), GENESIS);
    chain.push_block(vec![]); // GENESIS: start block, time == start_time
    chain.advance(4);
    chain.push_block(vec![
        chain.payment(500_000, Pool::Ironwood, vault.dig_memo(0, 3, 1).encode()),
        chain.change(1_000),
    ]);
    chain.push_block(vec![chain.payment(
        300_000,
        Pool::Ironwood,
        vault.dig_memo(0, 3, 2).encode(),
    )]);
    chain.advance(2);
    chain.push_block(vec![
        chain.payment(400_000, Pool::Ironwood, vault.dig_memo(0, 17, 3).encode()),
        chain.payment(200_000, Pool::Ironwood, vault.dig_memo(7, 17, 3).encode()),
        chain.payment(50_000, Pool::Ironwood, vault.dig_memo(0, 1, 4).encode()),
        chain.payment(150_000, Pool::Ironwood, [0xF6; 512]),
    ]);
    while chain.next_height() < GENESIS + config.params.blocks_per_round {
        chain.advance(1);
    }
    chain.delay(delay_close_secs);
    // Post-close blocks for the timing rule plus the confirmation depth.
    chain.advance(config.params.mtp_window + SETTLE_CONFIRMATIONS);
    Scenario { vault, chain }
}

#[tokio::test]
async fn round_is_ingested_settled_served_and_reproducible() {
    let Scenario { vault, mut chain } = scenario(0);
    let config = vault.config(GENESIS);
    let scanner = DigScanner::new(config.incoming_viewing_key().expect("ivk"));
    let mut db = Db::open_in_memory().expect("db");
    let mut processor = Processor::new(config.clone());
    let mut beacons = fixture_beacons();

    let report = ingest(&mut chain, &mut db, &scanner, &config)
        .await
        .expect("ingest");
    assert_eq!(
        report.new_notes, 6,
        "payments to the external address, never change"
    );

    // Before the beacon is published nothing settles.
    let (_, _, _) = processor.status(&db, 0).expect("status");
    let beacon_time = start_time(&config.params) + 600 + 900;
    let events = processor
        .process(&mut db, &mut beacons, beacon_time - 1)
        .await
        .expect("process");
    assert!(events.is_empty());
    let pending = processor.pending_rounds(&db).expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].drand_round, FIXTURE_ROUND);
    assert_eq!(pending[0].beacon_time, beacon_time);
    assert_eq!(pending[0].status, RoundStatus::Revealable);

    // Once published, the round settles.
    let events = processor
        .process(&mut db, &mut beacons, beacon_time + 5)
        .await
        .expect("process");
    assert!(matches!(
        events[0],
        Event::Reveal {
            round: 0,
            drand_round: FIXTURE_ROUND,
            ..
        }
    ));
    assert!(matches!(events[1], Event::Settled { round: 0, .. }));
    let row = db.settlement(0).expect("row").expect("settled");
    let RoundOutcome::Drawn {
        treasure_tile,
        cache_triggered,
    } = row.outcome
    else {
        panic!("expected a drawn round");
    };
    let s = &row.settlement;
    assert_eq!(row.rules_version, 2);
    assert_eq!(
        s.pot,
        1_200_000 - 3 * 5_000,
        "three dig fees come off first"
    );
    assert_eq!(s.rake, 59_250);
    assert_eq!(s.forfeited, 200_000, "dust dig and text memo");
    assert_eq!(
        s.dig_fees,
        4 * 5_000,
        "three valid digs and the paid refund"
    );
    let refunds: Vec<_> = s
        .payouts
        .iter()
        .filter(|p| {
            matches!(
                p.reason,
                PayoutReason::Refund(RefundReason::WrongRound { memo_round: 7 })
            )
        })
        .collect();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].net, 200_000 - 5_000 - 5_000);
    // The payout transaction must spend exactly the four charged notes.
    let inputs = processor.payout_inputs(&db, &row).expect("inputs");
    assert_eq!(inputs.len(), 4);
    assert!(inputs.iter().all(|n| n.value >= 200_000));
    assert!(inputs
        .windows(2)
        .all(|w| (w[0].height, &w[0].txid, w[0].output_index)
            <= (w[1].height, &w[1].txid, w[1].output_index)));
    let winners = s
        .payouts
        .iter()
        .filter(|p| matches!(p.reason, PayoutReason::Win { .. }))
        .count();
    match treasure_tile {
        3 => assert_eq!(winners, 2),
        17 => assert_eq!(winners, 1),
        _ => assert_eq!(winners, 0),
    }
    if !cache_triggered {
        assert_eq!(s.cache_paid, 0);
    }

    // API views.
    let config_view = config_response(&config);
    assert_eq!(config_view.genesis_height, GENESIS);
    let round_view = round_response(&processor, &db, 0).expect("round view");
    assert_eq!(round_view.status, RoundStatus::Settled);
    let settled = round_view.settled.as_ref().expect("settled view");
    assert_eq!(settled.rules_version, 2);
    assert_eq!(settled.dig_fees, 20_000);
    assert_eq!(settled.tile_totals[3], 800_000);
    assert_eq!(settled.tile_totals[17], 400_000);
    assert_eq!(settled.treasure_tile, Some(treasure_tile));
    assert_eq!(settled.payouts, s.payouts);
    let board = processor.board(&db).expect("board");
    assert_eq!(board.round, 1, "the tip is in round 1 now");
    assert_eq!(board.cache, s.new_cache);
    let pending = processor.pending_rounds(&db).expect("pending");
    assert!(pending.is_empty());

    // Proof bundle recomputes to the identical settlement with zeek-core only.
    let bundle = proof_response(&processor, &db, 0)
        .expect("proof")
        .expect("settled");
    let recomputed = proof::recompute(&bundle).expect("recompute");
    assert_eq!(
        serde_json::to_string(&recomputed).expect("json"),
        serde_json::to_string(&bundle.settlement).expect("json")
    );
    assert!(proof_response(&processor, &db, 1).expect("proof").is_none());

    // Privacy: no API output contains an opened payout receiver (the players'
    // receivers are [player; 43]); the sealed bytes are there instead.
    let mut all_json = String::new();
    for text in [
        serde_json::to_string(&config_view).expect("json"),
        serde_json::to_string(&round_view).expect("json"),
        serde_json::to_string(&board).expect("json"),
        serde_json::to_string(&bundle).expect("json"),
        serde_json::to_string(&events).expect("json"),
    ] {
        all_json.push_str(&text);
    }
    for player in 1u8..=4 {
        let opened = hex::encode([player; 43]);
        assert!(
            !all_json.contains(&opened),
            "opened receiver of player {player} leaked"
        );
    }
    let first_sealed = hex::encode(s.payouts[0].sealed_receiver.as_bytes());
    assert!(all_json.contains(&first_sealed));
    // The engine never needed the sealing private key; a holder of it can open.
    let opened = vault
        .dig_memo(0, 3, 1)
        .open(zeek_core::params::Network::Test, &vault.sealing_private)
        .expect("open");
    assert_eq!(opened.orchard, [1u8; 43]);

    // Idempotent restart: a fresh processor over the same database adds nothing,
    // and a fresh database over the same chain reproduces the state byte for byte.
    let dump = db.dump().expect("dump");
    let mut restarted = Processor::new(config.clone());
    let events = restarted
        .process(&mut db, &mut beacons, beacon_time + 1_000)
        .await
        .expect("process");
    assert!(events.is_empty());
    let again = ingest(&mut chain, &mut db, &scanner, &config)
        .await
        .expect("ingest");
    assert_eq!(again.new_blocks, 0);
    assert_eq!(db.dump().expect("dump"), dump);

    let mut fresh = Db::open_in_memory().expect("db");
    ingest(&mut chain, &mut fresh, &scanner, &config)
        .await
        .expect("ingest");
    let mut fresh_processor = Processor::new(config.clone());
    fresh_processor
        .process(&mut fresh, &mut fixture_beacons(), beacon_time + 5)
        .await
        .expect("process");
    assert_eq!(fresh.dump().expect("dump"), dump);
}

#[tokio::test]
async fn cache_carry_over_is_the_first_previous_cache() {
    let Scenario { vault, mut chain } = scenario(0);
    let mut config = vault.config(GENESIS);
    config.initial_cache_zat = 777_000;
    let scanner = DigScanner::new(config.incoming_viewing_key().expect("ivk"));
    let mut db = Db::open_in_memory().expect("db");
    let mut processor = Processor::new(config.clone());
    assert_eq!(processor.cache_balance(&db).expect("cache"), 777_000);
    assert_eq!(config_response(&config).initial_cache_zat, 777_000);
    ingest(&mut chain, &mut db, &scanner, &config)
        .await
        .expect("ingest");
    let beacon_time = start_time(&config.params) + 600 + 900;
    processor
        .process(&mut db, &mut fixture_beacons(), beacon_time + 5)
        .await
        .expect("process");
    let row = db.settlement(0).expect("row").expect("settled");
    assert_eq!(row.settlement.previous_cache, 777_000);
    assert!(
        row.settlement.new_cache >= 777_000 + 59_250,
        "rake lands on top of the carry-over"
    );
    let bundle = proof_response(&processor, &db, 0)
        .expect("proof")
        .expect("settled");
    assert_eq!(
        proof::recompute(&bundle).expect("recompute"),
        bundle.settlement
    );
}

#[tokio::test]
async fn late_window_is_voided_and_refunded() {
    // The window closes 20 minutes late: the post-close median lands after
    // the beacon, so the timing rule voids the round.
    let Scenario { vault, mut chain } = scenario(1_200);
    let config = vault.config(GENESIS);
    let scanner = DigScanner::new(config.incoming_viewing_key().expect("ivk"));
    let mut db = Db::open_in_memory().expect("db");
    let mut processor = Processor::new(config.clone());
    let mut beacons = fixture_beacons();
    ingest(&mut chain, &mut db, &scanner, &config)
        .await
        .expect("ingest");
    let (_, status, _) = processor.status(&db, 0).expect("status");
    assert_eq!(status, RoundStatus::Voided);

    let events = processor
        .process(&mut db, &mut beacons, 0)
        .await
        .expect("process");
    assert!(matches!(
        events[0],
        Event::Settled {
            round: 0,
            outcome: RoundOutcome::Voided,
            ..
        }
    ));
    let row = db.settlement(0).expect("row").expect("settled");
    assert_eq!(row.settlement.rake, 0);
    assert!(row
        .settlement
        .payouts
        .iter()
        .all(|p| matches!(p.reason, PayoutReason::Refund(_))));
    assert_eq!(
        row.settlement.payouts.len(),
        4,
        "three valid digs and the wrong-round dig"
    );
    assert_eq!(
        row.settlement.new_cache, 200_000,
        "forfeits still feed the cache"
    );
    let bundle = proof_response(&processor, &db, 0)
        .expect("proof")
        .expect("settled");
    assert!(bundle.beacon.is_none());
    let recomputed = proof::recompute(&bundle).expect("recompute");
    assert_eq!(recomputed, bundle.settlement);
}

/// The HTTP layer serves the same views, and 404s unsettled proofs.
#[tokio::test]
async fn http_api_serves_the_round() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use tokio::sync::{broadcast, Mutex};
    use tower::ServiceExt;
    use zeek_engine::api::{router, AppState, HealthResponse, PendingResponse, RoundResponse};
    use zeek_engine::proof::ProofBundle;

    let Scenario { vault, mut chain } = scenario(0);
    let config = vault.config(GENESIS);
    let scanner = DigScanner::new(config.incoming_viewing_key().expect("ivk"));
    let mut db = Db::open_in_memory().expect("db");
    let mut processor = Processor::new(config.clone());
    ingest(&mut chain, &mut db, &scanner, &config)
        .await
        .expect("ingest");
    let beacon_time = start_time(&config.params) + 600 + 900;
    processor
        .process(&mut db, &mut fixture_beacons(), beacon_time + 5)
        .await
        .expect("process");
    let (events, _) = broadcast::channel(8);
    let state = Arc::new(AppState {
        processor,
        db: Mutex::new(db),
        events,
        started: std::time::Instant::now(),
        coordinator: None,
        relay: None,
        campfire_filter: zeek_engine::campfire::Filter::default(),
        proof_limiter: zeek_engine::relay::RateLimiter::new(10),
        keyholder_ids: vec![1, 2, 3],
        threshold: 2,
        share_cache: zeek_engine::share::ShareCache::default(),
        app_url: "http://app".to_owned(),
        public_url: "http://engine".to_owned(),
    });
    let app = router(state, &["http://localhost:5173".to_owned()]);

    async fn get(app: &axum::Router, path: &str) -> (StatusCode, Vec<u8>) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes()
            .to_vec();
        (status, body)
    }

    let (status, body) = get(&app, "/v1/health").await;
    assert_eq!(status, StatusCode::OK);
    let health: HealthResponse = serde_json::from_slice(&body).expect("json");
    assert_eq!(health.settled_rounds, 1);

    let (status, body) = get(&app, "/v1/rounds/0").await;
    assert_eq!(status, StatusCode::OK);
    let round: RoundResponse = serde_json::from_slice(&body).expect("json");
    assert_eq!(round.status, RoundStatus::Settled);
    assert!(round.settled.is_some());

    let (status, body) = get(&app, "/v1/rounds/0/proof").await;
    assert_eq!(status, StatusCode::OK);
    let bundle: ProofBundle = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        proof::recompute(&bundle).expect("recompute"),
        bundle.settlement
    );
    for player in 1u8..=4 {
        assert!(!String::from_utf8_lossy(&body).contains(&hex::encode([player; 43])));
    }

    let (status, _) = get(&app, "/v1/rounds/1/proof").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = get(&app, "/v1/rounds/pending").await;
    assert_eq!(status, StatusCode::OK);
    let pending: PendingResponse = serde_json::from_slice(&body).expect("json");
    assert!(pending.rounds.is_empty());

    let (status, _) = get(&app, "/v1/board").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = get(&app, "/v1/config").await;
    assert_eq!(status, StatusCode::OK);

    // Phase 7a: past rounds, memo digests, keyholder status, share previews.
    let (status, body) = get(&app, "/v1/rounds").await;
    assert_eq!(status, StatusCode::OK);
    let rounds: zeek_engine::api::RoundsResponse = serde_json::from_slice(&body).expect("json");
    assert_eq!(rounds.rounds.len(), 1, "round 0 had digs");
    assert_eq!(rounds.rounds[0].round, 0);
    assert!(["paying", "no_winner", "paid"].contains(&rounds.rounds[0].status.as_str()));
    for player in 1u8..=4 {
        assert!(!String::from_utf8_lossy(&body).contains(&hex::encode([player; 43])));
    }
    let (status, body) = get(&app, "/v1/rounds/0/digests").await;
    assert_eq!(status, StatusCode::OK);
    let digests: zeek_engine::api::DigestsResponse = serde_json::from_slice(&body).expect("json");
    assert!(!digests.notes.is_empty());
    assert!(digests.notes.iter().all(|n| n.memo_sha256.len() == 64));
    let (status, body) = get(&app, "/v1/keyholders/status").await;
    assert_eq!(status, StatusCode::OK);
    let keys: zeek_engine::api::KeyholdersResponse = serde_json::from_slice(&body).expect("json");
    assert_eq!(keys.threshold, 2);
    assert_eq!(keys.keyholders.len(), 3);
    assert_eq!(keys.keyholders[0].label, "Keyholder A");
    assert_eq!(
        keys.keyholders[0].reachable, None,
        "never probed in this test"
    );
    let (status, body) = get(&app, "/share/round/0").await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains(r#"<meta property="og:image" content="http://engine/share/round/0.png">"#)
    );
    assert!(html.contains(r#"<meta name="twitter:card" content="summary_large_image">"#));
    assert!(html.contains("http://app/rounds/0"));
    let (status, png) = get(&app, "/share/round/0.png").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(&png[1..4], b"PNG");
    let (status, _) = get(&app, "/share/round/1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get(&app, &format!("/share/win/{}", "0".repeat(32))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Rules v3: from the activation height a dig may carry the `zeek1:` text
/// memo. The same chain settles differently before and after activation, the
/// board reports the prize pool next to the gross total, and the version 3
/// proof bundle recomputes.
#[tokio::test]
async fn text_memo_digs_count_from_the_activation_height() {
    let vault = TestVault::new(77);
    let base = vault.config(GENESIS);
    let mut chain = SyntheticChain::new(&vault, GENESIS, start_time(&base.params));
    chain.push_block(vec![]); // GENESIS
    let text_memo = |tile: u8, player: u8| {
        let text = vault.dig_memo(0, tile, player).encode_text();
        let mut field = [0u8; 512];
        field[..text.len()].copy_from_slice(text.as_bytes());
        field
    };
    // GENESIS + 1: a text dig before activation; GENESIS + 2: one after it,
    // next to a binary dig.
    chain.push_block(vec![chain.payment(
        300_000,
        Pool::Ironwood,
        text_memo(4, 1),
    )]);
    chain.push_block(vec![
        chain.payment(200_000, Pool::Ironwood, text_memo(5, 2)),
        chain.payment(100_000, Pool::Ironwood, vault.dig_memo(0, 6, 3).encode()),
    ]);
    while chain.next_height() < GENESIS + base.params.blocks_per_round {
        chain.advance(1);
    }
    chain.advance(base.params.mtp_window + SETTLE_CONFIRMATIONS);
    let beacon_time = start_time(&base.params) + 600 + 900;

    let mut v3 = base.clone();
    v3.params.text_memo_activation_height = Some(GENESIS + 2);
    v3.validate().expect("valid config");
    for (config, expected_version, valid_digs, forfeited) in
        [(base, 2u32, 1usize, 500_000u64), (v3, 3, 2, 300_000)]
    {
        let scanner = DigScanner::new(config.incoming_viewing_key().expect("ivk"));
        let mut db = Db::open_in_memory().expect("db");
        let mut processor = Processor::new(config.clone());
        ingest(&mut chain, &mut db, &scanner, &config)
            .await
            .expect("ingest");
        processor
            .process(&mut db, &mut fixture_beacons(), beacon_time + 5)
            .await
            .expect("process");
        let row = db.settlement(0).expect("row").expect("settled");
        assert_eq!(row.rules_version, expected_version);
        assert_eq!(row.settlement.forfeited, forfeited);
        let gross: u64 = if valid_digs == 2 { 300_000 } else { 100_000 };
        assert_eq!(row.settlement.pot, gross - valid_digs as u64 * 5_000);
        let view = round_response(&processor, &db, 0).expect("view");
        assert_eq!(view.digs, valid_digs);
        let bundle = proof_response(&processor, &db, 0)
            .expect("proof")
            .expect("settled");
        assert_eq!(bundle.version, proof::PROOF_VERSION);
        assert_eq!(bundle.rules_version, Some(expected_version));
        assert_eq!(bundle.claimed_rules_version(), Some(expected_version));
        assert_eq!(
            bundle.params.text_memo_activation_height.is_some(),
            expected_version == 3
        );
        assert_eq!(
            proof::recompute(&bundle).expect("recompute"),
            bundle.settlement
        );
    }
}

/// The board labels the prize pool (net of dig fees and rake) and the gross
/// total separately while a round is open.
#[tokio::test]
async fn board_reports_prize_pool_and_gross_total() {
    let vault = TestVault::new(78);
    let config = vault.config(GENESIS);
    let mut chain = SyntheticChain::new(&vault, GENESIS, start_time(&config.params));
    chain.push_block(vec![
        chain.payment(100_000, Pool::Ironwood, vault.dig_memo(0, 1, 1).encode()),
        chain.payment(100_000, Pool::Ironwood, vault.dig_memo(0, 2, 2).encode()),
    ]);
    chain.advance(2);
    let scanner = DigScanner::new(config.incoming_viewing_key().expect("ivk"));
    let mut db = Db::open_in_memory().expect("db");
    let processor = Processor::new(config.clone());
    ingest(&mut chain, &mut db, &scanner, &config)
        .await
        .expect("ingest");
    let board = processor.board(&db).expect("board");
    assert_eq!(board.round, 0);
    assert_eq!(board.digs, 2);
    assert_eq!(board.gross_dug_zat, 200_000);
    assert_eq!(board.prize_pool_zat, 180_500, "(200,000 - 2 x 5,000) x 95%");
    assert_eq!(board.rules_version, 2);
    let json = serde_json::to_string(&board).expect("json");
    assert!(json.contains("\"prize_pool_zat\":180500"));
    assert!(json.contains("\"gross_dug_zat\":200000"));
}

/// The relay serves tip, length-prefixed compact blocks, transactions and
/// broadcasts over HTTP, caps the block range and the body size, and rate
/// limits.
#[tokio::test]
async fn relay_serves_chain_data_with_limits() {
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use prost::Message;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tower::ServiceExt;
    use zcash_client_backend::proto::compact_formats::CompactBlock;
    use zcash_client_backend::proto::service::{RawTransaction, TreeState};
    use zeek_engine::chain::ChainSource;
    use zeek_engine::relay::{
        self, RelayBackend, RelayError, RelayState, RelayStatsResponse, TipResponse,
        MAX_BLOCK_RANGE, MAX_TX_BYTES,
    };

    struct Synthetic {
        chain: Mutex<SyntheticChain>,
        sent: Mutex<Vec<Vec<u8>>>,
    }

    #[async_trait]
    impl RelayBackend for Synthetic {
        async fn tip(&self) -> Result<(u32, String), RelayError> {
            let mut chain = self.chain.lock().await;
            let height = chain
                .latest_height()
                .await
                .map_err(|e| RelayError::Upstream(e.to_string()))?;
            let hash = chain
                .block(height)
                .map(|b| b.hash.clone())
                .unwrap_or_default();
            Ok((height, hex::encode(hash)))
        }
        async fn blocks(&self, start: u32, end: u32) -> Result<Vec<CompactBlock>, RelayError> {
            self.chain
                .lock()
                .await
                .block_range(start, end)
                .await
                .map_err(|e| RelayError::Upstream(e.to_string()))
        }
        async fn tree_state(&self, height: u32) -> Result<TreeState, RelayError> {
            Ok(TreeState {
                height: u64::from(height),
                ..TreeState::default()
            })
        }
        async fn broadcast(&self, raw: Vec<u8>) -> Result<(), RelayError> {
            if raw == b"bad" {
                return Err(RelayError::Rejected("bad-txns".into()));
            }
            self.sent.lock().await.push(raw);
            Ok(())
        }
        async fn transaction(&self, txid: [u8; 32]) -> Result<RawTransaction, RelayError> {
            self.chain
                .lock()
                .await
                .transaction(&txid[..])
                .map(|data| RawTransaction { data, height: 7 })
                .ok_or(RelayError::NotFound)
        }
    }

    let vault = TestVault::new(5);
    let mut chain = SyntheticChain::new(&vault, 100, 1_700_000_000);
    chain.advance(3);
    chain.push_block(vec![chain.payment(100_000, Pool::Ironwood, [0xF6; 512])]); // 103
    chain.advance(2);
    let txid_internal: [u8; 32] = chain.block(103).expect("block").vtx[0]
        .txid
        .clone()
        .try_into()
        .expect("32 bytes");
    let mut display = txid_internal;
    display.reverse();
    let backend = Arc::new(Synthetic {
        chain: Mutex::new(chain),
        sent: Mutex::new(Vec::new()),
    });
    struct Shared(Arc<Synthetic>);
    #[async_trait]
    impl RelayBackend for Shared {
        async fn tip(&self) -> Result<(u32, String), RelayError> {
            self.0.tip().await
        }
        async fn blocks(&self, s: u32, e: u32) -> Result<Vec<CompactBlock>, RelayError> {
            self.0.blocks(s, e).await
        }
        async fn tree_state(&self, h: u32) -> Result<TreeState, RelayError> {
            self.0.tree_state(h).await
        }
        async fn broadcast(&self, raw: Vec<u8>) -> Result<(), RelayError> {
            self.0.broadcast(raw).await
        }
        async fn transaction(&self, t: [u8; 32]) -> Result<RawTransaction, RelayError> {
            self.0.transaction(t).await
        }
    }
    let app = relay::router(Arc::new(RelayState::new(
        Box::new(Shared(backend.clone())),
        12,
    )));

    async fn call(app: &axum::Router, request: Request<Body>) -> (StatusCode, Vec<u8>) {
        let response = app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes()
            .to_vec();
        (status, body)
    }
    let get = |path: &str| {
        Request::builder()
            .uri(path)
            .body(Body::empty())
            .expect("request")
    };
    let post = |body: Vec<u8>| {
        Request::builder()
            .method("POST")
            .uri("/v1/tx/broadcast")
            .body(Body::from(body))
            .expect("request")
    };

    let (status, body) = call(&app, get("/v1/chain/tip")).await;
    assert_eq!(status, StatusCode::OK);
    let tip: TipResponse = serde_json::from_slice(&body).expect("json");
    assert_eq!(tip.height, 105);

    let (status, body) = call(&app, get("/v1/chain/blocks?start=102&end=104")).await;
    assert_eq!(status, StatusCode::OK);
    let mut rest = body.as_slice();
    let mut heights = Vec::new();
    let mut outputs = 0;
    while !rest.is_empty() {
        let len = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        let block = CompactBlock::decode(&rest[4..4 + len]).expect("decode");
        heights.push(block.height);
        outputs += block
            .vtx
            .iter()
            .map(|t| t.ironwood_actions.len())
            .sum::<usize>();
        rest = &rest[4 + len..];
    }
    assert_eq!(heights, vec![102, 103, 104]);
    assert!(
        outputs >= 1,
        "all compact outputs are relayed, not a filtered view"
    );

    let too_many = format!("/v1/chain/blocks?start=0&end={MAX_BLOCK_RANGE}");
    assert_eq!(call(&app, get(&too_many)).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        call(&app, get("/v1/chain/blocks?start=5&end=4")).await.0,
        StatusCode::BAD_REQUEST
    );

    let (status, body) = call(&app, get("/v1/chain/tree-state?height=103")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        TreeState::decode(body.as_slice()).expect("decode").height,
        103
    );

    let (status, body) = call(&app, get(&format!("/v1/tx/{}", hex::encode(display)))).await;
    assert_eq!(status, StatusCode::OK);
    let raw = RawTransaction::decode(body.as_slice()).expect("decode");
    assert_eq!(raw.height, 7);
    assert!(!raw.data.is_empty());
    assert_eq!(
        call(&app, get(&format!("/v1/tx/{}", "00".repeat(32))))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, get("/v1/tx/nothex")).await.0,
        StatusCode::BAD_REQUEST
    );

    assert_eq!(call(&app, post(vec![1, 2, 3])).await.0, StatusCode::OK);
    assert_eq!(backend.sent.lock().await.as_slice(), &[vec![1, 2, 3]]);
    assert_eq!(
        call(&app, post(b"bad".to_vec())).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call(&app, post(vec![0; MAX_TX_BYTES + 1])).await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );

    // 12 requests per minute: the bucket is empty by now.
    let mut limited = 0;
    for _ in 0..6 {
        if call(&app, get("/v1/chain/tip")).await.0 == StatusCode::TOO_MANY_REQUESTS {
            limited += 1;
        }
    }
    assert!(limited >= 4, "rate limit engaged ({limited})");
    let stats_app = relay::router(Arc::new(RelayState::new(Box::new(Shared(backend)), 600)));
    let (_, body) = call(&stats_app, get("/v1/relay/stats")).await;
    let stats: RelayStatsResponse = serde_json::from_slice(&body).expect("json");
    assert_eq!(stats.rate_per_minute, 600);
    assert_eq!(stats.max_block_range, MAX_BLOCK_RANGE);
}

/// Rules v4 end to end: a dig with a nicknamed message, a shout and a
/// filtered message; the shout reaches the Cache minus its reserved dig fee,
/// the format 4 bundle recomputes, and `/v1/campfire` serves the messages
/// newest first, hiding the filtered one unless asked.
#[tokio::test]
async fn campfire_messages_and_shouts() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use tokio::sync::{broadcast, Mutex};
    use tower::ServiceExt;
    use zeek_engine::api::{router, AppState, CampfireResponse};

    let vault = TestVault::new(79);
    let mut config = vault.config(GENESIS);
    config.params.text_memo_activation_height = Some(GENESIS);
    config.params.campfire = Some(zeek_core::params::CampfireParams {
        activation_height: GENESIS,
        min_shout_zat: zeek_core::params::MIN_SHOUT_ZAT,
    });
    config.validate().expect("valid v4 config");
    let mut chain = SyntheticChain::new(&vault, GENESIS, start_time(&config.params));
    let dig = vault
        .dig_memo(0, 4, 1)
        .with_message("[ember] warm hands")
        .expect("message");
    let shout = zeek_core::memo::ShoutMemo::new("the night is long").expect("shout");
    let spam = zeek_core::memo::ShoutMemo::new("free giveaway, dm me").expect("shout");
    let small = zeek_core::memo::ShoutMemo::new("too small to count").expect("shout");
    chain.push_block(vec![chain.payment(100_000, Pool::Ironwood, dig.encode())]);
    chain.push_block(vec![chain.payment(60_000, Pool::Ironwood, shout.encode())]);
    chain.push_block(vec![
        chain.payment(50_000, Pool::Ironwood, spam.encode()),
        chain.payment(10_000, Pool::Ironwood, small.encode()),
    ]);
    while chain.next_height() < GENESIS + config.params.blocks_per_round {
        chain.advance(1);
    }
    chain.advance(config.params.mtp_window + SETTLE_CONFIRMATIONS);
    let scanner = DigScanner::new(config.incoming_viewing_key().expect("ivk"));
    let mut db = Db::open_in_memory().expect("db");
    let mut processor = Processor::new(config.clone());
    ingest(&mut chain, &mut db, &scanner, &config)
        .await
        .expect("ingest");
    let beacon_time = start_time(&config.params) + 600 + 900;
    processor
        .process(&mut db, &mut fixture_beacons(), beacon_time + 5)
        .await
        .expect("process");
    let row = db.settlement(0).expect("row").expect("settled");
    assert_eq!(row.rules_version, 4);
    assert_eq!(row.settlement.shouts, 110_000);
    assert_eq!(row.settlement.shout_fees, 10_000);
    assert_eq!(row.settlement.forfeited, 10_000, "the undersized shout");
    let bundle = proof_response(&processor, &db, 0)
        .expect("proof")
        .expect("settled");
    assert_eq!(bundle.rules_version, Some(4));
    assert!(bundle.params.campfire.is_some());
    assert_eq!(
        proof::recompute(&bundle).expect("recompute"),
        bundle.settlement
    );

    let (events, _) = broadcast::channel(8);
    let filter: zeek_engine::campfire::Filter = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/campfire-filter.json"
        ))
        .expect("filter file"),
    )
    .expect("filter");
    let app = router(
        Arc::new(AppState {
            processor,
            db: Mutex::new(db),
            events,
            started: std::time::Instant::now(),
            coordinator: None,
            relay: None,
            campfire_filter: filter,
            proof_limiter: zeek_engine::relay::RateLimiter::new(10),
            keyholder_ids: vec![],
            threshold: 0,
            share_cache: zeek_engine::share::ShareCache::default(),
            app_url: "http://app".to_owned(),
            public_url: "http://engine".to_owned(),
        }),
        &[],
    );
    let get = |path: &'static str| {
        let app = app.clone();
        async move {
            let response = app
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK);
            let body = response
                .into_body()
                .collect()
                .await
                .expect("body")
                .to_bytes();
            serde_json::from_slice::<CampfireResponse>(&body).expect("json")
        }
    };
    let feed = get("/v1/campfire").await;
    let summary: Vec<(String, Option<String>, bool)> = feed
        .messages
        .iter()
        .map(|m| (m.kind.clone(), m.text.clone(), m.hidden))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("shout".to_owned(), None, true),
            (
                "shout".to_owned(),
                Some("the night is long".to_owned()),
                false
            ),
            ("dig".to_owned(), Some("warm hands".to_owned()), false),
        ],
        "newest first; the undersized shout is not shown"
    );
    assert_eq!(
        feed.messages[0].reason.as_deref(),
        Some("contains a filtered word (giveaway)")
    );
    assert_eq!(feed.messages[2].nickname.as_deref(), Some("ember"));
    assert_eq!(feed.messages[2].tiles, vec![4]);
    let with_hidden = get("/v1/campfire?include_hidden=1").await;
    assert_eq!(
        with_hidden.messages[0].text.as_deref(),
        Some("free giveaway, dm me")
    );
    let page = get("/v1/campfire?limit=1").await;
    assert_eq!(page.messages.len(), 1);
    assert!(page.next_before.is_some());
}
