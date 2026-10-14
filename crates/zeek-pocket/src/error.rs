//! Errors of the wallet core. Each carries a stable `code` the TypeScript
//! layer maps to a typed error, and a message written for people.

/// Everything the wallet core can report.
#[derive(Debug, thiserror::Error)]
pub enum PocketError {
    #[error("wrong passphrase, or the stored wallet is damaged")]
    BadPassphrase,
    #[error("the passphrase must be at least {0} characters")]
    WeakPassphrase(usize),
    #[error("that is not a valid 24-word recovery phrase: {0}")]
    InvalidMnemonic(String),
    #[error("not enough spendable funds: need {needed} zatoshis, {spendable} spendable now ({pending} more waiting for confirmations)")]
    InsufficientFunds {
        needed: u64,
        spendable: u64,
        pending: u64,
    },
    #[error("round {round} closes in {blocks_remaining} blocks; digging needs at least {margin} blocks of margin, wait for the next round")]
    ClosingTooSoon {
        round: u32,
        blocks_remaining: u32,
        margin: u32,
    },
    #[error(
        "the wallet is not synced to the chain tip (scanned {scanned}, tip {tip}); sync first"
    )]
    NotSynced { scanned: u32, tip: u32 },
    #[error("invalid address: {0}")]
    InvalidAddress(String),
    #[error("invalid amount: {0}")]
    InvalidAmount(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("the chain reorganised below block {fork_height}; rewind to it and sync again")]
    Reorg { fork_height: u32 },
    #[error("chain data is malformed: {0}")]
    BadChainData(String),
    #[error("the wallet state is corrupt: {0}")]
    Corrupt(String),
    #[error("building the transaction failed: {0}")]
    Build(String),
    #[error("arithmetic overflow in {0}")]
    Overflow(&'static str),
}

impl PocketError {
    /// Stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            PocketError::BadPassphrase => "bad_passphrase",
            PocketError::WeakPassphrase(_) => "weak_passphrase",
            PocketError::InvalidMnemonic(_) => "invalid_mnemonic",
            PocketError::InsufficientFunds { .. } => "insufficient_funds",
            PocketError::ClosingTooSoon { .. } => "closing_too_soon",
            PocketError::NotSynced { .. } => "not_synced",
            PocketError::InvalidAddress(_) => "invalid_address",
            PocketError::InvalidAmount(_) => "invalid_amount",
            PocketError::InvalidRequest(_) => "invalid_request",
            PocketError::Reorg { .. } => "reorg",
            PocketError::BadChainData(_) => "bad_chain_data",
            PocketError::Corrupt(_) => "corrupt_state",
            PocketError::Build(_) => "build_failed",
            PocketError::Overflow(_) => "overflow",
        }
    }
}
