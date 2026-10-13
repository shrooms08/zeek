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
    assert_eq!(s.pot, 1_200_000);
    assert_eq!(s.rake, 60_000);
    assert_eq!(s.forfeited, 200_000, "dust dig and text memo");
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
    assert_eq!(refunds[0].net, 200_000 - 5_000);
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
}
