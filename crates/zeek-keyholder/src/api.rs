//! The keyholder's HTTP API: DKG inbox, signing endpoints, health.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use zeek_engine::config::GameConfig;
use zeek_engine::processor::Processor;
use zeek_engine::store::Store;

use crate::config::{KeyholderConfig, COORDINATOR_ID};
use crate::db::NodeDb;
use crate::dkg::{DkgState, KeyMaterial};
use crate::error::KeyholderError;
use crate::signing::SignerState;
use crate::verify::{self, VerifyContext};
use zeek_engine::identity::Identity;
use zeek_engine::protocol::{
    self, kind, Envelope, Expect, Refusal, SignCommitments, SignPackages, SignRequest, SignShares,
};

/// Shared state.
pub struct AppState {
    pub config: KeyholderConfig,
    pub game: GameConfig,
    /// Classifies this node's notes for payout verification.
    pub processor: Processor,
    pub identity: Identity,
    /// The DKG inbox (also served alone by `zeek-keyholder dkg`).
    pub dkg: Arc<DkgNode>,
    pub signer: SignerState,
    pub keys: RwLock<Option<Arc<KeyMaterial>>>,
    /// This node's own chain view; the follower writes, handlers read.
    pub chain: Arc<Store>,
    pub node_db: Mutex<NodeDb>,
    /// This node's `lightwalletd` servers.
    pub chain_servers: Arc<zeek_vault::pool::Pool>,
}

/// `GET /v1/health`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Health {
    pub identifier: u16,
    pub has_key_share: bool,
    pub vault_address: Option<String>,
    pub tip_height: Option<u32>,
    pub settled_rounds: usize,
    pub open_sessions: usize,
    pub lightwalletd: Vec<zeek_vault::pool::ServerStatus>,
}

/// JSON error body.
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}

enum Reply {
    Ok(Envelope),
    Refused(Envelope),
    Error(StatusCode, String),
}

impl IntoResponse for Reply {
    fn into_response(self) -> Response {
        match self {
            Reply::Ok(env) => (StatusCode::OK, Json(env)).into_response(),
            Reply::Refused(env) => (StatusCode::FORBIDDEN, Json(env)).into_response(),
            Reply::Error(status, error) => (status, Json(ErrorBody { error })).into_response(),
        }
    }
}

/// What the DKG endpoints need: this node's config and its inbox.
pub struct DkgNode {
    pub config: KeyholderConfig,
    pub state: DkgState,
}

/// The DKG endpoints alone.
pub fn dkg_router(node: Arc<DkgNode>) -> Router {
    Router::new()
        .route("/v1/dkg/round1", post(dkg_round1))
        .route("/v1/dkg/round2", post(dkg_round2))
        .route("/v1/dkg/confirm", post(dkg_confirm))
        .with_state(node)
}

/// Builds the router.
/// Builds the router; the DKG endpoints only when the config allows them.
pub fn router(state: Arc<AppState>) -> Router {
    let app = Router::new()
        .route("/v1/sign/request", post(sign_request))
        .route("/v1/sign/round2", post(sign_round2))
        .route("/v1/health", get(health))
        .with_state(state.clone());
    if state.config.dkg {
        app.merge(dkg_router(state.dkg.clone()))
    } else {
        app
    }
}

fn bad_request(e: impl std::fmt::Display) -> Response {
    tracing::warn!(error = %e, "rejected message");
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorBody {
            error: e.to_string(),
        }),
    )
        .into_response()
}

async fn dkg_round1(State(node): State<Arc<DkgNode>>, Json(env): Json<Envelope>) -> Response {
    match node.state.accept_round1(&node.config, &env).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => bad_request(e),
    }
}

async fn dkg_round2(State(node): State<Arc<DkgNode>>, Json(env): Json<Envelope>) -> Response {
    match node.state.accept_round2(&node.config, &env).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => bad_request(e),
    }
}

async fn dkg_confirm(State(node): State<Arc<DkgNode>>, Json(env): Json<Envelope>) -> Response {
    match node.state.accept_confirm(&node.config, &env).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => bad_request(e),
    }
}

fn refusal(state: &AppState, session: &protocol::SessionId, reason: String) -> Reply {
    tracing::warn!(session = %hex::encode(session), reason = %reason, "REFUSED to sign");
    match protocol::seal(
        &state.identity,
        session,
        state.config.identifier,
        kind::REFUSAL,
        &Refusal {
            reason: reason.clone(),
        },
    ) {
        Ok(env) => Reply::Refused(env),
        Err(e) => Reply::Error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn sign_request(State(state): State<Arc<AppState>>, Json(env): Json<Envelope>) -> Response {
    let opened = protocol::open::<SignRequest>(
        &env,
        &Expect {
            kind: kind::SIGN_REQUEST,
            sender: COORDINATOR_ID,
            sender_key: &state.config.coordinator_identity,
            session: None,
        },
    );
    let (session, request) = match opened {
        Ok(v) => v,
        Err(e) => return bad_request(e),
    };
    let keys = match state.keys.read().await.clone() {
        Some(k) => k,
        None => {
            return refusal(&state, &session, KeyholderError::NoKeyShare.to_string())
                .into_response()
        }
    };
    let node = state.node_db.lock().await;
    let chain = match state.chain.reader() {
        Ok(c) => c,
        Err(e) => return refusal(&state, &session, e.to_string()).into_response(),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let ctx = VerifyContext {
        now,
        processor: &state.processor,
        fvk: &keys.fvk,
        sealing: &keys.sealing,
        chain: &chain,
        node: &node,
    };
    let approved = match verify::verify_payout(&ctx, &request.pczt, &request.rounds) {
        Ok(a) => a,
        Err(KeyholderError::Refused(reason)) => {
            return refusal(&state, &session, reason).into_response()
        }
        Err(e) => return refusal(&state, &session, e.to_string()).into_response(),
    };
    drop(node);
    drop(chain);
    let commitments = match state
        .signer
        .open_session(session, approved, &keys.key_package)
    {
        Ok(c) => c,
        Err(e) => return refusal(&state, &session, e.to_string()).into_response(),
    };
    let serialized: Result<Vec<Vec<u8>>, _> = commitments.iter().map(|c| c.serialize()).collect();
    let serialized = match serialized {
        Ok(s) => s,
        Err(e) => {
            return Reply::Error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    };
    match protocol::seal(
        &state.identity,
        &session,
        state.config.identifier,
        kind::SIGN_COMMITMENTS,
        &SignCommitments {
            commitments: serialized,
        },
    ) {
        Ok(env) => Reply::Ok(env).into_response(),
        Err(e) => Reply::Error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn sign_round2(State(state): State<Arc<AppState>>, Json(env): Json<Envelope>) -> Response {
    let opened = protocol::open::<SignPackages>(
        &env,
        &Expect {
            kind: kind::SIGN_PACKAGES,
            sender: COORDINATOR_ID,
            sender_key: &state.config.coordinator_identity,
            session: None,
        },
    );
    let (session, packages) = match opened {
        Ok(v) => v,
        Err(e) => return bad_request(e),
    };
    let keys = match state.keys.read().await.clone() {
        Some(k) => k,
        None => {
            return refusal(&state, &session, KeyholderError::NoKeyShare.to_string())
                .into_response()
        }
    };
    let node = state.node_db.lock().await;
    let shares = match state
        .signer
        .round2(session, &packages.packages, &keys.key_package, &node)
    {
        Ok(s) => s,
        Err(e) => return refusal(&state, &session, e.to_string()).into_response(),
    };
    drop(node);
    let serialized: Vec<Vec<u8>> = shares.iter().map(|s| s.serialize()).collect();
    match protocol::seal(
        &state.identity,
        &session,
        state.config.identifier,
        kind::SIGN_SHARES,
        &SignShares { shares: serialized },
    ) {
        Ok(env) => Reply::Ok(env).into_response(),
        Err(e) => Reply::Error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn health(State(state): State<Arc<AppState>>) -> Response {
    let keys = state.keys.read().await.clone();
    let (tip_height, settled_rounds) = state
        .chain
        .read(|db| Ok::<_, zeek_engine::EngineError>((db.tip()?, db.settled_rounds()?)))
        .map_or((None, 0), |(tip, rounds)| {
            (tip.map(|b| b.height), rounds.len())
        });
    let body = Health {
        identifier: state.config.identifier,
        has_key_share: keys.is_some(),
        vault_address: keys.map(|k| k.vault.address.clone()),
        tip_height,
        settled_rounds,
        open_sessions: state.signer.open_sessions(),
        lightwalletd: state.chain_servers.status(),
    };
    (StatusCode::OK, Json(body)).into_response()
}
