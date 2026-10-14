//! The dig memo.
//!
//! A dig is a shielded payment to the vault whose 512-byte memo (ZIP 302) is
//! arbitrary data (first byte `0xFF`) laid out as:
//!
//! ```text
//! 0xFF | "ZEEK" | version u8 = 1 | round u32 BE | tile u8 (0..=24) | sealed payout receiver (92 bytes)
//! ```
//!
//! followed by zero padding to 512 bytes. The sealed payout receiver is the
//! HPKE encryption (see [`crate::seal`]) of the 44-byte plaintext
//! `orchard_receiver (43 bytes) || network tag (1 byte)` under the vault's
//! sealing key with AAD `"ZEEK" || version || round BE || tile`, so it cannot
//! be moved to another round or tile. Only Orchard-protocol receivers are
//! representable: this is the receiver the Ironwood pool pays to, and
//! transparent payouts are not supported.
//!
//! # Text encoding (rules v3)
//!
//! Some wallets can only send UTF-8 text memos. The same payload may
//! therefore be sent as a ZIP 302 text memo:
//!
//! ```text
//! "zeek1:" | base64url, no padding, of the 102 bytes after the 0xFF byte
//! ```
//!
//! that is 142 ASCII characters, zero-padded to 512 bytes like any text
//! memo. Both encodings decode to the same [`DigMemo`]; the sealed receiver
//! and its AAD are identical. Text memos only count from the game's
//! activation height ([`crate::params::GameParams::text_memo_activation_height`]);
//! [`DigMemo::decode_at`] applies that rule.
//!
//! Decoding is strict: any deviation (wrong lead byte, magic, version, a tile
//! above 24, a sealed receiver of the wrong size, non-zero padding) is an
//! error, and such a memo is a forfeit (see [`crate::digs`]).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand_core_09::{CryptoRng, RngCore};
use serde::{Deserialize, Serialize};
use zcash_address::unified::{self, Container, Encoding, Receiver};
use zcash_address::{ConversionError, ToAddress, TryFromAddress, ZcashAddress};
use zcash_protocol::consensus::NetworkType;

use crate::params::Network;
use crate::seal::{
    self, SealError, SealingPrivateKey, SealingPublicKey, ENCAPPED_KEY_LEN, TAG_LEN,
};
use crate::TILE_COUNT;

/// Size of every memo field.
pub const MEMO_LEN: usize = 512;
/// ZIP 302 lead byte for arbitrary data.
pub const ARBITRARY_DATA_LEAD: u8 = 0xFF;
/// Magic bytes identifying a Zeek dig.
pub const MAGIC: [u8; 4] = *b"ZEEK";
/// Memo format version.
pub const VERSION: u8 = 1;
/// Length of a raw Orchard-protocol receiver.
pub const ORCHARD_RECEIVER_LEN: usize = 43;
/// Length of the sealed plaintext: receiver plus network tag.
pub const RECEIVER_PLAINTEXT_LEN: usize = ORCHARD_RECEIVER_LEN + 1;
/// Length of a sealed receiver: encapsulated key, plaintext, tag.
pub const SEALED_RECEIVER_LEN: usize = ENCAPPED_KEY_LEN + RECEIVER_PLAINTEXT_LEN + TAG_LEN;
/// Length of the public header `0xFF | magic | version | round | tile`.
pub const HEADER_LEN: usize = 1 + 4 + 1 + 4 + 1;
/// Length of an unpadded dig memo.
pub const DIG_MEMO_LEN: usize = HEADER_LEN + SEALED_RECEIVER_LEN;
/// Prefix of the text encoding of a dig memo.
pub const TEXT_PREFIX: &str = "zeek1:";
/// Length of the payload the text encoding carries (everything after `0xFF`).
pub const TEXT_PAYLOAD_LEN: usize = DIG_MEMO_LEN - 1;
/// Length of the text encoding: prefix plus unpadded base64url of the payload.
pub const TEXT_MEMO_LEN: usize = TEXT_PREFIX.len() + TEXT_PAYLOAD_LEN.div_ceil(3) * 4;
/// Length of the AAD: `magic | version | round | tile`.
pub const AAD_LEN: usize = 4 + 1 + 4 + 1;

/// Errors from encoding, decoding, sealing and opening dig memos.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MemoError {
    #[error("memo is {0} bytes, longer than the 512-byte memo field")]
    TooLong(usize),
    #[error("memo is {0} bytes, shorter than a dig memo ({DIG_MEMO_LEN} bytes)")]
    TooShort(usize),
    #[error("memo lead byte is {0:#04x}, not the ZIP 302 arbitrary-data byte 0xFF")]
    NotArbitraryData(u8),
    #[error("memo does not carry the ZEEK magic")]
    BadMagic,
    #[error("unsupported dig memo version {0}")]
    UnsupportedVersion(u8),
    #[error("tile {0} is out of range (0..=24)")]
    TileOutOfRange(u8),
    #[error("memo bytes after the dig memo are not all zero")]
    NonZeroPadding,
    #[error("text memo is not a well-formed `zeek1:` dig memo: {0}")]
    BadText(&'static str),
    #[error("sealed receiver must be {SEALED_RECEIVER_LEN} bytes, got {0}")]
    BadSealedLength(usize),
    #[error("opened receiver has {0} bytes, expected {RECEIVER_PLAINTEXT_LEN}")]
    BadPlaintextLength(usize),
    #[error("opened receiver carries unknown network tag {0:#04x}")]
    UnknownNetworkTag(u8),
    #[error("opened receiver is for the wrong network")]
    WrongNetwork,
    #[error("address is not a valid Zcash address: {0}")]
    InvalidAddress(String),
    #[error("address is for the wrong network")]
    AddressWrongNetwork,
    #[error(
        "address has no Orchard receiver; only shielded Orchard-protocol payouts are supported"
    )]
    NoOrchardReceiver,
    #[error("cannot encode the receiver as a unified address: {0}")]
    AddressEncoding(String),
    #[error(transparent)]
    Seal(#[from] SealError),
}

/// Where a dig's winnings or refund are paid: an Orchard-protocol receiver.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PayoutReceiver {
    /// Network the receiver belongs to.
    pub network: Network,
    /// Raw 43-byte Orchard receiver (ZIP 316 typecode 3).
    #[serde(with = "hex_43")]
    pub orchard: [u8; ORCHARD_RECEIVER_LEN],
}

mod hex_43 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 43], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 43], D::Error> {
        let text = String::deserialize(d)?;
        let bytes = hex::decode(text).map_err(serde::de::Error::custom)?;
        <[u8; 43]>::try_from(bytes.as_slice())
            .map_err(|_| serde::de::Error::custom("orchard receiver must be 43 bytes"))
    }
}

impl core::fmt::Debug for PayoutReceiver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "PayoutReceiver({:?}, {})",
            self.network,
            hex::encode(self.orchard)
        )
    }
}

/// Extracts the Orchard receiver of a unified address; every other address
/// kind is rejected.
struct OrchardOnly([u8; ORCHARD_RECEIVER_LEN]);

impl TryFromAddress for OrchardOnly {
    type Error = MemoError;

    fn try_from_unified(
        _net: NetworkType,
        data: unified::Address,
    ) -> Result<Self, ConversionError<Self::Error>> {
        data.items()
            .into_iter()
            .find_map(|item| match item {
                Receiver::Orchard(raw) => Some(OrchardOnly(raw)),
                _ => None,
            })
            .ok_or(ConversionError::User(MemoError::NoOrchardReceiver))
    }
}

fn network_type(network: Network) -> NetworkType {
    match network {
        Network::Main => NetworkType::Main,
        Network::Test => NetworkType::Test,
    }
}

impl PayoutReceiver {
    /// Parses a unified address of `network` and keeps its Orchard receiver.
    pub fn from_unified_address(network: Network, address: &str) -> Result<Self, MemoError> {
        let parsed = ZcashAddress::try_from_encoded(address.trim())
            .map_err(|e| MemoError::InvalidAddress(e.to_string()))?;
        match parsed.convert_if_network::<OrchardOnly>(network_type(network)) {
            Ok(OrchardOnly(orchard)) => Ok(PayoutReceiver { network, orchard }),
            Err(ConversionError::IncorrectNetwork { .. }) => Err(MemoError::AddressWrongNetwork),
            Err(ConversionError::Unsupported(_)) => Err(MemoError::NoOrchardReceiver),
            Err(ConversionError::User(e)) => Err(e),
        }
    }

    /// Encodes the receiver as a unified address with only an Orchard item.
    pub fn to_unified_address(&self) -> Result<String, MemoError> {
        let ua = unified::Address::try_from_items(vec![Receiver::Orchard(self.orchard)])
            .map_err(|e| MemoError::AddressEncoding(e.to_string()))?;
        Ok(ZcashAddress::from_unified(network_type(self.network), ua).encode())
    }

    /// The 44-byte sealed plaintext.
    pub fn to_plaintext(&self) -> [u8; RECEIVER_PLAINTEXT_LEN] {
        let mut out = [0u8; RECEIVER_PLAINTEXT_LEN];
        out[..ORCHARD_RECEIVER_LEN].copy_from_slice(&self.orchard);
        out[ORCHARD_RECEIVER_LEN] = self.network.receiver_tag();
        out
    }

    /// Parses a sealed plaintext, checking it is for `expected` network.
    pub fn from_plaintext(expected: Network, bytes: &[u8]) -> Result<Self, MemoError> {
        if bytes.len() != RECEIVER_PLAINTEXT_LEN {
            return Err(MemoError::BadPlaintextLength(bytes.len()));
        }
        let tag = bytes[ORCHARD_RECEIVER_LEN];
        let network = Network::from_receiver_tag(tag).ok_or(MemoError::UnknownNetworkTag(tag))?;
        if network != expected {
            return Err(MemoError::WrongNetwork);
        }
        let mut orchard = [0u8; ORCHARD_RECEIVER_LEN];
        orchard.copy_from_slice(&bytes[..ORCHARD_RECEIVER_LEN]);
        Ok(PayoutReceiver { network, orchard })
    }
}

/// An HPKE-sealed payout receiver (92 bytes, hex in serde).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SealedReceiver([u8; SEALED_RECEIVER_LEN]);

impl SealedReceiver {
    /// Wraps sealed bytes, checking the length.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MemoError> {
        <[u8; SEALED_RECEIVER_LEN]>::try_from(bytes)
            .map(SealedReceiver)
            .map_err(|_| MemoError::BadSealedLength(bytes.len()))
    }

    /// The sealed bytes.
    pub fn as_bytes(&self) -> &[u8; SEALED_RECEIVER_LEN] {
        &self.0
    }
}

impl core::fmt::Debug for SealedReceiver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "SealedReceiver({})", hex::encode(self.0))
    }
}

impl Serialize for SealedReceiver {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(self.0))
    }
}

impl<'de> Deserialize<'de> for SealedReceiver {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let bytes = hex::decode(text).map_err(serde::de::Error::custom)?;
        SealedReceiver::from_bytes(&bytes).map_err(serde::de::Error::custom)
    }
}

/// A decoded dig memo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigMemo {
    /// Round the digger is playing.
    pub round: u32,
    /// Tile being dug, `0..=24`.
    pub tile: u8,
    /// The digger's sealed payout receiver.
    pub sealed_receiver: SealedReceiver,
}

/// The AAD binding a sealed receiver to its round and tile.
pub fn aad(round: u32, tile: u8) -> [u8; AAD_LEN] {
    let mut out = [0u8; AAD_LEN];
    out[..4].copy_from_slice(&MAGIC);
    out[4] = VERSION;
    out[5..9].copy_from_slice(&round.to_be_bytes());
    out[9] = tile;
    out
}

impl DigMemo {
    /// Seals `receiver` to the vault's key and builds the memo for a dig.
    pub fn seal<R: CryptoRng + RngCore>(
        round: u32,
        tile: u8,
        receiver: &PayoutReceiver,
        sealing_key: &SealingPublicKey,
        rng: &mut R,
    ) -> Result<Self, MemoError> {
        if tile >= TILE_COUNT {
            return Err(MemoError::TileOutOfRange(tile));
        }
        let sealed = seal::seal(
            sealing_key,
            &receiver.to_plaintext(),
            &aad(round, tile),
            rng,
        )?;
        Ok(DigMemo {
            round,
            tile,
            sealed_receiver: SealedReceiver::from_bytes(&sealed)?,
        })
    }

    /// Opens the sealed receiver with the vault's private sealing key.
    pub fn open(
        &self,
        network: Network,
        sealing_key: &SealingPrivateKey,
    ) -> Result<PayoutReceiver, MemoError> {
        let plaintext = seal::open(
            sealing_key,
            self.sealed_receiver.as_bytes(),
            &aad(self.round, self.tile),
        )?;
        PayoutReceiver::from_plaintext(network, &plaintext)
    }

    /// The memo bytes without padding (`DIG_MEMO_LEN` bytes).
    pub fn encode_unpadded(&self) -> [u8; DIG_MEMO_LEN] {
        let mut out = [0u8; DIG_MEMO_LEN];
        out[0] = ARBITRARY_DATA_LEAD;
        out[1..5].copy_from_slice(&MAGIC);
        out[5] = VERSION;
        out[6..10].copy_from_slice(&self.round.to_be_bytes());
        out[10] = self.tile;
        out[HEADER_LEN..].copy_from_slice(self.sealed_receiver.as_bytes());
        out
    }

    /// The full 512-byte memo field.
    pub fn encode(&self) -> [u8; MEMO_LEN] {
        let mut out = [0u8; MEMO_LEN];
        out[..DIG_MEMO_LEN].copy_from_slice(&self.encode_unpadded());
        out
    }

    /// Strictly parses a memo field (padded to 512 bytes or unpadded).
    /// The text encoding: `zeek1:` + base64url (no padding) of the binary
    /// memo without its `0xFF` lead byte. Always [`TEXT_MEMO_LEN`] ASCII
    /// characters.
    pub fn encode_text(&self) -> String {
        let binary = self.encode_unpadded();
        let mut out = String::with_capacity(TEXT_MEMO_LEN);
        out.push_str(TEXT_PREFIX);
        URL_SAFE_NO_PAD.encode_string(&binary[1..], &mut out);
        out
    }

    /// Decodes the text encoding from a memo field (the text followed by
    /// zero padding). Strict: exactly the prefix and the canonical unpadded
    /// base64url of a well-formed payload, nothing before or after.
    pub fn decode_text(memo: &[u8]) -> Result<Self, MemoError> {
        if memo.len() > MEMO_LEN {
            return Err(MemoError::TooLong(memo.len()));
        }
        let end = memo.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
        let text = &memo[..end];
        let encoded = text
            .strip_prefix(TEXT_PREFIX.as_bytes())
            .ok_or(MemoError::BadText("missing the zeek1: prefix"))?;
        if text.len() != TEXT_MEMO_LEN {
            return Err(MemoError::BadText("wrong length"));
        }
        let payload = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| MemoError::BadText("payload is not canonical base64url"))?;
        if payload.len() != TEXT_PAYLOAD_LEN {
            return Err(MemoError::BadText("wrong payload length"));
        }
        let mut binary = [0u8; DIG_MEMO_LEN];
        binary[0] = ARBITRARY_DATA_LEAD;
        binary[1..].copy_from_slice(&payload);
        Self::decode(&binary)
    }

    /// Decodes a memo mined at `height` under `params`: the binary encoding
    /// always, the text encoding only from the activation height on (below
    /// it a text memo fails exactly as it did before rules v3).
    pub fn decode_at(
        params: &crate::params::GameParams,
        height: u32,
        memo: &[u8],
    ) -> Result<Self, MemoError> {
        match Self::decode(memo) {
            Ok(decoded) => Ok(decoded),
            Err(binary_error) => {
                if params.text_memo_active(height) && memo.starts_with(TEXT_PREFIX.as_bytes()) {
                    Self::decode_text(memo)
                } else {
                    Err(binary_error)
                }
            }
        }
    }

    pub fn decode(memo: &[u8]) -> Result<Self, MemoError> {
        if memo.len() > MEMO_LEN {
            return Err(MemoError::TooLong(memo.len()));
        }
        if memo.len() < DIG_MEMO_LEN {
            return Err(MemoError::TooShort(memo.len()));
        }
        if memo[0] != ARBITRARY_DATA_LEAD {
            return Err(MemoError::NotArbitraryData(memo[0]));
        }
        if memo[1..5] != MAGIC {
            return Err(MemoError::BadMagic);
        }
        if memo[5] != VERSION {
            return Err(MemoError::UnsupportedVersion(memo[5]));
        }
        let round = u32::from_be_bytes([memo[6], memo[7], memo[8], memo[9]]);
        let tile = memo[10];
        if tile >= TILE_COUNT {
            return Err(MemoError::TileOutOfRange(tile));
        }
        if memo[DIG_MEMO_LEN..].iter().any(|b| *b != 0) {
            return Err(MemoError::NonZeroPadding);
        }
        Ok(DigMemo {
            round,
            tile,
            sealed_receiver: SealedReceiver::from_bytes(&memo[HEADER_LEN..DIG_MEMO_LEN])?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seal::generate_sealing_keypair;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    pub(crate) fn receiver(seed: u8) -> PayoutReceiver {
        PayoutReceiver {
            network: Network::Test,
            orchard: [seed; ORCHARD_RECEIVER_LEN],
        }
    }

    #[test]
    fn memo_round_trips_and_fits_the_field() {
        let mut rng = StdRng::seed_from_u64(3);
        let (sk, pk) = generate_sealing_keypair(&mut rng);
        let memo = DigMemo::seal(7, 24, &receiver(9), &pk, &mut rng).expect("seal");
        assert_eq!(DIG_MEMO_LEN, 103);
        let padded = memo.encode();
        assert_eq!(padded.len(), MEMO_LEN);
        assert_eq!(DigMemo::decode(&padded).expect("decode"), memo);
        assert_eq!(
            DigMemo::decode(&memo.encode_unpadded()).expect("decode"),
            memo
        );
        assert_eq!(memo.open(Network::Test, &sk).expect("open"), receiver(9));
        assert_eq!(memo.open(Network::Main, &sk), Err(MemoError::WrongNetwork));
        let json = serde_json::to_string(&memo).expect("json");
        assert_eq!(serde_json::from_str::<DigMemo>(&json).expect("parse"), memo);
    }

    #[test]
    fn text_encoding_decodes_to_the_same_memo() {
        use crate::params::GameParams;
        let mut rng = StdRng::seed_from_u64(31);
        let (sk, pk) = generate_sealing_keypair(&mut rng);
        let memo = DigMemo::seal(16, 7, &receiver(5), &pk, &mut rng).expect("seal");
        let text = memo.encode_text();
        assert_eq!(text.len(), TEXT_MEMO_LEN);
        assert_eq!(TEXT_MEMO_LEN, 142);
        assert!(text.starts_with("zeek1:"));
        assert!(text.is_ascii());
        assert_eq!(
            text.as_bytes()[0],
            b'z',
            "a ZIP 302 text memo (lead byte <= 0xF4)"
        );
        // As the wallet stores it: the text zero-padded to 512 bytes.
        let mut field = [0u8; MEMO_LEN];
        field[..text.len()].copy_from_slice(text.as_bytes());
        let decoded = DigMemo::decode_text(&field).expect("decode");
        assert_eq!(decoded, memo);
        assert_eq!(
            DigMemo::decode_text(text.as_bytes()).expect("unpadded"),
            memo
        );
        assert_eq!(decoded.open(Network::Test, &sk).expect("open"), receiver(5));

        // Activation: below it the text memo fails exactly like before.
        let mut p = GameParams::testnet(1_000);
        let before = || DigMemo::decode(&field).expect_err("binary decode fails");
        assert_eq!(DigMemo::decode_at(&p, 2_000, &field), Err(before()));
        p.text_memo_activation_height = Some(2_000);
        assert_eq!(DigMemo::decode_at(&p, 1_999, &field), Err(before()));
        assert_eq!(DigMemo::decode_at(&p, 2_000, &field).expect("active"), memo);
        // The binary encoding is accepted at every height.
        assert_eq!(
            DigMemo::decode_at(&p, 1_000, &memo.encode()).expect("binary"),
            memo
        );
        assert_eq!(
            DigMemo::decode_at(&p, 3_000, &memo.encode()).expect("binary"),
            memo
        );
    }

    #[test]
    fn text_decoding_is_strict() {
        let mut rng = StdRng::seed_from_u64(32);
        let (_, pk) = generate_sealing_keypair(&mut rng);
        let memo = DigMemo::seal(1, 3, &receiver(1), &pk, &mut rng).expect("seal");
        let text = memo.encode_text();
        let bad = |t: String| DigMemo::decode_text(t.as_bytes());
        assert_eq!(
            bad(text.replacen("zeek1:", "zeek2:", 1)),
            Err(MemoError::BadText("missing the zeek1: prefix"))
        );
        assert_eq!(
            bad(format!("{text} ")),
            Err(MemoError::BadText("wrong length"))
        );
        assert_eq!(
            bad(format!("{text}\n")),
            Err(MemoError::BadText("wrong length"))
        );
        assert_eq!(
            bad(text[..text.len() - 1].to_owned()),
            Err(MemoError::BadText("wrong length"))
        );
        let mut standard = text.clone();
        standard.replace_range(6..7, "+");
        assert_eq!(
            bad(standard),
            Err(MemoError::BadText("payload is not canonical base64url"))
        );
        // A payload whose tile is out of range is rejected by the shared decoder.
        let mut binary = memo.encode_unpadded();
        binary[10] = 25;
        let forged = format!("zeek1:{}", URL_SAFE_NO_PAD.encode(&binary[1..]));
        assert_eq!(bad(forged), Err(MemoError::TileOutOfRange(25)));
        assert_eq!(
            DigMemo::decode_text(&[b'z'; 513]),
            Err(MemoError::TooLong(513))
        );
    }

    #[test]
    fn decode_is_strict() {
        let mut rng = StdRng::seed_from_u64(4);
        let (_, pk) = generate_sealing_keypair(&mut rng);
        let memo = DigMemo::seal(1, 3, &receiver(1), &pk, &mut rng).expect("seal");
        let good = memo.encode();

        let mut m = good;
        m[0] = 0xF6;
        assert_eq!(DigMemo::decode(&m), Err(MemoError::NotArbitraryData(0xF6)));
        let mut m = good;
        m[1] = b'X';
        assert_eq!(DigMemo::decode(&m), Err(MemoError::BadMagic));
        let mut m = good;
        m[5] = 2;
        assert_eq!(DigMemo::decode(&m), Err(MemoError::UnsupportedVersion(2)));
        let mut m = good;
        m[10] = 25;
        assert_eq!(DigMemo::decode(&m), Err(MemoError::TileOutOfRange(25)));
        let mut m = good;
        m[511] = 1;
        assert_eq!(DigMemo::decode(&m), Err(MemoError::NonZeroPadding));
        assert_eq!(DigMemo::decode(&good[..50]), Err(MemoError::TooShort(50)));
        assert_eq!(DigMemo::decode(&[0u8; 513]), Err(MemoError::TooLong(513)));
        assert_eq!(
            DigMemo::seal(1, 25, &receiver(1), &pk, &mut rng).err(),
            Some(MemoError::TileOutOfRange(25))
        );
    }

    #[test]
    fn sealed_receiver_is_bound_to_round_and_tile() {
        let mut rng = StdRng::seed_from_u64(5);
        let (sk, pk) = generate_sealing_keypair(&mut rng);
        let memo = DigMemo::seal(10, 4, &receiver(2), &pk, &mut rng).expect("seal");
        let mut moved = memo.clone();
        moved.round = 11;
        assert_eq!(moved.open(Network::Test, &sk), Err(SealError::Open.into()));
        let mut moved = memo.clone();
        moved.tile = 5;
        assert_eq!(moved.open(Network::Test, &sk), Err(SealError::Open.into()));
        let (other_sk, _) = generate_sealing_keypair(&mut rng);
        assert_eq!(
            memo.open(Network::Test, &other_sk),
            Err(SealError::Open.into())
        );
    }

    #[test]
    fn unified_address_round_trip_and_rejections() {
        let r = receiver(0x42);
        let ua = r.to_unified_address().expect("encode");
        assert!(ua.starts_with("utest1"), "{ua}");
        assert_eq!(
            PayoutReceiver::from_unified_address(Network::Test, &ua).expect("parse"),
            r
        );
        assert_eq!(
            PayoutReceiver::from_unified_address(Network::Main, &ua),
            Err(MemoError::AddressWrongNetwork)
        );
        // Transparent testnet address: shielded receivers only.
        assert_eq!(
            PayoutReceiver::from_unified_address(
                Network::Test,
                "tmEZhbWHTpdKMw5it8YDspUXSMGQyFwovpU"
            ),
            Err(MemoError::NoOrchardReceiver)
        );
        assert!(matches!(
            PayoutReceiver::from_unified_address(Network::Test, "nonsense"),
            Err(MemoError::InvalidAddress(_))
        ));
        let plaintext = r.to_plaintext();
        assert_eq!(plaintext[43], 0x01);
        assert_eq!(
            PayoutReceiver::from_plaintext(Network::Test, &plaintext).expect("pt"),
            r
        );
        assert_eq!(
            PayoutReceiver::from_plaintext(Network::Test, &plaintext[..40]),
            Err(MemoError::BadPlaintextLength(40))
        );
        let mut bad = plaintext;
        bad[43] = 9;
        assert_eq!(
            PayoutReceiver::from_plaintext(Network::Test, &bad),
            Err(MemoError::UnknownNetworkTag(9))
        );
    }
}
