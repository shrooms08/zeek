//! Encryption at rest.
//!
//! * **Seed envelope** (JSON, stored in IndexedDB): the 32 bytes of BIP 39
//!   entropy encrypted with XChaCha20-Poly1305 under a key derived from the
//!   passphrase with Argon2id (64 MiB, 3 passes, 1 lane, 16-byte random
//!   salt). The passphrase itself is never stored.
//! * **State envelope** (bytes): the serialized wallet state encrypted with
//!   XChaCha20-Poly1305 under a key derived from the entropy with
//!   HKDF-SHA256 (`info = "zeek-pocket/state/v1"`). Opening it needs the
//!   unlocked seed, so the state reveals nothing while the wallet is locked.
//!
//! Both use a fresh random 24-byte nonce per encryption and bind a domain
//! string as associated data.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::error::PocketError;

/// Argon2id memory cost (KiB): 64 MiB.
pub const ARGON2_M_COST_KIB: u32 = 64 * 1024;
/// Argon2id passes.
pub const ARGON2_T_COST: u32 = 3;
/// Argon2id lanes.
pub const ARGON2_P_COST: u32 = 1;
/// Shortest passphrase accepted.
pub const MIN_PASSPHRASE_LEN: usize = 8;

const SEED_AAD: &[u8] = b"zeek-pocket/seed/v1";
const STATE_AAD: &[u8] = b"zeek-pocket/state/v1";
const STATE_INFO: &[u8] = b"zeek-pocket/state/v1";
const NONCE_LEN: usize = 24;

/// The stored, encrypted seed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedEnvelope {
    pub version: u32,
    pub kdf: String,
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    /// Hex.
    pub salt: String,
    /// Hex.
    pub nonce: String,
    /// Hex: ciphertext and tag.
    pub ciphertext: String,
}

fn random<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    OsRng.fill_bytes(&mut out);
    out
}

fn passphrase_key(
    passphrase: &str,
    salt: &[u8],
    m: u32,
    t: u32,
    p: u32,
) -> Result<Zeroizing<[u8; 32]>, PocketError> {
    let params = Params::new(m, t, p, Some(32))
        .map_err(|e| PocketError::Corrupt(format!("argon2 parameters: {e}")))?;
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), salt, key.as_mut())
        .map_err(|e| PocketError::Corrupt(format!("argon2: {e}")))?;
    Ok(key)
}

fn seal(
    key: &[u8; 32],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<([u8; NONCE_LEN], Vec<u8>), PocketError> {
    let nonce = random::<NONCE_LEN>();
    let ciphertext = XChaCha20Poly1305::new(key.into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| PocketError::Corrupt("encryption failed".into()))?;
    Ok((nonce, ciphertext))
}

fn open(
    key: &[u8; 32],
    aad: &[u8],
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, PocketError> {
    if nonce.len() != NONCE_LEN {
        return Err(PocketError::BadPassphrase);
    }
    XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| PocketError::BadPassphrase)
}

/// Encrypts the seed entropy under `passphrase`.
pub fn seal_seed(entropy: &[u8; 32], passphrase: &str) -> Result<SeedEnvelope, PocketError> {
    if passphrase.chars().count() < MIN_PASSPHRASE_LEN {
        return Err(PocketError::WeakPassphrase(MIN_PASSPHRASE_LEN));
    }
    let salt = random::<16>();
    let key = passphrase_key(
        passphrase,
        &salt,
        ARGON2_M_COST_KIB,
        ARGON2_T_COST,
        ARGON2_P_COST,
    )?;
    let (nonce, ciphertext) = seal(&key, SEED_AAD, entropy)?;
    Ok(SeedEnvelope {
        version: 1,
        kdf: "argon2id".into(),
        m_cost_kib: ARGON2_M_COST_KIB,
        t_cost: ARGON2_T_COST,
        p_cost: ARGON2_P_COST,
        salt: hex::encode(salt),
        nonce: hex::encode(nonce),
        ciphertext: hex::encode(ciphertext),
    })
}

/// Decrypts the seed entropy; a wrong passphrase is [`PocketError::BadPassphrase`].
pub fn open_seed(
    envelope: &SeedEnvelope,
    passphrase: &str,
) -> Result<Zeroizing<[u8; 32]>, PocketError> {
    if envelope.version != 1 || envelope.kdf != "argon2id" {
        return Err(PocketError::Corrupt("unsupported seed envelope".into()));
    }
    let decode =
        |s: &str| hex::decode(s).map_err(|_| PocketError::Corrupt("seed envelope hex".into()));
    let key = passphrase_key(
        passphrase,
        &decode(&envelope.salt)?,
        envelope.m_cost_kib,
        envelope.t_cost,
        envelope.p_cost,
    )?;
    let plaintext = Zeroizing::new(open(
        &key,
        SEED_AAD,
        &decode(&envelope.nonce)?,
        &decode(&envelope.ciphertext)?,
    )?);
    let entropy = <[u8; 32]>::try_from(plaintext.as_slice())
        .map_err(|_| PocketError::Corrupt("seed is not 32 bytes".into()))?;
    Ok(Zeroizing::new(entropy))
}

/// The key that encrypts the wallet state, derived from the seed entropy.
pub fn state_key(entropy: &[u8; 32]) -> Result<Zeroizing<[u8; 32]>, PocketError> {
    let mut key = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(Some(b"zeek-pocket"), entropy)
        .expand(STATE_INFO, key.as_mut())
        .map_err(|_| PocketError::Corrupt("hkdf".into()))?;
    Ok(key)
}

/// Encrypts serialized state: `nonce (24) || ciphertext`.
pub fn seal_state(key: &[u8; 32], state: &[u8]) -> Result<Vec<u8>, PocketError> {
    let (nonce, ciphertext) = seal(key, STATE_AAD, state)?;
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypts what [`seal_state`] produced.
pub fn open_state(key: &[u8; 32], envelope: &[u8]) -> Result<Vec<u8>, PocketError> {
    if envelope.len() < NONCE_LEN {
        return Err(PocketError::Corrupt("state envelope is too short".into()));
    }
    open(
        key,
        STATE_AAD,
        &envelope[..NONCE_LEN],
        &envelope[NONCE_LEN..],
    )
    .map_err(|_| PocketError::Corrupt("state does not decrypt with this seed".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_envelope_round_trips_and_rejects_wrong_passphrases() {
        let entropy = [9u8; 32];
        let envelope = seal_seed(&entropy, "correct horse").expect("seal");
        assert_eq!(envelope.m_cost_kib, 65_536);
        let json = serde_json::to_string(&envelope).expect("json");
        assert!(
            !json.contains(&hex::encode(entropy)),
            "entropy is not stored in clear"
        );
        let opened = open_seed(&envelope, "correct horse").expect("open");
        assert_eq!(*opened, entropy);
        assert!(matches!(
            open_seed(&envelope, "wrong horse"),
            Err(PocketError::BadPassphrase)
        ));
        assert!(matches!(
            seal_seed(&entropy, "short"),
            Err(PocketError::WeakPassphrase(8))
        ));
        let again = seal_seed(&entropy, "correct horse").expect("seal");
        assert_ne!(again.salt, envelope.salt, "fresh salt per envelope");
    }

    #[test]
    fn state_envelope_is_bound_to_the_seed() {
        let key = state_key(&[1; 32]).expect("key");
        let other = state_key(&[2; 32]).expect("key");
        let sealed = seal_state(&key, b"wallet state").expect("seal");
        assert_eq!(open_state(&key, &sealed).expect("open"), b"wallet state");
        assert!(open_state(&other, &sealed).is_err());
        let mut tampered = sealed.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(open_state(&key, &tampered).is_err());
    }
}
