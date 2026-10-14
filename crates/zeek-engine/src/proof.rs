//! Proof bundles. The format and `recompute` live in [`zeek_core::proof`]
//! so that independent verifiers need nothing but `zeek-core`; they are
//! re-exported here for the engine's own users.

pub use zeek_core::proof::*;
