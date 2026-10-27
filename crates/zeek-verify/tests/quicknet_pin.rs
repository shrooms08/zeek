//! The beacon must come from drand quicknet: an operator that publishes its
//! own BLS key in the config (and the same key in the bundle) can sign the
//! beacon itself and so choose the treasure tile. Before the pin, such a
//! bundle passed check 1, because the signature does verify under the key
//! the bundle names.
//!
//! The forged bundle here is real testnet round 251 (docs/examples/phase8b)
//! with its beacon re-signed by another key and its draw recomputed from
//! that beacon, as a dishonest engine would publish it.

use std::collections::BTreeMap;

use bls12_381::hash_to_curve::{ExpandMsgXmd, HashToCurve};
use bls12_381::{G1Affine, G1Projective, G2Affine, G2Projective, Scalar};
use drand_verify::{G2PubkeyRfc, Pubkey};
use sha2_09::{Digest, Sha256};
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zeek_core::beacon::{verify_beacon, BeaconError, Randomness};
use zeek_core::params::{ParamsError, QUICKNET_CHAIN_HASH, QUICKNET_PUBLIC_KEY};
use zeek_core::proof::ProofBundle;
use zeek_core::random;
use zeek_core::settle::RoundOutcome;
use zeek_verify::checks::{verify_round, PublicConfig, RoundInput};
use zeek_verify::offline::{verify_offline, PartStatus, PublicBeacon};
use zeek_verify::{CheckId, CheckResult, CheckStatus};

const ROUND_251: &str = include_str!("../../../docs/examples/phase8b/round-251.proof.json");
const CONFIG: &str = include_str!("../../../docs/examples/phase8b/testnet-config.json");
/// The RFC 9380 domain drand quicknet signs under (drand-verify `verify.rs`).
const DST_G1: &[u8] = b"BLS_SIG_BLS12381G1_XMD:SHA-256_SSWU_RO_NUL_";

/// An operator's own key, in drand quicknet's scheme: (public key, signer).
fn operator_key() -> ([u8; 96], impl Fn(u64) -> [u8; 48]) {
    let sk = Scalar::from(0x00c0_ffee_u64) * Scalar::from(0x0bad_5eed_u64);
    let pk = G2Affine::from(G2Projective::generator() * sk).to_compressed();
    let sign = move |round: u64| {
        let message = Sha256::digest(&round.to_be_bytes());
        let point =
            <G1Projective as HashToCurve<ExpandMsgXmd<Sha256>>>::hash_to_curve(message, DST_G1);
        G1Affine::from(point * sk).to_compressed()
    };
    (pk, sign)
}

/// Check 1 of `bundle` against `config`, with the chain data check 1 reads
/// (the round's first block and its time).
fn check_1(bundle: &ProofBundle, config: &PublicConfig) -> CheckResult {
    let blocks = zeek_verify::chain::Blocks::new([CompactBlock {
        height: u64::from(bundle.window.start),
        hash: vec![0; 32],
        prev_hash: vec![0; 32],
        time: u32::try_from(bundle.start_block_time).expect("time fits"),
        ..CompactBlock::default()
    }]);
    let report = verify_round(&RoundInput {
        config,
        bundle,
        blocks: &blocks,
        txs: &BTreeMap::new(),
        ivk: &config.ivk().expect("ivk"),
    });
    report
        .checks
        .into_iter()
        .find(|c| c.id == CheckId::Beacon)
        .expect("check 1 runs")
}

fn round_251() -> (ProofBundle, PublicConfig) {
    (
        serde_json::from_str(ROUND_251).expect("bundle"),
        serde_json::from_str(CONFIG).expect("config"),
    )
}

/// `bundle` re-signed by `sign` (the operator's key), with the randomness
/// and draw that beacon gives: what a dishonest engine would publish.
fn forge(bundle: &mut ProofBundle, sign: &impl Fn(u64) -> [u8; 48]) {
    let signature = sign(bundle.drand_round);
    let randomness = Randomness(drand_verify::derive_randomness(&signature));
    bundle
        .beacon
        .as_mut()
        .expect("round 251 was drawn")
        .signature = hex::encode(signature);
    bundle.randomness = Some(randomness);
    bundle.outcome = RoundOutcome::Drawn {
        treasure_tile: random::treasure_tile(bundle.round, &randomness).expect("tile"),
        cache_triggered: random::cache_triggers(&bundle.params, bundle.round, &randomness)
            .expect("cache"),
    };
}

#[test]
fn the_real_round_251_passes_check_1() {
    let (bundle, config) = round_251();
    assert_eq!(bundle.params.drand.chain_hash, QUICKNET_CHAIN_HASH);
    assert_eq!(bundle.params.drand.public_key, QUICKNET_PUBLIC_KEY);
    let check = check_1(&bundle, &config);
    assert_eq!(check.status, CheckStatus::Pass, "{check:?}");
    assert!(check.summary.contains("drand quicknet round"));
}

#[test]
fn a_beacon_signed_under_another_key_fails_check_1() {
    let (mut bundle, mut config) = round_251();
    let (pk, sign) = operator_key();

    // The forgery is cryptographically sound: the signature verifies under
    // the operator's key exactly as quicknet's verify under quicknet's. This
    // is what check 1 used to accept.
    let key = G2PubkeyRfc::from_fixed(pk).expect("valid G2 point");
    assert!(key
        .verify(bundle.drand_round, &[], &sign(bundle.drand_round))
        .expect("verify"));

    // The operator publishes its key in both the config and the bundle
    // (they must agree), and settles with the beacon it signed.
    let mut forged_chain = bundle.params.drand.clone();
    forged_chain.public_key = hex::encode(pk);
    forged_chain.chain_hash = hex::encode(Sha256::digest(&pk));
    bundle.params.drand = forged_chain.clone();
    config.params.drand = forged_chain.clone();
    forge(&mut bundle, &sign);

    let check = check_1(&bundle, &config);
    assert_eq!(check.status, CheckStatus::Fail, "{check:?}");
    assert!(
        check.summary.contains("beacon key is not drand quicknet"),
        "{}",
        check.summary
    );

    // The same pin in zeek-core: the beacon does not verify, and a game
    // config naming that key does not load (engine and keyholders refuse to
    // start).
    let beacon = bundle.beacon.as_ref().expect("beacon");
    let err = verify_beacon(&forged_chain, beacon).expect_err("not quicknet");
    assert_eq!(
        err,
        BeaconError::NotQuicknet {
            field: "chain hash"
        }
    );
    assert!(err.to_string().contains("beacon key is not drand quicknet"));
    assert_eq!(
        config.params.validate(),
        Err(ParamsError::NotQuicknet {
            field: "chain hash"
        })
    );

    // Replay mode (no chain) refuses the signature too.
    let offline = verify_offline(
        &bundle,
        &config,
        Some(&PublicBeacon::Fetched {
            beacon: beacon.clone(),
            source: "the operator".into(),
        }),
    );
    let signature = offline
        .parts
        .iter()
        .find(|p| p.check == CheckId::Beacon && p.part == "signature")
        .expect("signature part");
    assert_eq!(signature.status, PartStatus::Fail);

    // Keeping quicknet's chain hash but swapping the key is caught too.
    let mut same_hash = forged_chain;
    same_hash.chain_hash = QUICKNET_CHAIN_HASH.to_owned();
    bundle.params.drand = same_hash.clone();
    config.params.drand = same_hash.clone();
    let check = check_1(&bundle, &config);
    assert_eq!(check.status, CheckStatus::Fail);
    assert!(check.summary.contains("beacon key is not drand quicknet"));
    assert_eq!(
        verify_beacon(&same_hash, bundle.beacon.as_ref().expect("beacon")),
        Err(BeaconError::NotQuicknet {
            field: "public key"
        })
    );
}

#[test]
fn moving_quicknets_genesis_or_period_fails_check_1() {
    // Same key, but a shifted clock: the round would bind to a drand round
    // that was already published.
    let (mut bundle, mut config) = round_251();
    bundle.params.drand.genesis_time -= 3_600;
    config.params.drand.genesis_time -= 3_600;
    let check = check_1(&bundle, &config);
    assert_eq!(check.status, CheckStatus::Fail);
    assert!(check.summary.contains("beacon key is not drand quicknet"));
    assert!(check.detail.iter().any(|d| d.contains("genesis time")));
}
