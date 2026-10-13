//! `zeek-keyholder`: one of the vault's independent keyholders.
//!
//! A keyholder owns one FROST share of the vault key (produced by the DKG
//! ceremony in [`dkg`]), follows the chain itself with its own lightwalletd
//! endpoints and its own database ([`follower`]), recomputes every round's
//! settlement with `zeek-core`, and signs a payout only after [`verify`]
//! has checked that the PCZT pays exactly what its own settlement says and
//! nothing else. Every message it sends or accepts is signed by a pinned
//! ed25519 identity ([`identity`], [`protocol`]).
//!
//! Secrets live under the node's own secrets directory (identity key, FROST
//! key package, payout sealing key), mode 0600, and are never logged.

#![forbid(unsafe_code)]

pub mod api;
pub mod config;
pub mod db;
pub mod dkg;
pub mod error;
pub mod follower;
pub use zeek_engine::{identity, protocol};
pub mod signing;
pub mod verify;

pub use error::KeyholderError;
