//! The API never waits for the chain loop.
//!
//! While one engine pass is stuck on a stalled lightwalletd (blocks and
//! transactions), a stalled drand relay and a slow proving step, the API's
//! reads keep answering in well under 50 ms. The pass runs through the same
//! code as the engine binary: `chain::ingest`, `Processor::process`, and the
//! coordinator's payout shape (read the settlement, record `signing`, prove
//! on the blocking pool, record the broadcast). The proving step here is a
//! stand-in that burns CPU for the same kind of wall time as a real Orchard
//! proof; the real one needs a funded vault wallet and keyholders.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tokio::sync::broadcast;
use tower::ServiceExt;
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zeek_core::params::DrandChain;
use zeek_engine::api::{router, AppState};
use zeek_engine::beacon::{BeaconFetcher, FetchedBeacon, FixedBeacons};
use zeek_engine::chain::{ingest, ChainSource};
use zeek_engine::db::{PayoutRecord, PayoutState, PlanStatus};
use zeek_engine::decrypt::DigScanner;
use zeek_engine::processor::Processor;
use zeek_engine::store::Store;
use zeek_engine::synthetic::SyntheticChain;
use zeek_engine::EngineError;

use common::{fixture_beacons, scenario, start_time, Scenario, GENESIS};

/// How long each stalled call hangs.
const STALL: Duration = Duration::from_millis(1_500);
/// The bar every read must clear.
const READ_BUDGET: Duration = Duration::from_millis(50);

/// A lightwalletd that answers, but only after `STALL`.
struct StalledChain(SyntheticChain);

#[async_trait]
impl ChainSource for StalledChain {
    async fn latest_height(&mut self) -> Result<u32, EngineError> {
        ChainSource::latest_height(&mut self.0).await
    }

    async fn block_range(
        &mut self,
        start: u32,
        end: u32,
    ) -> Result<Vec<CompactBlock>, EngineError> {
        tokio::time::sleep(STALL).await;
        ChainSource::block_range(&mut self.0, start, end).await
    }

    async fn transaction(&mut self, txid: &[u8; 32]) -> Result<Vec<u8>, EngineError> {
        tokio::time::sleep(STALL).await;
        ChainSource::transaction(&mut self.0, txid).await
    }

    async fn transaction_height(&mut self, txid: &[u8; 32]) -> Result<u32, EngineError> {
        ChainSource::transaction_height(&mut self.0, txid).await
    }
}

/// The chain as it stood at `tip`, without delays.
struct Earlier<'a> {
    chain: &'a mut SyntheticChain,
    tip: u32,
}

#[async_trait]
impl ChainSource for Earlier<'_> {
    async fn latest_height(&mut self) -> Result<u32, EngineError> {
        Ok(self.tip)
    }

    async fn block_range(
        &mut self,
        start: u32,
        end: u32,
    ) -> Result<Vec<CompactBlock>, EngineError> {
        ChainSource::block_range(self.chain, start, end.min(self.tip)).await
    }

    async fn transaction(&mut self, txid: &[u8; 32]) -> Result<Vec<u8>, EngineError> {
        ChainSource::transaction(self.chain, txid).await
    }

    async fn transaction_height(&mut self, txid: &[u8; 32]) -> Result<u32, EngineError> {
        ChainSource::transaction_height(self.chain, txid).await
    }
}

/// A drand relay that answers after `STALL`.
struct StalledBeacons(FixedBeacons);

#[async_trait]
impl BeaconFetcher for StalledBeacons {
    async fn fetch(
        &mut self,
        chain: &DrandChain,
        round: u64,
    ) -> Result<FetchedBeacon, EngineError> {
        tokio::time::sleep(STALL).await;
        self.0.fetch(chain, round).await
    }
}

/// CPU-bound work standing in for `send::prove`.
fn slow_proof() -> u64 {
    let started = Instant::now();
    let mut x = 0u64;
    while started.elapsed() < STALL {
        for i in 0..10_000u64 {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(i);
        }
    }
    x
}

/// One engine pass: ingest, settle, pay (the coordinator's write pattern).
async fn engine_pass(
    store: Arc<Store>,
    mut source: StalledChain,
    config: zeek_engine::config::GameConfig,
    now: u64,
) -> Result<(), EngineError> {
    let scanner = DigScanner::new(config.incoming_viewing_key()?);
    ingest(&mut source, &store, &scanner, &config).await?;
    let mut processor = Processor::new(config);
    let mut beacons = StalledBeacons(fixture_beacons());
    processor.process(&store, &mut beacons, now).await?;

    let row = store
        .read(|db| db.settlement(0))?
        .ok_or_else(|| EngineError::Corrupt("round 0 did not settle".into()))?;
    store.write(|db| {
        db.insert_payout(&PayoutRecord {
            round: row.round,
            state: PayoutState::Signing,
            session: "00".repeat(32),
            txid: None,
            height: row.settled_at_height,
            fee: 10_000,
            signers: vec![],
        })
    })?;
    tokio::task::spawn_blocking(slow_proof)
        .await
        .map_err(|e| EngineError::Server(e.to_string()))?;
    store.write(|db| {
        db.transaction(|tx| {
            tx.mark_payout_broadcast(row.round, &"ab".repeat(32), row.settled_at_height, &[1, 2])?;
            tx.set_plan_status(row.round, PlanStatus::Paid)
        })
    })
}

async fn timed_get(app: &axum::Router, path: &str) -> Duration {
    let started = Instant::now();
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
    let _ = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let elapsed = started.elapsed();
    assert!(
        status == StatusCode::OK || status == StatusCode::NOT_FOUND,
        "{path}: {status}"
    );
    elapsed
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn api_reads_stay_fast_while_the_pass_is_stalled() {
    let dir = tempfile::tempdir().expect("dir");
    let store = Arc::new(Store::open(&dir.path().join("engine.sqlite")).expect("store"));
    let Scenario { vault, mut chain } = scenario(0);
    let config = vault.config(GENESIS);

    // Most of the chain is already stored; the stalled pass fetches the
    // round's last blocks (and its dig transactions) through the stall.
    let scanner = DigScanner::new(config.incoming_viewing_key().expect("ivk"));
    let mut warm = Earlier {
        chain: &mut chain,
        tip: GENESIS + 6,
    };
    ingest(&mut warm, &store, &scanner, &config)
        .await
        .expect("warm ingest");

    let (events, _) = broadcast::channel(8);
    let state = Arc::new(AppState {
        processor: Processor::new(config.clone()),
        store: store.clone(),
        events,
        started: Instant::now(),
        ledger: None,
        chain_servers: None,
        stream_slots: std::sync::Arc::new(tokio::sync::Semaphore::new(
            zeek_engine::api::MAX_STREAMS,
        )),
        relay: None,
        campfire_filter: zeek_engine::campfire::Filter::default(),
        proof_limiter: zeek_engine::relay::RateLimiter::new(10),
        keyholder_ids: vec![1, 2, 3],
        threshold: 2,
        share_cache: zeek_engine::share::ShareCache::default(),
        app_url: "http://app".to_owned(),
        public_url: "http://engine".to_owned(),
        landing: Default::default(),
    });
    let app = router(state, &[]);

    let beacon_time = start_time(&config.params) + 600 + 900;
    let pass = tokio::spawn(engine_pass(
        store.clone(),
        StalledChain(chain),
        config,
        beacon_time + 5,
    ));

    let paths = [
        "/v1/board",
        "/v1/rounds/0",
        "/v1/rounds/0/proof",
        "/v1/pending",
        "/v1/health",
        "/v1/rounds?include_empty=1",
        "/v1/campfire",
        "/v1/keyholders/status",
    ];
    let started = Instant::now();
    let mut samples = Vec::new();
    while !pass.is_finished() {
        for path in paths {
            samples.push(timed_get(&app, path).await);
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let wall = started.elapsed();
    pass.await.expect("join").expect("pass");

    // The pass did the full ingest → settle → pay cycle through the stalls.
    assert!(wall >= STALL * 3, "the pass was stalled ({wall:?})");
    let payout = store
        .read(|db| db.payout(0))
        .expect("read")
        .expect("payout recorded");
    assert_eq!(payout.state, PayoutState::Broadcast);

    samples.sort();
    let p95 = samples[samples.len() * 95 / 100];
    let max = samples[samples.len() - 1];
    println!(
        "{} reads over {wall:?} of stalled pass: p50 {:?}, p95 {p95:?}, max {max:?}",
        samples.len(),
        samples[samples.len() / 2]
    );
    assert!(samples.len() > 100, "the API was exercised throughout");
    assert!(max < READ_BUDGET, "slowest read {max:?} (p95 {p95:?})");
}
