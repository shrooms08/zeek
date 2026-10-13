//! Error type of the engine library.

use std::path::PathBuf;

/// Every failure the engine can report.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not parse {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("database row is corrupt: {0}")]
    Corrupt(String),

    #[error("lightwalletd: {0}")]
    Lightwalletd(#[from] zeek_vault::VaultError),

    #[error("lightwalletd returned block {got} where {expected} was requested")]
    UnexpectedBlock { expected: u32, got: u64 },

    #[error("compact block {height} is malformed: {reason}")]
    MalformedBlock { height: u32, reason: String },

    #[error("transaction {txid} could not be parsed: {reason}")]
    MalformedTransaction { txid: String, reason: String },

    #[error("reorg deeper than the stored chain (fork below height {0}); delete the database and resync")]
    ReorgTooDeep(u32),

    #[error("game rule error: {0}")]
    Rounds(#[from] zeek_core::rounds::RoundError),

    #[error("settlement error: {0}")]
    Settle(#[from] zeek_core::settle::SettleError),

    #[error("timing rule error: {0}")]
    Timing(#[from] zeek_core::timing::TimingError),

    #[error("beacon error: {0}")]
    Beacon(#[from] zeek_core::beacon::BeaconError),

    #[error("draw error: {0}")]
    Random(#[from] zeek_core::random::RandomError),

    #[error("no relay returned a verifiable beacon for drand round {round}: {attempts}")]
    BeaconUnavailable { round: u64, attempts: String },

    #[error("relay request failed: {0}")]
    Relay(String),

    #[error("round {round} cannot be settled before round {missing} is settled")]
    SettlementOrder { round: u32, missing: u32 },

    #[error("arithmetic overflow in {0}")]
    Overflow(&'static str),

    #[error("server error: {0}")]
    Server(String),

    #[error("message rejected: {0}")]
    Protocol(String),

    #[error("keyholder {identifier} refused to sign: {reason}")]
    Refused { identifier: u16, reason: String },

    #[error("not enough keyholders approved the payout: {approved} of {needed} ({details})")]
    NotEnoughSigners {
        approved: usize,
        needed: usize,
        details: String,
    },

    #[error("payout for rounds {rounds:?} would need {deficit} zatoshis more than the fees collected, but the operator float is only {float}; refusing to build it")]
    OperatorFloatExhausted {
        rounds: Vec<u32>,
        deficit: u64,
        float: u64,
    },

    #[error("rounds {0:?} already have a payout transaction")]
    AlreadyPaid(Vec<u32>),
}

impl EngineError {
    /// Builds an I/O error carrying the path that was being accessed.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        EngineError::Io {
            path: path.into(),
            source,
        }
    }
}
