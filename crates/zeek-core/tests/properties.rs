//! Property tests over random rounds, memos and draws.

use proptest::collection::vec;
use proptest::prelude::*;
use rand::rngs::StdRng;
use rand::SeedableRng;
use zeek_core::beacon::Randomness;
use zeek_core::digs::{
    ClassifiedDigs, Forfeit, ForfeitReason, ReceivedNote, RefundDig, RefundReason, ValidDig,
};
use zeek_core::memo::{
    DigMemo, PayoutReceiver, SealedReceiver, ORCHARD_RECEIVER_LEN, SEALED_RECEIVER_LEN,
};
use zeek_core::params::{GameParams, Network};
use zeek_core::random::{self, CACHE_DOMAIN, TREASURE_DOMAIN};
use zeek_core::seal::generate_sealing_keypair;
use zeek_core::settle::{fees_cover, payout_inputs, settle, zip317_fee, RoundOutcome};

fn note(value: u64) -> ReceivedNote {
    ReceivedNote {
        txid: "11".repeat(32),
        output_index: 0,
        height: 1,
        value,
        memo: vec![],
    }
}

fn sealed(seed: u8) -> SealedReceiver {
    SealedReceiver::from_bytes(&[seed; SEALED_RECEIVER_LEN]).expect("sealed")
}

/// Random classified digs: values bounded so that totals stay far below MAX_MONEY.
fn arb_digs() -> impl Strategy<Value = ClassifiedDigs> {
    (
        vec((0u8..25, 100_000u64..50_000_000_000), 0..12),
        vec(1u64..50_000_000_000, 0..4),
        vec(1u64..100_000, 0..4),
    )
        .prop_map(|(valid, refunds, forfeits)| ClassifiedDigs {
            valid: valid
                .into_iter()
                .enumerate()
                .map(|(i, (tile, value))| ValidDig {
                    note: note(value),
                    tile,
                    sealed_receiver: sealed(i as u8),
                })
                .collect(),
            refunds: refunds
                .into_iter()
                .map(|value| RefundDig {
                    note: note(value),
                    memo: DigMemo {
                        round: 0,
                        tile: 0,
                        sealed_receiver: sealed(200),
                    },
                    reason: RefundReason::WrongRound { memo_round: 0 },
                })
                .collect(),
            forfeits: forfeits
                .into_iter()
                .map(|value| Forfeit {
                    note: note(value),
                    reason: ForfeitReason::BelowMinimum { minimum: 100_000 },
                })
                .collect(),
        })
}

fn arb_outcome() -> impl Strategy<Value = RoundOutcome> {
    prop_oneof![
        Just(RoundOutcome::Voided),
        (0u8..25, any::<bool>()).prop_map(|(treasure_tile, cache_triggered)| RoundOutcome::Drawn {
            treasure_tile,
            cache_triggered
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn settlement_conserves_value_and_never_overpays(
        digs in arb_digs(),
        previous_cache in 0u64..1_000_000_000_000,
        outcome in arb_outcome(),
        round in any::<u32>(),
    ) {
        let p = GameParams::testnet(1);
        let s = settle(&p, round, &digs, previous_cache, outcome).expect("settle");
        let pot = digs.pot().expect("pot");
        let refunded = digs.refunded_value().expect("refunded");
        let forfeited = digs.forfeited_value().expect("forfeited");
        // Conservation (also checked inside `settle`, re-derived here independently).
        prop_assert_eq!(
            pot + refunded + forfeited + previous_cache,
            s.total_paid + s.total_fees + s.new_cache
        );
        // Winnings never exceed the pot plus everything the Cache could hold
        // (its previous balance plus this round's forfeits); no single output
        // exceeds the round's total inflow.
        let win_gross: u64 = s.payouts.iter().filter_map(|o| match o.reason {
            zeek_core::settle::PayoutReason::Win { .. } => Some(o.gross),
            _ => None,
        }).sum();
        prop_assert!(win_gross <= pot + previous_cache + forfeited);
        prop_assert!(s.cache_paid <= previous_cache + s.rake + forfeited + s.distributable);
        for o in &s.payouts {
            prop_assert!(o.gross <= pot + previous_cache + refunded + forfeited);
            prop_assert_eq!(o.net + o.fee, o.gross);
            prop_assert!(o.net > 0);
        }
        // The Cache never goes negative and only shrinks when it triggered with winners.
        if !matches!(outcome, RoundOutcome::Drawn { cache_triggered: true, .. }) || s.winners == 0 {
            prop_assert!(s.new_cache >= previous_cache);
            prop_assert_eq!(s.cache_paid, 0);
        }
        let threshold = p.dig_fee_zat + p.marginal_fee_zat;
        if matches!(outcome, RoundOutcome::Voided) {
            prop_assert_eq!(s.payouts.len(), digs.valid.iter().filter(|d| d.note.value > threshold).count()
                + digs.refunds.iter().filter(|d| d.note.value > threshold).count());
        }
        // The notes to spend are exactly the charged ones, and the dig fees
        // are one per spent note.
        let inputs = payout_inputs(&p, &digs, outcome).expect("inputs");
        prop_assert_eq!(s.dig_fees, inputs.len() as u64 * p.dig_fee_zat);
        let charged_refunds = digs.refunds.iter().filter(|d| d.note.value > threshold).count();
        let charged_valid = match outcome {
            RoundOutcome::Drawn { .. } => digs.valid.len(),
            RoundOutcome::Voided => digs.valid.iter().filter(|d| d.note.value > threshold).count(),
        };
        prop_assert_eq!(inputs.len(), charged_valid + charged_refunds);
        prop_assert!(s.total_fees >= s.dig_fees);
        prop_assert_eq!(s.total_fees - s.dig_fees, s.payouts.len() as u64 * p.marginal_fee_zat);
        // Whenever there is something to pay, the fees collected cover the
        // ZIP 317 fee of the transaction that pays it (charged notes in, payouts
        // plus change out).
        if !s.payouts.is_empty() {
            prop_assert!(!inputs.is_empty());
            let fee = zip317_fee(&p, inputs.len() as u64, s.payouts.len() as u64 + 1).expect("fee");
            prop_assert!(fee <= s.total_fees, "fee {} > collected {}", fee, s.total_fees);
            prop_assert!(fees_cover(&p, inputs.len() as u64, 0, s.payouts.len() as u64).expect("cover"));
        }
    }

    /// Fee coverage over arbitrary transaction shapes: `n` charged notes,
    /// `k` operator-float notes, `p` payouts and one change output.
    #[test]
    fn fee_coverage_holds_exactly_when_float_notes_do_not_exceed_payouts(
        n in 1u64..200, k in 0u64..200, p in 1u64..200,
    ) {
        let params = GameParams::testnet(1);
        let fee = zip317_fee(&params, n + k, p + 1).expect("fee");
        let collected = (n + p) * params.marginal_fee_zat;
        prop_assert_eq!(fee <= collected, k <= p);
        prop_assert_eq!(fees_cover(&params, n, k, p).expect("cover"), k <= p);
        // Without float notes the fee is always covered.
        prop_assert!(fees_cover(&params, n, 0, p).expect("cover"));
    }

    #[test]
    fn memo_round_trips(round in any::<u32>(), tile in 0u8..25, seed in any::<u64>(), receiver in any::<[u8; ORCHARD_RECEIVER_LEN]>()) {
        let mut rng = StdRng::seed_from_u64(seed);
        let (sk, pk) = generate_sealing_keypair(&mut rng);
        let receiver = PayoutReceiver { network: Network::Test, orchard: receiver };
        let memo = DigMemo::seal(round, tile, &receiver, &pk, &mut rng).expect("seal");
        prop_assert_eq!(DigMemo::decode(&memo.encode()).expect("decode"), memo.clone());
        prop_assert_eq!(DigMemo::decode(&memo.encode_unpadded()).expect("decode"), memo.clone());
        prop_assert_eq!(memo.open(Network::Test, &sk).expect("open"), receiver);
    }

    /// Rules v3: the text and the binary encoding of any memo decode to the
    /// same dig from the activation height on, and a text memo below it
    /// fails exactly as the binary decoder fails on it.
    #[test]
    fn text_and_binary_memos_decode_identically(
        round in any::<u32>(), tile in 0u8..25, seed in any::<u64>(),
        receiver in any::<[u8; ORCHARD_RECEIVER_LEN]>(),
        activation in 1u32..1_000_000, height in 1u32..1_000_000,
    ) {
        let mut rng = StdRng::seed_from_u64(seed);
        let (sk, pk) = generate_sealing_keypair(&mut rng);
        let receiver = PayoutReceiver { network: Network::Test, orchard: receiver };
        let memo = DigMemo::seal(round, tile, &receiver, &pk, &mut rng).expect("seal");
        let text = memo.encode_text();
        prop_assert_eq!(text.len(), zeek_core::memo::TEXT_MEMO_LEN);
        prop_assert_eq!(DigMemo::decode_text(text.as_bytes()).expect("text"), memo.clone());
        let mut p = GameParams::testnet(1);
        p.text_memo_activation_height = Some(activation);
        prop_assert_eq!(DigMemo::decode_at(&p, height, &memo.encode()).expect("binary"), memo.clone());
        let decoded = DigMemo::decode_at(&p, height, text.as_bytes());
        if height >= activation {
            let decoded = decoded.expect("active");
            prop_assert_eq!(decoded.open(Network::Test, &sk).expect("open"), receiver);
            prop_assert_eq!(decoded, memo);
        } else {
            prop_assert_eq!(decoded, DigMemo::decode(text.as_bytes()));
            prop_assert!(DigMemo::decode(text.as_bytes()).is_err());
        }
    }

    #[test]
    fn sealed_receiver_rejects_wrong_key_and_wrong_aad(
        round in any::<u32>(), tile in 0u8..25, other_round in any::<u32>(), other_tile in 0u8..25, seed in any::<u64>()
    ) {
        prop_assume!(round != other_round || tile != other_tile);
        let mut rng = StdRng::seed_from_u64(seed);
        let (sk, pk) = generate_sealing_keypair(&mut rng);
        let (wrong_sk, _) = generate_sealing_keypair(&mut rng);
        let receiver = PayoutReceiver { network: Network::Test, orchard: [3; ORCHARD_RECEIVER_LEN] };
        let memo = DigMemo::seal(round, tile, &receiver, &pk, &mut rng).expect("seal");
        prop_assert!(memo.open(Network::Test, &wrong_sk).is_err());
        let moved = DigMemo { round: other_round, tile: other_tile, sealed_receiver: memo.sealed_receiver.clone() };
        prop_assert!(moved.open(Network::Test, &sk).is_err());
    }

    #[test]
    fn draws_are_always_in_range(round in any::<u32>(), seed in any::<[u8; 32]>()) {
        let r = Randomness(seed);
        prop_assert!(random::treasure_tile(round, &r).expect("tile") < 25);
        let p = GameParams::testnet(1);
        prop_assert!(random::uniform_below(CACHE_DOMAIN, round, &r, p.cache_odds).expect("v") < p.cache_odds);
    }
}

/// Chi-squared check that the treasure draw is uniform over the 25 tiles.
#[test]
fn treasure_draw_is_unbiased_over_many_samples() {
    const SAMPLES: u32 = 100_000;
    let mut counts = [0u64; 25];
    for i in 0..SAMPLES {
        let mut seed = [0u8; 32];
        seed[..4].copy_from_slice(&i.to_be_bytes());
        seed[4] = 0xA5;
        let tile = random::treasure_tile(7, &Randomness(seed)).expect("tile");
        counts[usize::from(tile)] += 1;
    }
    let expected = f64::from(SAMPLES) / 25.0;
    let chi2: f64 = counts
        .iter()
        .map(|c| {
            let d = *c as f64 - expected;
            d * d / expected
        })
        .sum();
    // 24 degrees of freedom: the 0.999 quantile is 51.2.
    assert!(chi2 < 51.2, "chi2 = {chi2}, counts = {counts:?}");
}

/// The Deep Cache triggers with frequency close to 1/625.
#[test]
fn cache_trigger_frequency_matches_the_odds() {
    const SAMPLES: u32 = 400_000;
    let p = GameParams::testnet(1);
    let mut hits = 0u64;
    let mut bucket_counts = [0u64; 625];
    for i in 0..SAMPLES {
        let mut seed = [0u8; 32];
        seed[..4].copy_from_slice(&i.to_be_bytes());
        seed[5] = 0x5A;
        let r = Randomness(seed);
        if random::cache_triggers(&p, 3, &r).expect("trigger") {
            hits += 1;
        }
        let v = random::uniform_below(CACHE_DOMAIN, 3, &r, 625).expect("v");
        bucket_counts[v as usize] += 1;
    }
    let expected = f64::from(SAMPLES) / 625.0; // 640
    assert!(
        (hits as f64 - expected).abs() < 4.0 * expected.sqrt(),
        "hits = {hits}"
    );
    let chi2: f64 = bucket_counts
        .iter()
        .map(|c| {
            let d = *c as f64 - expected;
            d * d / expected
        })
        .sum();
    // 624 degrees of freedom: the 0.999 quantile is about 741.
    assert!(chi2 < 741.0, "chi2 = {chi2}");
    // And the two domains are independent draws.
    let seed = [9u8; 32];
    assert_ne!(
        random::uniform_below(TREASURE_DOMAIN, 1, &Randomness(seed), 1 << 20).expect("a"),
        random::uniform_below(CACHE_DOMAIN, 1, &Randomness(seed), 1 << 20).expect("b")
    );
}
