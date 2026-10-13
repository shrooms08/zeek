//! Sealing of payout receivers with HPKE (RFC 9180).
//!
//! The board's viewing key is public, so every dig memo is readable by anyone.
//! If the payout receiver were in the clear, an observer could link every dig
//! that shares a payout address. Instead the receiver is encrypted to the
//! vault's **payout sealing key** with HPKE base mode
//! (`DHKEM(X25519, HKDF-SHA256)`, `HKDF-SHA256`, `ChaCha20Poly1305`), and the
//! dig's public header (magic, version, round, tile) is bound in as AAD so a
//! sealed receiver cannot be cut from one dig and pasted onto another.
//!
//! Randomness for the ephemeral key is supplied by the caller (a
//! `rand_core 0.9` [`CryptoRng`] + [`RngCore`]); this crate never reaches for
//! an OS RNG, so it stays wasm-neutral.

use core::fmt;

use hpke::aead::ChaCha20Poly1305;
use hpke::kdf::HkdfSha256;
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable, HpkeError, Kem as KemTrait, OpModeR, OpModeS, Serializable};
use rand_core_09::{CryptoRng, RngCore};
use serde::{Deserialize, Serialize};

/// The KEM every sealing key uses.
pub type Kem = X25519HkdfSha256;

/// HPKE `info` string, fixing the application context of the key schedule.
pub const SEAL_INFO: &[u8] = b"zeek/payout-seal/v1";
/// Size of an X25519 encapsulated key.
pub const ENCAPPED_KEY_LEN: usize = 32;
/// Size of a ChaCha20Poly1305 authentication tag.
pub const TAG_LEN: usize = 16;

/// Errors from sealing and opening.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SealError {
    #[error("sealing public key is not a valid 32-byte X25519 point")]
    InvalidPublicKey,
    #[error("sealing private key is not a valid 32-byte X25519 scalar")]
    InvalidPrivateKey,
    #[error("sealed data is too short to contain an encapsulated key and a tag")]
    TooShort,
    #[error("HPKE encryption failed: {0}")]
    Seal(String),
    #[error("HPKE decryption failed: wrong key, wrong AAD, or corrupted data")]
    Open,
}

/// The vault's public payout sealing key (32 bytes, hex in serde).
#[derive(Clone, PartialEq, Eq)]
pub struct SealingPublicKey(<Kem as KemTrait>::PublicKey);

impl SealingPublicKey {
    /// Parses a 32-byte X25519 public key.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SealError> {
        <Kem as KemTrait>::PublicKey::from_bytes(bytes)
            .map(SealingPublicKey)
            .map_err(|_| SealError::InvalidPublicKey)
    }

    /// Returns the 32-byte encoding.
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        out.copy_from_slice(&self.0.to_bytes());
        out
    }
}

impl fmt::Debug for SealingPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SealingPublicKey({})", hex::encode(self.to_bytes()))
    }
}

impl Serialize for SealingPublicKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(self.to_bytes()))
    }
}

impl<'de> Deserialize<'de> for SealingPublicKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let bytes = hex::decode(text).map_err(serde::de::Error::custom)?;
        SealingPublicKey::from_bytes(&bytes).map_err(serde::de::Error::custom)
    }
}

/// The vault's private payout sealing key. Has no `Debug`, `Display` or serde
/// implementation so it cannot be printed or logged by accident; persist it
/// through [`SealingPrivateKey::to_bytes`] under the caller's own secret
/// storage.
#[derive(Clone)]
pub struct SealingPrivateKey(<Kem as KemTrait>::PrivateKey);

impl SealingPrivateKey {
    /// Parses a 32-byte X25519 private key.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SealError> {
        <Kem as KemTrait>::PrivateKey::from_bytes(bytes)
            .map(SealingPrivateKey)
            .map_err(|_| SealError::InvalidPrivateKey)
    }

    /// Returns the 32-byte encoding. Handle with care.
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        out.copy_from_slice(&self.0.to_bytes());
        out
    }

    /// The matching public key.
    pub fn public_key(&self) -> SealingPublicKey {
        SealingPublicKey(<Kem as KemTrait>::sk_to_pk(&self.0))
    }
}

/// Generates a fresh sealing key pair from the caller's CSPRNG.
pub fn generate_sealing_keypair<R: CryptoRng + RngCore>(
    rng: &mut R,
) -> (SealingPrivateKey, SealingPublicKey) {
    let (sk, pk) = <Kem as KemTrait>::gen_keypair(rng);
    (SealingPrivateKey(sk), SealingPublicKey(pk))
}

/// Seals `plaintext` to `recipient` with `aad` bound in.
///
/// Returns `encapped_key || ciphertext`, where the ciphertext is
/// `plaintext.len() + 16` bytes.
pub fn seal<R: CryptoRng + RngCore>(
    recipient: &SealingPublicKey,
    plaintext: &[u8],
    aad: &[u8],
    rng: &mut R,
) -> Result<Vec<u8>, SealError> {
    let (encapped, ciphertext) = hpke::single_shot_seal::<ChaCha20Poly1305, HkdfSha256, Kem, R>(
        &OpModeS::Base,
        &recipient.0,
        SEAL_INFO,
        plaintext,
        aad,
        rng,
    )
    .map_err(|e| SealError::Seal(hpke_error_text(e)))?;
    let mut out = Vec::with_capacity(ENCAPPED_KEY_LEN + ciphertext.len());
    out.extend_from_slice(&encapped.to_bytes());
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Opens data produced by [`seal`]; fails if the key or the AAD differ.
pub fn open(key: &SealingPrivateKey, sealed: &[u8], aad: &[u8]) -> Result<Vec<u8>, SealError> {
    if sealed.len() < ENCAPPED_KEY_LEN + TAG_LEN {
        return Err(SealError::TooShort);
    }
    let (encapped, ciphertext) = sealed.split_at(ENCAPPED_KEY_LEN);
    let encapped =
        <Kem as KemTrait>::EncappedKey::from_bytes(encapped).map_err(|_| SealError::Open)?;
    hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, Kem>(
        &OpModeR::Base,
        &key.0,
        &encapped,
        SEAL_INFO,
        ciphertext,
        aad,
    )
    .map_err(|_| SealError::Open)
}

fn hpke_error_text(e: HpkeError) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn seal_open_round_trip_and_tamper_detection() {
        let mut rng = StdRng::seed_from_u64(1);
        let (sk, pk) = generate_sealing_keypair(&mut rng);
        assert_eq!(sk.public_key(), pk);
        let sealed = seal(&pk, b"hello", b"aad", &mut rng).expect("seal");
        assert_eq!(sealed.len(), ENCAPPED_KEY_LEN + 5 + TAG_LEN);
        assert_eq!(open(&sk, &sealed, b"aad").expect("open"), b"hello");

        assert_eq!(open(&sk, &sealed, b"other"), Err(SealError::Open));
        let (other_sk, _) = generate_sealing_keypair(&mut rng);
        assert_eq!(open(&other_sk, &sealed, b"aad"), Err(SealError::Open));
        let mut corrupted = sealed.clone();
        corrupted[40] ^= 1;
        assert_eq!(open(&sk, &corrupted, b"aad"), Err(SealError::Open));
        assert_eq!(open(&sk, &sealed[..40], b"aad"), Err(SealError::TooShort));
    }

    #[test]
    fn keys_round_trip_through_bytes_and_serde() {
        let mut rng = StdRng::seed_from_u64(2);
        let (sk, pk) = generate_sealing_keypair(&mut rng);
        let sk2 = SealingPrivateKey::from_bytes(&sk.to_bytes()).expect("sk");
        assert_eq!(sk2.public_key(), pk);
        let json = serde_json::to_string(&pk).expect("json");
        let pk2: SealingPublicKey = serde_json::from_str(&json).expect("parse");
        assert_eq!(pk, pk2);
        assert_eq!(
            SealingPublicKey::from_bytes(&[1u8; 5]),
            Err(SealError::InvalidPublicKey)
        );
    }
}
