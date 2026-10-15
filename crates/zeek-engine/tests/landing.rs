//! Landing digs over HTTP, and a live check of `GetMempoolStream`.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tokio::sync::broadcast;
use tower::ServiceExt;
use zeek_engine::api::{router, AppState, BoardResponse};
use zeek_engine::chain::ingest;
use zeek_engine::db::Pool;
use zeek_engine::decrypt::DigScanner;
use zeek_engine::mempool::{run_session, Landing, MempoolSource, MEMPOOL_STALL};
use zeek_engine::processor::{BoardState, Processor};
use zeek_engine::store::Store;
use zeek_engine::synthetic::{SyntheticChain, TestVault};
use zeek_engine::EngineError;

const GENESIS: u32 = 2_000;

/// One session of raw transactions.
struct OneSession {
    tip: u32,
    txs: VecDeque<Vec<u8>>,
}

#[async_trait]
impl MempoolSource for OneSession {
    async fn latest_height(&mut self) -> Result<u32, EngineError> {
        Ok(self.tip)
    }
    async fn open(&mut self) -> Result<(), EngineError> {
        Ok(())
    }
    async fn next(&mut self) -> Result<Option<Vec<u8>>, EngineError> {
        Ok(self.txs.pop_front())
    }
}

async fn get_board(app: &axum::Router) -> (serde_json::Value, BoardResponse) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/board")
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
    // Clients that only know the board keep parsing it.
    let _: BoardState = serde_json::from_slice(&body).expect("board fields unchanged");
    (
        serde_json::from_slice(&body).expect("json"),
        serde_json::from_slice(&body).expect("board response"),
    )
}

#[tokio::test]
async fn board_serves_landing_digs_separately() {
    let vault = TestVault::new(31);
    let config = vault.config(GENESIS);
    let scanner = DigScanner::new(config.incoming_viewing_key().expect("ivk"));
    let mut chain = SyntheticChain::new(&vault, GENESIS, 1_700_000_000);
    chain.advance(3);
    let store = Store::open_in_memory().expect("db");
    ingest(&mut chain, &store, &scanner, &config)
        .await
        .expect("ingest");
    let (events, mut rx) = broadcast::channel(16);
    let landing = Arc::new(Landing::new(events.clone()));
    let state = Arc::new(AppState {
        processor: Processor::new(config.clone()),
        store: Arc::new(store),
        events,
        started: std::time::Instant::now(),
        ledger: None,
        chain_servers: None,
        stream_slots: std::sync::Arc::new(tokio::sync::Semaphore::new(
            zeek_engine::api::MAX_STREAMS,
        )),
        relay: None,
        campfire_filter: zeek_engine::campfire::Filter::default(),
        proof_limiter: zeek_engine::relay::RateLimiter::new(10),
        keyholder_ids: vec![],
        threshold: 0,
        share_cache: zeek_engine::share::ShareCache::default(),
        app_url: "http://app".to_owned(),
        public_url: "http://engine".to_owned(),
        landing: landing.clone(),
    });
    let app = router(state, &[]);

    let (json, board) = get_board(&app).await;
    assert_eq!(json["landing"], serde_json::json!([]));
    assert!(board.landing.is_empty());
    assert_eq!(json["tip_height"], GENESIS + 2);

    let raw = chain.unmined_transaction(&chain.payment(
        350_000,
        Pool::Ironwood,
        vault.dig_memo(0, 9, 1).encode(),
    ));
    let mut source = OneSession {
        tip: GENESIS + 2,
        txs: VecDeque::from([raw]),
    };
    run_session(
        &mut source,
        &scanner,
        &config.params,
        &landing,
        MEMPOOL_STALL,
    )
    .await
    .expect("session");

    let (json, board) = get_board(&app).await;
    assert_eq!(board.landing.len(), 1);
    assert_eq!(board.landing[0].round, 0);
    assert_eq!(board.landing[0].tile, 9);
    assert_eq!(board.landing[0].value_zat, 350_000);
    assert_eq!(board.board.digs, 0, "landing digs are not board digs");
    assert_eq!(board.board.prize_pool_zat, 0);
    assert_eq!(json["landing"][0]["tile"], 9);

    // The WebSocket event carries the same set.
    let mut seen = None;
    while let Ok(event) = rx.try_recv() {
        let value = serde_json::to_value(&event).expect("json");
        if value["event"] == "landing" {
            seen = Some(value);
        }
    }
    let seen = seen.expect("landing event");
    assert_eq!(seen["digs"], json["landing"]);
}

/// Live: opens `GetMempoolStream` on the public testnet server for about a
/// minute and reports what it saw. Run with
/// `cargo test -p zeek-engine --test landing -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "needs the network"]
async fn live_mempool_stream() {
    use zcash_client_backend::proto::service::Empty;
    let url = std::env::var("ZEEK_LIGHTWALLETD_URL")
        .unwrap_or_else(|_| "https://testnet.zec.rocks:443".to_owned());
    let mut client = zeek_vault::lightwalletd::connect(&url)
        .await
        .expect("connect");
    let tip = zeek_vault::lightwalletd::latest_height(&mut client)
        .await
        .expect("tip");
    println!("{url}: tip {}", u32::from(tip));
    let mut stream = match client.get_mempool_stream(Empty {}).await {
        Ok(response) => response.into_inner(),
        Err(status) => {
            println!(
                "GetMempoolStream NOT supported: {:?} {}",
                status.code(),
                status.message()
            );
            return;
        }
    };
    println!("GetMempoolStream supported");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let (mut txs, mut sessions) = (0usize, 1usize);
    loop {
        match tokio::time::timeout_at(deadline, stream.message()).await {
            Err(_) => break,
            Ok(Ok(Some(tx))) => {
                txs += 1;
                println!("tx: {} bytes, height {}", tx.data.len(), tx.height);
            }
            Ok(Ok(None)) => {
                println!("stream closed (a block was mined); reopening");
                sessions += 1;
                stream = client
                    .get_mempool_stream(Empty {})
                    .await
                    .expect("reopen")
                    .into_inner();
            }
            Ok(Err(status)) => {
                println!("stream error: {status}");
                break;
            }
        }
    }
    println!("saw {txs} mempool transaction(s) in {sessions} session(s) over 60 s");
}
