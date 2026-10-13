//! Error type of the keyholder.

use std::path::PathBuf;

/// Every failure a keyholder can report.
#[derive(Debug, thiserror::Error)]
pub enum KeyholderError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("I/O error at {path}: {source}")]
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

    #[error("DKG ceremony failed: {0}")]
    Dkg(String),

    #[error("FROST error: {0}")]
    Frost(#[from] reddsa::frost::redpallas::Error),

    #[error(transparent)]
    Vault(#[from] zeek_vault::VaultError),

    #[error(transparent)]
    Engine(#[from] zeek_engine::EngineError),

    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("peer request failed: {0}")]
    Peer(String),

    #[error("refusing to sign: {0}")]
    Refused(String),

    #[error("no signing session with that id (expired, consumed, or never opened)")]
    UnknownSession,

    #[error("the node has no FROST key share yet; run the DKG ceremony first")]
    NoKeyShare,
}

impl KeyholderError {
    /// Builds an I/O error carrying the path that was being accessed.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        KeyholderError::Io {
            path: path.into(),
            source,
        }
    }
}

/// A protocol-level rejection (bad signature, session, sender or payload).
pub fn protocol(message: String) -> KeyholderError {
    KeyholderError::Engine(zeek_engine::EngineError::Protocol(message))
}
