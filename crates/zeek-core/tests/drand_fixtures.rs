//! Verification of real, published drand quicknet beacons.
//!
//! The fixture file records its source URLs; every beacon in it was fetched
//! from the League of Entropy's public API and must verify under the chain's
//! public key, while any tampering must be rejected.

use serde::Deserialize;
use zeek_core::beacon::{self, Beacon, BeaconError};
use zeek_core::params::{DrandChain, GameParams};
use zeek_core::random;

#[derive(Deserialize)]
struct Fixture {
    source: String,
    chain: DrandChain,
    beacons: Vec<Beacon>,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/quicknet.json")).expect("fixture json")
}

#[test]
fn fixture_chain_matches_the_built_in_quicknet_description() {
    let f = fixture();
    assert!(f.source.starts_with("https://api.drand.sh/"));
    assert_eq!(f.chain, DrandChain::quicknet());
}

#[test]
fn real_quicknet_beacons_verify() {
    let f = fixture();
    assert!(f.beacons.len() >= 4);
    for b in &f.beacons {
        let randomness = beacon::verify_beacon(&f.chain, b).expect("real beacon verifies");
        // Randomness is sha256(signature), 32 bytes, and feeds the draws.
        assert_ne!(randomness.0, [0u8; 32]);
        let tile = random::treasure_tile(1, &randomness).expect("tile");
        assert!(tile < 25);
    }
    // Round 1's randomness pins the derivation against the drand reference.
    let first = f.beacons.iter().find(|b| b.round == 1).expect("round 1");
    let r = beacon::verify_beacon(&f.chain, first).expect("verify");
    assert_eq!(
        hex::encode(r.0),
        hex::encode(sha2_256(&hex::decode(&first.signature).expect("hex")))
    );
}

#[test]
fn tampered_beacons_are_rejected() {
    let f = fixture();
    let real = f.beacons[1].clone();

    let wrong_round = Beacon {
        round: real.round + 1,
        signature: real.signature.clone(),
    };
    assert!(matches!(
        beacon::verify_beacon(&f.chain, &wrong_round),
        Err(BeaconError::InvalidSignature { .. })
    ));

    let mut bytes = hex::decode(&real.signature).expect("hex");
    bytes[10] ^= 0x01;
    let flipped = Beacon {
        round: real.round,
        signature: hex::encode(bytes),
    };
    assert!(matches!(
        beacon::verify_beacon(&f.chain, &flipped),
        Err(BeaconError::InvalidSignature { .. }) | Err(BeaconError::Verification(_))
    ));

    let other_chain = DrandChain {
        public_key: f.chain.public_key.replace('8', "9"),
        ..f.chain.clone()
    };
    assert!(beacon::verify_beacon(&other_chain, &real).is_err());
}

#[test]
fn fixture_rounds_sit_at_their_published_times() {
    let p = GameParams::testnet(1);
    let f = fixture();
    for b in &f.beacons {
        let t = beacon::beacon_time(&p, b.round).expect("time");
        assert_eq!(beacon::latest_round_at(&p, t).expect("round"), b.round);
        assert_eq!(beacon::round_at_or_after(&p, t).expect("round"), b.round);
    }
}

fn sha2_256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).into()
}
