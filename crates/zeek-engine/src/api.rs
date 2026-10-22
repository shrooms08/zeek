//! HTTP API (axum): public game state for players, UIs and verifiers.
//!
//! Every response is built from the database through the processor; sealed
//! receivers are served as sealed bytes, nothing here can open them.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tower_http::cors::{AllowOrigin, CorsLayer};
use zeek_core::board::TileTotals;
use zeek_core::digs;
use zeek_core::params::GameParams;
use zeek_core::rounds::{RoundContext, RoundStatus, RoundWindow};
use zeek_core::seal::SealingPublicKey;
use zeek_core::settle::{Payout, RoundOutcome};

use crate::config::{GameConfig, SETTLE_CONFIRMATIONS};
use crate::coordinator::LedgerView;
use crate::db::{Db, PayoutRecord, PlanStatus};
use crate::error::EngineError;
use crate::ledger::Ledger;
use crate::processor::{BoardState, Event, PendingRound, Processor};
use crate::proof::ProofBundle;
use crate::store::{Reader, Store};

/// Shared state of the HTTP handlers.
pub struct AppState {
    pub processor: Processor,
    /// Shared with the engine loop; handlers read through its read pool.
    pub store: Arc<Store>,
    pub events: broadcast::Sender<Event>,
    pub started: Instant,
    /// Present when this engine also coordinates payouts.
    /// The vault ledger (payout coordinator engines only).
    pub ledger: Option<LedgerView>,
    /// The engine's `lightwalletd` servers (for the status page).
    pub chain_servers: Option<Arc<zeek_vault::pool::Pool>>,
    /// Whether digs are open (false: mainnet readiness, vault withheld).
    pub digging: bool,
    /// Whether the demo digger runs (`ZEEK_DEMO_DIGS=on`).
    pub demo_digs: bool,
    /// Open `/v1/stream` connections allowed at once.
    pub stream_slots: Arc<tokio::sync::Semaphore>,
    /// Present when this engine relays chain data to browser wallets.
    pub relay: Option<Arc<crate::relay::RelayState>>,
    /// Campfire display filter.
    pub campfire_filter: crate::campfire::Filter,
    /// Configured keyholder identifiers and the signing threshold (empty
    /// when this engine does not coordinate payouts).
    pub keyholder_ids: Vec<u16>,
    pub threshold: u16,
    /// Rendered share images, by path (see [`crate::share`]).
    pub share_cache: crate::share::ShareCache,
    /// Public base URL of the web app, for share links and redirects.
    pub app_url: String,
    /// Public base URL of this engine, for absolute image URLs in share pages.
    pub public_url: String,
    /// Rate limit of win proof submissions (verification costs about a
    /// second of CPU each).
    pub proof_limiter: crate::relay::RateLimiter,
    /// Digs seen in the mempool, not mined yet (see [`crate::mempool`]).
    pub landing: Arc<crate::mempool::Landing>,
}

/// `GET /v1/board`: the board, plus the digs still in the mempool as a
/// separate field that nothing on the board counts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardResponse {
    #[serde(flatten)]
    pub board: BoardState,
    /// Digs to the vault seen in the mempool, oldest first.
    pub landing: Vec<crate::mempool::LandingDig>,
}

/// Win proof submissions per minute per client address.
pub const PROOF_SUBMISSIONS_PER_MIN: u32 = 10;
/// Largest accepted submission body (a proof is about 7.5 kB, 10 kB as base64url).
pub const MAX_PROOF_BODY: usize = 32 * 1024;

/// `GET /v1/campfire` query.
#[derive(Clone, Debug, Deserialize)]
pub struct CampfireQuery {
    /// Only messages mined below this height (pagination).
    pub before: Option<u32>,
    pub limit: Option<usize>,
    /// `1` to include the text of filtered messages.
    pub include_hidden: Option<u8>,
}

/// `GET /v1/campfire`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampfireResponse {
    pub messages: Vec<crate::campfire::CampfireMessage>,
    /// Pass as `before` for the next page (absent at the end).
    pub next_before: Option<u32>,
}

/// `POST /v1/proofs` body.
#[derive(Clone, Debug, Deserialize)]
pub struct SubmitProof {
    /// The win proof, base64url (no padding).
    pub proof: String,
}

/// `POST /v1/proofs` and `GET /v1/proofs/{id}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredProof {
    pub id: String,
    pub round: u32,
    pub created_at: u64,
    /// The proof, base64url.
    pub proof: String,
    /// The engine's verdict (anyone can recheck with `zeek-verify win`).
    pub verdict: zeek_verify::win::WinVerdict,
}

/// `GET /v1/config`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigResponse {
    pub params: GameParams,
    /// Whether this engine accepts digs. When false (mainnet readiness) the
    /// vault's address, sealing key and viewing key are withheld, so nobody
    /// can send funds to a vault that is not open.
    pub digging_enabled: bool,
    /// Whether Zeek's own test wallets dig on this network (scripts/
    /// demo-digger.sh), so the app can say so.
    #[serde(default)]
    pub demo_digs: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealing_public_key: Option<SealingPublicKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uivk: Option<String>,
    pub genesis_height: u32,
    /// Confirmations the engine waits for after the timing-rule blocks.
    pub settle_confirmations: u32,
    /// Deep Cache balance carried into round 0 from a previous vault.
    pub initial_cache_zat: u64,
}

/// `GET /v1/rounds/pending`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingResponse {
    pub rounds: Vec<PendingRound>,
}

/// Post-settlement details of a round.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettledRound {
    pub outcome: RoundOutcome,
    pub treasure_tile: Option<u8>,
    pub cache_triggered: bool,
    /// Exact value dug per tile.
    pub tile_totals: [u64; 25],
    pub drand_round: u64,
    pub beacon_time: u64,
    pub pot: u64,
    pub rake: u64,
    pub distributable: u64,
    pub winners: usize,
    /// Outputs the keyholders must sign (sealed receivers only).
    pub payouts: Vec<Payout>,
    pub previous_cache: u64,
    pub new_cache: u64,
    pub cache_in: u64,
    pub cache_paid: u64,
    /// Dig fees plus output fees.
    pub total_fees: u64,
    /// Dig fees charged to the notes the payout transaction spends (rules v2;
    /// absent in views saved under rules v1).
    #[serde(default)]
    pub dig_fees: u64,
    pub total_paid: u64,
    pub forfeited: u64,
    /// Shouts (rules v4): their value minus `shout_fees` went to the Cache.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub shouts: u64,
    /// Dig fees reserved for spending the shout notes later.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub shout_fees: u64,
    /// Rules the round was settled under (1 or 2; views saved before the
    /// field existed are rules v1).
    #[serde(default = "rules_v1")]
    pub rules_version: u32,
    pub plan_status: PlanStatus,
    pub settled_at_height: u32,
    /// Payout transaction, once broadcast.
    pub payout: Option<PayoutRecord>,
}

fn rules_v1() -> u32 {
    1
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// `GET /v1/rounds/{n}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundResponse {
    pub round: u32,
    pub status: RoundStatus,
    pub window: RoundWindow,
    pub context: RoundContext,
    /// Valid digs counted so far (or at settlement).
    pub digs: usize,
    /// Relative depths while the round is not settled.
    pub depths: [u8; 25],
    pub settled: Option<SettledRound>,
}

/// `GET /v1/health`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub tip_height: Option<u32>,
    pub settled_rounds: usize,
    pub uptime_secs: u64,
}

/// JSON error body.
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}

/// Error wrapper for handlers.
pub struct ApiError(StatusCode, String);

impl From<EngineError> for ApiError {
    /// Internal details (database, paths, configuration) stay in the log.
    fn from(e: EngineError) -> Self {
        if let EngineError::Busy(why) = e {
            tracing::warn!(why, "request refused: busy");
            return ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "busy; try again in a moment".into(),
            );
        }
        tracing::error!(error = %e, "request failed");
        internal()
    }
}

/// The body of every internal error (security review L1).
fn internal() -> ApiError {
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal error; it has been logged".into(),
    )
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(ErrorBody { error: self.1 })).into_response()
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

/// A read-only connection: never waits for the engine loop's writes.
fn reader(state: &AppState) -> Result<Reader<'_>, ApiError> {
    Ok(state.store.reader()?)
}

/// Builds the router.
pub fn router(state: Arc<AppState>, cors_origins: &[String]) -> Router {
    let mut router = Router::new()
        .route("/v1/config", get(config))
        .route("/v1/board", get(board))
        .route("/v1/rounds/pending", get(pending))
        .route("/v1/rounds/{round}", get(round))
        .route("/v1/rounds/{round}/proof", get(proof))
        .route("/v1/health", get(health))
        .route("/v1/status", get(status))
        .route("/v1/vault/ledger", get(vault_ledger))
        .route("/v1/stream", get(stream))
        .route("/v1/campfire", get(campfire))
        .route("/v1/rounds", get(rounds_list))
        .route("/v1/rounds/{round}/digests", get(round_digests))
        .route("/v1/keyholders/status", get(keyholders_status))
        .route("/share/round/{name}", get(crate::share::round_page))
        .route("/share/win/{name}", get(crate::share::win_page))
        .route("/v1/proofs", axum::routing::post(submit_proof))
        .route("/v1/proofs/{id}", get(get_proof))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_PROOF_BODY))
        .with_state(state.clone());
    if let Some(relay) = &state.relay {
        router = router.merge(crate::relay::router(relay.clone()));
    }
    if !cors_origins.is_empty() {
        let origins: Vec<HeaderValue> = cors_origins
            .iter()
            .filter_map(|o| HeaderValue::from_str(o).ok())
            .collect();
        router = router.layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(origins))
                .allow_methods([Method::GET, Method::POST])
                .allow_headers([axum::http::header::CONTENT_TYPE]),
        );
    }
    router
}

/// The public config response (`digging`: whether digs are open;
/// `demo_digs`: whether Zeek's own test wallets dig).
pub fn config_response(config: &GameConfig, digging: bool, demo_digs: bool) -> ConfigResponse {
    let open = |v| if digging { Some(v) } else { None };
    ConfigResponse {
        params: config.params.clone(),
        digging_enabled: digging,
        demo_digs,
        vault_address: open(config.vault_address.clone()),
        sealing_public_key: digging.then(|| config.sealing_public_key.clone()),
        uivk: open(config.uivk.clone()),
        genesis_height: config.genesis_height,
        settle_confirmations: SETTLE_CONFIRMATIONS,
        initial_cache_zat: config.initial_cache_zat,
    }
}

async fn config(State(state): State<Arc<AppState>>) -> ApiResult<ConfigResponse> {
    Ok(Json(config_response(
        state.processor.config(),
        state.digging,
        state.demo_digs,
    )))
}

async fn board(State(state): State<Arc<AppState>>) -> ApiResult<BoardResponse> {
    // Landing digs whose transaction is already stored are left out: the
    // board counts them (one indexed lookup per pending transaction).
    let (board, landing) = {
        let db = reader(&state)?;
        let board = state.processor.board(&db)?;
        let landing = state
            .landing
            .digs_unmined(|txid| db.has_note_from_tx(txid))?;
        (board, landing)
    };
    Ok(Json(BoardResponse { board, landing }))
}

async fn pending(State(state): State<Arc<AppState>>) -> ApiResult<PendingResponse> {
    let db = reader(&state)?;
    Ok(Json(PendingResponse {
        rounds: state.processor.pending_rounds(&db)?,
    }))
}

/// Builds the round response (shared with tests).
pub fn round_response(
    processor: &Processor,
    db: &Db,
    round: u32,
) -> Result<RoundResponse, EngineError> {
    let params = &processor.config().params;
    let (window, status, context) = processor.status(db, round)?;
    let settlement = db.settlement(round)?;
    let payout = db.payout(round)?;
    let voided = status == RoundStatus::Voided;
    let notes = db
        .notes_in_range(window.start, window.end)?
        .iter()
        .map(|n| n.to_received_note())
        .collect::<Vec<_>>();
    let classified = digs::classify_round(params, round, voided, notes)?;
    let totals =
        TileTotals::from_digs(&classified).map_err(|e| EngineError::Corrupt(e.to_string()))?;
    let settled = settlement.map(|row| {
        let (treasure_tile, cache_triggered) = match row.outcome {
            RoundOutcome::Voided => (None, false),
            RoundOutcome::Drawn {
                treasure_tile,
                cache_triggered,
            } => (Some(treasure_tile), cache_triggered),
        };
        let s = row.settlement;
        SettledRound {
            outcome: row.outcome,
            treasure_tile,
            cache_triggered,
            tile_totals: totals.0,
            drand_round: row.drand_round,
            beacon_time: row.beacon_time,
            pot: s.pot,
            rake: s.rake,
            distributable: s.distributable,
            winners: s.winners,
            payouts: s.payouts,
            previous_cache: s.previous_cache,
            new_cache: s.new_cache,
            cache_in: s.cache_in,
            cache_paid: s.cache_paid,
            total_fees: s.total_fees,
            dig_fees: s.dig_fees,
            total_paid: s.total_paid,
            forfeited: s.forfeited,
            shouts: s.shouts,
            shout_fees: s.shout_fees,
            rules_version: row.rules_version,
            plan_status: row.status,
            settled_at_height: row.settled_at_height,
            payout: payout.clone(),
        }
    });
    Ok(RoundResponse {
        round,
        status,
        window,
        context,
        digs: classified.valid.len(),
        depths: totals.relative_depths(),
        settled,
    })
}

async fn round(
    State(state): State<Arc<AppState>>,
    Path(round): Path<u32>,
) -> ApiResult<RoundResponse> {
    let db = reader(&state)?;
    Ok(Json(round_response(&state.processor, &db, round)?))
}

/// Builds the proof bundle of a settled round (shared with tests).
pub fn proof_response(
    processor: &Processor,
    db: &Db,
    round: u32,
) -> Result<Option<ProofBundle>, EngineError> {
    match db.settlement(round)? {
        Some(row) => Ok(Some(processor.proof_bundle(db, &row)?)),
        None => Ok(None),
    }
}

/// Cache policy of data that can never change again.
pub const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// A proof bundle is final once its payout is mined (its openings are in)
/// or when it pays nothing; until then it may still gain the openings.
pub fn bundle_is_final(bundle: &ProofBundle) -> bool {
    bundle.payout_openings.is_some() || bundle.settlement.payouts.is_empty()
}

async fn proof(
    State(state): State<Arc<AppState>>,
    Path(round): Path<u32>,
) -> Result<Response, ApiError> {
    let db = reader(&state)?;
    match proof_response(&state.processor, &db, round)? {
        Some(bundle) => {
            let cache = if bundle_is_final(&bundle) {
                IMMUTABLE
            } else {
                "no-store"
            };
            Ok(([(axum::http::header::CACHE_CONTROL, cache)], Json(bundle)).into_response())
        }
        None => Err(ApiError(
            StatusCode::NOT_FOUND,
            format!("round {round} is not settled yet"),
        )),
    }
}

async fn vault_ledger(State(state): State<Arc<AppState>>) -> ApiResult<Ledger> {
    let Some(view) = &state.ledger else {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "this engine is not configured as the payout coordinator".into(),
        ));
    };
    Ok(Json(view.ledger(&state.store, &state.processor)?))
}

async fn health(State(state): State<Arc<AppState>>) -> ApiResult<HealthResponse> {
    let db = reader(&state)?;
    Ok(Json(HealthResponse {
        ok: true,
        tip_height: db.tip()?.map(|b| b.height),
        settled_rounds: db.settled_rounds()?.len(),
        uptime_secs: state.started.elapsed().as_secs(),
    }))
}

/// The engine's own state, for the status page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineStatus {
    pub uptime_secs: u64,
    /// Highest block stored.
    pub stored_tip: Option<u32>,
    /// Chain tip the servers last reported.
    pub chain_tip: Option<u32>,
    /// `nTime` of the highest stored block.
    pub last_block_time: Option<u64>,
    pub last_settled_round: Option<u32>,
}

/// `GET /v1/status`: everything the public status page shows. Each part
/// reports its own last success, so a stale or failing part is visible.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusResponse {
    /// Unix seconds when this was produced.
    pub now: u64,
    pub network: zeek_core::params::Network,
    pub digging_enabled: bool,
    pub engine: EngineStatus,
    pub threshold: u16,
    pub keyholders: Vec<KeyholderStatus>,
    pub chain_servers: Vec<zeek_vault::pool::ServerStatus>,
}

async fn status(State(state): State<Arc<AppState>>) -> ApiResult<StatusResponse> {
    let db = reader(&state)?;
    let tip = db.tip()?;
    Ok(Json(StatusResponse {
        now: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        network: state.processor.config().params.network,
        digging_enabled: state.digging,
        engine: EngineStatus {
            uptime_secs: state.started.elapsed().as_secs(),
            stored_tip: tip.as_ref().map(|b| b.height),
            chain_tip: db.chain_tip()?,
            last_block_time: tip.map(|b| b.time),
            last_settled_round: db.last_settled_round()?,
        },
        threshold: state.threshold,
        keyholders: keyholder_statuses(&state, &db)?,
        chain_servers: state
            .chain_servers
            .as_ref()
            .map(|p| p.status())
            .unwrap_or_default(),
    }))
}

/// `GET /v1/rounds` query.
#[derive(Clone, Debug, Deserialize)]
pub struct RoundsQuery {
    /// Only rounds below this one (pagination).
    pub before: Option<u32>,
    pub limit: Option<u32>,
    /// `1` to include rounds nobody dug in.
    pub include_empty: Option<u8>,
}

/// One past round, amounts only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundSummary {
    pub round: u32,
    /// `paid` (winners paid), `paying` (payout being signed or not yet
    /// broadcast), `no_winner` (nobody dug the treasure tile: the prize went
    /// to the Deep Cache), `voided` (closed too late to be safe: refunded).
    pub status: String,
    pub treasure_tile: Option<u8>,
    pub cache_triggered: bool,
    /// Sum of the valid digs after dig fees.
    pub pot: u64,
    /// What the winners split (after the rake).
    pub prize: u64,
    pub total_paid: u64,
    pub winners: usize,
    pub refunds: usize,
    /// Unix time of the window's last block, if stored.
    pub closed_time: Option<u64>,
    /// Unix time the beacon was published.
    pub revealed_time: u64,
    pub payout_txid: Option<String>,
    pub rules_version: u32,
}

/// `GET /v1/rounds`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundsResponse {
    pub rounds: Vec<RoundSummary>,
    pub next_before: Option<u32>,
}

/// Summarises a settled round.
pub fn round_summary(
    db: &Db,
    processor: &Processor,
    row: &crate::db::SettlementRow,
) -> Result<RoundSummary, EngineError> {
    let s = &row.settlement;
    let (treasure_tile, cache_triggered) = match row.outcome {
        RoundOutcome::Voided => (None, false),
        RoundOutcome::Drawn {
            treasure_tile,
            cache_triggered,
        } => (Some(treasure_tile), cache_triggered),
    };
    let payout = db.payout(row.round)?;
    let refunds = s
        .payouts
        .iter()
        .filter(|p| matches!(p.reason, zeek_core::settle::PayoutReason::Refund(_)))
        .count();
    let status = match row.outcome {
        RoundOutcome::Voided => "voided",
        RoundOutcome::Drawn { .. } if s.payouts.is_empty() => "no_winner",
        RoundOutcome::Drawn { .. } if s.winners == 0 => "no_winner",
        RoundOutcome::Drawn { .. } => match &payout {
            Some(p) if p.state == crate::db::PayoutState::Broadcast => "paid",
            _ => "paying",
        },
    };
    let window = zeek_core::rounds::round_window(&processor.config().params, row.round)?;
    Ok(RoundSummary {
        round: row.round,
        status: status.to_owned(),
        treasure_tile,
        cache_triggered,
        pot: s.pot,
        prize: s.distributable,
        total_paid: s.total_paid,
        winners: s.winners,
        refunds,
        closed_time: db.block(window.last())?.map(|b| b.time),
        revealed_time: row.beacon_time,
        payout_txid: payout.and_then(|p| p.txid),
        rules_version: row.rules_version,
    })
}

async fn rounds_list(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<RoundsQuery>,
) -> ApiResult<RoundsResponse> {
    let limit = q.limit.unwrap_or(20).clamp(1, 100) as usize;
    let include_empty = q.include_empty == Some(1);
    let db = reader(&state)?;
    let Some(last) = db.last_settled_round()? else {
        return Ok(Json(RoundsResponse {
            rounds: Vec::new(),
            next_before: None,
        }));
    };
    let mut round = q
        .before
        .map_or(Some(last), |b| b.checked_sub(1).map(|r| r.min(last)));
    let mut rounds = Vec::new();
    while let Some(r) = round {
        if rounds.len() == limit {
            break;
        }
        if let Some(row) = db.settlement(r)? {
            let s = &row.settlement;
            let empty = s.pot == 0 && s.forfeited == 0 && s.shouts == 0 && s.payouts.is_empty();
            if include_empty || !empty {
                rounds.push(round_summary(&db, &state.processor, &row)?);
            }
        }
        round = r.checked_sub(1);
    }
    let next_before = if rounds.len() == limit {
        rounds.last().map(|r| r.round)
    } else {
        None
    };
    Ok(Json(RoundsResponse {
        rounds,
        next_before,
    }))
}

/// One note the vault received in a round, as anyone with the public UIVK
/// sees it, with its memo reduced to a digest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteDigest {
    pub height: u32,
    pub value: u64,
    /// SHA-256 of the 512-byte memo field (hex): lets a wallet that knows
    /// its memo confirm the payment landed without the engine learning
    /// anything new.
    pub memo_sha256: String,
}

/// `GET /v1/rounds/{n}/digests`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestsResponse {
    pub round: u32,
    pub notes: Vec<NoteDigest>,
}

async fn round_digests(
    State(state): State<Arc<AppState>>,
    Path(round): Path<u32>,
) -> ApiResult<DigestsResponse> {
    use sha2::{Digest, Sha256};
    let window = zeek_core::rounds::round_window(&state.processor.config().params, round)
        .map_err(|e| bad(StatusCode::BAD_REQUEST, e.to_string()))?;
    let db = reader(&state)?;
    // Late payments land after the window; include the next round's window
    // too so a wallet can see where its payment went.
    let notes = db
        .notes_in_range(
            window.start,
            window.end.saturating_add(window.end - window.start),
        )?
        .into_iter()
        .map(|n| NoteDigest {
            height: n.height,
            value: n.value,
            memo_sha256: hex::encode(Sha256::digest(&n.memo)),
        })
        .collect();
    Ok(Json(DigestsResponse { round, notes }))
}

/// One keyholder's public status.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyholderStatus {
    pub identifier: u16,
    /// "Keyholder A", "B", …
    pub label: String,
    pub operator: String,
    pub reachable: Option<bool>,
    /// Unix seconds of the last successful health check.
    pub last_seen: Option<u64>,
    /// Unix seconds of the last health check.
    pub last_check: Option<u64>,
    /// Signing requests it approved / refused (recorded since Phase 7a).
    pub approved: u64,
    pub refused: u64,
}

/// A refused payout attempt (one signing session).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusedAttempt {
    pub time: u64,
    pub rounds: Vec<u32>,
    /// What the keyholders said.
    pub reason: String,
    /// How many keyholders refused it.
    pub refusals: usize,
}

/// `GET /v1/keyholders/status`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyholdersResponse {
    pub threshold: u16,
    pub keyholders: Vec<KeyholderStatus>,
    pub refused_attempts: Vec<RefusedAttempt>,
}

/// Every configured keyholder's public status.
fn keyholder_statuses(state: &AppState, db: &Db) -> Result<Vec<KeyholderStatus>, EngineError> {
    let health = db.keyholder_health()?;
    let counts = db.keyholder_counts()?;
    Ok(state
        .keyholder_ids
        .iter()
        .map(|id| {
            let h = health.iter().find(|h| h.keyholder == *id);
            let (approved, refused) = counts
                .iter()
                .find(|c| c.0 == *id)
                .map_or((0, 0), |c| (c.1, c.2));
            let letter =
                char::from(b'A'.saturating_add(u8::try_from(id.saturating_sub(1)).unwrap_or(25)));
            KeyholderStatus {
                identifier: *id,
                label: format!("Keyholder {letter}"),
                operator: "Zeek team".to_owned(),
                reachable: h.map(|h| h.reachable),
                last_seen: h.and_then(|h| h.last_seen),
                last_check: h.map(|h| h.last_check),
                approved,
                refused,
            }
        })
        .collect())
}

async fn keyholders_status(State(state): State<Arc<AppState>>) -> ApiResult<KeyholdersResponse> {
    let db = reader(&state)?;
    let keyholders = keyholder_statuses(&state, &db)?;
    let mut refused_attempts: Vec<RefusedAttempt> = Vec::new();
    let mut sessions: Vec<String> = Vec::new();
    for event in db.keyholder_events(500)? {
        if event.outcome != crate::db::KeyholderOutcome::Refused {
            continue;
        }
        match sessions.iter().position(|s| *s == event.session) {
            Some(i) => refused_attempts[i].refusals += 1,
            None => {
                sessions.push(event.session.clone());
                refused_attempts.push(RefusedAttempt {
                    time: event.time,
                    rounds: event.rounds.clone(),
                    reason: event.reason.clone(),
                    refusals: 1,
                });
            }
        }
    }
    refused_attempts.truncate(50);
    Ok(Json(KeyholdersResponse {
        threshold: state.threshold,
        keyholders,
        refused_attempts,
    }))
}

async fn campfire(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<CampfireQuery>,
) -> ApiResult<CampfireResponse> {
    let limit = q.limit.unwrap_or(50);
    let db = reader(&state)?;
    let messages = crate::campfire::messages(
        &db,
        &state.processor.config().params,
        &state.campfire_filter,
        q.before,
        limit,
        q.include_hidden == Some(1),
    )?;
    let next_before = (messages.len() >= limit.clamp(1, crate::campfire::MAX_LIMIT))
        .then(|| messages.last().map(|m| m.height))
        .flatten();
    Ok(Json(CampfireResponse {
        messages,
        next_before,
    }))
}

fn bad(status: StatusCode, message: impl Into<String>) -> ApiError {
    ApiError(status, message.into())
}

/// Validates a win proof completely (against this engine's bundle of the
/// round and the payout transaction fetched from its chain source) before
/// storing it; proofs are public data.
async fn submit_proof(
    State(state): State<Arc<AppState>>,
    client: Option<axum::Extension<crate::edge::ClientIp>>,
    Json(body): Json<SubmitProof>,
) -> Result<(StatusCode, Json<StoredProof>), ApiError> {
    use base64::Engine;
    let address = client.map_or(std::net::IpAddr::from([0, 0, 0, 0]), |c| c.0 .0);
    if !state.proof_limiter.allow(address, Instant::now()).await {
        return Err(bad(
            StatusCode::TOO_MANY_REQUESTS,
            "too many proof submissions; retry in a minute",
        ));
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(body.proof.trim())
        .map_err(|_| bad(StatusCode::BAD_REQUEST, "the proof is not base64url"))?;
    let proof = zeek_verify::win::WinProof::decode(&bytes)
        .map_err(|e| bad(StatusCode::BAD_REQUEST, e.to_string()))?;
    let id = zeek_verify::win::WinProof::id(&bytes);
    // A proof already verified is answered from the store (review N6).
    if let Some(stored) = stored_proof(&state, &id)? {
        return Ok((StatusCode::OK, Json(stored)));
    }
    // At most two verifications at once: they share the blocking pool with
    // payout proving (review N6).
    let Ok(_slot) = PROOF_VERIFICATIONS.try_acquire() else {
        return Err(bad(
            StatusCode::SERVICE_UNAVAILABLE,
            "busy verifying other proofs; try again in a few seconds",
        ));
    };
    let bundle = {
        let db = reader(&state)?;
        let row = db.settlement(proof.round)?.ok_or_else(|| {
            bad(
                StatusCode::NOT_FOUND,
                format!("round {} is not settled", proof.round),
            )
        })?;
        state.processor.proof_bundle(&db, &row)?
    };
    let relay = state.relay.as_ref().ok_or_else(|| {
        bad(
            StatusCode::SERVICE_UNAVAILABLE,
            "this engine has no chain source for verification",
        )
    })?;
    let txid = zeek_verify::chain::txid_bytes(&proof.payout_txid)
        .ok_or_else(|| bad(StatusCode::BAD_REQUEST, "malformed payout txid"))?;
    let raw = relay
        .backend()
        .transaction(txid)
        .await
        .map_err(|e| bad(StatusCode::BAD_GATEWAY, e.to_string()))?;
    let height = u32::try_from(raw.height).unwrap_or(0);
    let blocks = relay
        .backend()
        .blocks(height, height)
        .await
        .map_err(|e| bad(StatusCode::BAD_GATEWAY, e.to_string()))?;
    let included = height > 0 && zeek_verify::chain::Blocks::new(blocks).contains_tx(height, &txid);
    let mined = zeek_verify::chain::MinedTx {
        raw: raw.data,
        height,
    };
    let verdict = tokio::task::spawn_blocking(move || {
        zeek_verify::win::verify(&proof, &bundle, &mined, included)
    })
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "proof verification task failed");
        internal()
    })?;
    let round = verdict_round(&bytes);
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stored = StoredProof {
        id: id.clone(),
        round,
        created_at,
        proof: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes),
        verdict: verdict.clone(),
    };
    if !verdict.valid {
        return Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(stored)));
    }
    state
        .store
        .write(|db| db.insert_win_proof(&id, round, created_at, &bytes))?;
    Ok((StatusCode::CREATED, Json(stored)))
}

fn verdict_round(bytes: &[u8]) -> u32 {
    zeek_verify::win::WinProof::decode(bytes).map_or(0, |p| p.round)
}

/// A stored (verified) win proof, restated.
fn stored_proof(state: &AppState, id: &str) -> Result<Option<StoredProof>, ApiError> {
    use base64::Engine;
    let Some((round, created_at, bytes)) = state.store.read(|db| db.win_proof(id))? else {
        return Ok(None);
    };
    // Stored proofs were fully verified on submission; the stored verdict
    // is restated here (the claim depends only on public data).
    let proof = zeek_verify::win::WinProof::decode(&bytes).map_err(|e| {
        tracing::error!(error = %e, "a stored proof does not decode");
        internal()
    })?;
    Ok(Some(StoredProof {
        id: id.to_owned(),
        round,
        created_at,
        proof: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes),
        verdict: zeek_verify::win::WinVerdict {
            valid: true,
            claim: format!(
                "Verified on submission: the holder controls the address that received round {}'s treasure payout {} (transaction {}).",
                proof.round, proof.payout_index, proof.payout_txid
            ),
            reason: None,
            steps: Vec::new(),
        },
    }))
}

async fn get_proof(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    if id.len() != 32 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "a proof id is 32 hex characters",
        ));
    }
    // A stored proof was verified once and never changes.
    stored_proof(&state, &id)?
        .map(|p| ([(axum::http::header::CACHE_CONTROL, IMMUTABLE)], Json(p)).into_response())
        .ok_or_else(|| bad(StatusCode::NOT_FOUND, "no such proof"))
}

/// Proof verifications allowed at once.
static PROOF_VERIFICATIONS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

/// Most simultaneous `/v1/stream` connections.
pub const MAX_STREAMS: usize = 2_000;
/// Most simultaneous `/v1/stream` connections of one client (review N10).
pub const MAX_STREAMS_PER_CLIENT: usize = 8;

/// Open streams per client.
static STREAMS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<std::net::IpAddr, usize>>,
> = std::sync::LazyLock::new(Default::default);

/// One client's open stream; released on drop.
struct StreamSlot(std::net::IpAddr);

impl StreamSlot {
    fn take(client: std::net::IpAddr) -> Option<StreamSlot> {
        let mut open = STREAMS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let count = open.entry(client).or_insert(0);
        if *count >= MAX_STREAMS_PER_CLIENT {
            return None;
        }
        *count += 1;
        Some(StreamSlot(client))
    }
}

impl Drop for StreamSlot {
    fn drop(&mut self) {
        let mut open = STREAMS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(count) = open.get_mut(&self.0) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                open.remove(&self.0);
            }
        }
    }
}

/// Largest client message accepted on `/v1/stream` (clients only listen).
const MAX_CLIENT_MESSAGE: usize = 1_024;

async fn stream(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    client: Option<axum::Extension<crate::edge::ClientIp>>,
) -> Response {
    let address = client.map_or(std::net::IpAddr::from([0, 0, 0, 0]), |c| c.0 .0);
    let Some(own) = StreamSlot::take(address) else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorBody {
                error: "too many live connections from this address".into(),
            }),
        )
            .into_response();
    };
    let Ok(slot) = state.stream_slots.clone().try_acquire_owned() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorBody {
                error: "too many live connections; the app falls back to polling".into(),
            }),
        )
            .into_response();
    };
    ws.max_message_size(MAX_CLIENT_MESSAGE)
        .max_frame_size(MAX_CLIENT_MESSAGE)
        .on_upgrade(move |socket| async move {
            handle_socket(socket, state).await;
            drop(slot);
            drop(own);
        })
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let initial = state
        .store
        .read(|db| state.processor.board(db))
        .map(Event::Board);
    let mut rx = state.events.subscribe();
    if let Ok(event) = initial {
        if send_event(&mut socket, &event).await.is_err() {
            return;
        }
    }
    let landing = Event::Landing {
        digs: state.landing.digs(),
    };
    if send_event(&mut socket, &landing).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    // Pings are answered by the library; the stream is
                    // one-way, so any data from the client ends it.
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    _ => return,
                }
            }
            event = rx.recv() => {
                match event {
                    Ok(event) => {
                        if send_event(&mut socket, &event).await.is_err() {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        }
    }
}

async fn send_event(socket: &mut WebSocket, event: &Event) -> Result<(), ()> {
    let text = serde_json::to_string(event).map_err(|_| ())?;
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(|_| ())
}
