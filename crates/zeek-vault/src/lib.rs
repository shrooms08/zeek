//! `zeek-vault`: a 2-of-3 FROST threshold key that controls a Zcash Orchard
//! address and spends from it on **testnet**.
//!
//! The crate is organised as a pipeline:
//!
//! * [`frost`] generates the threshold key (trusted dealer for this spike) and
//!   runs the rerandomized FROST signing rounds over the RedPallas ciphersuite.
//! * [`keys`] maps the FROST group verifying key onto an Orchard full viewing
//!   key (`ak`, `nk`, `rivk`) and derives the unified viewing key and address.
//! * [`store`] persists one key file per keyholder plus a public vault file.
//! * [`lightwalletd`] and [`wallet`] talk to a `lightwalletd` server and keep a
//!   view-only `zcash_client_sqlite` wallet in sync.
//! * [`send`] builds a PCZT, proves it, FROST-signs every Orchard spend with
//!   the correct per-action randomizer, extracts the transaction and
//!   broadcasts it.
//!
//! # What is simulated in this spike
//!
//! The three keyholders do not run as separate processes. Each keyholder's
//! share lives in its own file under the secrets directory, and `send` loads
//! only the files of the signers that were selected. The FROST rounds are then
//! executed in a single process, with each selected keyholder's [`KeyPackage`]
//! held in a separate value. Production will replace this with a networked
//! coordinator and distributed key generation (DKG).
//!
//! [`KeyPackage`]: frost::KeyPackage

pub mod config;
pub mod error;
pub mod frost;
pub mod keys;
pub mod lightwalletd;
pub mod send;
pub mod store;
pub mod wallet;

pub use error::VaultError;

/// The only network this spike supports.
pub const NETWORK: zcash_protocol::consensus::TestNetwork = zcash_protocol::consensus::TEST_NETWORK;
