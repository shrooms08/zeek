//! ZIP 321 payment request URIs for digs.
//!
//! Grammar used (ZIP 321): `zcash:<address>?amount=<decimal ZEC>&memo=<base64url>`.
//! The amount is decimal ZEC with at most eight fractional digits and no
//! trailing dot; the memo is the base64url encoding (RFC 4648 § 5, no `=`
//! padding) of the memo bytes, which the receiving wallet pads with zeros to
//! 512 bytes. A memo may only be attached to an address that can receive one,
//! so transparent-only addresses are rejected.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use zcash_address::unified;
use zcash_address::{ConversionError, TryFromAddress, ZcashAddress};
use zcash_protocol::consensus::NetworkType;

use crate::memo::{DigMemo, MemoError};
use crate::params::Network;
use crate::COIN;

/// Errors from building a payment URI.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PaymentError {
    #[error("amount must be positive")]
    ZeroAmount,
    #[error("amount {0} zatoshis exceeds the 21,000,000 ZEC supply")]
    AmountTooLarge(u64),
    #[error("a payment request needs at least one dig")]
    NoPayments,
    #[error("address cannot receive a memo (it has no shielded receiver)")]
    NoMemoReceiver,
    #[error(transparent)]
    Memo(#[from] MemoError),
}

/// Accepts every address kind; used only to run `zcash_address`'s network check.
struct AnyKind;

impl TryFromAddress for AnyKind {
    type Error = core::convert::Infallible;

    fn try_from_sprout(_: NetworkType, _: [u8; 64]) -> Result<Self, ConversionError<Self::Error>> {
        Ok(AnyKind)
    }

    fn try_from_sapling(_: NetworkType, _: [u8; 43]) -> Result<Self, ConversionError<Self::Error>> {
        Ok(AnyKind)
    }

    fn try_from_unified(
        _: NetworkType,
        _: unified::Address,
    ) -> Result<Self, ConversionError<Self::Error>> {
        Ok(AnyKind)
    }

    fn try_from_transparent_p2pkh(
        _: NetworkType,
        _: [u8; 20],
    ) -> Result<Self, ConversionError<Self::Error>> {
        Ok(AnyKind)
    }

    fn try_from_transparent_p2sh(
        _: NetworkType,
        _: [u8; 20],
    ) -> Result<Self, ConversionError<Self::Error>> {
        Ok(AnyKind)
    }

    fn try_from_tex(_: NetworkType, _: [u8; 20]) -> Result<Self, ConversionError<Self::Error>> {
        Ok(AnyKind)
    }
}

/// Largest amount ZIP 321 admits: 21,000,000 ZEC.
pub const MAX_AMOUNT_ZAT: u64 = 21_000_000 * COIN;

/// Formats zatoshis as a ZIP 321 decimal amount (`0.001`, `1`, `2.5`).
pub fn amount_param(zatoshis: u64) -> String {
    let whole = zatoshis / COIN;
    let frac = zatoshis % COIN;
    if frac == 0 {
        return whole.to_string();
    }
    let digits = format!("{frac:08}");
    format!("{whole}.{}", digits.trim_end_matches('0'))
}

/// How the dig memo is carried in the payment request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoEncoding {
    /// ZIP 302 arbitrary data (`0xFF` lead byte): for wallets that pass the
    /// request's memo bytes through unchanged.
    Binary,
    /// ZIP 302 text (`zeek1:` + base64url): for wallets that only send text
    /// memos. Valid from the game's text-memo activation height (rules v3).
    Text,
}

fn memo_bytes(memo: &DigMemo, encoding: MemoEncoding) -> Vec<u8> {
    match encoding {
        MemoEncoding::Binary => memo.encode_unpadded(),
        MemoEncoding::Text => memo.encode_text().into_bytes(),
    }
}

/// Builds the payment request a digger's wallet should pay: `amount` zatoshis
/// to the vault's `address` on `network`, carrying `memo` in the binary
/// encoding.
pub fn dig_payment_uri(
    network: Network,
    address: &str,
    amount_zat: u64,
    memo: &DigMemo,
) -> Result<String, PaymentError> {
    dig_request_uri(
        network,
        address,
        &[(amount_zat, memo)],
        MemoEncoding::Binary,
    )
}

/// Builds a ZIP 321 request for a shout (rules v4): `amount_zat` to the
/// vault's `address` carrying `memo`.
pub fn shout_request_uri(
    network: Network,
    address: &str,
    amount_zat: u64,
    memo: &crate::memo::ShoutMemo,
    encoding: MemoEncoding,
) -> Result<String, PaymentError> {
    if amount_zat == 0 {
        return Err(PaymentError::ZeroAmount);
    }
    if amount_zat > MAX_AMOUNT_ZAT {
        return Err(PaymentError::AmountTooLarge(amount_zat));
    }
    let parsed = ZcashAddress::try_from_encoded(address.trim())
        .map_err(|e| MemoError::InvalidAddress(e.to_string()))?;
    let expected = match network {
        Network::Main => NetworkType::Main,
        Network::Test => NetworkType::Test,
    };
    parsed
        .clone()
        .convert_if_network::<AnyKind>(expected)
        .map_err(|_| MemoError::AddressWrongNetwork)?;
    if !parsed.can_receive_memo() {
        return Err(PaymentError::NoMemoReceiver);
    }
    let bytes = match encoding {
        MemoEncoding::Binary => memo.encode_unpadded(),
        MemoEncoding::Text => memo.encode_text().into_bytes(),
    };
    Ok(format!(
        "zcash:{}?amount={}&memo={}",
        parsed.encode(),
        amount_param(amount_zat),
        URL_SAFE_NO_PAD.encode(bytes)
    ))
}

/// Builds one ZIP 321 request with one payment per dig, all to the vault's
/// `address`. A single payment uses the short form
/// `zcash:<address>?amount=..&memo=..`; several use the indexed form
/// `zcash:?address=..&amount=..&memo=..&address.1=..` (ZIP 321 `paramindex`).
pub fn dig_request_uri(
    network: Network,
    address: &str,
    digs: &[(u64, &DigMemo)],
    encoding: MemoEncoding,
) -> Result<String, PaymentError> {
    if digs.is_empty() {
        return Err(PaymentError::NoPayments);
    }
    for (amount_zat, _) in digs {
        if *amount_zat == 0 {
            return Err(PaymentError::ZeroAmount);
        }
        if *amount_zat > MAX_AMOUNT_ZAT {
            return Err(PaymentError::AmountTooLarge(*amount_zat));
        }
    }
    let parsed = ZcashAddress::try_from_encoded(address.trim())
        .map_err(|e| MemoError::InvalidAddress(e.to_string()))?;
    let expected = match network {
        Network::Main => NetworkType::Main,
        Network::Test => NetworkType::Test,
    };
    parsed
        .clone()
        .convert_if_network::<AnyKind>(expected)
        .map_err(|_| MemoError::AddressWrongNetwork)?;
    if !parsed.can_receive_memo() {
        return Err(PaymentError::NoMemoReceiver);
    }
    let address = parsed.encode();
    if let [(amount_zat, memo)] = digs {
        return Ok(format!(
            "zcash:{address}?amount={}&memo={}",
            amount_param(*amount_zat),
            URL_SAFE_NO_PAD.encode(memo_bytes(memo, encoding))
        ));
    }
    let mut uri = String::from("zcash:?");
    for (index, (amount_zat, memo)) in digs.iter().enumerate() {
        let suffix = if index == 0 {
            String::new()
        } else {
            format!(".{index}")
        };
        if index > 0 {
            uri.push('&');
        }
        uri.push_str(&format!(
            "address{suffix}={address}&amount{suffix}={}&memo{suffix}={}",
            amount_param(*amount_zat),
            URL_SAFE_NO_PAD.encode(memo_bytes(memo, encoding))
        ));
    }
    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memo::{PayoutReceiver, ORCHARD_RECEIVER_LEN};
    use crate::seal::generate_sealing_keypair;
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use zcash_protocol::consensus::TEST_NETWORK;

    #[test]
    fn amounts_follow_zip321_formatting() {
        assert_eq!(amount_param(COIN), "1");
        assert_eq!(amount_param(100_000), "0.001");
        assert_eq!(amount_param(250_000_000), "2.5");
        assert_eq!(amount_param(1), "0.00000001");
        assert_eq!(amount_param(123_456_789), "1.23456789");
    }

    #[test]
    fn uri_is_accepted_by_the_reference_zip321_parser() {
        let mut rng = StdRng::seed_from_u64(6);
        let (_, pk) = generate_sealing_keypair(&mut rng);
        let receiver = PayoutReceiver {
            network: Network::Test,
            orchard: [7; ORCHARD_RECEIVER_LEN],
        };
        let memo = DigMemo::seal(12, 3, &receiver, &pk, &mut rng).expect("seal");
        let vault = receiver.to_unified_address().expect("ua");
        let uri = dig_payment_uri(Network::Test, &vault, 100_000, &memo).expect("uri");
        assert!(
            uri.starts_with(&format!("zcash:{vault}?amount=0.001&memo=")),
            "{uri}"
        );

        let request = zip321::TransactionRequest::from_uri(&uri).expect("zip321 parses it");
        let payment = request.payments().get(&0).expect("one payment");
        assert_eq!(payment.amount().map(u64::from), Some(100_000));
        assert_eq!(payment.recipient_address().encode(), vault);
        let decoded = DigMemo::decode(payment.memo().expect("memo").as_slice()).expect("decode");
        assert_eq!(decoded, memo);
        assert_eq!(request.to_uri(), uri, "canonical form is stable");
        let _ = TEST_NETWORK;
    }

    #[test]
    fn text_and_multi_payment_requests_are_accepted_by_the_reference_parser() {
        let mut rng = StdRng::seed_from_u64(8);
        let (_, pk) = generate_sealing_keypair(&mut rng);
        let receiver = PayoutReceiver {
            network: Network::Test,
            orchard: [9; ORCHARD_RECEIVER_LEN],
        };
        let vault = receiver.to_unified_address().expect("ua");
        let a = DigMemo::seal(20, 3, &receiver, &pk, &mut rng).expect("seal");
        let b = DigMemo::seal(20, 4, &receiver, &pk, &mut rng).expect("seal");

        // One payment, text memo.
        let uri = dig_request_uri(Network::Test, &vault, &[(100_000, &a)], MemoEncoding::Text)
            .expect("uri");
        let request = zip321::TransactionRequest::from_uri(&uri).expect("zip321 parses it");
        assert_eq!(request.to_uri(), uri, "canonical form is stable");
        let payment = request.payments().get(&0).expect("payment");
        let memo = payment.memo().expect("memo");
        assert_eq!(memo.as_slice(), a.encode_text().as_bytes());
        assert!(memo.as_slice()[0] <= 0xF4, "a ZIP 302 text memo");
        assert_eq!(DigMemo::decode_text(memo.as_slice()).expect("decode"), a);

        // Two payments: one per tile, each with its own memo.
        for encoding in [MemoEncoding::Text, MemoEncoding::Binary] {
            let uri = dig_request_uri(
                Network::Test,
                &vault,
                &[(100_000, &a), (250_000, &b)],
                encoding,
            )
            .expect("uri");
            assert!(uri.starts_with("zcash:?address="), "{uri}");
            let request = zip321::TransactionRequest::from_uri(&uri).expect("zip321 parses it");
            assert_eq!(request.payments().len(), 2);
            for (index, (amount, expected)) in [(100_000u64, &a), (250_000, &b)].iter().enumerate()
            {
                let payment = request.payments().get(&index).expect("payment");
                assert_eq!(payment.amount().map(u64::from), Some(*amount));
                assert_eq!(payment.recipient_address().encode(), vault);
                let bytes = payment.memo().expect("memo").as_slice().to_vec();
                let decoded = match encoding {
                    MemoEncoding::Text => DigMemo::decode_text(&bytes),
                    MemoEncoding::Binary => DigMemo::decode(&bytes),
                }
                .expect("decode");
                assert_eq!(&decoded, *expected);
            }
        }
        assert_eq!(
            dig_request_uri(Network::Test, &vault, &[], MemoEncoding::Text),
            Err(PaymentError::NoPayments)
        );
    }

    #[test]
    fn campfire_requests_carry_messages_through_the_reference_parser() {
        use crate::memo::{ShoutMemo, ZeekMemo};
        let mut rng = StdRng::seed_from_u64(9);
        let (_, pk) = generate_sealing_keypair(&mut rng);
        let receiver = PayoutReceiver {
            network: Network::Test,
            orchard: [3; ORCHARD_RECEIVER_LEN],
        };
        let vault = receiver.to_unified_address().expect("ua");
        let dig = DigMemo::seal(30, 8, &receiver, &pk, &mut rng)
            .expect("seal")
            .with_message(&"m".repeat(crate::memo::MAX_DIG_MESSAGE_LEN))
            .expect("message");
        let uri = dig_request_uri(
            Network::Test,
            &vault,
            &[(100_000, &dig)],
            MemoEncoding::Text,
        )
        .expect("uri");
        let request = zip321::TransactionRequest::from_uri(&uri).expect("zip321");
        let memo = request
            .payments()
            .get(&0)
            .expect("payment")
            .memo()
            .expect("memo");
        assert!(memo.as_slice().len() <= 512);
        assert_eq!(
            ZeekMemo::decode_text(memo.as_slice(), true).expect("decode"),
            ZeekMemo::Dig(dig)
        );
        let shout = ShoutMemo::new("warm hands").expect("shout");
        for encoding in [MemoEncoding::Text, MemoEncoding::Binary] {
            let uri =
                shout_request_uri(Network::Test, &vault, 50_000, &shout, encoding).expect("uri");
            let request = zip321::TransactionRequest::from_uri(&uri).expect("zip321");
            assert_eq!(request.to_uri(), uri);
            let payment = request.payments().get(&0).expect("payment");
            assert_eq!(payment.amount().map(u64::from), Some(50_000));
            let bytes = payment.memo().expect("memo").as_slice().to_vec();
            let decoded = match encoding {
                MemoEncoding::Text => ZeekMemo::decode_text(&bytes, true),
                MemoEncoding::Binary => ZeekMemo::decode(&bytes, true),
            }
            .expect("decode");
            assert_eq!(decoded, ZeekMemo::Shout(shout.clone()));
        }
    }

    #[test]
    fn rejects_bad_amounts_and_addresses() {
        let mut rng = StdRng::seed_from_u64(7);
        let (_, pk) = generate_sealing_keypair(&mut rng);
        let receiver = PayoutReceiver {
            network: Network::Test,
            orchard: [1; ORCHARD_RECEIVER_LEN],
        };
        let memo = DigMemo::seal(1, 1, &receiver, &pk, &mut rng).expect("seal");
        let vault = receiver.to_unified_address().expect("ua");
        assert_eq!(
            dig_payment_uri(Network::Test, &vault, 0, &memo),
            Err(PaymentError::ZeroAmount)
        );
        assert_eq!(
            dig_payment_uri(Network::Test, &vault, MAX_AMOUNT_ZAT + 1, &memo),
            Err(PaymentError::AmountTooLarge(MAX_AMOUNT_ZAT + 1))
        );
        assert_eq!(
            dig_payment_uri(Network::Main, &vault, 1, &memo),
            Err(MemoError::AddressWrongNetwork.into())
        );
        assert_eq!(
            dig_payment_uri(
                Network::Test,
                "tmEZhbWHTpdKMw5it8YDspUXSMGQyFwovpU",
                1,
                &memo
            ),
            Err(PaymentError::NoMemoReceiver)
        );
    }
}
