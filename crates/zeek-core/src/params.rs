//! Per-network game parameters.
//!
//! Every number below is sourced:
//!
//! * Block spacing: Zcash protocol spec § 5.3 sets `PostBlossomPoWTargetSpacing`
//!   to 75 s (mainnet today; NU7 has no mainnet activation height). ZIP 218
//!   lowers it to 25 s from NU7, which is live on testnet (height 4,465,026).
//! * Rounds are sized to about ten minutes: 24 x 25 s on testnet, 8 x 75 s on
//!   mainnet.
//! * `marginal_fee_zat`: ZIP 317 `marginal_fee = 5000` zatoshis per logical
//!   action.
//! * Block timestamp rules: spec § 7.6, median-time-past over
//!   `PoWMedianBlockSpan = 11` blocks (see [`crate::timing`]).
//! * drand quicknet chain info: `https://api.drand.sh/v2/chains/52db…e971/info`
//!   (scheme `bls-unchained-g1-rfc9380`, period 3 s, genesis 1692803367).

use serde::{Deserialize, Serialize};

use crate::COIN;

/// Zcash network a game runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    /// Zcash mainnet.
    Main,
    /// Zcash testnet.
    Test,
}

impl Network {
    /// The byte that tags a payout receiver's network inside a sealed memo.
    pub const fn receiver_tag(self) -> u8 {
        match self {
            Network::Main => 0x00,
            Network::Test => 0x01,
        }
    }

    /// Inverse of [`Network::receiver_tag`].
    pub const fn from_receiver_tag(tag: u8) -> Option<Self> {
        match tag {
            0x00 => Some(Network::Main),
            0x01 => Some(Network::Test),
            _ => None,
        }
    }
}

/// Identity of a drand chain, as served by `/v2/chains/<hash>/info`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrandChain {
    /// Beacon id (`quicknet`).
    pub beacon_id: String,
    /// Chain hash, hex.
    pub chain_hash: String,
    /// BLS12-381 G2 public key (96 bytes), hex.
    pub public_key: String,
    /// Seconds between rounds.
    pub period_secs: u64,
    /// Unix time of round 1.
    pub genesis_time: u64,
    /// Signature scheme id; only `bls-unchained-g1-rfc9380` is supported.
    pub scheme: String,
}

impl DrandChain {
    /// The League of Entropy quicknet chain.
    ///
    /// Source: `https://api.drand.sh/v2/chains/52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971/info`.
    pub fn quicknet() -> Self {
        DrandChain {
            beacon_id: "quicknet".to_owned(),
            chain_hash: "52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971".to_owned(),
            public_key: "83cf0f2896adee7eb8b5f01fcad3912212c437e0073e911fb90022d3e760183c8c4b450b6a0a6c3ac6a5776a2d1064510d1fec758c921cc22b0e17e63aaf4bcb5ed66304de9cf809bd274ca73bab4af5a6e9c76a4bc09e76eae8991ef5ece45a".to_owned(),
            period_secs: 3,
            genesis_time: 1_692_803_367,
            scheme: SCHEME_BLS_UNCHAINED_G1_RFC9380.to_owned(),
        }
    }
}

/// The only drand scheme this crate verifies: signatures on G1, public key on
/// G2, unchained (message = SHA-256 of the big-endian round number), RFC 9380
/// hash-to-curve.
pub const SCHEME_BLS_UNCHAINED_G1_RFC9380: &str = "bls-unchained-g1-rfc9380";

/// Rake in basis points: 5%.
pub const RAKE_BPS: u64 = 500;
/// Deep Cache odds: 1 in 625 per round.
pub const CACHE_ODDS: u32 = 625;
/// Minimum dig: 0.001 ZEC.
pub const MIN_DIG_ZAT: u64 = COIN / 1000;
/// ZIP 317 marginal fee per logical action.
pub const ZIP317_MARGINAL_FEE_ZAT: u64 = 5_000;
/// Fee charged to every dig for the logical action that spends its note
/// (rules v2). Equal to the ZIP 317 marginal fee.
pub const DIG_FEE_ZAT: u64 = ZIP317_MARGINAL_FEE_ZAT;
/// Spec § 5.3 `PoWMedianBlockSpan`.
pub const POW_MEDIAN_BLOCK_SPAN: u32 = 11;
/// Spec § 5.3 `PostBlossomPoWTargetSpacing` (mainnet).
pub const POST_BLOSSOM_TARGET_SPACING_SECS: u64 = 75;
/// ZIP 218 `PostNU7PoWTargetSpacing` (testnet, NU7 active).
pub const POST_NU7_TARGET_SPACING_SECS: u64 = 25;
/// Bound on how far an honest miner's clock may lag or lead real time.
///
/// Honest Zcash nodes reject blocks more than two hours in the future (spec
/// § 7.6) and in practice keep clocks within seconds; five minutes is a
/// deliberately loose assumption for the argument in [`crate::timing`].
pub const HONEST_CLOCK_SKEW_SECS: u64 = 300;

/// Parameters fixing every rule of a Zeek game on one network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameParams {
    /// Network the vault lives on.
    pub network: Network,
    /// First block of round 0.
    pub genesis_height: u32,
    /// Blocks per round.
    pub blocks_per_round: u32,
    /// Target block spacing in effect, seconds.
    pub target_spacing_secs: u64,
    /// Rake in basis points of the pot, sent to the Deep Cache.
    pub rake_bps: u64,
    /// The Cache breaks open with probability `1 / cache_odds` per round.
    pub cache_odds: u32,
    /// Smallest dig that is counted; smaller digs are forfeited.
    pub min_dig_zat: u64,
    /// Fee charged per payout or refund output (ZIP 317 marginal fee).
    pub marginal_fee_zat: u64,
    /// Fee charged per dig note for the action that spends it (rules v2).
    /// Absent in configs and proof bundles written under rules v1, where it
    /// is zero: with no dig fee the v2 rules reduce exactly to v1.
    #[serde(default)]
    pub dig_fee_zat: u64,
    /// Block height from which a dig memo may also be sent as ZIP 302 text
    /// (`zeek1:` + base64url of the binary payload); rules v3. Absent (never)
    /// in configs and proof bundles of earlier rules versions: a text memo
    /// mined below this height is classified exactly as before, as an
    /// undecodable memo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_memo_activation_height: Option<u32>,
    /// Campfire (rules v4): from its activation height, dig memos may carry
    /// a message and shout memos (message only) are accepted. Absent in
    /// configs and proof bundles of earlier rules versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campfire: Option<CampfireParams>,
    /// drand chain that supplies the treasure randomness.
    pub drand: DrandChain,
    /// Seconds after the *expected* window close at which the round's beacon
    /// is scheduled; see [`crate::beacon::beacon_round_for`].
    pub reveal_delay_secs: u64,
    /// Blocks after the window whose timestamps form the median-time-past
    /// used by the timing rule (`PoWMedianBlockSpan`).
    pub mtp_window: u32,
    /// Assumed bound on honest miners' clock error, seconds.
    pub honest_clock_skew_secs: u64,
}

/// Smallest shout: 0.0005 ZEC.
pub const MIN_SHOUT_ZAT: u64 = 50_000;

/// Campfire parameters (rules v4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampfireParams {
    /// First block at which messages and shouts are recognised. Must not be
    /// below the text-memo activation height (rules v4 includes v3).
    pub activation_height: u32,
    /// Smallest shout that counts as a shout; smaller ones are forfeited
    /// whole to the Cache, like an undersized dig.
    pub min_shout_zat: u64,
}

/// Errors from validating [`GameParams`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParamsError {
    #[error("blocks_per_round must be at least 1")]
    ZeroBlocksPerRound,
    #[error("rake_bps {0} exceeds 10000")]
    RakeTooLarge(u64),
    #[error("cache_odds must be at least 1")]
    ZeroCacheOdds,
    #[error("mtp_window must be at least 1")]
    ZeroMtpWindow,
    #[error("target_spacing_secs must be at least 1")]
    ZeroTargetSpacing,
    #[error("drand period must be at least 1 second")]
    ZeroDrandPeriod,
    #[error("unsupported drand scheme `{0}`; only `bls-unchained-g1-rfc9380` is supported")]
    UnsupportedDrandScheme(String),
    #[error("drand public key is not 96 hex-encoded bytes")]
    BadDrandPublicKey,
    #[error("reveal_delay_secs is too short: it must cover the mtp_window blocks after the close plus the honest clock skew ({minimum} s), got {actual} s")]
    RevealDelayTooShort { minimum: u64, actual: u64 },
    #[error("min_dig_zat {min_dig} must exceed dig_fee_zat {dig_fee} so that every valid dig contributes to the pot")]
    DigFeeNotBelowMinimum { min_dig: u64, dig_fee: u64 },
    #[error("text_memo_activation_height {activation} is below genesis_height {genesis}")]
    TextMemoBeforeGenesis { activation: u32, genesis: u32 },
    #[error("campfire (rules v4) needs text memos (rules v3) active no later than its activation height")]
    CampfireBeforeTextMemos,
    #[error("min_shout_zat {min_shout} must exceed the dig fee {dig_fee}")]
    ShoutNotAboveDigFee { min_shout: u64, dig_fee: u64 },
    #[error("text memos (rules v3) require the dig fee of rules v2")]
    TextMemoWithoutDigFee,
    #[error("arithmetic overflow in parameters")]
    Overflow,
}

impl GameParams {
    /// Testnet parameters: 24 blocks of 25 s (NU7, ZIP 218) per round.
    pub fn testnet(genesis_height: u32) -> Self {
        Self::with_spacing(
            Network::Test,
            genesis_height,
            POST_NU7_TARGET_SPACING_SECS,
            24,
        )
    }

    /// Mainnet parameters: 8 blocks of 75 s (post-Blossom, spec § 5.3) per round.
    pub fn mainnet(genesis_height: u32) -> Self {
        Self::with_spacing(
            Network::Main,
            genesis_height,
            POST_BLOSSOM_TARGET_SPACING_SECS,
            8,
        )
    }

    fn with_spacing(
        network: Network,
        genesis_height: u32,
        target_spacing_secs: u64,
        blocks_per_round: u32,
    ) -> Self {
        GameParams {
            network,
            genesis_height,
            blocks_per_round,
            target_spacing_secs,
            rake_bps: RAKE_BPS,
            cache_odds: CACHE_ODDS,
            min_dig_zat: MIN_DIG_ZAT,
            marginal_fee_zat: ZIP317_MARGINAL_FEE_ZAT,
            dig_fee_zat: DIG_FEE_ZAT,
            text_memo_activation_height: None,
            campfire: None,
            drand: DrandChain::quicknet(),
            reveal_delay_secs: Self::minimum_reveal_delay(
                target_spacing_secs,
                POW_MEDIAN_BLOCK_SPAN,
                HONEST_CLOCK_SKEW_SECS,
            ),
            mtp_window: POW_MEDIAN_BLOCK_SPAN,
            honest_clock_skew_secs: HONEST_CLOCK_SKEW_SECS,
        }
    }

    /// The smallest reveal delay the timing rule can work with: time for the
    /// `mtp_window` post-close blocks plus one more at **twice** the target
    /// spacing (so a moderately slow chain is not voided), plus the assumed
    /// clock skew. Testnet: 300 + 12·25·2 = 900 s; mainnet: 300 + 12·75·2 =
    /// 2100 s.
    pub fn minimum_reveal_delay(target_spacing_secs: u64, mtp_window: u32, skew: u64) -> u64 {
        2 * target_spacing_secs * (u64::from(mtp_window) + 1) + skew
    }

    /// Checks internal consistency.
    pub fn validate(&self) -> Result<(), ParamsError> {
        if self.blocks_per_round == 0 {
            return Err(ParamsError::ZeroBlocksPerRound);
        }
        if self.rake_bps > 10_000 {
            return Err(ParamsError::RakeTooLarge(self.rake_bps));
        }
        if self.cache_odds == 0 {
            return Err(ParamsError::ZeroCacheOdds);
        }
        if self.mtp_window == 0 {
            return Err(ParamsError::ZeroMtpWindow);
        }
        if self.target_spacing_secs == 0 {
            return Err(ParamsError::ZeroTargetSpacing);
        }
        if self.min_dig_zat <= self.dig_fee_zat {
            return Err(ParamsError::DigFeeNotBelowMinimum {
                min_dig: self.min_dig_zat,
                dig_fee: self.dig_fee_zat,
            });
        }
        if self.drand.period_secs == 0 {
            return Err(ParamsError::ZeroDrandPeriod);
        }
        if self.drand.scheme != SCHEME_BLS_UNCHAINED_G1_RFC9380 {
            return Err(ParamsError::UnsupportedDrandScheme(
                self.drand.scheme.clone(),
            ));
        }
        if let Some(activation) = self.text_memo_activation_height {
            if activation < self.genesis_height {
                return Err(ParamsError::TextMemoBeforeGenesis {
                    activation,
                    genesis: self.genesis_height,
                });
            }
            if self.dig_fee_zat == 0 {
                return Err(ParamsError::TextMemoWithoutDigFee);
            }
        }
        if let Some(campfire) = &self.campfire {
            match self.text_memo_activation_height {
                Some(text) if text <= campfire.activation_height => {}
                _ => return Err(ParamsError::CampfireBeforeTextMemos),
            }
            if campfire.min_shout_zat <= self.dig_fee_zat {
                return Err(ParamsError::ShoutNotAboveDigFee {
                    min_shout: campfire.min_shout_zat,
                    dig_fee: self.dig_fee_zat,
                });
            }
        }
        match hex::decode(&self.drand.public_key) {
            Ok(bytes) if bytes.len() == 96 => {}
            _ => return Err(ParamsError::BadDrandPublicKey),
        }
        let minimum = self
            .target_spacing_secs
            .checked_mul(2)
            .and_then(|s| s.checked_mul(u64::from(self.mtp_window) + 1))
            .and_then(|s| s.checked_add(self.honest_clock_skew_secs))
            .ok_or(ParamsError::Overflow)?;
        if self.reveal_delay_secs < minimum {
            return Err(ParamsError::RevealDelayTooShort {
                minimum,
                actual: self.reveal_delay_secs,
            });
        }
        Ok(())
    }

    /// Fee rules version the parameters encode: 1 without a dig fee, 2 with
    /// one. Whether a given round is additionally under rules v3 (text memos)
    /// depends on its window; see [`crate::rounds::rules_version`].
    pub fn rules_version(&self) -> u32 {
        if self.dig_fee_zat == 0 {
            1
        } else {
            2
        }
    }

    /// Whether a note mined at `height` may carry the text encoding of a dig memo.
    pub fn text_memo_active(&self, height: u32) -> bool {
        self.text_memo_activation_height
            .is_some_and(|activation| height >= activation)
    }

    /// Whether a note mined at `height` may carry a Campfire message or be a
    /// shout (rules v4).
    pub fn campfire_active(&self, height: u32) -> bool {
        self.campfire
            .as_ref()
            .is_some_and(|c| height >= c.activation_height)
    }

    /// Expected wall-clock length of a round.
    pub fn round_duration_secs(&self) -> Result<u64, ParamsError> {
        u64::from(self.blocks_per_round)
            .checked_mul(self.target_spacing_secs)
            .ok_or(ParamsError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate_and_last_about_ten_minutes() {
        for params in [GameParams::testnet(1), GameParams::mainnet(1)] {
            params.validate().expect("valid");
            assert_eq!(params.round_duration_secs().expect("duration"), 600);
            assert_eq!(params.rake_bps, 500);
            assert_eq!(params.cache_odds, 625);
            assert_eq!(params.min_dig_zat, 100_000);
            assert_eq!(params.marginal_fee_zat, 5_000);
            assert_eq!(params.dig_fee_zat, 5_000);
            assert_eq!(params.rules_version(), 2);
        }
        assert_eq!(GameParams::testnet(1).reveal_delay_secs, 900);
        assert_eq!(GameParams::mainnet(1).reveal_delay_secs, 2_100);
    }

    #[test]
    fn validation_catches_bad_values() {
        let mut p = GameParams::testnet(1);
        p.rake_bps = 10_001;
        assert_eq!(p.validate(), Err(ParamsError::RakeTooLarge(10_001)));
        let mut p = GameParams::testnet(1);
        p.reveal_delay_secs = 10;
        assert!(matches!(
            p.validate(),
            Err(ParamsError::RevealDelayTooShort {
                minimum: 900,
                actual: 10
            })
        ));
        let mut p = GameParams::testnet(1);
        p.drand.scheme = "pedersen-bls-chained".into();
        assert!(matches!(
            p.validate(),
            Err(ParamsError::UnsupportedDrandScheme(_))
        ));
        let mut p = GameParams::testnet(1);
        p.drand.public_key = "abcd".into();
        assert_eq!(p.validate(), Err(ParamsError::BadDrandPublicKey));
        let mut p = GameParams::testnet(1);
        p.blocks_per_round = 0;
        assert_eq!(p.validate(), Err(ParamsError::ZeroBlocksPerRound));
        let mut p = GameParams::testnet(1);
        p.dig_fee_zat = p.min_dig_zat;
        assert_eq!(
            p.validate(),
            Err(ParamsError::DigFeeNotBelowMinimum {
                min_dig: 100_000,
                dig_fee: 100_000
            })
        );
    }

    #[test]
    fn text_memo_activation_is_optional_and_validated() {
        let mut p = GameParams::testnet(100);
        assert!(!p.text_memo_active(u32::MAX));
        let json = serde_json::to_string(&p).expect("json");
        assert!(
            !json.contains("text_memo_activation_height"),
            "rules v2 parameters serialize unchanged"
        );
        p.text_memo_activation_height = Some(150);
        p.validate().expect("valid");
        assert!(!p.text_memo_active(149));
        assert!(p.text_memo_active(150));
        let back: GameParams =
            serde_json::from_str(&serde_json::to_string(&p).expect("json")).expect("parse");
        assert_eq!(back, p);
        p.text_memo_activation_height = Some(99);
        assert_eq!(
            p.validate(),
            Err(ParamsError::TextMemoBeforeGenesis {
                activation: 99,
                genesis: 100
            })
        );
        p.text_memo_activation_height = Some(150);
        p.dig_fee_zat = 0;
        assert_eq!(p.validate(), Err(ParamsError::TextMemoWithoutDigFee));
    }

    #[test]
    fn campfire_is_optional_and_validated() {
        let mut p = GameParams::testnet(100);
        assert!(!p.campfire_active(u32::MAX));
        assert!(!serde_json::to_string(&p)
            .expect("json")
            .contains("campfire"));
        p.campfire = Some(CampfireParams {
            activation_height: 200,
            min_shout_zat: MIN_SHOUT_ZAT,
        });
        assert_eq!(p.validate(), Err(ParamsError::CampfireBeforeTextMemos));
        p.text_memo_activation_height = Some(150);
        p.validate().expect("valid");
        assert!(!p.campfire_active(199));
        assert!(p.campfire_active(200));
        p.text_memo_activation_height = Some(201);
        assert_eq!(p.validate(), Err(ParamsError::CampfireBeforeTextMemos));
        p.text_memo_activation_height = Some(150);
        p.campfire = Some(CampfireParams {
            activation_height: 200,
            min_shout_zat: 5_000,
        });
        assert_eq!(
            p.validate(),
            Err(ParamsError::ShoutNotAboveDigFee {
                min_shout: 5_000,
                dig_fee: 5_000
            })
        );
    }

    #[test]
    fn rules_v1_params_parse_without_a_dig_fee() {
        let mut json: serde_json::Value =
            serde_json::to_value(GameParams::testnet(1)).expect("json");
        json.as_object_mut()
            .expect("object")
            .remove("dig_fee_zat")
            .expect("present");
        let p: GameParams = serde_json::from_value(json).expect("parse");
        assert_eq!(p.dig_fee_zat, 0);
        assert_eq!(p.rules_version(), 1);
        p.validate().expect("v1 parameters stay valid");
    }

    #[test]
    fn params_round_trip_through_serde() {
        let p = GameParams::testnet(4_500_000);
        let json = serde_json::to_string(&p).expect("json");
        let back: GameParams = serde_json::from_str(&json).expect("parse");
        assert_eq!(p, back);
        assert!(json.contains("\"network\":\"test\""));
    }

    #[test]
    fn network_tags_round_trip() {
        for n in [Network::Main, Network::Test] {
            assert_eq!(Network::from_receiver_tag(n.receiver_tag()), Some(n));
        }
        assert_eq!(Network::from_receiver_tag(0x02), None);
    }
}
