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

/// Builds the payment request a digger's wallet should pay: `amount` zatoshis
/// to the vault's `address` on `network`, carrying `memo`.
pub fn dig_payment_uri(
    network: Network,
    address: &str,
    amount_zat: u64,
    memo: &DigMemo,
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
    let memo_b64 = URL_SAFE_NO_PAD.encode(memo.encode_unpadded());
    Ok(format!(
        "zcash:{}?amount={}&memo={}",
        parsed.encode(),
        amount_param(amount_zat),
        memo_b64
    ))
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
