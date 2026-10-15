//! Independent verification of Zeek rounds and win proofs.
//!
//! The library does no I/O: callers fetch chain data from wherever they
//! trust (the `zeek-verify` CLI talks gRPC to any lightwalletd; the browser
//! verifier in `packages/zeek-pocket` fetches through a relay and
//! cross-checks block hashes with a second source) and hand it in. Nothing
//! here needs anything from Zeek except the round's proof bundle and the
//! game's public configuration (parameters, vault address, UIVK).
//!
//! * [`checks`]: the five round checks (beacon, digs, timing, settlement,
//!   payout), each with a plain-language verdict and technical detail.
//! * [`win`]: proofs that a payout was received at an address the prover
//!   controls ("prove-a-win").
//! * [`offline`]: the parts of the round checks that need no chain data
//!   (Replay mode, while the engine is off).
//! * [`replay`]: a round's depth stages block by block, for playback.

#![forbid(unsafe_code)]

pub mod chain;
pub mod checks;
pub mod offline;
pub mod replay;
pub mod report;
pub mod win;

pub use report::{CheckId, CheckResult, CheckStatus, RoundReport};
