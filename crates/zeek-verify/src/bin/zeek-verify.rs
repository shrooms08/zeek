//! `zeek-verify`: verify a Zeek round or a win proof against any lightwalletd.
//!
//! ```text
//! zeek-verify round 74 --engine http://127.0.0.1:8787 --lightwalletd https://testnet.zec.rocks:443
//! zeek-verify round 74 --bundle round-74.proof.json --config config/game.testnet.json --lightwalletd ...
//! zeek-verify win <proof id | file> --engine ... --lightwalletd ...
//! zeek-verify relay --lightwalletd https://zaino.testnet.unsafe.zec.rocks:443 --bind 127.0.0.1:8790
//! ```
//!
//! Only the proof bundle and the game's public configuration (parameters,
//! vault address, UIVK) come from Zeek; every block and transaction comes
//! from the lightwalletd given, which can be anyone's.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::Engine;
use clap::{Parser, Subcommand};
use prost::Message;
use tokio::sync::Mutex;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use tower_http::cors::{AllowOrigin, CorsLayer};
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_backend::proto::service::compact_tx_streamer_client::CompactTxStreamerClient;
use zcash_client_backend::proto::service::{BlockId, BlockRange, ChainSpec, Empty, TxFilter};
use zeek_core::proof::ProofBundle;
use zeek_verify::chain::{txid_bytes, Blocks, MinedTx};
use zeek_verify::checks::{self, PublicConfig, RoundInput};
use zeek_verify::report::CheckStatus;
use zeek_verify::win::{self, WinProof};

#[derive(Parser)]
#[command(
    name = "zeek-verify",
    about = "Independently verify Zeek rounds and win proofs"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Args, Clone)]
struct Sources {
    /// Any lightwalletd (gRPC, http:// or https://).
    #[arg(long, env = "ZEEK_VERIFY_LIGHTWALLETD")]
    lightwalletd: String,
    /// Zeek engine to fetch the proof bundle and public config from.
    #[arg(
        long,
        env = "ZEEK_VERIFY_ENGINE",
        default_value = "http://127.0.0.1:8787"
    )]
    engine: String,
    /// Public game config (JSON with params, vault_address, uivk) instead of
    /// the engine's /v1/config.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Print the report as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Run the five checks on a settled round.
    Round {
        round: u32,
        /// Proof bundle file instead of the engine's /v1/rounds/<n>/proof.
        #[arg(long)]
        bundle: Option<PathBuf>,
        #[command(flatten)]
        sources: Sources,
    },
    /// Verify a win proof (an engine proof id, or a file holding the
    /// base64url proof).
    Win {
        proof: String,
        #[command(flatten)]
        sources: Sources,
    },
    /// Serve the read-only chain relay routes (tip, compact blocks,
    /// transactions) from a lightwalletd of your choice, for browser
    /// verifiers that need an independent second source.
    Relay {
        /// lightwalletd servers, comma-separated, tried in order with
        /// failover. Choose servers other than the engine's.
        #[arg(long)]
        lightwalletd: String,
        #[arg(long, default_value = "127.0.0.1:8790")]
        bind: SocketAddr,
        /// Allowed browser origins (comma-separated); `*` for any.
        #[arg(long, default_value = "*")]
        cors: String,
        /// Behind the public site's proxy: a file holding the secret the
        /// proxy sends (X-Zeek-Proxy). Requests without it are refused, and
        /// clients are counted by the address the proxy names.
        #[arg(long)]
        proxy_secret_file: Option<PathBuf>,
        /// Direct mode, behind a host whose own edge is the only way in
        /// (Railway): browsers call the relay straight, and each is counted
        /// by the last address in this header, the one the edge appended.
        /// Requests with the proxy secret still name their client.
        #[arg(long)]
        client_ip_header: Option<String>,
    },
    /// Print a settled round's replay (depth stages block by block, the
    /// draw and the payout; never amounts per tile) as JSON, from its proof
    /// bundle file. Used to build the web app's Replay snapshot.
    Replay {
        #[arg(long)]
        bundle: PathBuf,
    },
}

fn run_replay(file: &std::path::Path) -> anyhow::Result<bool> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let bundle: ProofBundle = serde_json::from_str(&text).context("parsing the proof bundle")?;
    let replay = zeek_verify::replay::replay(&bundle)?;
    println!("{}", serde_json::to_string(&replay)?);
    Ok(true)
}

type Client = CompactTxStreamerClient<Channel>;

/// Longest wait for a connection, for one call, and for the next block of a
/// block-range stream. Without them a dead connection hangs every request
/// (seen live after a network outage: relay calls took 60 s each).
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const STREAM_STALL: std::time::Duration = std::time::Duration::from_secs(30);

async fn connect(url: &str) -> anyhow::Result<Client> {
    // Keep-alives only during calls, no more often than lightwalletd allows
    // (it answers frequent or idle pings with GOAWAY "too_many_pings").
    let mut endpoint = Endpoint::from_shared(url.to_owned())?
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(CALL_TIMEOUT)
        .http2_keep_alive_interval(std::time::Duration::from_secs(300))
        .keep_alive_timeout(std::time::Duration::from_secs(20))
        .keep_alive_while_idle(false);
    if url.starts_with("https://") {
        endpoint = endpoint.tls_config(ClientTlsConfig::new().with_webpki_roots())?;
    } else if !url.starts_with("http://") {
        bail!("lightwalletd URL must start with http:// or https://");
    }
    let channel = endpoint
        .connect()
        .await
        .with_context(|| format!("connecting to {url}"))?;
    let mut client = CompactTxStreamerClient::new(channel);
    let info = client.get_lightd_info(Empty {}).await?.into_inner();
    eprintln!(
        "lightwalletd {url}: chain {} at height {} ({} {})",
        info.chain_name, info.block_height, info.vendor, info.version
    );
    Ok(client)
}

async fn blocks(client: &mut Client, start: u32, end: u32) -> anyhow::Result<Vec<CompactBlock>> {
    let id = |h: u32| BlockId {
        height: u64::from(h),
        hash: vec![],
    };
    let mut stream = client
        .get_block_range(BlockRange {
            start: Some(id(start)),
            end: Some(id(end)),
            pool_types: vec![],
        })
        .await?
        .into_inner();
    let mut out = Vec::new();
    while let Some(block) = tokio::time::timeout(STREAM_STALL, stream.message())
        .await
        .map_err(|_| anyhow::anyhow!("block stream stalled for {STREAM_STALL:?}"))??
    {
        out.push(block);
    }
    Ok(out)
}

async fn transaction(client: &mut Client, txid: &str) -> anyhow::Result<MinedTx> {
    let hash = txid_bytes(txid).context("malformed txid")?;
    let raw = client
        .get_transaction(TxFilter {
            block: None,
            index: 0,
            hash: hash.to_vec(),
        })
        .await?
        .into_inner();
    Ok(MinedTx {
        raw: raw.data,
        height: u32::try_from(raw.height)?,
    })
}

async fn engine_json<T: serde::de::DeserializeOwned>(
    engine: &str,
    path: &str,
) -> anyhow::Result<T> {
    let url = format!("{}{path}", engine.trim_end_matches('/'));
    let response = reqwest::get(&url)
        .await
        .with_context(|| format!("fetching {url}"))?;
    if !response.status().is_success() {
        bail!("{url}: {}", response.status());
    }
    Ok(response.json().await?)
}

async fn public_config(sources: &Sources) -> anyhow::Result<PublicConfig> {
    match &sources.config {
        Some(path) => Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?),
        None => engine_json(&sources.engine, "/v1/config").await,
    }
}

async fn round_bundle(
    sources: &Sources,
    round: u32,
    file: Option<&PathBuf>,
) -> anyhow::Result<ProofBundle> {
    match file {
        Some(path) => Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?),
        None => engine_json(&sources.engine, &format!("/v1/rounds/{round}/proof")).await,
    }
}

async fn run_round(round: u32, file: Option<PathBuf>, sources: Sources) -> anyhow::Result<bool> {
    let config = public_config(&sources).await?;
    let bundle = round_bundle(&sources, round, file.as_ref()).await?;
    if bundle.round != round {
        bail!("the bundle is for round {}, not {round}", bundle.round);
    }
    checks::check_window(&bundle)?;
    let ivk = config.ivk()?;
    let mut client = connect(&sources.lightwalletd).await?;
    let (start, end) = checks::block_range(&bundle);
    let mut fetched = blocks(&mut client, start, end).await?;
    if let Some(h) = checks::payout_height(&bundle) {
        if h < start || h > end {
            fetched.extend(blocks(&mut client, h, h).await?);
        }
    }
    let chain = Blocks::new(fetched);
    let hits = checks::scan_window(&bundle, &ivk, &chain);
    let mut txs = BTreeMap::new();
    for txid in checks::transactions_to_fetch(&bundle, &hits) {
        txs.insert(txid.clone(), transaction(&mut client, &txid).await?);
    }
    let report = checks::verify_round(&RoundInput {
        config: &config,
        bundle: &bundle,
        blocks: &chain,
        txs: &txs,
        ivk: &ivk,
    });
    if sources.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Round {} (bundle format {}, rules v{})",
            report.round,
            report.bundle_version,
            report.rules_version.unwrap_or(0)
        );
        println!(
            "Chain source: {} (blocks {start}..={end}, {} transactions)",
            sources.lightwalletd,
            txs.len()
        );
        for (i, check) in report.checks.iter().enumerate() {
            let mark = match check.status {
                CheckStatus::Pass => "PASS",
                CheckStatus::Warn => "WARN",
                CheckStatus::Fail => "FAIL",
                CheckStatus::NotAvailable => "N/A ",
            };
            println!(
                "\n{}. [{mark}] {}\n   {}",
                i + 1,
                check.title,
                check.summary
            );
            for line in &check.detail {
                println!("   - {line}");
            }
        }
        println!(
            "\nResult: {}",
            if report.passed() {
                "every available check passed"
            } else {
                "VERIFICATION FAILED"
            }
        );
    }
    Ok(report.passed())
}

async fn run_win(proof_ref: String, sources: Sources) -> anyhow::Result<bool> {
    let encoded = if std::path::Path::new(&proof_ref).exists() {
        std::fs::read_to_string(&proof_ref)?.trim().to_owned()
    } else {
        #[derive(serde::Deserialize)]
        struct Stored {
            proof: String,
        }
        engine_json::<Stored>(&sources.engine, &format!("/v1/proofs/{proof_ref}"))
            .await?
            .proof
    };
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded.trim())?;
    let proof = WinProof::decode(&bytes)?;
    let bundle = round_bundle(&sources, proof.round, None).await?;
    let mut client = connect(&sources.lightwalletd).await?;
    let mined = transaction(&mut client, &proof.payout_txid).await?;
    let chain = Blocks::new(blocks(&mut client, mined.height, mined.height).await?);
    let included =
        txid_bytes(&proof.payout_txid).is_some_and(|t| chain.contains_tx(mined.height, &t));
    let verdict = win::verify(&proof, &bundle, &mined, included);
    if sources.json {
        println!("{}", serde_json::to_string_pretty(&verdict)?);
    } else {
        println!(
            "Win proof {} (round {}, payout {})",
            WinProof::id(&bytes),
            proof.round,
            proof.payout_index
        );
        for step in &verdict.steps {
            println!(
                "  [{}] {}: {}",
                if step.passed { "ok" } else { "FAIL" },
                step.name,
                step.detail
            );
        }
        match &verdict.reason {
            None => println!("VALID: {}", verdict.claim),
            Some(reason) => println!("INVALID: {reason}"),
        }
    }
    Ok(verdict.valid)
}

// ---- relay ----

/// The relay's upstreams: a list tried in order, failing over when a call
/// fails. Calls run concurrently on clones of one connected client.
struct RelayState {
    urls: Vec<String>,
    current: Mutex<Option<(usize, Client)>>,
}

impl RelayState {
    async fn client(&self) -> Result<(usize, Client), String> {
        let mut current = self.current.lock().await;
        if let Some((index, client)) = current.as_ref() {
            return Ok((*index, client.clone()));
        }
        let mut errors = Vec::new();
        for (index, url) in self.urls.iter().enumerate() {
            match connect(url).await {
                Ok(client) => {
                    *current = Some((index, client.clone()));
                    return Ok((index, client));
                }
                Err(e) => errors.push(format!("{url}: {e}")),
            }
        }
        Err(format!("no lightwalletd answered: {}", errors.join("; ")))
    }

    /// Drops the connection to `index` after a failed call; the next call
    /// connects again, starting after it.
    async fn failed(&self, index: usize) {
        let mut current = self.current.lock().await;
        if current.as_ref().is_some_and(|(i, _)| *i == index) {
            *current = None;
            if self.urls.len() > 1 {
                eprintln!("lightwalletd {} failed; failing over", self.urls[index]);
            }
        }
    }
}

/// gRPC statuses that mean the request itself is at fault.
fn upstream_fault(status: &tonic::Status) -> bool {
    !matches!(
        status.code(),
        tonic::Code::NotFound | tonic::Code::InvalidArgument | tonic::Code::OutOfRange
    )
}

/// Logs an upstream failure and answers without its details.
fn upstream_error(detail: &str) -> Response {
    eprintln!("relay upstream error: {detail}");
    relay_error(
        StatusCode::BAD_GATEWAY,
        "the chain server did not answer; try again shortly".to_owned(),
    )
}

fn relay_error(status: StatusCode, message: String) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

async fn relay_tip(State(state): State<Arc<RelayState>>) -> Response {
    let (index, mut client) = match state.client().await {
        Ok(c) => c,
        Err(e) => return upstream_error(&e),
    };
    match client.get_latest_block(ChainSpec {}).await {
        Ok(block) => {
            let block = block.into_inner();
            let mut hash = block.hash;
            hash.reverse();
            Json(serde_json::json!({ "height": block.height, "hash": hex::encode(hash) }))
                .into_response()
        }
        Err(e) => {
            state.failed(index).await;
            upstream_error(e.message())
        }
    }
}

#[derive(serde::Deserialize)]
struct RangeQuery {
    start: u32,
    end: u32,
}

async fn relay_blocks(
    State(state): State<Arc<RelayState>>,
    Query(q): Query<RangeQuery>,
) -> Response {
    if q.end < q.start || q.end - q.start >= 500 {
        return relay_error(
            StatusCode::BAD_REQUEST,
            "at most 500 blocks per request".into(),
        );
    }
    let (index, mut client) = match state.client().await {
        Ok(c) => c,
        Err(e) => return upstream_error(&e),
    };
    match blocks(&mut client, q.start, q.end).await {
        Ok(list) => {
            let mut body = Vec::new();
            for block in list {
                let len = u32::try_from(block.encoded_len()).unwrap_or(u32::MAX);
                body.extend_from_slice(&len.to_le_bytes());
                if block.encode(&mut body).is_err() {
                    return relay_error(StatusCode::INTERNAL_SERVER_ERROR, "encoding".into());
                }
            }
            (
                [(header::CONTENT_TYPE, "application/octet-stream")],
                Bytes::from(body),
            )
                .into_response()
        }
        Err(e) => {
            state.failed(index).await;
            upstream_error(&e.to_string())
        }
    }
}

async fn relay_tx(State(state): State<Arc<RelayState>>, Path(txid): Path<String>) -> Response {
    let Some(hash) = txid_bytes(&txid) else {
        return relay_error(
            StatusCode::BAD_REQUEST,
            "txid is not 32 bytes of hex".into(),
        );
    };
    let (index, mut client) = match state.client().await {
        Ok(c) => c,
        Err(e) => return upstream_error(&e),
    };
    match client
        .get_transaction(TxFilter {
            block: None,
            index: 0,
            hash: hash.to_vec(),
        })
        .await
    {
        Ok(raw) => (
            [(header::CONTENT_TYPE, "application/octet-stream")],
            raw.into_inner().encode_to_vec(),
        )
            .into_response(),
        Err(e) => {
            if upstream_fault(&e) {
                state.failed(index).await;
                return upstream_error(e.message());
            }
            relay_error(StatusCode::NOT_FOUND, "transaction not found".to_owned())
        }
    }
}

/// Requests per minute per client, and for all clients together.
const RELAY_PER_CLIENT_PER_MIN: f64 = 300.0;
const RELAY_GLOBAL_PER_MIN: f64 = 3_000.0;
/// Most clients tracked at once.
const RELAY_MAX_CLIENTS: usize = 100_000;

/// Token buckets per client (IPv6 per /64) and one for everyone.
#[derive(Default)]
struct Limits {
    buckets:
        std::sync::Mutex<std::collections::HashMap<std::net::IpAddr, (f64, std::time::Instant)>>,
}

impl Limits {
    fn take(
        buckets: &mut std::collections::HashMap<std::net::IpAddr, (f64, std::time::Instant)>,
        key: std::net::IpAddr,
        per_min: f64,
        now: std::time::Instant,
    ) -> bool {
        let (tokens, at) = buckets.entry(key).or_insert((per_min, now));
        *tokens = (*tokens + now.duration_since(*at).as_secs_f64() * per_min / 60.0).min(per_min);
        *at = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn allow(&self, client: std::net::IpAddr) -> bool {
        let now = std::time::Instant::now();
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if buckets.len() > RELAY_MAX_CLIENTS {
            buckets.retain(|_, (_, at)| now.duration_since(*at).as_secs() < 60);
        }
        let everyone = std::net::IpAddr::from([0, 0, 0, 0]);
        if !Self::take(&mut buckets, everyone, RELAY_GLOBAL_PER_MIN, now) {
            return false;
        }
        if buckets.len() > RELAY_MAX_CLIENTS && !buckets.contains_key(&client) {
            return false;
        }
        Self::take(&mut buckets, client, RELAY_PER_CLIENT_PER_MIN, now)
    }
}

/// Counts IPv6 clients per /64.
fn client_key(ip: std::net::IpAddr) -> std::net::IpAddr {
    match ip {
        std::net::IpAddr::V4(_) => ip,
        std::net::IpAddr::V6(v6) => {
            let mut o = v6.octets();
            o[8..].fill(0);
            std::net::IpAddr::from(o)
        }
    }
}

/// Who may call the relay and how a client is named.
#[derive(Clone, Default)]
struct Access {
    /// The proxy's secret, when the relay sits behind the public site.
    secret: Option<Arc<str>>,
    /// Direct mode: the header the host's edge sets to the client address.
    direct: Option<axum::http::HeaderName>,
}

/// The last address in a header value (`a, b, 203.0.113.7`,
/// `[2001:db8::1]:443`, `203.0.113.7:443`).
fn last_address(value: &str) -> Option<std::net::IpAddr> {
    let last = value.rsplit(',').next()?.trim();
    last.parse::<std::net::IpAddr>()
        .ok()
        .or_else(|| last.parse::<SocketAddr>().ok().map(|a| a.ip()))
}

/// The client a request is counted against, or the refusal.
fn client_of(
    access: &Access,
    headers: &axum::http::HeaderMap,
    peer: std::net::IpAddr,
) -> Result<std::net::IpAddr, (StatusCode, &'static str)> {
    let given = headers.get("x-zeek-proxy").map(|v| v.as_bytes());
    if let (Some(secret), Some(given)) = (&access.secret, given) {
        let expected = secret.as_bytes();
        let same = given.len() == expected.len()
            && given
                .iter()
                .zip(expected)
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0;
        if !same {
            return Err((
                StatusCode::FORBIDDEN,
                "use the public site; this address only serves its proxy",
            ));
        }
        return headers
            .get("x-zeek-client")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<std::net::IpAddr>().ok())
            .ok_or((StatusCode::BAD_REQUEST, "the proxy did not name the client"));
    }
    if let Some(header) = &access.direct {
        if given.is_some() {
            return Err((StatusCode::FORBIDDEN, "unexpected proxy header"));
        }
        return headers
            .get(header)
            .and_then(|v| v.to_str().ok())
            .and_then(last_address)
            .ok_or((StatusCode::BAD_REQUEST, "the client address is missing"));
    }
    if access.secret.is_some() {
        return Err((
            StatusCode::FORBIDDEN,
            "use the public site; this address only serves its proxy",
        ));
    }
    Ok(peer)
}

/// Checks who calls (see [`client_of`]) and the rate limits.
async fn guard(
    State((access, limits)): State<(Access, Arc<Limits>)>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<SocketAddr>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let client = match client_of(&access, request.headers(), peer.ip()) {
        Ok(ip) => ip,
        Err((status, message)) => return relay_error(status, message.to_owned()),
    };
    if !limits.allow(client_key(client)) {
        return relay_error(
            StatusCode::TOO_MANY_REQUESTS,
            "too many requests; try again in a minute".to_owned(),
        );
    }
    next.run(request).await
}

async fn run_relay(
    lightwalletd: String,
    bind: SocketAddr,
    cors: String,
    proxy_secret_file: Option<PathBuf>,
    client_ip_header: Option<String>,
) -> anyhow::Result<()> {
    let secret: Option<Arc<str>> = match &proxy_secret_file {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            let text = text.trim();
            anyhow::ensure!(
                text.len() >= 32,
                "the proxy secret must be at least 32 characters"
            );
            Some(text.into())
        }
        None => None,
    };
    let direct = match client_ip_header.as_deref().map(str::trim) {
        Some(h) if !h.is_empty() => {
            let name = axum::http::HeaderName::try_from(h.to_ascii_lowercase())
                .with_context(|| format!("`{h}` is not a header name"))?;
            anyhow::ensure!(
                !name.as_str().starts_with("x-zeek-"),
                "the client address header cannot be `{name}`"
            );
            Some(name)
        }
        _ => None,
    };
    let access = Access { secret, direct };
    let urls: Vec<String> = lightwalletd
        .split(',')
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(str::to_owned)
        .collect();
    anyhow::ensure!(!urls.is_empty(), "--lightwalletd needs at least one URL");
    let state = Arc::new(RelayState {
        urls,
        current: Mutex::new(None),
    });
    if let Err(e) = state.client().await {
        bail!(e);
    }
    let origins = if cors.trim() == "*" {
        AllowOrigin::any()
    } else {
        AllowOrigin::list(
            cors.split(',')
                .filter_map(|o| HeaderValue::from_str(o.trim()).ok()),
        )
    };
    let app = Router::new()
        .route("/v1/chain/tip", get(relay_tip))
        .route("/v1/chain/blocks", get(relay_blocks))
        .route("/v1/tx/{txid}", get(relay_tx))
        .layer(
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods([Method::GET]),
        )
        .with_state(state)
        .layer(axum::middleware::from_fn_with_state(
            (access, Arc::new(Limits::default())),
            guard,
        ));
    eprintln!("read-only relay for {lightwalletd} on http://{bind}");
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let ok = runtime.block_on(async {
        match cli.command {
            Command::Round {
                round,
                bundle,
                sources,
            } => run_round(round, bundle, sources).await,
            Command::Win { proof, sources } => run_win(proof, sources).await,
            Command::Relay {
                lightwalletd,
                bind,
                cors,
                proxy_secret_file,
                client_ip_header,
            } => run_relay(
                lightwalletd,
                bind,
                cors,
                proxy_secret_file,
                client_ip_header,
            )
            .await
            .map(|()| true),
            Command::Replay { bundle } => run_replay(&bundle),
        }
    })?;
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";
    const PEER: std::net::IpAddr = std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().expect("value"));
        }
        h
    }

    fn ip(s: &str) -> std::net::IpAddr {
        s.parse().expect("ip")
    }

    #[test]
    fn local_relays_count_the_peer() {
        assert_eq!(client_of(&Access::default(), &headers(&[]), PEER), Ok(PEER));
    }

    #[test]
    fn behind_the_proxy_only_the_secret_gets_through() {
        let access = Access {
            secret: Some(SECRET.into()),
            direct: None,
        };
        let ok = headers(&[("x-zeek-proxy", SECRET), ("x-zeek-client", "203.0.113.7")]);
        assert_eq!(client_of(&access, &ok, PEER), Ok(ip("203.0.113.7")));
        let none = headers(&[("x-forwarded-for", "203.0.113.7")]);
        assert_eq!(
            client_of(&access, &none, PEER).map_err(|e| e.0),
            Err(StatusCode::FORBIDDEN)
        );
        let wrong = headers(&[("x-zeek-proxy", "x"), ("x-zeek-client", "203.0.113.7")]);
        assert_eq!(
            client_of(&access, &wrong, PEER).map_err(|e| e.0),
            Err(StatusCode::FORBIDDEN)
        );
    }

    #[test]
    fn direct_mode_takes_the_address_the_edge_appended() {
        let access = Access {
            secret: None,
            direct: Some(axum::http::HeaderName::from_static("x-forwarded-for")),
        };
        let h = headers(&[
            ("x-forwarded-for", "10.0.0.1, 203.0.113.9"),
            ("x-zeek-client", "10.9.9.9"),
        ]);
        assert_eq!(client_of(&access, &h, PEER), Ok(ip("203.0.113.9")));
        let v6 = headers(&[("x-forwarded-for", "[2001:db8::5]:443")]);
        assert_eq!(client_of(&access, &v6, PEER), Ok(ip("2001:db8::5")));
        assert_eq!(
            client_of(&access, &headers(&[]), PEER).map_err(|e| e.0),
            Err(StatusCode::BAD_REQUEST)
        );
        let spoof = headers(&[("x-zeek-proxy", SECRET), ("x-forwarded-for", "203.0.113.9")]);
        assert_eq!(
            client_of(&access, &spoof, PEER).map_err(|e| e.0),
            Err(StatusCode::FORBIDDEN)
        );
    }
}
