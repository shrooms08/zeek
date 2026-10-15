//! The offline checks and the replay timeline on real testnet rounds
//! (docs/examples/phase8b): round 251 (25 digs, one winner, payout mined)
//! and round 245 (one dig, no winner).

use zeek_core::beacon::Beacon;
use zeek_core::proof::ProofBundle;
use zeek_verify::checks::PublicConfig;
use zeek_verify::offline::{verify_offline, PartStatus, PublicBeacon};
use zeek_verify::replay::{replay, ReplayOutcome};
use zeek_verify::CheckId;

const ROUND_251: &str = include_str!("../../../docs/examples/phase8b/round-251.proof.json");
const ROUND_245: &str = include_str!("../../../docs/examples/phase8b/round-245.proof.json");
const CONFIG: &str = include_str!("../../../docs/examples/phase8b/testnet-config.json");

fn load(json: &str) -> ProofBundle {
    serde_json::from_str(json).expect("bundle parses")
}

fn config() -> PublicConfig {
    serde_json::from_str(CONFIG).expect("config parses")
}

fn status(report: &zeek_verify::offline::OfflineReport, check: CheckId, part: &str) -> PartStatus {
    report
        .parts
        .iter()
        .find(|p| p.check == check && p.part == part)
        .map(|p| p.status)
        .expect("part present")
}

fn drand_copy(bundle: &ProofBundle) -> PublicBeacon {
    PublicBeacon::Fetched {
        beacon: bundle.beacon.clone().expect("drawn round"),
        source: "api.drand.sh".into(),
    }
}

#[test]
fn a_won_round_passes_every_part_it_can_run_and_marks_the_rest() {
    let bundle = load(ROUND_251);
    let report = verify_offline(&bundle, &config(), Some(&drand_copy(&bundle)));
    for (check, part) in [
        (CheckId::Beacon, "rules"),
        (CheckId::Beacon, "signature"),
        (CheckId::Beacon, "public"),
        (CheckId::Beacon, "draw"),
        (CheckId::Settlement, "recompute"),
        (CheckId::Payout, "openings"),
    ] {
        assert_eq!(
            status(&report, check, part),
            PartStatus::Pass,
            "{check:?} {part}"
        );
    }
    for (check, part) in [
        (CheckId::Beacon, "binding"),
        (CheckId::Digs, "chain"),
        (CheckId::Timing, "chain"),
        (CheckId::Payout, "chain"),
    ] {
        assert_eq!(
            status(&report, check, part),
            PartStatus::NeedsChain,
            "{check:?} {part}"
        );
    }
}

#[test]
fn a_different_public_beacon_fails_and_an_unreachable_one_is_not_a_tick() {
    let bundle = load(ROUND_251);
    let mut other = bundle.beacon.clone().expect("drawn");
    other.signature = "00".repeat(48);
    let forged = PublicBeacon::Fetched {
        beacon: Beacon { ..other },
        source: "relay".into(),
    };
    let report = verify_offline(&bundle, &config(), Some(&forged));
    assert_eq!(status(&report, CheckId::Beacon, "public"), PartStatus::Fail);
    let down = PublicBeacon::Unreachable {
        reason: "timeout".into(),
    };
    let report = verify_offline(&bundle, &config(), Some(&down));
    assert_eq!(
        status(&report, CheckId::Beacon, "public"),
        PartStatus::NotAvailable
    );
}

#[test]
fn tampering_is_caught_offline() {
    // A forged beacon signature.
    let mut bundle = load(ROUND_251);
    if let Some(b) = bundle.beacon.as_mut() {
        b.signature = format!("{}00", &b.signature[..b.signature.len() - 2]);
    }
    let report = verify_offline(&bundle, &config(), None);
    assert_eq!(
        status(&report, CheckId::Beacon, "signature"),
        PartStatus::Fail
    );

    // A payout opening for a different amount.
    let mut bundle = load(ROUND_251);
    if let Some(o) = bundle.payout_openings.as_mut() {
        o.outputs[0].value += 1;
    }
    let report = verify_offline(&bundle, &config(), None);
    assert_eq!(
        status(&report, CheckId::Payout, "openings"),
        PartStatus::Fail
    );

    // A settlement that pays more than the rules give.
    let mut bundle = load(ROUND_251);
    bundle.settlement.total_paid += 1;
    let report = verify_offline(&bundle, &config(), None);
    assert_eq!(
        status(&report, CheckId::Settlement, "recompute"),
        PartStatus::Fail
    );
}

#[test]
fn a_round_without_payout_has_nothing_to_open() {
    let bundle = load(ROUND_245);
    let report = verify_offline(&bundle, &config(), Some(&drand_copy(&bundle)));
    assert_eq!(
        status(&report, CheckId::Payout, "openings"),
        PartStatus::NotAvailable
    );
    assert_eq!(
        status(&report, CheckId::Settlement, "recompute"),
        PartStatus::Pass
    );
    assert_eq!(status(&report, CheckId::Beacon, "draw"), PartStatus::Pass);
}

#[test]
fn the_replay_fills_the_field_block_by_block_without_amounts() {
    let bundle = load(ROUND_251);
    let r = replay(&bundle).expect("replay");
    assert_eq!(r.round, 251);
    assert_eq!(
        r.outcome,
        ReplayOutcome::Drawn {
            treasure_tile: 8,
            cache_triggered: false
        }
    );
    // Bob's 3 tiles landed first, then alice's 22.
    let landed: Vec<usize> = r.frames.iter().map(|f| f.landed.len()).collect();
    assert_eq!(landed.iter().sum::<usize>(), 25);
    let last = r.frames.last().expect("frames");
    assert!(last.stages.iter().all(|s| *s > 0));
    assert!(r.frames.windows(2).all(|w| w[0].height < w[1].height));
    let json = serde_json::to_string(&r).expect("json");
    assert!(!json.contains("value") && !json.contains("memo") && !json.contains("tile_totals"));
    assert_eq!(
        r.payout_txid.as_deref(),
        Some("2bcda19e216a952c1eca44e92d69513efa9e43e8d04120183d9726c44187a51e")
    );
}
