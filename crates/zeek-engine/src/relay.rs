//! Chain relay for browser wallets.
//!
//! Browsers cannot speak gRPC to a lightwalletd, so the engine relays the
//! five calls a light wallet needs over plain HTTP:
//!
//! | Route | lightwalletd call | Response |
//! |---|---|---|
//! | `GET /v1/chain/tip` | `GetLatestBlock` | JSON `{ height, hash }` |
//! | `GET /v1/chain/blocks?start=&end=` | `GetBlockRange` | binary: per block a `u32` little-endian length, then the `CompactBlock` protobuf |
//! | `GET /v1/chain/tree-state?height=` | `GetTreeState` | `TreeState` protobuf |
//! | `POST /v1/tx/broadcast` | `SendTransaction` | body: raw transaction bytes; JSON `{ ok }` or an error |
//! | `GET /v1/tx/{txid}` | `GetTransaction` | `RawTransaction` protobuf (bytes and mined height) |
//!
//! # Privacy
//!
//! A wallet downloads **every** compact block of the range it scans and
//! trial-decrypts locally, so the relay never learns which outputs are the
//! wallet's. Reading a memo needs the full transaction, and asking for one
//! does reveal interest in that txid; wallets hide the wanted txid among
//! random decoy txids from the same block range, requested in shuffled order.
//! The relay still sees the client's network address and request timing, as
//! any lightwalletd would.
//!
//! Nothing here logs request bodies, txids, heights or client addresses. The
//! only state kept per client is an in-memory rate-limit bucket keyed by IP,
//! dropped when idle; `GET /v1/relay/stats` exposes aggregate counters only.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use prost::Message;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_backend::proto::service::{
    BlockId, BlockRange, ChainSpec, RawTransaction, TreeState, TxFilter,
};
use zeek_vault::lightwalletd::Client;

use crate::api::ErrorBody;

/// Most blocks one `/v1/chain/blocks` request may ask for.
pub const MAX_BLOCK_RANGE: u32 = 500;
/// Largest transaction the relay will broadcast (bytes). A 25-tile dig is
/// about 85 kB.
pub const MAX_TX_BYTES: usize = 256 * 1024;
/// Default requests per minute per client address.
pub const DEFAULT_RATE_PER_MIN: u32 = 600;
/// Environment variable overriding [`DEFAULT_RATE_PER_MIN`].
pub const RATE_VAR: &str = "ZEEK_RELAY_RATE_PER_MIN";

/// Why a relay call failed.
#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error("{0}")]
    BadRequest(String),
    #[error("not found")]
    NotFound,
    #[error("the node rejected the transaction: {0}")]
    Rejected(String),
    #[error("upstream lightwalletd error: {0}")]
    Upstream(String),
}

/// The chain backend the relay forwards to.
#[async_trait]
pub trait RelayBackend: Send + Sync {
    /// Height and hash (display order hex) of the chain tip.
    async fn tip(&self) -> Result<(u32, String), RelayError>;
    /// Compact blocks `start..=end`.
    async fn blocks(&self, start: u32, end: u32) -> Result<Vec<CompactBlock>, RelayError>;
    /// Tree state as of the end of block `height`.
    async fn tree_state(&self, height: u32) -> Result<TreeState, RelayError>;
    /// Broadcasts a raw transaction.
    async fn broadcast(&self, raw: Vec<u8>) -> Result<(), RelayError>;
    /// A transaction by txid (internal byte order).
    async fn transaction(&self, txid: [u8; 32]) -> Result<RawTransaction, RelayError>;
}

/// lightwalletd as the relay backend.
pub struct LightwalletdBackend {
    client: Client,
}

impl LightwalletdBackend {
    /// Wraps a connected client (cheap to clone per call).
    pub fn new(client: Client) -> Self {
        LightwalletdBackend { client }
    }
}

fn upstream(status: tonic::Status) -> RelayError {
    if status.code() == tonic::Code::NotFound {
        RelayError::NotFound
    } else {
        RelayError::Upstream(status.message().to_owned())
    }
}

#[async_trait]
impl RelayBackend for LightwalletdBackend {
    async fn tip(&self) -> Result<(u32, String), RelayError> {
        let block = self
            .client
            .clone()
            .get_latest_block(ChainSpec {})
            .await
            .map_err(upstream)?
            .into_inner();
        let height = u32::try_from(block.height)
            .map_err(|_| RelayError::Upstream("tip height overflows u32".into()))?;
        let mut hash = block.hash;
        hash.reverse();
        Ok((height, hex::encode(hash)))
    }

    async fn blocks(&self, start: u32, end: u32) -> Result<Vec<CompactBlock>, RelayError> {
        let id = |h: u32| BlockId {
            height: u64::from(h),
            hash: vec![],
        };
        let mut stream = self
            .client
            .clone()
            .get_block_range(BlockRange {
                start: Some(id(start)),
                end: Some(id(end)),
                pool_types: vec![],
            })
            .await
            .map_err(upstream)?
            .into_inner();
        let mut blocks = Vec::new();
        while let Some(block) = stream.message().await.map_err(upstream)? {
            blocks.push(block);
        }
        Ok(blocks)
    }

    async fn tree_state(&self, height: u32) -> Result<TreeState, RelayError> {
        Ok(self
            .client
            .clone()
            .get_tree_state(BlockId {
                height: u64::from(height),
                hash: vec![],
            })
            .await
            .map_err(upstream)?
            .into_inner())
    }

    async fn broadcast(&self, raw: Vec<u8>) -> Result<(), RelayError> {
        let response = self
            .client
            .clone()
            .send_transaction(RawTransaction {
                data: raw,
                height: 0,
            })
            .await
            .map_err(upstream)?
            .into_inner();
        if response.error_code != 0 {
            return Err(RelayError::Rejected(response.error_message));
        }
        Ok(())
    }

    async fn transaction(&self, txid: [u8; 32]) -> Result<RawTransaction, RelayError> {
        Ok(self
            .client
            .clone()
            .get_transaction(TxFilter {
                block: None,
                index: 0,
                hash: txid.to_vec(),
            })
            .await
            .map_err(upstream)?
            .into_inner())
    }
}

/// Aggregate counters; no per-client or per-request detail.
#[derive(Default)]
pub struct RelayStats {
    requests: AtomicU64,
    rate_limited: AtomicU64,
    blocks_served: AtomicU64,
    bytes_served: AtomicU64,
    transactions_served: AtomicU64,
    broadcasts_accepted: AtomicU64,
    broadcasts_rejected: AtomicU64,
    upstream_errors: AtomicU64,
}

/// `GET /v1/relay/stats`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayStatsResponse {
    pub requests: u64,
    pub rate_limited: u64,
    pub blocks_served: u64,
    pub bytes_served: u64,
    pub transactions_served: u64,
    pub broadcasts_accepted: u64,
    pub broadcasts_rejected: u64,
    pub upstream_errors: u64,
    pub rate_per_minute: u32,
    pub max_block_range: u32,
    pub max_tx_bytes: usize,
}

/// A token bucket: `capacity` requests, refilled continuously over a minute.
struct Bucket {
    tokens: f64,
    updated: Instant,
}

/// Per-address rate limiter. Addresses live only in this map, in memory.
pub struct RateLimiter {
    per_minute: u32,
    buckets: Mutex<HashMap<IpAddr, Bucket>>,
}

impl RateLimiter {
    /// A limiter allowing `per_minute` requests per address (bursting to the
    /// same number).
    pub fn new(per_minute: u32) -> Self {
        RateLimiter {
            per_minute: per_minute.max(1),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Takes one token for `address` at time `now`; `false` when exhausted.
    pub async fn allow(&self, address: IpAddr, now: Instant) -> bool {
        let capacity = f64::from(self.per_minute);
        let mut buckets = self.buckets.lock().await;
        // Forget addresses idle for over a minute (their buckets are full).
        if buckets.len() > 1_024 {
            buckets.retain(|_, b| now.duration_since(b.updated) < Duration::from_secs(60));
        }
        let bucket = buckets.entry(address).or_insert(Bucket {
            tokens: capacity,
            updated: now,
        });
        let elapsed = now.duration_since(bucket.updated).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * capacity / 60.0).min(capacity);
        bucket.updated = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Shared state of the relay routes.
pub struct RelayState {
    backend: Box<dyn RelayBackend>,
    limiter: RateLimiter,
    stats: RelayStats,
}

impl RelayState {
    /// A relay over `backend` allowing `rate_per_minute` requests per address.
    pub fn new(backend: Box<dyn RelayBackend>, rate_per_minute: u32) -> Self {
        RelayState {
            backend,
            limiter: RateLimiter::new(rate_per_minute),
            stats: RelayStats::default(),
        }
    }

    /// Reads the rate limit from the environment.
    pub fn rate_from_env() -> u32 {
        std::env::var(RATE_VAR)
            .ok()
            .and_then(|v| v.trim().parse::<u32>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(DEFAULT_RATE_PER_MIN)
    }
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorBody {
            error: message.into(),
        }),
    )
        .into_response()
}

impl RelayState {
    fn fail(&self, e: RelayError) -> Response {
        match e {
            RelayError::BadRequest(m) => error(StatusCode::BAD_REQUEST, m),
            RelayError::NotFound => error(StatusCode::NOT_FOUND, "not found"),
            RelayError::Rejected(m) => {
                self.stats
                    .broadcasts_rejected
                    .fetch_add(1, Ordering::Relaxed);
                error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("the node rejected the transaction: {m}"),
                )
            }
            RelayError::Upstream(m) => {
                self.stats.upstream_errors.fetch_add(1, Ordering::Relaxed);
                error(
                    StatusCode::BAD_GATEWAY,
                    format!("upstream lightwalletd error: {m}"),
                )
            }
        }
    }

    fn binary(&self, body: Vec<u8>) -> Response {
        self.stats
            .bytes_served
            .fetch_add(body.len() as u64, Ordering::Relaxed);
        (
            [
                (header::CONTENT_TYPE, "application/octet-stream"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            body,
        )
            .into_response()
    }
}

/// Rate limiting per client address. Requests without a known peer address
/// (in-process tests) share one bucket.
async fn rate_limit(
    State(state): State<Arc<RelayState>>,
    request: Request,
    next: Next,
) -> Response {
    let address = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip())
        .unwrap_or(IpAddr::from([0, 0, 0, 0]));
    state.stats.requests.fetch_add(1, Ordering::Relaxed);
    if !state.limiter.allow(address, Instant::now()).await {
        state.stats.rate_limited.fetch_add(1, Ordering::Relaxed);
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate limit exceeded; retry in a few seconds",
        );
    }
    next.run(request).await
}

/// `GET /v1/chain/tip`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TipResponse {
    pub height: u32,
    /// Block hash, display-order hex.
    pub hash: String,
}

async fn tip(State(state): State<Arc<RelayState>>) -> Response {
    match state.backend.tip().await {
        Ok((height, hash)) => Json(TipResponse { height, hash }).into_response(),
        Err(e) => state.fail(e),
    }
}

#[derive(Deserialize)]
struct BlocksQuery {
    start: u32,
    end: u32,
}

/// Encodes blocks as `u32` little-endian length prefixes followed by the
/// protobuf encoding of each block.
pub fn encode_blocks(blocks: &[CompactBlock]) -> Result<Vec<u8>, RelayError> {
    let mut out = Vec::new();
    for block in blocks {
        let len = u32::try_from(block.encoded_len())
            .map_err(|_| RelayError::Upstream("compact block larger than 4 GiB".into()))?;
        out.extend_from_slice(&len.to_le_bytes());
        block
            .encode(&mut out)
            .map_err(|e| RelayError::Upstream(e.to_string()))?;
    }
    Ok(out)
}

async fn blocks(State(state): State<Arc<RelayState>>, Query(q): Query<BlocksQuery>) -> Response {
    if q.end < q.start {
        return state.fail(RelayError::BadRequest("end is below start".into()));
    }
    if q.end - q.start >= MAX_BLOCK_RANGE {
        return state.fail(RelayError::BadRequest(format!(
            "at most {MAX_BLOCK_RANGE} blocks per request"
        )));
    }
    match state.backend.blocks(q.start, q.end).await {
        Ok(blocks) => match encode_blocks(&blocks) {
            Ok(body) => {
                state
                    .stats
                    .blocks_served
                    .fetch_add(blocks.len() as u64, Ordering::Relaxed);
                state.binary(body)
            }
            Err(e) => state.fail(e),
        },
        Err(e) => state.fail(e),
    }
}

#[derive(Deserialize)]
struct TreeStateQuery {
    height: u32,
}

async fn tree_state(
    State(state): State<Arc<RelayState>>,
    Query(q): Query<TreeStateQuery>,
) -> Response {
    match state.backend.tree_state(q.height).await {
        Ok(tree) => state.binary(tree.encode_to_vec()),
        Err(e) => state.fail(e),
    }
}

/// `POST /v1/tx/broadcast`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BroadcastResponse {
    pub ok: bool,
}

async fn broadcast(State(state): State<Arc<RelayState>>, body: Bytes) -> Response {
    if body.is_empty() {
        return state.fail(RelayError::BadRequest("empty transaction".into()));
    }
    match state.backend.broadcast(body.to_vec()).await {
        Ok(()) => {
            state
                .stats
                .broadcasts_accepted
                .fetch_add(1, Ordering::Relaxed);
            Json(BroadcastResponse { ok: true }).into_response()
        }
        Err(e) => state.fail(e),
    }
}

/// Parses a display-order hex txid into internal byte order.
pub fn parse_txid(text: &str) -> Result<[u8; 32], RelayError> {
    let bytes = hex::decode(text).map_err(|_| RelayError::BadRequest("txid is not hex".into()))?;
    let mut txid = <[u8; 32]>::try_from(bytes.as_slice())
        .map_err(|_| RelayError::BadRequest("txid must be 32 bytes".into()))?;
    txid.reverse();
    Ok(txid)
}

async fn transaction(State(state): State<Arc<RelayState>>, Path(txid): Path<String>) -> Response {
    let txid = match parse_txid(&txid) {
        Ok(t) => t,
        Err(e) => return state.fail(e),
    };
    match state.backend.transaction(txid).await {
        Ok(raw) => {
            state
                .stats
                .transactions_served
                .fetch_add(1, Ordering::Relaxed);
            state.binary(raw.encode_to_vec())
        }
        Err(e) => state.fail(e),
    }
}

async fn stats(State(state): State<Arc<RelayState>>) -> Json<RelayStatsResponse> {
    let s = &state.stats;
    let get = |c: &AtomicU64| c.load(Ordering::Relaxed);
    Json(RelayStatsResponse {
        requests: get(&s.requests),
        rate_limited: get(&s.rate_limited),
        blocks_served: get(&s.blocks_served),
        bytes_served: get(&s.bytes_served),
        transactions_served: get(&s.transactions_served),
        broadcasts_accepted: get(&s.broadcasts_accepted),
        broadcasts_rejected: get(&s.broadcasts_rejected),
        upstream_errors: get(&s.upstream_errors),
        rate_per_minute: state.limiter.per_minute,
        max_block_range: MAX_BLOCK_RANGE,
        max_tx_bytes: MAX_TX_BYTES,
    })
}

/// The relay routes, rate limited and size limited.
pub fn router(state: Arc<RelayState>) -> Router {
    Router::new()
        .route("/v1/chain/tip", get(tip))
        .route("/v1/chain/blocks", get(blocks))
        .route("/v1/chain/tree-state", get(tree_state))
        .route("/v1/tx/broadcast", post(broadcast))
        .route("/v1/tx/{txid}", get(transaction))
        .route("/v1/relay/stats", get(stats))
        .layer(DefaultBodyLimit::max(MAX_TX_BYTES))
        .layer(middleware::from_fn_with_state(state.clone(), rate_limit))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bucket_allows_a_burst_then_refills() {
        let limiter = RateLimiter::new(60);
        let a = IpAddr::from([10, 0, 0, 1]);
        let b = IpAddr::from([10, 0, 0, 2]);
        let t0 = Instant::now();
        for _ in 0..60 {
            assert!(limiter.allow(a, t0).await);
        }
        assert!(!limiter.allow(a, t0).await, "burst exhausted");
        assert!(limiter.allow(b, t0).await, "other clients are unaffected");
        // One token per second at 60 per minute.
        assert!(limiter.allow(a, t0 + Duration::from_millis(1_100)).await);
        assert!(!limiter.allow(a, t0 + Duration::from_millis(1_150)).await);
    }

    #[test]
    fn txids_are_display_order_hex() {
        let mut display = [0u8; 32];
        display[0] = 0xab;
        let parsed = parse_txid(&hex::encode(display)).expect("parse");
        assert_eq!(parsed[31], 0xab);
        assert!(parse_txid("zz").is_err());
        assert!(parse_txid("abcd").is_err());
    }

    #[test]
    fn blocks_are_length_prefixed_protobuf() {
        let block = |height: u64| CompactBlock {
            height,
            hash: vec![7; 32],
            ..CompactBlock::default()
        };
        let encoded = encode_blocks(&[block(5), block(6)]).expect("encode");
        let mut rest = encoded.as_slice();
        let mut heights = Vec::new();
        while !rest.is_empty() {
            let len = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
            let decoded = CompactBlock::decode(&rest[4..4 + len]).expect("decode");
            heights.push(decoded.height);
            rest = &rest[4 + len..];
        }
        assert_eq!(heights, vec![5, 6]);
    }
}
