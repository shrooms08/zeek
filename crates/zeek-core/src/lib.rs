//! `zeek-core`: the single source of truth for Zeek's game rules.
//!
//! Zeek is a 5x5 grid game on shielded Zcash (Ironwood pool). Each round,
//! players "dig" by sending shielded ZEC to the vault with a memo naming a
//! tile. When the round closes, a drand beacon picks the treasure tile and the
//! diggers on that tile split the pot pro rata; a rake feeds the Deep Cache
//! jackpot, which breaks open with 1-in-`cache_odds` odds per round.
//!
//! Everything here is **pure and deterministic**: no I/O, no clocks, no
//! networking, no randomness except what the caller passes in. The same code
//! runs in the round engine, in each keyholder node and in a browser verifier,
//! so it also compiles to `wasm32-unknown-unknown`. All money is `u64`
//! zatoshis under checked arithmetic.
//!
//! Module map, in the order a round flows through them:
//!
//! * [`params`] — per-network [`GameParams`](params::GameParams).
//! * [`rounds`] — height ↔ round arithmetic and [`RoundStatus`](rounds::RoundStatus).
//! * [`memo`] / [`seal`] / [`payment`] — the dig memo (ZIP 302 arbitrary
//!   data), the HPKE-sealed payout receiver, and the ZIP 321 payment URI.
//! * [`beacon`] — drand quicknet verification and the binding of each game
//!   round to one drand round.
//! * [`timing`] — the median-time-past rule that voids a round whose window
//!   cannot be shown to have closed before its beacon was published.
//! * [`random`] — unbiased treasure-tile and Deep Cache draws.
//! * [`digs`] — classification of received notes into valid digs, refunds and
//!   forfeits.
//! * [`settle`] — the settlement function and its conservation invariant.
//! * [`board`] — the public board views.

#![forbid(unsafe_code)]

pub mod beacon;
pub mod board;
pub mod digs;
pub mod memo;
pub mod params;
pub mod payment;
pub mod random;
pub mod rounds;
pub mod seal;
pub mod settle;
pub mod timing;

/// Number of tiles on the board (5 x 5).
pub const TILE_COUNT: u8 = 25;

/// Zatoshis per ZEC.
pub const COIN: u64 = 100_000_000;
