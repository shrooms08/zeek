//! FROST threshold signing over the RedPallas ciphersuite.
//!
//! Orchard spend authorization signatures are RedPallas signatures made with a
//! *rerandomized* key: for each action the signer uses `rsk = ask + alpha` and
//! the verifier uses `rk = ak + [alpha] G`. FROST supports this through
//! [`frost_rerandomized`], where the coordinator supplies the randomizer
//! `alpha` and the signers add it to their shares before signing.
//!
//! Key generation here uses a trusted dealer. That is acceptable for this spike
//! because every share is produced on one machine anyway; production will use
//! the DKG from `reddsa::frost::redpallas::keys::dkg` so that no party ever
//! holds the full key.
//!
//! The ciphersuite is `reddsa`'s [`PallasBlake2b512`], which also fixes the sign
//! of the group key: its `post_generate` hook negates every share when the
//! group verifying key has an odd y-coordinate, so `ak` always satisfies the
//! Orchard encoding rule. [`dealer_keygen`] re-checks this rather than trusting
//! it.
//!
//! [`PallasBlake2b512`]: reddsa::frost::redpallas::PallasBlake2b512

use std::collections::{BTreeMap, BTreeSet};

use frost_rerandomized::{RandomizedParams, Randomizer};
use pasta_curves::pallas;
use rand_core::{CryptoRng, RngCore};
use reddsa::frost::redpallas as rp;
use reddsa::frost::redpallas::keys::EvenY;

use crate::error::VaultError;

/// The ciphersuite every type in this module is instantiated with.
pub type Ciphersuite = rp::PallasBlake2b512;
/// Error type produced by the FROST library for this ciphersuite.
pub type FrostError = rp::Error;
/// A participant identifier (1-based).
pub type Identifier = rp::Identifier;
/// A keyholder's secret share plus the public data needed to sign.
pub type KeyPackage = rp::keys::KeyPackage;
/// The group verifying key and every participant's verifying share.
pub type PublicKeyPackage = rp::keys::PublicKeyPackage;
/// Round 1 secret nonces (single use).
pub type SigningNonces = rp::round1::SigningNonces;
/// Round 1 public commitments.
pub type SigningCommitments = rp::round1::SigningCommitments;
/// Coordinator's signing package: message plus every signer's commitments.
pub type SigningPackage = rp::SigningPackage;
/// Round 2 signature share.
pub type SignatureShare = rp::round2::SignatureShare;

/// Output of key generation: one key package per keyholder plus the public
/// package shared by everyone.
pub struct KeygenOutput {
    /// Key packages keyed by participant identifier.
    pub key_packages: BTreeMap<Identifier, KeyPackage>,
    /// Public key package.
    pub public_key_package: PublicKeyPackage,
}

/// Runs trusted-dealer key generation for a `threshold`-of-`signers` vault.
///
/// Identifiers are `1..=signers`. Fails if `threshold < 2` or
/// `threshold > signers`, or if (contrary to the ciphersuite's guarantee) the
/// resulting group key has an odd y-coordinate.
pub fn dealer_keygen<R: RngCore + CryptoRng>(
    threshold: u16,
    signers: u16,
    rng: R,
) -> Result<KeygenOutput, VaultError> {
    if threshold < 2 || threshold > signers {
        return Err(VaultError::InvalidThreshold { threshold, signers });
    }

    let (secret_shares, public_key_package) =
        rp::keys::generate_with_dealer(signers, threshold, rp::keys::IdentifierList::Default, rng)?;

    if !public_key_package.has_even_y() {
        return Err(VaultError::OddGroupKey);
    }

    let mut key_packages = BTreeMap::new();
    for (identifier, share) in secret_shares {
        // `try_from` verifies the share against the VSS commitment.
        let key_package = KeyPackage::try_from(share)?;
        if !key_package.has_even_y() {
            return Err(VaultError::OddGroupKey);
        }
        key_packages.insert(identifier, key_package);
    }

    Ok(KeygenOutput {
        key_packages,
        public_key_package,
    })
}

/// Returns the 32-byte encoding of the group verifying key.
pub fn group_key_bytes(public_key_package: &PublicKeyPackage) -> Result<[u8; 32], VaultError> {
    let bytes = public_key_package.verifying_key().serialize()?;
    to_array_32(&bytes, "group verifying key")
}

/// Builds a FROST identifier from a 1-based keyholder number.
pub fn identifier(number: u16) -> Result<Identifier, VaultError> {
    Ok(Identifier::try_from(number)?)
}

/// Converts an identifier back into its 1-based keyholder number.
///
/// Identifiers made by [`identifier`] serialize as a little-endian scalar whose
/// first two bytes hold the number.
pub fn identifier_number(id: &Identifier) -> Result<u16, VaultError> {
    let bytes = id.serialize();
    let tail_is_zero = bytes.iter().skip(2).all(|b| *b == 0);
    match (bytes.first(), bytes.get(1), tail_is_zero) {
        (Some(lo), Some(hi), true) => Ok(u16::from_le_bytes([*lo, *hi])),
        _ => Err(VaultError::BadLength {
            field: "identifier",
            expected: 2,
            actual: bytes.len(),
        }),
    }
}

/// A rerandomized signature together with the key it verifies under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RandomizedSignature {
    /// RedPallas signature bytes (`R || s`).
    pub signature: [u8; 64],
    /// The randomized verifying key `ak + [alpha] G`, as 32 bytes.
    pub randomized_verifying_key: [u8; 32],
}

/// Round 1 of signing for one keyholder: fresh single-use nonces and the
/// commitments to publish. The nonces must be used at most once and dropped
/// on any abort.
pub fn commit<R: RngCore + CryptoRng>(
    key_package: &KeyPackage,
    rng: &mut R,
) -> (rp::round1::SigningNonces, rp::round1::SigningCommitments) {
    rp::round1::commit(key_package.signing_share(), rng)
}

/// Round 2 of signing for one keyholder: its share of the rerandomized
/// signature over `signing_package`, using the spend's randomizer `alpha`.
///
/// `frost_rerandomized::sign` is marked deprecated in favour of the seed-based
/// API, which lets the coordinator *choose* a fresh randomizer. Orchard fixes
/// the randomizer (`alpha`) in the PCZT before signing starts, so the
/// seed-based flow does not apply here; the `Randomize` trait that would let
/// us do this by hand is private.
pub fn sign_share(
    signing_package: &rp::SigningPackage,
    nonces: &rp::round1::SigningNonces,
    key_package: &KeyPackage,
    alpha: &pallas::Scalar,
) -> Result<rp::round2::SignatureShare, VaultError> {
    let randomizer = Randomizer::<Ciphersuite>::from_scalar(*alpha);
    #[allow(deprecated)]
    let share = frost_rerandomized::sign(signing_package, nonces, key_package, randomizer)?;
    Ok(share)
}

/// Produces a rerandomized FROST signature over `message` using exactly the
/// given signers.
///
/// Every selected signer must be a member of `public_key_package`, the set must
/// be free of duplicates and at least `min_signers` large. All three rounds run
/// in this process (see the crate docs for why that is acceptable here). The
/// aggregated signature is verified against the randomized verifying key before
/// it is returned, so a corrupted share or nonce can never leak out as a
/// "signature".
pub fn randomized_sign<R: RngCore + CryptoRng>(
    message: &[u8],
    alpha: &pallas::Scalar,
    signers: &[&KeyPackage],
    public_key_package: &PublicKeyPackage,
    mut rng: R,
) -> Result<RandomizedSignature, VaultError> {
    check_signer_set(signers, public_key_package)?;

    // Round 1: each signer draws fresh nonces and publishes commitments.
    let mut nonces = BTreeMap::new();
    let mut commitments = BTreeMap::new();
    for key_package in signers {
        let (signer_nonces, signer_commitments) = commit(key_package, &mut rng);
        nonces.insert(*key_package.identifier(), signer_nonces);
        commitments.insert(*key_package.identifier(), signer_commitments);
    }

    // The coordinator assembles the signing package and the randomizer.
    let signing_package = rp::SigningPackage::new(commitments, message);

    // Round 2: each signer produces a signature share with its randomized key.
    let mut shares = BTreeMap::new();
    for key_package in signers {
        let signer_nonces = nonces
            .get(key_package.identifier())
            .ok_or_else(|| VaultError::UnknownSigner(0))?;
        let share = sign_share(&signing_package, signer_nonces, key_package, alpha)?;
        shares.insert(*key_package.identifier(), share);
    }

    aggregate(&signing_package, &shares, public_key_package, alpha)
}

/// Aggregates signature shares for the spend with randomizer `alpha` and
/// verifies the result against the randomized group key before returning it.
pub fn aggregate(
    signing_package: &rp::SigningPackage,
    shares: &BTreeMap<Identifier, rp::round2::SignatureShare>,
    public_key_package: &PublicKeyPackage,
    alpha: &pallas::Scalar,
) -> Result<RandomizedSignature, VaultError> {
    let randomizer = Randomizer::<Ciphersuite>::from_scalar(*alpha);
    let randomized_params =
        RandomizedParams::from_randomizer(public_key_package.verifying_key(), randomizer);
    aggregate_shares(
        signing_package,
        shares,
        public_key_package,
        &randomized_params,
    )
}

/// Aggregates signature shares and verifies the result against the randomized
/// verifying key. Split out so tests can feed in a corrupted share.
pub(crate) fn aggregate_shares(
    signing_package: &rp::SigningPackage,
    shares: &BTreeMap<Identifier, rp::round2::SignatureShare>,
    public_key_package: &PublicKeyPackage,
    randomized_params: &RandomizedParams<Ciphersuite>,
) -> Result<RandomizedSignature, VaultError> {
    let signature = frost_rerandomized::aggregate(
        signing_package,
        shares,
        public_key_package,
        randomized_params,
    )?;

    let randomized_verifying_key = randomized_params.randomized_verifying_key();
    randomized_verifying_key
        .verify(signing_package.message(), &signature)
        .map_err(|_| VaultError::SignatureVerification)?;

    Ok(RandomizedSignature {
        signature: to_array_64(&signature.serialize()?, "signature")?,
        randomized_verifying_key: to_array_32(
            &randomized_verifying_key.serialize()?,
            "randomized verifying key",
        )?,
    })
}

/// Rejects signer sets that are too small, contain duplicates, or contain a
/// package that does not belong to this group.
pub fn check_signer_set(
    signers: &[&KeyPackage],
    public_key_package: &PublicKeyPackage,
) -> Result<(), VaultError> {
    let threshold = public_key_package
        .min_signers()
        .ok_or_else(|| VaultError::Frost(FrostError::IncorrectNumberOfIdentifiers))?;
    if signers.len() < usize::from(threshold) {
        return Err(VaultError::NotEnoughSigners {
            given: signers.len(),
            threshold,
        });
    }

    let group_key = public_key_package.verifying_key();
    let mut seen = BTreeSet::new();
    for key_package in signers {
        let number = identifier_number(key_package.identifier())?;
        if !seen.insert(*key_package.identifier()) {
            return Err(VaultError::DuplicateSigner(number));
        }
        let expected_share = public_key_package
            .verifying_shares()
            .get(key_package.identifier())
            .ok_or(VaultError::UnknownSigner(number))?;
        if key_package.verifying_key() != group_key
            || key_package.verifying_share() != expected_share
        {
            return Err(VaultError::UnknownSigner(number));
        }
    }
    Ok(())
}

fn to_array_32(bytes: &[u8], field: &'static str) -> Result<[u8; 32], VaultError> {
    <[u8; 32]>::try_from(bytes).map_err(|_| VaultError::BadLength {
        field,
        expected: 32,
        actual: bytes.len(),
    })
}

fn to_array_64(bytes: &[u8], field: &'static str) -> Result<[u8; 64], VaultError> {
    <[u8; 64]>::try_from(bytes).map_err(|_| VaultError::BadLength {
        field,
        expected: 64,
        actual: bytes.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ff::{Field, PrimeField};
    use rand_core::OsRng;

    fn keygen() -> KeygenOutput {
        dealer_keygen(2, 3, OsRng).expect("keygen")
    }

    fn packages<'a>(out: &'a KeygenOutput, numbers: &[u16]) -> Vec<&'a KeyPackage> {
        numbers
            .iter()
            .map(|n| {
                let id = identifier(*n).expect("identifier");
                out.key_packages.get(&id).expect("package")
            })
            .collect()
    }

    #[test]
    fn keygen_produces_three_shares_with_even_y_group_key() {
        let out = keygen();
        assert_eq!(out.key_packages.len(), 3);
        let group = group_key_bytes(&out.public_key_package).expect("group key");
        assert_eq!(group[31] & 0x80, 0, "sign bit must be clear");
        for (id, kp) in &out.key_packages {
            assert_eq!(kp.verifying_key(), out.public_key_package.verifying_key());
            assert!(out.public_key_package.verifying_shares().contains_key(id));
        }
    }

    #[test]
    fn rejects_bad_threshold_parameters() {
        assert!(matches!(
            dealer_keygen(1, 3, OsRng),
            Err(VaultError::InvalidThreshold { .. })
        ));
        assert!(matches!(
            dealer_keygen(4, 3, OsRng),
            Err(VaultError::InvalidThreshold { .. })
        ));
    }

    #[test]
    fn any_two_signers_produce_a_verifying_rerandomized_signature() {
        let out = keygen();
        let message = [7u8; 32];
        for pair in [[1u16, 2], [1, 3], [2, 3]] {
            let alpha = pallas::Scalar::random(OsRng);
            let sig = randomized_sign(
                &message,
                &alpha,
                &packages(&out, &pair),
                &out.public_key_package,
                OsRng,
            )
            .expect("signing with two signers");

            // Independently verify with the RedPallas verification key.
            let rk = orchard::primitives::redpallas::VerificationKey::<
                orchard::primitives::redpallas::SpendAuth,
            >::try_from(sig.randomized_verifying_key)
            .expect("rk");
            let signature = orchard::primitives::redpallas::Signature::from(sig.signature);
            rk.verify(&message, &signature)
                .expect("RedPallas verification");

            // And check the randomized key is ak + [alpha] G.
            let ak = group_key_bytes(&out.public_key_package).expect("ak");
            let ak_vk = orchard::primitives::redpallas::VerificationKey::<
                orchard::primitives::redpallas::SpendAuth,
            >::try_from(ak)
            .expect("ak key");
            let expected: [u8; 32] = (&ak_vk.randomize(&alpha)).into();
            assert_eq!(expected, sig.randomized_verifying_key);
        }
    }

    #[test]
    fn one_signer_cannot_sign() {
        let out = keygen();
        let alpha = pallas::Scalar::random(OsRng);
        let err = randomized_sign(
            &[1u8; 32],
            &alpha,
            &packages(&out, &[2]),
            &out.public_key_package,
            OsRng,
        )
        .expect_err("one signer must fail");
        assert!(matches!(
            err,
            VaultError::NotEnoughSigners {
                given: 1,
                threshold: 2
            }
        ));
    }

    #[test]
    fn duplicate_and_foreign_signers_are_rejected() {
        let out = keygen();
        let other = keygen();
        let alpha = pallas::Scalar::random(OsRng);
        let dup = randomized_sign(
            &[1u8; 32],
            &alpha,
            &packages(&out, &[1, 1]),
            &out.public_key_package,
            OsRng,
        )
        .expect_err("duplicate signer");
        assert!(matches!(dup, VaultError::DuplicateSigner(1)));

        let mut mixed = packages(&out, &[1]);
        mixed.extend(packages(&other, &[2]));
        let foreign = randomized_sign(&[1u8; 32], &alpha, &mixed, &out.public_key_package, OsRng)
            .expect_err("foreign signer");
        assert!(matches!(foreign, VaultError::UnknownSigner(2)));
    }

    #[test]
    fn corrupted_signature_share_is_detected() {
        let out = keygen();
        let message = [3u8; 32];
        let alpha = pallas::Scalar::random(OsRng);
        let signers = packages(&out, &[1, 3]);

        let mut nonces = BTreeMap::new();
        let mut commitments = BTreeMap::new();
        for kp in &signers {
            let (n, c) = rp::round1::commit(kp.signing_share(), &mut OsRng);
            nonces.insert(*kp.identifier(), n);
            commitments.insert(*kp.identifier(), c);
        }
        let signing_package = rp::SigningPackage::new(commitments, &message);
        let randomizer = Randomizer::<Ciphersuite>::from_scalar(alpha);
        let params =
            RandomizedParams::from_randomizer(out.public_key_package.verifying_key(), randomizer);

        let mut shares = BTreeMap::new();
        for kp in &signers {
            #[allow(deprecated)]
            let share = frost_rerandomized::sign(
                &signing_package,
                &nonces[kp.identifier()],
                kp,
                randomizer,
            )
            .expect("share");
            shares.insert(*kp.identifier(), share);
        }

        // Replace one share with a random scalar.
        let bogus =
            rp::round2::SignatureShare::deserialize(&pallas::Scalar::random(OsRng).to_repr())
                .expect("bogus share");
        let victim = *signers[0].identifier();
        shares.insert(victim, bogus);

        let err = aggregate_shares(&signing_package, &shares, &out.public_key_package, &params)
            .expect_err("corrupted share must be rejected");
        assert!(matches!(err, VaultError::Frost(_)), "got {err}");
    }

    #[test]
    fn identifier_round_trip() {
        for n in [1u16, 2, 3, 250] {
            let id = identifier(n).expect("id");
            assert_eq!(identifier_number(&id).expect("number"), n);
        }
    }
}
