//! drand quicknet beacons: verification, round/time arithmetic, and the
//! binding of a game round to one drand round.
//!
//! quicknet uses scheme `bls-unchained-g1-rfc9380` (drand `crypto/schemes.go`):
//! BLS12-381 signatures on G1 (48 bytes), the group public key on G2
//! (96 bytes), and the signed message is `SHA-256(round as big-endian u64)`;
//! hash-to-curve uses the RFC 9380 DST `BLS_SIG_BLS12381G1_XMD:SHA-256_SSWU_RO_NUL_`.
//! The beacon's randomness is `SHA-256(signature)`.
//!
//! Round/time arithmetic follows drand `common/time.go`: round 1 is published
//! at `genesis_time` and round `n` at `genesis_time + (n − 1)·period`.
//!
//! # Binding a game round to a drand round
//!
//! `beacon_round_for(params, t_start)` with `t_start` the `nTime` of the
//! round's first block (public once that block is mined, before any dig of the
//! round can be confirmed):
//!
//! ```text
//! t_reveal     = t_start + blocks_per_round · target_spacing_secs + reveal_delay_secs
//! drand_round  = smallest n ≥ 1 with genesis_time + (n − 1)·period ≥ t_reveal
//!              = 1                                        if t_reveal ≤ genesis_time
//!              = ceil((t_reveal − genesis_time) / period) + 1   otherwise
//! ```
//!
//! The miner of the start block can shift `t_start` only within the spec's
//! timestamp bounds (strictly above the median-time-past, at most 90 minutes
//! above it); pushing it later only delays the reveal, pushing it earlier can
//! at worst cause the round to be voided by the timing rule, never let a
//! beacon be known while the window is open.

use drand_verify::{G2PubkeyRfc, Pubkey};
use serde::{Deserialize, Serialize};

use crate::params::{DrandChain, GameParams, ParamsError, SCHEME_BLS_UNCHAINED_G1_RFC9380};

/// Errors from beacon handling.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BeaconError {
    #[error("unsupported drand scheme `{0}`")]
    UnsupportedScheme(String),
    #[error("drand public key is not a valid 96-byte compressed G2 point")]
    InvalidPublicKey,
    #[error("beacon signature must be 48 bytes, got {0}")]
    BadSignatureLength(usize),
    #[error("beacon signature does not verify for round {round} under the chain public key")]
    InvalidSignature { round: u64 },
    #[error("BLS verification failed: {0}")]
    Verification(String),
    #[error("drand rounds start at 1")]
    RoundZero,
    #[error("arithmetic overflow in beacon timing")]
    Overflow,
    #[error(transparent)]
    Params(#[from] ParamsError),
}

/// A published drand beacon (`/v2/beacons/<id>/rounds/<n>`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Beacon {
    /// drand round number.
    pub round: u64,
    /// BLS signature, hex (48 bytes for quicknet).
    pub signature: String,
}

/// 32 bytes of verified randomness.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Randomness(pub [u8; 32]);

impl core::fmt::Debug for Randomness {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Randomness({})", hex::encode(self.0))
    }
}

impl Serialize for Randomness {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(self.0))
    }
}

impl<'de> Deserialize<'de> for Randomness {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let bytes = hex::decode(text).map_err(serde::de::Error::custom)?;
        <[u8; 32]>::try_from(bytes.as_slice())
            .map(Randomness)
            .map_err(|_| serde::de::Error::custom("randomness must be 32 bytes"))
    }
}

/// Verifies `beacon` against `chain` and returns its randomness.
pub fn verify_beacon(chain: &DrandChain, beacon: &Beacon) -> Result<Randomness, BeaconError> {
    if chain.scheme != SCHEME_BLS_UNCHAINED_G1_RFC9380 {
        return Err(BeaconError::UnsupportedScheme(chain.scheme.clone()));
    }
    if beacon.round == 0 {
        return Err(BeaconError::RoundZero);
    }
    let pk_bytes = hex::decode(&chain.public_key).map_err(|_| BeaconError::InvalidPublicKey)?;
    let pk_fixed =
        <[u8; 96]>::try_from(pk_bytes.as_slice()).map_err(|_| BeaconError::InvalidPublicKey)?;
    let pk = G2PubkeyRfc::from_fixed(pk_fixed).map_err(|_| BeaconError::InvalidPublicKey)?;
    let signature =
        hex::decode(&beacon.signature).map_err(|_| BeaconError::BadSignatureLength(0))?;
    if signature.len() != 48 {
        return Err(BeaconError::BadSignatureLength(signature.len()));
    }
    // Unchained: the previous signature is not part of the message.
    let ok = pk
        .verify(beacon.round, &[], &signature)
        .map_err(|e| BeaconError::Verification(format!("{e:?}")))?;
    if !ok {
        return Err(BeaconError::InvalidSignature {
            round: beacon.round,
        });
    }
    Ok(Randomness(drand_verify::derive_randomness(&signature)))
}

/// Unix time at which drand round `round` (≥ 1) is published.
pub fn beacon_time(params: &GameParams, round: u64) -> Result<u64, BeaconError> {
    if round == 0 {
        return Err(BeaconError::RoundZero);
    }
    (round - 1)
        .checked_mul(params.drand.period_secs)
        .and_then(|d| d.checked_add(params.drand.genesis_time))
        .ok_or(BeaconError::Overflow)
}

/// The latest drand round published at or before `time` (0 before genesis).
pub fn latest_round_at(params: &GameParams, time: u64) -> Result<u64, BeaconError> {
    if params.drand.period_secs == 0 {
        return Err(ParamsError::ZeroDrandPeriod.into());
    }
    Ok(match time.checked_sub(params.drand.genesis_time) {
        None => 0,
        Some(elapsed) => elapsed / params.drand.period_secs + 1,
    })
}

/// The first drand round published at or after `time`.
pub fn round_at_or_after(params: &GameParams, time: u64) -> Result<u64, BeaconError> {
    if params.drand.period_secs == 0 {
        return Err(ParamsError::ZeroDrandPeriod.into());
    }
    Ok(match time.checked_sub(params.drand.genesis_time) {
        None | Some(0) => 1,
        Some(elapsed) => elapsed.div_ceil(params.drand.period_secs) + 1,
    })
}

/// The drand round bound to the game round whose first block has `nTime`
/// `start_block_time`; see the module docs for the formula.
pub fn beacon_round_for(params: &GameParams, start_block_time: u64) -> Result<u64, BeaconError> {
    let t_reveal = start_block_time
        .checked_add(params.round_duration_secs()?)
        .and_then(|t| t.checked_add(params.reveal_delay_secs))
        .ok_or(BeaconError::Overflow)?;
    round_at_or_after(params, t_reveal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> GameParams {
        GameParams::testnet(1)
    }

    #[test]
    fn round_time_arithmetic_matches_drand() {
        let p = params();
        let g = p.drand.genesis_time;
        assert_eq!(beacon_time(&p, 1).expect("t"), g);
        assert_eq!(beacon_time(&p, 2).expect("t"), g + 3);
        assert_eq!(beacon_time(&p, 0), Err(BeaconError::RoundZero));
        assert_eq!(latest_round_at(&p, g - 1).expect("r"), 0);
        assert_eq!(latest_round_at(&p, g).expect("r"), 1);
        assert_eq!(latest_round_at(&p, g + 5).expect("r"), 2);
        assert_eq!(round_at_or_after(&p, g - 100).expect("r"), 1);
        assert_eq!(round_at_or_after(&p, g).expect("r"), 1);
        assert_eq!(round_at_or_after(&p, g + 1).expect("r"), 2);
        assert_eq!(round_at_or_after(&p, g + 3).expect("r"), 2);
        assert_eq!(round_at_or_after(&p, g + 4).expect("r"), 3);
        // Published at or after the requested time, never before.
        for t in [g, g + 1, g + 2, g + 3, g + 1000, g + 1001] {
            let r = round_at_or_after(&p, t).expect("r");
            assert!(beacon_time(&p, r).expect("t") >= t);
            assert!(r == 1 || beacon_time(&p, r - 1).expect("t") < t);
        }
    }

    #[test]
    fn game_round_binds_to_the_first_beacon_after_the_reveal_time() {
        let p = params();
        let g = p.drand.genesis_time;
        let start = g + 3_000;
        let r = beacon_round_for(&p, start).expect("round");
        let t_reveal = start + 600 + 900;
        assert!(beacon_time(&p, r).expect("t") >= t_reveal);
        assert!(beacon_time(&p, r - 1).expect("t") < t_reveal);
        assert_eq!(beacon_round_for(&p, u64::MAX), Err(BeaconError::Overflow));
    }

    #[test]
    fn rejects_unsupported_scheme_and_malformed_inputs() {
        let mut chain = DrandChain::quicknet();
        let beacon = Beacon {
            round: 1,
            signature: "00".repeat(48),
        };
        assert!(matches!(
            verify_beacon(&chain, &beacon),
            Err(BeaconError::InvalidSignature { round: 1 }) | Err(BeaconError::Verification(_))
        ));
        assert_eq!(
            verify_beacon(
                &chain,
                &Beacon {
                    round: 1,
                    signature: "abcd".into()
                }
            ),
            Err(BeaconError::BadSignatureLength(2))
        );
        assert_eq!(
            verify_beacon(
                &chain,
                &Beacon {
                    round: 0,
                    signature: "00".repeat(48)
                }
            ),
            Err(BeaconError::RoundZero)
        );
        chain.scheme = "pedersen-bls-chained".into();
        assert!(matches!(
            verify_beacon(&chain, &beacon),
            Err(BeaconError::UnsupportedScheme(_))
        ));
    }
}
