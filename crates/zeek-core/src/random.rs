//! Unbiased draws from verified beacon randomness.
//!
//! Every draw is `SHA-256(domain || round u32 BE || randomness || counter u32 BE)`
//! read as big-endian `u32` words, accepted only when the word is below the
//! largest multiple of `n` that fits in 32 bits (rejection sampling, so no
//! modulo bias), and extended with a fresh counter when a whole digest is
//! rejected. The treasure tile and the Deep Cache trigger use different
//! domains so they are independent draws from the same beacon.

use sha2::{Digest, Sha256};

use crate::beacon::Randomness;
use crate::params::GameParams;
use crate::TILE_COUNT;

/// Domain separator of the treasure-tile draw.
pub const TREASURE_DOMAIN: &[u8] = b"zeek/treasure/v1";
/// Domain separator of the Deep Cache draw.
pub const CACHE_DOMAIN: &[u8] = b"zeek/cache/v1";

/// Errors from drawing.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RandomError {
    #[error("cannot draw from an empty range")]
    EmptyRange,
    #[error("rejection sampling did not terminate")]
    Exhausted,
}

/// Draws a uniformly distributed value in `0..n`.
pub fn uniform_below(
    domain: &[u8],
    round: u32,
    randomness: &Randomness,
    n: u32,
) -> Result<u32, RandomError> {
    if n == 0 {
        return Err(RandomError::EmptyRange);
    }
    // Largest multiple of `n` that is at most 2^32.
    let limit = (1u64 << 32) / u64::from(n) * u64::from(n);
    for counter in 0..u32::MAX {
        let digest = Sha256::new()
            .chain_update(domain)
            .chain_update(round.to_be_bytes())
            .chain_update(randomness.0)
            .chain_update(counter.to_be_bytes())
            .finalize();
        for word in digest.chunks_exact(4) {
            let value = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
            if u64::from(value) < limit {
                return Ok(value % n);
            }
        }
    }
    Err(RandomError::Exhausted)
}

/// The treasure tile (0..25) of `round`.
pub fn treasure_tile(round: u32, randomness: &Randomness) -> Result<u8, RandomError> {
    let tile = uniform_below(TREASURE_DOMAIN, round, randomness, u32::from(TILE_COUNT))?;
    u8::try_from(tile).map_err(|_| RandomError::Exhausted)
}

/// Whether the Deep Cache breaks open in `round` (value 0 of `0..cache_odds`).
pub fn cache_triggers(
    params: &GameParams,
    round: u32,
    randomness: &Randomness,
) -> Result<bool, RandomError> {
    Ok(uniform_below(CACHE_DOMAIN, round, randomness, params.cache_odds)? == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn randomness(seed: u8) -> Randomness {
        Randomness([seed; 32])
    }

    #[test]
    fn draws_are_deterministic_and_in_range() {
        let r = randomness(7);
        let a = treasure_tile(3, &r).expect("tile");
        let b = treasure_tile(3, &r).expect("tile");
        assert_eq!(a, b);
        assert!(a < TILE_COUNT);
        assert_ne!(
            uniform_below(TREASURE_DOMAIN, 3, &r, 1_000_000).expect("v"),
            uniform_below(CACHE_DOMAIN, 3, &r, 1_000_000).expect("v"),
            "domains must separate the draws"
        );
        assert_ne!(
            treasure_tile(3, &r).expect("t"),
            treasure_tile(4, &randomness(8)).expect("t")
        );
        assert_eq!(uniform_below(TREASURE_DOMAIN, 0, &r, 1).expect("v"), 0);
        assert_eq!(
            uniform_below(TREASURE_DOMAIN, 0, &r, 0),
            Err(RandomError::EmptyRange)
        );
    }

    #[test]
    fn known_answer_is_stable() {
        // Regression pin computed from this implementation on 2026-10-13, so
        // that engine, keyholders and browser verifier can never silently
        // disagree once deployed.
        let r = Randomness(
            hex::decode("7f7f0a0e0e2c1c84a4c9f5fcd5b06b8f45b6b9a3a2d8c0e1f2a3b4c5d6e7f809")
                .expect("hex")
                .try_into()
                .expect("32 bytes"),
        );
        let p = GameParams::testnet(1);
        let tile = treasure_tile(42, &r).expect("tile");
        let cache = uniform_below(CACHE_DOMAIN, 42, &r, p.cache_odds).expect("cache");
        assert_eq!((tile, cache), (20, 283));
    }
}
