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
//! # Campfire (rules v4)
//!
//! From the Campfire activation height two more layouts are accepted
//! (both padded with zeros to 512 bytes, ZIP 302 arbitrary data):
//!
//! ```text
//! dig with message:  0xFF | "ZEEK" | 2 | round (u32 BE) | tile | sealed receiver (92)
//!                    | message length (u16 BE) | message (UTF-8)
//! shout:             0xFF | "ZEEK" | 3 | message length (u16 BE) | message (UTF-8)
//! ```
//!
//! A message is valid UTF-8 without control characters, at most
//! [`MAX_DIG_MESSAGE_LEN`] bytes on a dig and [`MAX_MESSAGE_LEN`] on a shout.
//! The dig limit is the real one: a dig's text encoding (below) must fit the
//! 512-byte memo, which leaves `floor(506 · 3 / 4) − 104 = 275` bytes. An
//! invalid message is dropped and the memo still decodes, so a dig with a bad
//! message stays a valid dig. The sealed receiver and its AAD are unchanged
//! (the AAD keeps version byte 1 whatever the layout).
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
/// Memo version of a dig that carries a Campfire message (rules v4).
pub const DIG_MESSAGE_VERSION: u8 = 2;
/// Memo version of a shout: a message only, no tile and no receiver (rules v4).
pub const SHOUT_VERSION: u8 = 3;
/// Bytes of the message length field.
pub const MESSAGE_LEN_BYTES: usize = 2;
/// Campfire message limit (bytes of UTF-8).
pub const MAX_MESSAGE_LEN: usize = 280;
/// Payload bytes a text memo can carry: the base64url of `n` bytes is
/// `ceil(4n/3)` characters, and `"zeek1:"` plus that must fit 512 bytes.
pub const TEXT_PAYLOAD_CAPACITY: usize = (MEMO_LEN - TEXT_PREFIX.len()) * 3 / 4;
/// Longest message on a dig: the text encoding of a dig with message
/// carries `DIG_MEMO_LEN - 1 + 2 + m` payload bytes, so `m` is at most
/// `TEXT_PAYLOAD_CAPACITY - 104 = 275` (below the 280-byte Campfire limit).
pub const MAX_DIG_MESSAGE_LEN: usize = {
    let fit = TEXT_PAYLOAD_CAPACITY - (DIG_MEMO_LEN - 1) - MESSAGE_LEN_BYTES;
    if fit < MAX_MESSAGE_LEN {
        fit
    } else {
        MAX_MESSAGE_LEN
    }
};
/// Header of a shout: lead byte, magic, version, length.
pub const SHOUT_HEADER_LEN: usize = 1 + 4 + 1 + MESSAGE_LEN_BYTES;
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
    #[error("message length runs past the end of the memo")]
    MessageOverrun,
    #[error("the memo is a shout, not a dig")]
    NotADig,
    #[error(transparent)]
    Message(#[from] MessageError),
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

/// Why a Campfire message is not accepted.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum MessageError {
    #[error("the message is empty")]
    Empty,
    #[error("the message is {len} bytes, more than the {max}-byte limit")]
    TooLong { len: usize, max: usize },
    #[error("the message is not valid UTF-8")]
    NotUtf8,
    #[error("the message contains the control character U+{0:04X}")]
    ControlCharacter(u32),
}

/// Checks a Campfire message: non-empty, valid UTF-8, at most `max` bytes,
/// no control characters (Unicode general category Cc, which includes
/// newlines and tabs).
pub fn validate_message(bytes: &[u8], max: usize) -> Result<String, MessageError> {
    if bytes.is_empty() {
        return Err(MessageError::Empty);
    }
    if bytes.len() > max {
        return Err(MessageError::TooLong {
            len: bytes.len(),
            max,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| MessageError::NotUtf8)?;
    if let Some(c) = text.chars().find(|c| c.is_control()) {
        return Err(MessageError::ControlCharacter(u32::from(c)));
    }
    Ok(text.to_owned())
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
    /// Campfire message (rules v4). Encoders write it only when it passes
    /// [`validate_message`] with [`MAX_DIG_MESSAGE_LEN`]; decoders keep it
    /// only then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// A decoded shout memo (rules v4): a message and nothing else. Its note's
/// value goes to the Deep Cache.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShoutMemo {
    /// The message, when valid (at most [`MAX_MESSAGE_LEN`] bytes).
    pub message: Option<String>,
}

/// Any Zeek memo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ZeekMemo {
    Dig(DigMemo),
    Shout(ShoutMemo),
}

fn checked_message(message: Option<&str>, max: usize) -> Option<&str> {
    message.filter(|m| validate_message(m.as_bytes(), max).is_ok())
}

fn push_message(out: &mut Vec<u8>, message: &str) {
    // `checked_message` bounds the length far below u16::MAX.
    let len = u16::try_from(message.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(message.as_bytes());
}

fn text_of(binary: &[u8]) -> String {
    let mut out = String::with_capacity(TEXT_PREFIX.len() + binary.len().div_ceil(3) * 4);
    out.push_str(TEXT_PREFIX);
    URL_SAFE_NO_PAD.encode_string(&binary[1..], &mut out);
    out
}

fn padded(binary: &[u8]) -> [u8; MEMO_LEN] {
    let mut out = [0u8; MEMO_LEN];
    let len = binary.len().min(MEMO_LEN);
    out[..len].copy_from_slice(&binary[..len]);
    out
}

impl ShoutMemo {
    /// A shout with `message`, which must be valid.
    pub fn new(message: &str) -> Result<Self, MemoError> {
        validate_message(message.as_bytes(), MAX_MESSAGE_LEN)?;
        Ok(ShoutMemo {
            message: Some(message.to_owned()),
        })
    }

    /// The memo bytes without padding.
    pub fn encode_unpadded(&self) -> Vec<u8> {
        let message = checked_message(self.message.as_deref(), MAX_MESSAGE_LEN).unwrap_or("");
        let mut out = Vec::with_capacity(SHOUT_HEADER_LEN + message.len());
        out.push(ARBITRARY_DATA_LEAD);
        out.extend_from_slice(&MAGIC);
        out.push(SHOUT_VERSION);
        push_message(&mut out, message);
        out
    }

    /// The full 512-byte memo field.
    pub fn encode(&self) -> [u8; MEMO_LEN] {
        padded(&self.encode_unpadded())
    }

    /// The text encoding: `zeek1:` + base64url of the binary memo without
    /// its lead byte.
    pub fn encode_text(&self) -> String {
        text_of(&self.encode_unpadded())
    }
}

/// Parses `memo` (without padding beyond `end` being checked here) and
/// returns the memo and the index just past its last byte.
fn decode_structure(memo: &[u8], campfire: bool) -> Result<(ZeekMemo, usize), MemoError> {
    if !campfire {
        return DigMemo::decode(memo).map(|d| (ZeekMemo::Dig(d), DIG_MEMO_LEN));
    }
    if memo.len() > MEMO_LEN {
        return Err(MemoError::TooLong(memo.len()));
    }
    if memo.len() < SHOUT_HEADER_LEN {
        return Err(MemoError::TooShort(memo.len()));
    }
    if memo[0] != ARBITRARY_DATA_LEAD {
        return Err(MemoError::NotArbitraryData(memo[0]));
    }
    if memo[1..5] != MAGIC {
        return Err(MemoError::BadMagic);
    }
    let message_at = |start: usize, max: usize| -> Result<(Option<String>, usize), MemoError> {
        let len_bytes = memo
            .get(start..start + MESSAGE_LEN_BYTES)
            .ok_or(MemoError::TooShort(memo.len()))?;
        let len = usize::from(u16::from_be_bytes([len_bytes[0], len_bytes[1]]));
        let body_start = start + MESSAGE_LEN_BYTES;
        let end = body_start + len;
        let body = memo.get(body_start..end).ok_or(MemoError::MessageOverrun)?;
        Ok((validate_message(body, max).ok(), end))
    };
    match memo[5] {
        VERSION => {
            if memo.len() < DIG_MEMO_LEN {
                return Err(MemoError::TooShort(memo.len()));
            }
            let mut dig = DigMemo::decode_fields(memo)?;
            dig.message = None;
            Ok((ZeekMemo::Dig(dig), DIG_MEMO_LEN))
        }
        DIG_MESSAGE_VERSION => {
            if memo.len() < DIG_MEMO_LEN {
                return Err(MemoError::TooShort(memo.len()));
            }
            let mut dig = DigMemo::decode_fields(memo)?;
            let (message, end) = message_at(DIG_MEMO_LEN, MAX_DIG_MESSAGE_LEN)?;
            dig.message = message;
            Ok((ZeekMemo::Dig(dig), end))
        }
        SHOUT_VERSION => {
            let (message, end) = message_at(SHOUT_HEADER_LEN - MESSAGE_LEN_BYTES, MAX_MESSAGE_LEN)?;
            Ok((ZeekMemo::Shout(ShoutMemo { message }), end))
        }
        other => Err(MemoError::UnsupportedVersion(other)),
    }
}

impl ZeekMemo {
    /// Strictly parses a binary memo field (padded or not). Versions 2 and
    /// 3 are accepted only when `campfire` is set; otherwise this is exactly
    /// [`DigMemo::decode`].
    pub fn decode(memo: &[u8], campfire: bool) -> Result<Self, MemoError> {
        let (decoded, end) = decode_structure(memo, campfire)?;
        if memo[end.min(memo.len())..].iter().any(|b| *b != 0) {
            return Err(MemoError::NonZeroPadding);
        }
        Ok(decoded)
    }

    /// Strictly parses the text encoding (followed by zero padding): the
    /// prefix and the canonical unpadded base64url of exactly one memo.
    pub fn decode_text(memo: &[u8], campfire: bool) -> Result<Self, MemoError> {
        if !campfire {
            return DigMemo::decode_text(memo).map(ZeekMemo::Dig);
        }
        if memo.len() > MEMO_LEN {
            return Err(MemoError::TooLong(memo.len()));
        }
        let end = memo.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
        let encoded = memo[..end]
            .strip_prefix(TEXT_PREFIX.as_bytes())
            .ok_or(MemoError::BadText("missing the zeek1: prefix"))?;
        let payload = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| MemoError::BadText("payload is not canonical base64url"))?;
        let mut binary = Vec::with_capacity(payload.len() + 1);
        binary.push(ARBITRARY_DATA_LEAD);
        binary.extend_from_slice(&payload);
        let (decoded, used) = decode_structure(&binary, true)?;
        if used != binary.len() {
            return Err(MemoError::BadText("payload has bytes after the memo"));
        }
        Ok(decoded)
    }

    /// Decodes a memo mined at `height` under `params`: the binary encoding
    /// always; the text encoding from the text-memo activation height; dig
    /// messages and shouts from the Campfire activation height. Below an
    /// activation height a memo is classified exactly as before it.
    pub fn decode_at(
        params: &crate::params::GameParams,
        height: u32,
        memo: &[u8],
    ) -> Result<Self, MemoError> {
        let campfire = params.campfire_active(height);
        match Self::decode(memo, campfire) {
            Ok(decoded) => Ok(decoded),
            Err(binary_error) => {
                if params.text_memo_active(height) && memo.starts_with(TEXT_PREFIX.as_bytes()) {
                    Self::decode_text(memo, campfire)
                } else {
                    Err(binary_error)
                }
            }
        }
    }

    /// The message, if any.
    pub fn message(&self) -> Option<&str> {
        match self {
            ZeekMemo::Dig(d) => d.message.as_deref(),
            ZeekMemo::Shout(s) => s.message.as_deref(),
        }
    }
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
            message: None,
        })
    }

    /// Attaches a Campfire message (rules v4) after checking it against the
    /// dig limit [`MAX_DIG_MESSAGE_LEN`].
    pub fn with_message(mut self, message: &str) -> Result<Self, MemoError> {
        validate_message(message.as_bytes(), MAX_DIG_MESSAGE_LEN)?;
        self.message = Some(message.to_owned());
        Ok(self)
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

    /// The memo bytes without padding: `DIG_MEMO_LEN` bytes (version 1),
    /// or version 2 with the message when it carries a valid one.
    pub fn encode_unpadded(&self) -> Vec<u8> {
        let message = checked_message(self.message.as_deref(), MAX_DIG_MESSAGE_LEN);
        let mut out = Vec::with_capacity(DIG_MEMO_LEN + MESSAGE_LEN_BYTES + MAX_DIG_MESSAGE_LEN);
        out.push(ARBITRARY_DATA_LEAD);
        out.extend_from_slice(&MAGIC);
        out.push(if message.is_some() {
            DIG_MESSAGE_VERSION
        } else {
            VERSION
        });
        out.extend_from_slice(&self.round.to_be_bytes());
        out.push(self.tile);
        out.extend_from_slice(self.sealed_receiver.as_bytes());
        if let Some(message) = message {
            push_message(&mut out, message);
        }
        out
    }

    /// The full 512-byte memo field.
    pub fn encode(&self) -> [u8; MEMO_LEN] {
        padded(&self.encode_unpadded())
    }

    /// Strictly parses a memo field (padded to 512 bytes or unpadded).
    /// The text encoding: `zeek1:` + base64url (no padding) of the binary
    /// memo without its `0xFF` lead byte: [`TEXT_MEMO_LEN`] ASCII characters
    /// without a message, at most 512 with one.
    pub fn encode_text(&self) -> String {
        text_of(&self.encode_unpadded())
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
        match ZeekMemo::decode_at(params, height, memo)? {
            ZeekMemo::Dig(dig) => Ok(dig),
            ZeekMemo::Shout(_) => Err(MemoError::NotADig),
        }
    }

    /// Round, tile and sealed receiver of a dig memo whose lead byte, magic
    /// and length were checked (the version byte is the caller's).
    fn decode_fields(memo: &[u8]) -> Result<Self, MemoError> {
        let round = u32::from_be_bytes([memo[6], memo[7], memo[8], memo[9]]);
        let tile = memo[10];
        if tile >= TILE_COUNT {
            return Err(MemoError::TileOutOfRange(tile));
        }
        Ok(DigMemo {
            round,
            tile,
            sealed_receiver: SealedReceiver::from_bytes(&memo[HEADER_LEN..DIG_MEMO_LEN])?,
            message: None,
        })
    }

    /// Strictly parses a version 1 dig memo (padded to 512 bytes or not):
    /// the only layout before rules v4.
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
            message: None,
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

    fn campfire_params() -> crate::params::GameParams {
        let mut p = crate::params::GameParams::testnet(1_000);
        p.text_memo_activation_height = Some(2_000);
        p.campfire = Some(crate::params::CampfireParams {
            activation_height: 3_000,
            min_shout_zat: crate::params::MIN_SHOUT_ZAT,
        });
        p
    }

    #[test]
    fn message_limits_are_the_real_ones() {
        assert_eq!(TEXT_PAYLOAD_CAPACITY, 379);
        assert_eq!(MAX_DIG_MESSAGE_LEN, 275);
        let mut rng = StdRng::seed_from_u64(40);
        let (_, pk) = generate_sealing_keypair(&mut rng);
        let longest = "é".repeat(137) + "x"; // 275 bytes
        assert_eq!(longest.len(), 275);
        let dig = DigMemo::seal(9, 4, &receiver(1), &pk, &mut rng)
            .expect("seal")
            .with_message(&longest)
            .expect("fits");
        assert!(dig.encode_unpadded().len() <= MEMO_LEN);
        assert_eq!(
            dig.encode_text().len(),
            512,
            "the text encoding fills the memo exactly"
        );
        assert_eq!(
            DigMemo::seal(9, 4, &receiver(1), &pk, &mut rng)
                .expect("seal")
                .with_message(&(longest.clone() + "x")),
            Err(MemoError::Message(MessageError::TooLong {
                len: 276,
                max: 275
            }))
        );
        let shout = ShoutMemo::new(&"s".repeat(MAX_MESSAGE_LEN)).expect("280 bytes");
        assert!(shout.encode_text().len() <= MEMO_LEN);
        assert!(ShoutMemo::new(&"s".repeat(281)).is_err());
        assert_eq!(validate_message(b"", 10), Err(MessageError::Empty));
        assert_eq!(
            validate_message(b"a\nb", 10),
            Err(MessageError::ControlCharacter(10))
        );
        assert_eq!(validate_message(&[0xC3], 10), Err(MessageError::NotUtf8));
        assert_eq!(
            validate_message("ok 🔥".as_bytes(), 10).expect("valid"),
            "ok 🔥"
        );
    }

    #[test]
    fn campfire_memos_decode_only_from_activation() {
        let p = campfire_params();
        let mut rng = StdRng::seed_from_u64(41);
        let (_, pk) = generate_sealing_keypair(&mut rng);
        let dig = DigMemo::seal(9, 4, &receiver(1), &pk, &mut rng)
            .expect("seal")
            .with_message("hello from tile 4")
            .expect("message");
        let shout = ShoutMemo::new("the fire is warm").expect("shout");
        for (binary, text, expected) in [
            (dig.encode(), dig.encode_text(), ZeekMemo::Dig(dig.clone())),
            (
                shout.encode(),
                shout.encode_text(),
                ZeekMemo::Shout(shout.clone()),
            ),
        ] {
            let mut text_field = [0u8; MEMO_LEN];
            text_field[..text.len()].copy_from_slice(text.as_bytes());
            assert_eq!(
                ZeekMemo::decode_at(&p, 3_000, &binary).expect("binary"),
                expected
            );
            assert_eq!(
                ZeekMemo::decode_at(&p, 3_000, &text_field).expect("text"),
                expected
            );
            // Before activation: exactly the v3 behaviour (version unsupported).
            assert_eq!(
                ZeekMemo::decode_at(&p, 2_999, &binary),
                Err(DigMemo::decode(&binary).expect_err("v1 decoder rejects"))
            );
            assert!(ZeekMemo::decode_at(&p, 2_999, &text_field).is_err());
        }
        // A version 1 dig is still a dig, without a message.
        let plain = DigMemo::seal(9, 5, &receiver(2), &pk, &mut rng).expect("seal");
        assert_eq!(plain.encode()[5], VERSION);
        assert_eq!(
            ZeekMemo::decode_at(&p, 3_000, &plain.encode()).expect("v1"),
            ZeekMemo::Dig(plain)
        );
        assert_eq!(
            DigMemo::decode_at(&p, 3_000, &shout.encode()),
            Err(MemoError::NotADig)
        );
    }

    #[test]
    fn invalid_messages_are_dropped_not_fatal() {
        let p = campfire_params();
        let mut rng = StdRng::seed_from_u64(42);
        let (_, pk) = generate_sealing_keypair(&mut rng);
        let dig = DigMemo::seal(9, 4, &receiver(1), &pk, &mut rng).expect("seal");
        // Hand-built version 2 memos with a bad message: control character,
        // invalid UTF-8, empty.
        for body in [&b"bad\x07bell"[..], &[0xFF, 0xFE][..], &b""[..]] {
            let mut memo = dig.encode_unpadded();
            memo[5] = DIG_MESSAGE_VERSION;
            memo.extend_from_slice(&(body.len() as u16).to_be_bytes());
            memo.extend_from_slice(body);
            match ZeekMemo::decode_at(&p, 3_000, &memo).expect("still a dig") {
                ZeekMemo::Dig(d) => {
                    assert_eq!(d.message, None);
                    assert_eq!((d.round, d.tile), (9, 4));
                }
                other => panic!("expected a dig, got {other:?}"),
            }
        }
        // A length that runs past the memo makes it undecodable.
        let mut memo = dig.encode_unpadded();
        memo[5] = DIG_MESSAGE_VERSION;
        memo.extend_from_slice(&500u16.to_be_bytes());
        assert_eq!(
            ZeekMemo::decode(&memo, true),
            Err(MemoError::MessageOverrun)
        );
        // Bytes after the message must be zero.
        let mut memo = ShoutMemo::new("hi").expect("shout").encode();
        memo[20] = 1;
        assert_eq!(
            ZeekMemo::decode(&memo, true),
            Err(MemoError::NonZeroPadding)
        );
        // Encoders never write an invalid message.
        let sneaky = DigMemo {
            message: Some("tab\there".to_owned()),
            ..dig.clone()
        };
        assert_eq!(sneaky.encode_unpadded(), dig.encode_unpadded());
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
