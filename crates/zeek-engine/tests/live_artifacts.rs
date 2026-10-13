//! Independent recomputation of the real testnet rounds saved under
//! `docs/examples/`: every `round-<n>.proof.json` is a proof bundle fetched
//! from a live engine, and `round-<n>.round.json` the matching round view.
//! The settlement is recomputed from the bundle's inputs with zeek-core only
//! and must match the engine's byte for byte.

use std::fs;
use std::path::PathBuf;

use zeek_core::beacon;
use zeek_engine::api::RoundResponse;
use zeek_engine::proof::{self, ProofBundle};

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("docs")
        .join("examples")
}

#[test]
fn saved_live_rounds_recompute_byte_for_byte() {
    let dir = examples_dir();
    let mut bundles: Vec<PathBuf> = Vec::new();
    for sub in [dir.clone(), dir.join("phase4")] {
        let Ok(entries) = fs::read_dir(&sub) else {
            continue;
        };
        bundles.extend(
            entries
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.ends_with(".proof.json"))
                }),
        );
    }
    bundles.sort();
    assert!(
        !bundles.is_empty(),
        "no live proof bundles under {}",
        dir.display()
    );

    for path in bundles {
        let text = fs::read_to_string(&path).expect("read bundle");
        let bundle: ProofBundle = serde_json::from_str(&text).expect("parse bundle");
        let recomputed = proof::recompute(&bundle).expect("recompute with zeek-core");
        assert_eq!(
            serde_json::to_string(&recomputed).expect("json"),
            serde_json::to_string(&bundle.settlement).expect("json"),
            "{} does not recompute to the engine's settlement",
            path.display()
        );
        // The serialized bundle itself must round-trip unchanged.
        let reserialized: serde_json::Value = serde_json::to_value(&bundle).expect("value");
        let original: serde_json::Value = serde_json::from_str(&text).expect("value");
        assert_eq!(
            reserialized,
            original,
            "{} does not round-trip",
            path.display()
        );

        // The bound drand round is the one the published beacon carries, and
        // its randomness is sha256(signature).
        if let (Some(beacon), Some(randomness)) = (&bundle.beacon, &bundle.randomness) {
            assert_eq!(beacon.round, bundle.drand_round);
            assert_eq!(
                beacon::verify_beacon(&bundle.params.drand, beacon).expect("verify"),
                *randomness
            );
        }

        // The round view published next to it agrees on the payouts.
        let view_path = path.with_file_name(
            path.file_name()
                .and_then(|n| n.to_str())
                .expect("name")
                .replace(".proof.json", ".round.json"),
        );
        let view: RoundResponse =
            serde_json::from_str(&fs::read_to_string(&view_path).expect("read round view"))
                .expect("parse round view");
        let settled = view.settled.expect("settled view");
        assert_eq!(settled.payouts, bundle.settlement.payouts);
        assert_eq!(settled.new_cache, bundle.settlement.new_cache);
        assert_eq!(settled.drand_round, bundle.drand_round);
    }
}
