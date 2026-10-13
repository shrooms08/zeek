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
    pub total_fees: u64,
    pub total_paid: u64,
    pub forfeited: u64,
    pub plan_status: PlanStatus,
    pub settled_at_height: u32,
    /// Payout transaction, once broadcast.
    pub payout: Option<PayoutRecord>,
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
        .with_state(state);
    if !cors_origins.is_empty() {
        let origins: Vec<HeaderValue> = cors_origins
            .iter()
            .filter_map(|o| HeaderValue::from_str(o).ok())
            .collect();
        router = router.layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(origins))
                .allow_methods([Method::GET]),
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
            total_paid: s.total_paid,
            forfeited: s.forfeited,
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
