//! Keys: a BIP 39 24-word phrase (256 bits of entropy), the ZIP 32 Orchard
//! account 0 of Zcash testnet derived from its seed, and Orchard-only unified
//! addresses. Ironwood notes use the same Orchard keys and receivers.
//!
//! Diversifier index 0 is the wallet's receive address. Every dig seals a
//! fresh address at the next unused index (1, 2, 3, …), never reused; change
//! goes to the internal-scope address.

use bip39::{Language, Mnemonic};
use orchard::keys::{FullViewingKey, Scope, SpendAuthorizingKey, SpendingKey};
use zcash_keys::keys::UnifiedFullViewingKey;
use zeek_core::memo::PayoutReceiver;
use zeek_core::params::Network;
use zeroize::Zeroizing;

use crate::error::PocketError;

/// Testnet coin type (SLIP 44).
pub const TESTNET_COIN_TYPE: u32 = 1;

/// The wallet's keys. Holds secrets; never serialized, never logged.
pub struct Keys {
    entropy: Zeroizing<[u8; 32]>,
    spending_key: SpendingKey,
    fvk: FullViewingKey,
}

impl Keys {
    /// Derives the account from 32 bytes of BIP 39 entropy.
    pub fn from_entropy(entropy: [u8; 32]) -> Result<Self, PocketError> {
        let entropy = Zeroizing::new(entropy);
        let mnemonic = Mnemonic::from_entropy_in(Language::English, entropy.as_slice())
            .map_err(|e| PocketError::InvalidMnemonic(e.to_string()))?;
        let seed = Zeroizing::new(mnemonic.to_seed(""));
        let spending_key = SpendingKey::from_zip32_seed(
            seed.as_slice(),
            TESTNET_COIN_TYPE,
            zip32::AccountId::ZERO,
        )
        .map_err(|e| PocketError::InvalidMnemonic(format!("seed derives no Orchard key: {e:?}")))?;
        let fvk = FullViewingKey::from(&spending_key);
        Ok(Keys {
            entropy,
            spending_key,
            fvk,
        })
    }

    /// Parses a 24-word English phrase.
    pub fn from_phrase(phrase: &str) -> Result<Self, PocketError> {
        let mnemonic = Mnemonic::parse_in_normalized(Language::English, phrase.trim())
            .map_err(|e| PocketError::InvalidMnemonic(e.to_string()))?;
        if mnemonic.word_count() != 24 {
            return Err(PocketError::InvalidMnemonic(format!(
                "{} words; Zeek Pocket phrases have 24",
                mnemonic.word_count()
            )));
        }
        let bytes = Zeroizing::new(mnemonic.to_entropy());
        let entropy = <[u8; 32]>::try_from(bytes.as_slice())
            .map_err(|_| PocketError::InvalidMnemonic("phrase is not 256 bits".into()))?;
        Self::from_entropy(entropy)
    }

    /// The 32 entropy bytes (for sealing at rest).
    pub fn entropy(&self) -> &[u8; 32] {
        &self.entropy
    }

    /// The 24-word phrase.
    pub fn phrase(&self) -> Result<Zeroizing<String>, PocketError> {
        let mnemonic = Mnemonic::from_entropy_in(Language::English, self.entropy.as_slice())
            .map_err(|e| PocketError::InvalidMnemonic(e.to_string()))?;
        Ok(Zeroizing::new(mnemonic.to_string()))
    }

    pub fn fvk(&self) -> &FullViewingKey {
        &self.fvk
    }

    /// The spend authorizing key, for signing.
    pub fn ask(&self) -> SpendAuthorizingKey {
        SpendAuthorizingKey::from(&self.spending_key)
    }

    /// The unified full viewing key the scanner uses (Orchard only).
    pub fn ufvk(&self) -> Result<UnifiedFullViewingKey, PocketError> {
        UnifiedFullViewingKey::from_orchard_fvk(self.fvk.clone())
            .map_err(|e| PocketError::Corrupt(format!("viewing key: {e:?}")))
    }

    /// External-scope address at diversifier index `index`.
    pub fn address(&self, index: u64) -> orchard::Address {
        self.fvk.address_at(index, Scope::External)
    }

    /// The internal-scope (change) address.
    pub fn change_address(&self) -> orchard::Address {
        self.fvk.address_at(0u32, Scope::Internal)
    }
}

/// Encodes an Orchard address as an Orchard-only testnet unified address.
pub fn encode_address(address: &orchard::Address) -> Result<String, PocketError> {
    PayoutReceiver {
        network: Network::Test,
        orchard: address.to_raw_address_bytes(),
    }
    .to_unified_address()
    .map_err(|e| PocketError::InvalidAddress(e.to_string()))
}

/// Parses a testnet address that has an Orchard-protocol receiver.
pub fn parse_address(text: &str) -> Result<orchard::Address, PocketError> {
    let receiver = PayoutReceiver::from_unified_address(Network::Test, text)
        .map_err(|e| PocketError::InvalidAddress(e.to_string()))?;
    Option::from(orchard::Address::from_raw_address_bytes(&receiver.orchard))
        .ok_or_else(|| PocketError::InvalidAddress("not a valid Orchard receiver".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derivation_is_deterministic_and_round_trips_the_phrase() {
        let keys = Keys::from_entropy([7; 32]).expect("keys");
        let phrase = keys.phrase().expect("phrase");
        assert_eq!(phrase.split_whitespace().count(), 24);
        let again = Keys::from_phrase(&phrase).expect("restore");
        assert_eq!(again.fvk(), keys.fvk());
        assert_eq!(again.entropy(), keys.entropy());
        let a0 = encode_address(&keys.address(0)).expect("ua");
        assert!(a0.starts_with("utest1"));
        assert_eq!(parse_address(&a0).expect("parse"), keys.address(0));
        assert_ne!(
            keys.address(1),
            keys.address(0),
            "diversified addresses differ"
        );
        assert_ne!(keys.change_address(), keys.address(0));
    }

    #[test]
    fn rejects_bad_phrases_and_addresses() {
        assert!(matches!(
            Keys::from_phrase("abandon abandon"),
            Err(PocketError::InvalidMnemonic(_))
        ));
        let twelve = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        assert!(matches!(
            Keys::from_phrase(twelve),
            Err(PocketError::InvalidMnemonic(_))
        ));
        assert!(matches!(
            parse_address("tmEZhbWHTpdKMw5it8YDspUXSMGQyFwovpU"),
            Err(PocketError::InvalidAddress(_))
        ));
    }
}
