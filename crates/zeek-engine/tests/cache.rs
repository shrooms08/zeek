//! Which proof bundles may be cached for good (`/v1/rounds/{n}/proof`):
//! real testnet rounds from docs/examples/phase8b.

use zeek_core::proof::ProofBundle;
use zeek_engine::api::bundle_is_final;

const ROUND_251: &str = include_str!("../../../docs/examples/phase8b/round-251.proof.json");
const ROUND_245: &str = include_str!("../../../docs/examples/phase8b/round-245.proof.json");

#[test]
fn a_mined_payout_or_nothing_to_pay_is_final() {
    let paid: ProofBundle = serde_json::from_str(ROUND_251).expect("bundle");
    assert!(bundle_is_final(&paid));
    let nothing: ProofBundle = serde_json::from_str(ROUND_245).expect("bundle");
    assert!(bundle_is_final(&nothing));
}

#[test]
fn a_payout_not_mined_yet_is_not_final() {
    let mut waiting: ProofBundle = serde_json::from_str(ROUND_251).expect("bundle");
    waiting.payout_openings = None;
    assert!(!bundle_is_final(&waiting));
}
