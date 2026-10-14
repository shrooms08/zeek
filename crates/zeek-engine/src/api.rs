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
use tokio::sync::{broadcast, Mutex, MutexGuard};
use tower_http::cors::{AllowOrigin, CorsLayer};
use zeek_core::board::TileTotals;
use zeek_core::digs;
use zeek_core::params::GameParams;
use zeek_core::rounds::{RoundContext, RoundStatus, RoundWindow};
use zeek_core::seal::SealingPublicKey;
use zeek_core::settle::{Payout, RoundOutcome};

use crate::config::{GameConfig, SETTLE_CONFIRMATIONS};
use crate::coordinator::Coordinator;
use crate::db::{Db, PayoutRecord, PlanStatus};
use crate::error::EngineError;
use crate::ledger::Ledger;
use crate::processor::{BoardState, Event, PendingRound, Processor};
use crate::proof::ProofBundle;

/// Shared state of the HTTP handlers.
pub struct AppState {
    pub processor: Processor,
    pub db: Mutex<Db>,
    pub events: broadcast::Sender<Event>,
    pub started: Instant,
    /// Present when this engine also coordinates payouts.
    pub coordinator: Option<Arc<Mutex<Coordinator>>>,
    /// Present when this engine relays chain data to browser wallets.
    pub relay: Option<Arc<crate::relay::RelayState>>,
    /// Campfire display filter.
    pub campfire_filter: crate::campfire::Filter,
    /// Rate limit of win proof submissions (verification costs about a
    /// second of CPU each).
    pub proof_limiter: crate::relay::RateLimiter,
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
    pub vault_address: String,
    pub sealing_public_key: SealingPublicKey,
    pub uivk: String,
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
    fn from(e: EngineError) -> Self {
        tracing::error!(error = %e, "request failed");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(ErrorBody { error: self.1 })).into_response()
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

async fn lock(state: &AppState) -> MutexGuard<'_, Db> {
    state.db.lock().await
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
        .route("/v1/vault/ledger", get(vault_ledger))
        .route("/v1/stream", get(stream))
        .route("/v1/campfire", get(campfire))
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

/// The public config response.
pub fn config_response(config: &GameConfig) -> ConfigResponse {
    ConfigResponse {
        params: config.params.clone(),
        vault_address: config.vault_address.clone(),
        sealing_public_key: config.sealing_public_key.clone(),
        uivk: config.uivk.clone(),
        genesis_height: config.genesis_height,
        settle_confirmations: SETTLE_CONFIRMATIONS,
        initial_cache_zat: config.initial_cache_zat,
    }
}

async fn config(State(state): State<Arc<AppState>>) -> ApiResult<ConfigResponse> {
    Ok(Json(config_response(state.processor.config())))
}

async fn board(State(state): State<Arc<AppState>>) -> ApiResult<BoardState> {
    let db = lock(&state).await;
    Ok(Json(state.processor.board(&db)?))
}

async fn pending(State(state): State<Arc<AppState>>) -> ApiResult<PendingResponse> {
    let db = lock(&state).await;
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
    let db = lock(&state).await;
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

async fn proof(
    State(state): State<Arc<AppState>>,
    Path(round): Path<u32>,
) -> ApiResult<ProofBundle> {
    let db = lock(&state).await;
    match proof_response(&state.processor, &db, round)? {
        Some(bundle) => Ok(Json(bundle)),
        None => Err(ApiError(
            StatusCode::NOT_FOUND,
            format!("round {round} is not settled yet"),
        )),
    }
}

async fn vault_ledger(State(state): State<Arc<AppState>>) -> ApiResult<Ledger> {
    let Some(coordinator) = &state.coordinator else {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "this engine is not configured as the payout coordinator".into(),
        ));
    };
    let db = lock(&state).await;
    let coordinator = coordinator.lock().await;
    Ok(Json(coordinator.ledger(&db, &state.processor)?))
}

async fn health(State(state): State<Arc<AppState>>) -> ApiResult<HealthResponse> {
    let db = lock(&state).await;
    Ok(Json(HealthResponse {
        ok: true,
        tip_height: db.tip()?.map(|b| b.height),
        settled_rounds: db.settled_rounds()?.len(),
        uptime_secs: state.started.elapsed().as_secs(),
    }))
}

async fn campfire(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<CampfireQuery>,
) -> ApiResult<CampfireResponse> {
    let limit = q.limit.unwrap_or(50);
    let db = lock(&state).await;
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
    connect: Option<axum::Extension<axum::extract::ConnectInfo<std::net::SocketAddr>>>,
    Json(body): Json<SubmitProof>,
) -> Result<(StatusCode, Json<StoredProof>), ApiError> {
    use base64::Engine;
    let address = connect.map_or(std::net::IpAddr::from([0, 0, 0, 0]), |c| c.0 .0.ip());
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
    let bundle = {
        let db = lock(&state).await;
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
    .map_err(|e| bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
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
    lock(&state)
        .await
        .insert_win_proof(&id, round, created_at, &bytes)?;
    Ok((StatusCode::CREATED, Json(stored)))
}

fn verdict_round(bytes: &[u8]) -> u32 {
    zeek_verify::win::WinProof::decode(bytes).map_or(0, |p| p.round)
}

async fn get_proof(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<StoredProof>, ApiError> {
    use base64::Engine;
    if id.len() != 32 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "a proof id is 32 hex characters",
        ));
    }
    let (round, created_at, bytes) = lock(&state)
        .await
        .win_proof(&id)?
        .ok_or_else(|| bad(StatusCode::NOT_FOUND, "no such proof"))?;
    // Stored proofs were fully verified on submission; the stored verdict
    // is restated here (the claim depends only on public data).
    let proof = zeek_verify::win::WinProof::decode(&bytes)
        .map_err(|e| bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(StoredProof {
        id,
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

async fn stream(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let initial = {
        let db = lock(&state).await;
        state.processor.board(&db).map(Event::Board)
    };
    let mut rx = state.events.subscribe();
    if let Ok(event) = initial {
        if send_event(&mut socket, &event).await.is_err() {
            return;
        }
    }
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                    Some(Ok(_)) => {}
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
