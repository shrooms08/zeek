//! Shared fixtures of the engine's integration tests: a synthetic chain
//! with one round of digs, and the recorded drand beacon it settles with.

use std::collections::HashMap;

use zeek_core::beacon::Beacon;
use zeek_core::params::GameParams;
use zeek_engine::beacon::FixedBeacons;
use zeek_engine::config::SETTLE_CONFIRMATIONS;
use zeek_engine::db::Pool;
use zeek_engine::synthetic::{SyntheticChain, TestVault};

/// A real quicknet beacon (tests/fixtures/quicknet.json in zeek-core, from
/// https://api.drand.sh/v2/beacons/quicknet/rounds/10000000).
pub const FIXTURE_ROUND: u64 = 10_000_000;
const FIXTURE_SIGNATURE: &str =
    "a187c9f5800521e986b5658924696682071ce7ae965486e8fd08bc2c4758bdbb9ed9ecb7beee392a38214e321ecd434f";

pub const GENESIS: u32 = 5_000;

/// Start time of round 0 such that its bound drand round is the fixture round.
pub fn start_time(params: &GameParams) -> u64 {
    let beacon_time = params.drand.genesis_time + (FIXTURE_ROUND - 1) * params.drand.period_secs;
    beacon_time - params.round_duration_secs().expect("duration") - params.reveal_delay_secs
}

pub fn fixture_beacons() -> FixedBeacons {
    let mut beacons = HashMap::new();
    beacons.insert(
        FIXTURE_ROUND,
        Beacon {
            round: FIXTURE_ROUND,
            signature: FIXTURE_SIGNATURE.to_owned(),
        },
    );
    FixedBeacons { beacons }
}

pub struct Scenario {
    pub vault: TestVault,
    pub chain: SyntheticChain,
}

/// Round 0 with three valid digs (alice 0.005 on tile 3, bob 0.003 on tile
/// 3, carol 0.004 on tile 17), one wrong-round dig, one dust dig and one
/// text-memo note, followed by the blocks the timing rule and the
/// confirmation policy need.
pub fn scenario(delay_close_secs: u64) -> Scenario {
    scenario_with(delay_close_secs, |_, _| {})
}

/// The same round with `extra` transactions mined inside its window (after
/// bob's dig).
pub fn scenario_with(
    delay_close_secs: u64,
    extra: impl FnOnce(&mut SyntheticChain, &TestVault),
) -> Scenario {
    let vault = TestVault::new(42);
    let config = vault.config(GENESIS);
    let mut chain = SyntheticChain::new(&vault, GENESIS - 3, start_time(&config.params) - 75);
    chain.advance(3); // GENESIS-3 .. GENESIS-1 (operator float era)
    assert_eq!(chain.next_height(), GENESIS);
    chain.push_block(vec![]); // GENESIS: start block, time == start_time
    chain.advance(4);
    chain.push_block(vec![
        chain.payment(500_000, Pool::Ironwood, vault.dig_memo(0, 3, 1).encode()),
        chain.change(1_000),
    ]);
    chain.push_block(vec![chain.payment(
        300_000,
        Pool::Ironwood,
        vault.dig_memo(0, 3, 2).encode(),
    )]);
    extra(&mut chain, &vault);
    chain.advance(2);
    chain.push_block(vec![
        chain.payment(400_000, Pool::Ironwood, vault.dig_memo(0, 17, 3).encode()),
        chain.payment(200_000, Pool::Ironwood, vault.dig_memo(7, 17, 3).encode()),
        chain.payment(50_000, Pool::Ironwood, vault.dig_memo(0, 1, 4).encode()),
        chain.payment(150_000, Pool::Ironwood, [0xF6; 512]),
    ]);
    while chain.next_height() < GENESIS + config.params.blocks_per_round {
        chain.advance(1);
    }
    chain.delay(delay_close_secs);
    // Post-close blocks for the timing rule plus the confirmation depth.
    chain.advance(config.params.mtp_window + SETTLE_CONFIRMATIONS);
    Scenario { vault, chain }
}
