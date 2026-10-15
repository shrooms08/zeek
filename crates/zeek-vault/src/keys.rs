//! Mapping the FROST group key onto Orchard key material.
//!
//! An Orchard full viewing key is the triple `(ak, nk, rivk)` (protocol spec
//! § 4.2.3, "Orchard Key Components"). Only `ak` is tied to spend
//! authorization: it must be the public key whose private counterpart signs
//! spends, so for a threshold vault `ak` **is** the FROST group verifying key.
//! `nk` (nullifier deriving key) and `rivk` (ivk commitment randomness) are
//! independent secrets that only need to be uniformly random; they are
//! generated here from a throw-away spending key using the spec's own PRF
//! derivation and then kept inside the viewing key.
//!
//! # Ironwood
//!
//! The Ironwood pool introduced by NU6.3 reuses Orchard key material and the
//! Orchard receiver type: a unified address with an Orchard receiver is how an
//! Ironwood payment is addressed (`zcash_keys` treats the two pools as sharing
//! that receiver, and the wallet backend trial-decrypts Ironwood outputs with
//! the Orchard incoming viewing key under the ZIP 2005 note-encryption
//! domain). No new receiver or address encoding is needed for the vault.
//!
//! # The `ak` encoding constraint
//!
//! The spec defines `ak = SpendAuthSig.DerivePublic(ask)` and requires the
//! encoding of `ak` to have a *positive* (even) y-coordinate: when deriving
//! from `ask`, the spec negates `ask` if the sign bit of `repr(ak)` is 1. The
//! `orchard` crate enforces this in `SpendValidatingKey::from_bytes`, which
//! rejects any 32-byte encoding whose top bit (`b[31] & 0x80`) is set, and in
//! `SpendAuthorizingKey::from(sk)`, which performs the negation. For a FROST
//! key there is no single `ask` to negate, so instead every share and
//! commitment is negated together: `reddsa`'s RedPallas ciphersuite does this
//! in its `post_generate` hook (`keys::EvenY`). [`derive_full_viewing_key`]
//! re-validates the sign bit and fails loudly if it is ever set.

use orchard::keys::{FullViewingKey, SpendingKey};
use rand_core::{CryptoRng, RngCore};
use zcash_keys::address::UnifiedAddress;
use zcash_keys::keys::{UnifiedAddressRequest, UnifiedFullViewingKey, UnifiedIncomingViewingKey};
use zeroize::Zeroize;

use crate::error::VaultError;
use crate::Params;

/// Size of the raw Orchard full viewing key encoding (`ak || nk || rivk`).
pub const FVK_LEN: usize = 96;

/// Returns `true` if `ak` satisfies the Orchard encoding rule: not the
/// identity and with the y-coordinate sign bit clear.
pub fn ak_is_encodable(ak: &[u8; 32]) -> bool {
    *ak != [0u8; 32] && ak[31] & 0x80 == 0
}

/// Assembles an Orchard full viewing key whose `ak` is `group_key`.
///
/// `nk` and `rivk` are derived from a fresh random spending key exactly as the
/// protocol specifies (`PRF^expand(sk, [7])` and `PRF^expand(sk, [8])`), after
/// which the spending key is discarded. The resulting viewing key is validated
/// by the `orchard` crate (sign bit of `ak`, non-zero `ivk` for both scopes).
pub fn derive_full_viewing_key<R: RngCore + CryptoRng>(
    group_key: &[u8; 32],
    mut rng: R,
) -> Result<FullViewingKey, VaultError> {
    let throwaway = random_spending_key(&mut rng);
    full_viewing_key_with_ak(group_key, &throwaway)
}

/// Like [`derive_full_viewing_key`], but `nk` and `rivk` come from the
/// spending key derived from `seed`, so that every party holding the same
/// seed derives the same viewing key (used after a DKG ceremony, where the
/// seed is a transcript every participant contributed to). `seed` must not be
/// one of the negligible fraction of byte strings the `orchard` crate rejects.
pub fn derive_full_viewing_key_from_seed(
    group_key: &[u8; 32],
    seed: [u8; 32],
) -> Result<FullViewingKey, VaultError> {
    let candidate = SpendingKey::from_bytes(seed);
    let sk = Option::<SpendingKey>::from(candidate)
        .ok_or_else(|| VaultError::KeyDerivation("seed is not a valid spending key".into()))?;
    full_viewing_key_with_ak(group_key, &sk)
}

fn full_viewing_key_with_ak(
    group_key: &[u8; 32],
    throwaway: &SpendingKey,
) -> Result<FullViewingKey, VaultError> {
    if group_key[31] & 0x80 != 0 {
        return Err(VaultError::OddGroupKey);
    }
    if !ak_is_encodable(group_key) {
        return Err(VaultError::InvalidGroupKey);
    }
    let mut raw = FullViewingKey::from(throwaway).to_bytes();
    raw[..32].copy_from_slice(group_key);
    let fvk = FullViewingKey::from_bytes(&raw).ok_or(VaultError::InvalidFullViewingKey);
    raw.zeroize();
    fvk
}

/// Draws random bytes until they form a valid Orchard spending key.
///
/// `SpendingKey::from_bytes` rejects the negligible fraction of byte strings
/// whose derived `ask` would be zero, so this loop terminates almost always on
/// the first iteration.
fn random_spending_key<R: RngCore + CryptoRng>(rng: &mut R) -> SpendingKey {
    loop {
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        let candidate = SpendingKey::from_bytes(bytes);
        bytes.zeroize();
        if let Some(sk) = Option::<SpendingKey>::from(candidate) {
            return sk;
        }
    }
}

/// Returns the `ak` component (first 32 bytes) of a full viewing key.
pub fn ak_bytes(fvk: &FullViewingKey) -> [u8; 32] {
    let raw = fvk.to_bytes();
    let mut ak = [0u8; 32];
    ak.copy_from_slice(&raw[..32]);
    ak
}

/// Wraps an Orchard full viewing key in a unified full viewing key that
/// contains only an Orchard item.
pub fn unified_full_viewing_key(fvk: &FullViewingKey) -> Result<UnifiedFullViewingKey, VaultError> {
    UnifiedFullViewingKey::from_orchard_fvk(fvk.clone())
        .map_err(|e| VaultError::KeyDerivation(format!("unified full viewing key: {e}")))
}

/// Derives the vault's default unified address with only an Orchard receiver.
///
/// After NU6.3 payments to this receiver are created in the Ironwood pool.
pub fn orchard_only_address(ufvk: &UnifiedFullViewingKey) -> Result<UnifiedAddress, VaultError> {
    ufvk.default_address(UnifiedAddressRequest::ORCHARD)
        .map(|(address, _index)| address)
        .map_err(|e| VaultError::KeyDerivation(format!("default address: {e}")))
}

/// The vault's unified *incoming* viewing key: the external-scope Orchard IVK
/// wrapped in a UIVK (ZIP 316). It detects payments to the vault's address but
/// not the vault's own change, which is sent to the internal-scope address, so
/// a board built from it never mistakes change for digs. It is public config.
pub fn unified_incoming_viewing_key(ufvk: &UnifiedFullViewingKey) -> UnifiedIncomingViewingKey {
    ufvk.to_unified_incoming_viewing_key()
}

/// Encodes a unified incoming viewing key for `params`' network.
pub fn encode_uivk(uivk: &UnifiedIncomingViewingKey, params: &Params) -> String {
    uivk.encode(params)
}

/// Encodes a unified address for `params`' network.
pub fn encode_address(address: &UnifiedAddress, params: &Params) -> String {
    address.encode(params)
}

/// Encodes a unified full viewing key for `params`' network.
pub fn encode_ufvk(ufvk: &UnifiedFullViewingKey, params: &Params) -> String {
    ufvk.encode(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frost;
    use rand_core::OsRng;
    use zcash_address::unified::{self, Container, Encoding};
    use zcash_address::ZcashAddress;
    use zcash_keys::address::Address;
    use zcash_protocol::consensus::NetworkType;

    fn group_key() -> [u8; 32] {
        let out = frost::dealer_keygen(2, 3, OsRng).expect("keygen");
        frost::group_key_bytes(&out.public_key_package).expect("group key")
    }

    #[test]
    fn fvk_ak_is_the_frost_group_key() {
        let ak = group_key();
        let fvk = derive_full_viewing_key(&ak, OsRng).expect("fvk");
        assert_eq!(ak_bytes(&fvk), ak);
        assert!(ak_is_encodable(&ak_bytes(&fvk)));
    }

    #[test]
    fn ak_constraint_is_enforced() {
        let mut ak = group_key();
        assert_eq!(ak[31] & 0x80, 0);
        ak[31] |= 0x80;
        assert!(matches!(
            derive_full_viewing_key(&ak, OsRng),
            Err(VaultError::OddGroupKey)
        ));
        assert!(matches!(
            derive_full_viewing_key(&[0u8; 32], OsRng),
            Err(VaultError::InvalidGroupKey)
        ));
    }

    #[test]
    fn seeded_derivation_is_deterministic() {
        let ak = group_key();
        let a = derive_full_viewing_key_from_seed(&ak, [5u8; 32]).expect("a");
        let b = derive_full_viewing_key_from_seed(&ak, [5u8; 32]).expect("b");
        let c = derive_full_viewing_key_from_seed(&ak, [6u8; 32]).expect("c");
        assert_eq!(a.to_bytes(), b.to_bytes());
        assert_ne!(a.to_bytes(), c.to_bytes());
        assert_eq!(ak_bytes(&a), ak);
    }

    #[test]
    fn nk_and_rivk_are_fresh_per_derivation() {
        let ak = group_key();
        let a = derive_full_viewing_key(&ak, OsRng)
            .expect("fvk a")
            .to_bytes();
        let b = derive_full_viewing_key(&ak, OsRng)
            .expect("fvk b")
            .to_bytes();
        assert_eq!(a[..32], b[..32]);
        assert_ne!(a[32..], b[32..]);
    }

    #[test]
    fn ufvk_and_address_round_trip_on_testnet() {
        let fvk = derive_full_viewing_key(&group_key(), OsRng).expect("fvk");
        let ufvk = unified_full_viewing_key(&fvk).expect("ufvk");

        let encoded_ufvk = encode_ufvk(&ufvk, &crate::Params::TestNetwork);
        assert!(encoded_ufvk.starts_with("uviewtest"), "{encoded_ufvk}");
        let decoded = UnifiedFullViewingKey::decode(&crate::Params::TestNetwork, &encoded_ufvk)
            .expect("decode ufvk");
        assert_eq!(
            decoded.orchard().map(|o| o.to_bytes()),
            Some(fvk.to_bytes()),
            "orchard item survives the round trip"
        );
        assert!(decoded.sapling().is_none());

        let address = orchard_only_address(&ufvk).expect("address");
        let encoded = encode_address(&address, &crate::Params::TestNetwork);
        assert!(encoded.starts_with("utest"), "{encoded}");

        ZcashAddress::try_from_encoded(&encoded).expect("parse with zcash_address");
        let (net, ua) = unified::Address::decode(&encoded).expect("unified decode");
        assert_eq!(net, NetworkType::Test);
        let receivers = ua.items();
        assert_eq!(receivers.len(), 1, "exactly one receiver");
        assert!(matches!(receivers[0], unified::Receiver::Orchard(_)));

        // The UIVK carries only the external-scope Orchard IVK, which derives
        // the same default address and nothing for the internal scope.
        let uivk = unified_incoming_viewing_key(&ufvk);
        let encoded_uivk = encode_uivk(&uivk, &crate::Params::TestNetwork);
        assert!(encoded_uivk.starts_with("uivktest"), "{encoded_uivk}");
        let decoded_uivk =
            UnifiedIncomingViewingKey::decode(&crate::Params::TestNetwork, &encoded_uivk)
                .expect("uivk");
        let ivk = decoded_uivk.orchard().as_ref().expect("orchard ivk");
        assert_eq!(
            ivk.to_bytes(),
            fvk.to_ivk(orchard::keys::Scope::External).to_bytes()
        );
        assert_ne!(
            ivk.to_bytes(),
            fvk.to_ivk(orchard::keys::Scope::Internal).to_bytes()
        );

        let reparsed = match Address::decode(&crate::Params::TestNetwork, &encoded) {
            Some(Address::Unified(ua)) => ua,
            other => panic!("expected a unified address, got {other:?}"),
        };
        assert_eq!(reparsed.orchard(), address.orchard());
        assert!(reparsed.sapling().is_none());
        assert!(reparsed.transparent().is_none());
    }
}
