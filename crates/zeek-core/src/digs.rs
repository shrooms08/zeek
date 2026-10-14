//! Classification of the notes a round received.
//!
//! Game rule (documented here because it decides who gets money back):
//!
//! * **Valid** — the note is inside the round's window, its memo decodes, the
//!   memo's round is this round, the tile is in range and the value is at
//!   least `min_dig_zat`. It takes part in the draw.
//! * **Refund** — the memo decodes (so the sealed payout receiver is known)
//!   but the dig cannot play: it names another round, lies outside the window,
//!   or the round was voided. The value comes back minus one output fee.
//! * **Forfeit to the Deep Cache** — the memo does not decode (there is no
//!   receiver to refund to; a memo with a tile above 24 is undecodable), or
//!   the value is below `min_dig_zat` (a refund would cost more in fees than
//!   it returns). The value is added to the Cache.
//!
//! From the text-memo activation height (rules v3) a memo may use either the
//! binary or the `zeek1:` text encoding (see [`crate::memo`]); below it a
//! text memo is undecodable, as it always was.
//!
//! From the Campfire activation height (rules v4) a memo may also be a
//! **shout** (a message, no tile, no receiver). A shout of at least
//! `min_shout_zat` is never refunded, whatever its round or window: its
//! value goes to the Deep Cache minus the dig fee reserved for spending its
//! note later. A smaller shout is forfeited whole. Dig messages never affect
//! classification.
//!
//! Classification never looks inside the sealed receiver, so it runs without
//! the vault's private sealing key.

use serde::{Deserialize, Serialize};

use crate::memo::{DigMemo, SealedReceiver, ZeekMemo};
use crate::params::GameParams;
use crate::rounds::{round_window, RoundError};

/// A shielded note the vault received, as the wallet reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceivedNote {
    /// Transaction id (display hex), for reporting.
    pub txid: String,
    /// Action index within the transaction.
    pub output_index: u16,
    /// Block height the note was mined at.
    pub height: u32,
    /// Note value in zatoshis.
    pub value: u64,
    /// The 512-byte memo field.
    #[serde(with = "hex_bytes")]
    pub memo: Vec<u8>,
}

mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        hex::decode(text).map_err(serde::de::Error::custom)
    }
}

/// Why a dig is refunded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundReason {
    /// The memo names a different round.
    WrongRound { memo_round: u32 },
    /// The note was mined outside the round's window.
    OutOfWindow,
    /// The round was voided by the timing rule.
    RoundVoided,
}

/// Why a note is forfeited to the Deep Cache.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForfeitReason {
    /// The memo is not a decodable dig memo.
    UndecodableMemo { error: String },
    /// The value is below `min_dig_zat` (or `min_shout_zat` for a shout).
    BelowMinimum { minimum: u64 },
}

/// A shout (rules v4): its value feeds the Deep Cache.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shout {
    /// Source note.
    pub note: ReceivedNote,
    /// The message, when valid.
    pub message: Option<String>,
}

/// A dig that takes part in the draw.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidDig {
    /// Source note.
    pub note: ReceivedNote,
    /// Tile dug.
    pub tile: u8,
    /// Sealed payout receiver.
    pub sealed_receiver: SealedReceiver,
}

/// A dig that is returned to its digger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundDig {
    /// Source note.
    pub note: ReceivedNote,
    /// Sealed payout receiver, with the memo's own round and tile needed to
    /// open it.
    pub memo: DigMemo,
    /// Why it is refunded.
    pub reason: RefundReason,
}

/// A note forfeited to the Deep Cache.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forfeit {
    /// Source note.
    pub note: ReceivedNote,
    /// Why it is forfeited.
    pub reason: ForfeitReason,
}

/// The classification of one note.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Classified {
    Valid(ValidDig),
    Refund(RefundDig),
    Forfeit(Forfeit),
    Shout(Shout),
}

/// Every note of a round, classified.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassifiedDigs {
    pub valid: Vec<ValidDig>,
    pub refunds: Vec<RefundDig>,
    pub forfeits: Vec<Forfeit>,
    /// Shouts (rules v4); empty before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shouts: Vec<Shout>,
}

impl ClassifiedDigs {
    /// Sum of the valid digs (the pot before rake).
    pub fn pot(&self) -> Option<u64> {
        self.valid
            .iter()
            .try_fold(0u64, |acc, d| acc.checked_add(d.note.value))
    }

    /// Sum of the refunded digs.
    pub fn refunded_value(&self) -> Option<u64> {
        self.refunds
            .iter()
            .try_fold(0u64, |acc, d| acc.checked_add(d.note.value))
    }

    /// Sum of the shouts.
    pub fn shouted_value(&self) -> Option<u64> {
        self.shouts
            .iter()
            .try_fold(0u64, |acc, d| acc.checked_add(d.note.value))
    }

    /// Sum of the forfeited notes.
    pub fn forfeited_value(&self) -> Option<u64> {
        self.forfeits
            .iter()
            .try_fold(0u64, |acc, d| acc.checked_add(d.note.value))
    }
}

/// Classifies one note received for `round`.
pub fn classify(
    params: &GameParams,
    round: u32,
    voided: bool,
    note: ReceivedNote,
) -> Result<Classified, RoundError> {
    let window = round_window(params, round)?;
    let memo = match ZeekMemo::decode_at(params, note.height, &note.memo) {
        Ok(ZeekMemo::Dig(memo)) => memo,
        Ok(ZeekMemo::Shout(shout)) => {
            let minimum = params
                .campfire
                .as_ref()
                .map_or(u64::MAX, |c| c.min_shout_zat);
            return Ok(if note.value < minimum {
                Classified::Forfeit(Forfeit {
                    note,
                    reason: ForfeitReason::BelowMinimum { minimum },
                })
            } else {
                Classified::Shout(Shout {
                    note,
                    message: shout.message,
                })
            });
        }
        Err(error) => {
            return Ok(Classified::Forfeit(Forfeit {
                note,
                reason: ForfeitReason::UndecodableMemo {
                    error: error.to_string(),
                },
            }))
        }
    };
    if note.value < params.min_dig_zat {
        return Ok(Classified::Forfeit(Forfeit {
            note,
            reason: ForfeitReason::BelowMinimum {
                minimum: params.min_dig_zat,
            },
        }));
    }
    let reason = if voided {
        Some(RefundReason::RoundVoided)
    } else if !window.contains(note.height) {
        Some(RefundReason::OutOfWindow)
    } else if memo.round != round {
        Some(RefundReason::WrongRound {
            memo_round: memo.round,
        })
    } else {
        None
    };
    Ok(match reason {
        Some(reason) => Classified::Refund(RefundDig { note, memo, reason }),
        None => Classified::Valid(ValidDig {
            tile: memo.tile,
            sealed_receiver: memo.sealed_receiver,
            note,
        }),
    })
}

/// Classifies every note of `round`.
pub fn classify_round(
    params: &GameParams,
    round: u32,
    voided: bool,
    notes: impl IntoIterator<Item = ReceivedNote>,
) -> Result<ClassifiedDigs, RoundError> {
    let mut out = ClassifiedDigs::default();
    for note in notes {
        match classify(params, round, voided, note)? {
            Classified::Valid(d) => out.valid.push(d),
            Classified::Refund(d) => out.refunds.push(d),
            Classified::Forfeit(d) => out.forfeits.push(d),
            Classified::Shout(d) => out.shouts.push(d),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memo::{PayoutReceiver, ORCHARD_RECEIVER_LEN};
    use crate::params::Network;
    use crate::seal::generate_sealing_keypair;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn params() -> GameParams {
        GameParams::testnet(1_000)
    }

    fn memo_for(round: u32, tile: u8, rng: &mut StdRng) -> DigMemo {
        let (_, pk) = generate_sealing_keypair(rng);
        let receiver = PayoutReceiver {
            network: Network::Test,
            orchard: [tile; ORCHARD_RECEIVER_LEN],
        };
        DigMemo::seal(round, tile, &receiver, &pk, rng).expect("seal")
    }

    fn note(height: u32, value: u64, memo: Vec<u8>) -> ReceivedNote {
        ReceivedNote {
            txid: "ab".repeat(32),
            output_index: 0,
            height,
            value,
            memo,
        }
    }

    #[test]
    fn classification_follows_the_rules() {
        let p = params();
        let mut rng = StdRng::seed_from_u64(8);
        let w = round_window(&p, 2).expect("window");
        let good = memo_for(2, 5, &mut rng);

        let valid =
            classify(&p, 2, false, note(w.start, 100_000, good.encode().to_vec())).expect("c");
        assert!(matches!(valid, Classified::Valid(ValidDig { tile: 5, .. })));

        let wrong_round = classify(
            &p,
            2,
            false,
            note(w.start, 100_000, memo_for(3, 5, &mut rng).encode().to_vec()),
        )
        .expect("c");
        assert!(matches!(
            wrong_round,
            Classified::Refund(RefundDig {
                reason: RefundReason::WrongRound { memo_round: 3 },
                ..
            })
        ));

        let outside =
            classify(&p, 2, false, note(w.end, 100_000, good.encode().to_vec())).expect("c");
        assert!(matches!(
            outside,
            Classified::Refund(RefundDig {
                reason: RefundReason::OutOfWindow,
                ..
            })
        ));

        let voided =
            classify(&p, 2, true, note(w.start, 100_000, good.encode().to_vec())).expect("c");
        assert!(matches!(
            voided,
            Classified::Refund(RefundDig {
                reason: RefundReason::RoundVoided,
                ..
            })
        ));

        let dust =
            classify(&p, 2, false, note(w.start, 99_999, good.encode().to_vec())).expect("c");
        assert!(matches!(
            dust,
            Classified::Forfeit(Forfeit {
                reason: ForfeitReason::BelowMinimum { minimum: 100_000 },
                ..
            })
        ));

        let text_memo =
            classify(&p, 2, false, note(w.start, 1_000_000, b"hello".to_vec())).expect("c");
        assert!(matches!(
            text_memo,
            Classified::Forfeit(Forfeit {
                reason: ForfeitReason::UndecodableMemo { .. },
                ..
            })
        ));

        let mut bad_tile = good.encode();
        bad_tile[10] = 25;
        let out_of_range =
            classify(&p, 2, false, note(w.start, 1_000_000, bad_tile.to_vec())).expect("c");
        assert!(matches!(
            out_of_range,
            Classified::Forfeit(Forfeit {
                reason: ForfeitReason::UndecodableMemo { .. },
                ..
            })
        ));
    }

    #[test]
    fn text_memos_count_only_from_the_activation_height() {
        let mut p = params();
        let mut rng = StdRng::seed_from_u64(10);
        let w = round_window(&p, 2).expect("window");
        let memo = memo_for(2, 5, &mut rng);
        let text = memo.encode_text().into_bytes();
        // Rules v2: a text memo is undecodable and forfeited.
        let before = classify(&p, 2, false, note(w.start + 3, 100_000, text.clone())).expect("c");
        assert!(matches!(
            before,
            Classified::Forfeit(Forfeit {
                reason: ForfeitReason::UndecodableMemo { .. },
                ..
            })
        ));
        // Rules v3 from w.start + 2: the note at w.start + 1 is still a
        // forfeit, the one at w.start + 3 is the same dig as the binary memo.
        p.text_memo_activation_height = Some(w.start + 2);
        let early = classify(&p, 2, false, note(w.start + 1, 100_000, text.clone())).expect("c");
        assert_eq!(
            early,
            classify(
                &params(),
                2,
                false,
                note(w.start + 1, 100_000, text.clone())
            )
            .expect("c"),
            "classified exactly as before activation"
        );
        let active = classify(&p, 2, false, note(w.start + 3, 100_000, text)).expect("c");
        let binary = classify(
            &p,
            2,
            false,
            note(w.start + 3, 100_000, memo.encode().to_vec()),
        )
        .expect("c");
        match (active, binary) {
            (Classified::Valid(a), Classified::Valid(b)) => {
                assert_eq!(a.tile, b.tile);
                assert_eq!(a.sealed_receiver, b.sealed_receiver);
            }
            other => panic!("expected two valid digs, got {other:?}"),
        }
    }

    #[test]
    fn shouts_are_classified_from_campfire_activation() {
        use crate::memo::ShoutMemo;
        use crate::params::{CampfireParams, MIN_SHOUT_ZAT};
        let mut p = params();
        let w = round_window(&p, 3).expect("window");
        p.text_memo_activation_height = Some(w.start);
        p.campfire = Some(CampfireParams {
            activation_height: w.start + 2,
            min_shout_zat: MIN_SHOUT_ZAT,
        });
        let shout = ShoutMemo::new("hello").expect("shout").encode().to_vec();
        // Before activation: undecodable, forfeited (as under rules v3).
        assert!(matches!(
            classify(&p, 3, false, note(w.start + 1, 60_000, shout.clone())).expect("c"),
            Classified::Forfeit(Forfeit {
                reason: ForfeitReason::UndecodableMemo { .. },
                ..
            })
        ));
        // From activation: a shout, in a voided round too; below the minimum
        // it is forfeited whole.
        for voided in [false, true] {
            match classify(&p, 3, voided, note(w.start + 2, 60_000, shout.clone())).expect("c") {
                Classified::Shout(s) => assert_eq!(s.message.as_deref(), Some("hello")),
                other => panic!("expected a shout, got {other:?}"),
            }
        }
        assert!(matches!(
            classify(&p, 3, false, note(w.start + 2, 49_999, shout)).expect("c"),
            Classified::Forfeit(Forfeit {
                reason: ForfeitReason::BelowMinimum { minimum: 50_000 },
                ..
            })
        ));
    }

    #[test]
    fn classify_round_sums_each_bucket() {
        let p = params();
        let mut rng = StdRng::seed_from_u64(9);
        let w = round_window(&p, 1).expect("window");
        let notes = vec![
            note(w.start, 200_000, memo_for(1, 0, &mut rng).encode().to_vec()),
            note(
                w.last(),
                300_000,
                memo_for(1, 0, &mut rng).encode().to_vec(),
            ),
            note(w.start, 400_000, memo_for(9, 0, &mut rng).encode().to_vec()),
            note(w.start, 50_000, memo_for(1, 0, &mut rng).encode().to_vec()),
            note(w.start, 60_000, vec![0xF6; 512]),
        ];
        let c = classify_round(&p, 1, false, notes).expect("classify");
        assert_eq!(c.valid.len(), 2);
        assert_eq!(c.refunds.len(), 1);
        assert_eq!(c.forfeits.len(), 2);
        assert_eq!(c.pot(), Some(500_000));
        assert_eq!(c.refunded_value(), Some(400_000));
        assert_eq!(c.forfeited_value(), Some(110_000));
    }
}
