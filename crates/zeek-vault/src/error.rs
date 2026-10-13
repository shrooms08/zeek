//! Error type for the vault library.

use std::path::PathBuf;

use zcash_protocol::value::Zatoshis;

/// Every failure the vault library can report.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error(
        "invalid threshold parameters: need 2 <= threshold <= signers, got threshold {threshold} and signers {signers}"
    )]
    InvalidThreshold { threshold: u16, signers: u16 },

    #[error("FROST error: {0}")]
    Frost(#[from] crate::frost::FrostError),

    #[error(
        "the FROST group verifying key has an odd y-coordinate and cannot be used as an Orchard ak"
    )]
    OddGroupKey,

    #[error("the FROST group verifying key is not a valid Orchard spend validating key")]
    InvalidGroupKey,

    #[error("could not assemble an Orchard full viewing key from the group key")]
    InvalidFullViewingKey,

    #[error("key derivation failed: {0}")]
    KeyDerivation(String),

    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not parse {path} as JSON: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("refusing to overwrite existing key material at {0}; pass --force to replace it")]
    KeyFileExists(PathBuf),

    #[error("no vault found at {0}; run `zeek-vault keygen` first")]
    VaultNotFound(PathBuf),

    #[error("keyholder file not found at {0}")]
    KeyholderNotFound(PathBuf),

    #[error("field `{field}` is not valid hex: {source}")]
    Hex {
        field: &'static str,
        #[source]
        source: hex::FromHexError,
    },

    #[error("field `{field}` has length {actual}, expected {expected}")]
    BadLength {
        field: &'static str,
        expected: usize,
        actual: usize,
    },

    #[error("vault file was written for network `{0}`, but this build only supports `test`")]
    WrongVaultNetwork(String),

    #[error("keyholder file {path} does not belong to this vault (group verifying key mismatch)")]
    ForeignKeyholder { path: PathBuf },

    #[error("keyholder identifier {0} is out of range (vault has signers 1..={1})")]
    IdentifierOutOfRange(u16, u16),

    #[error("not enough signers: {given} selected but the threshold is {threshold}")]
    NotEnoughSigners { given: usize, threshold: u16 },

    #[error("signer {0} was selected more than once")]
    DuplicateSigner(u16),

    #[error("signer {0} is not a member of this vault")]
    UnknownSigner(u16),

    #[error("aggregated FROST signature failed verification against the randomized verifying key")]
    SignatureVerification,

    #[error("randomized verifying key for action {index} does not match the action's rk")]
    RkMismatch { index: usize },

    #[error("Orchard action {index} is missing its spend-auth randomizer (alpha)")]
    MissingAlpha { index: usize },

    #[error("Orchard action {index} has an invalid spend-auth randomizer (alpha)")]
    InvalidAlpha { index: usize },

    #[error("Orchard action {index} still carries a dummy spending key; refusing to sign")]
    DummyKeyPresent { index: usize },

    #[error("the PCZT contains no Orchard spends to sign")]
    NothingToSign,

    #[error("configuration error: {0}")]
    Config(String),

    #[error("lightwalletd transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    #[error("lightwalletd RPC error: {0}")]
    Rpc(#[from] tonic::Status),

    #[error("lightwalletd is serving chain `{0}` but this vault only works on `test`")]
    WrongNetwork(String),

    #[error("lightwalletd returned an unusable response: {0}")]
    BadResponse(String),

    #[error("wallet database error: {0}")]
    Wallet(String),

    #[error("chain sync failed: {0}")]
    Sync(String),

    #[error("the wallet has no account for this vault; run `zeek-vault sync` first")]
    AccountMissing,

    #[error("insufficient funds: {required} zatoshis required but only {available} spendable")]
    InsufficientFunds { available: u64, required: u64 },

    #[error("invalid destination address: {0}")]
    InvalidAddress(String),

    #[error("destination address has no Orchard receiver; this vault only sends to Orchard")]
    NoOrchardReceiver,

    #[error("invalid ZEC amount `{0}`: expected a decimal with at most 8 fractional digits")]
    InvalidAmount(String),

    #[error("memo is invalid: {0}")]
    InvalidMemo(String),

    #[error("transaction building failed: {0}")]
    Build(String),

    #[error("proof generation failed: {0}")]
    Prove(String),

    #[error("PCZT error: {0}")]
    Pczt(String),

    #[error("the current consensus branch has no supported Orchard circuit; refusing to prove")]
    UnsupportedBranch,

    #[error("lightwalletd rejected the transaction (code {code}): {message}")]
    BroadcastRejected { code: i32, message: String },
}

impl VaultError {
    /// Builds an I/O error carrying the path that was being accessed.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        VaultError::Io {
            path: path.into(),
            source,
        }
    }

    /// Builds an insufficient-funds error from wallet amounts.
    pub fn insufficient(available: Zatoshis, required: Zatoshis) -> Self {
        VaultError::InsufficientFunds {
            available: available.into_u64(),
            required: required.into_u64(),
        }
    }
}
