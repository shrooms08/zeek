//! The DKG ceremony: FROST distributed key generation (RFC 9591 / frost-core
//! `keys::dkg`) between the keyholders over signed HTTP messages, followed by
//! the derivation of the vault's viewing keys and address, which every node
//! computes and cross-checks.
//!
//! # Even-y `ak`
//!
//! Orchard requires the spend validating key `ak` to encode with an even
//! y-coordinate. `frost_core::keys::dkg::part3` ends by calling
//! `Ciphersuite::post_dkg`, which reddsa's RedPallas ciphersuite implements
//! as `keys::into_even_y` on the key package and the public key package
//! (`reddsa-0.5.2/src/frost/redpallas.rs`, `post_dkg`; `keys::EvenY` negates
//! the verifying key, every verifying share and the signing share when the
//! y-coordinate is odd, which is consistent because the negation is linear
//! through Shamir sharing). This module re-checks `has_even_y()` on both
//! packages rather than trusting it.
//!
//! # `nk` and `rivk`
//!
//! The viewing key needs a nullifier key and a commitment randomness beside
//! `ak`. They are derived from a **transcript seed**:
//! `SHA-256("zeek/dkg-nk-rivk/v1" || group key || id_1 || round1_pkg_1 || …)`
//! over every participant's round-1 package in identifier order. Each
//! participant's package contains its own fresh randomness (the commitment to
//! its secret polynomial), so no single participant can choose the seed, and
//! every participant knows all packages, so all derive the same keys. The
//! seed is then used as the Orchard spending key from which `nk` and `rivk`
//! are taken (`zeek_vault::keys::derive_full_viewing_key_from_seed`); its
//! `ask` is discarded. The nodes exchange their derived address and viewing
//! key and abort unless all are byte-identical.
//!
//! # Payout sealing key
//!
//! After a successful ceremony the lead node (lowest identifier) generates
//! the HPKE payout sealing key pair and writes it to its secrets directory.
//! The other keyholders and the engine need the same private key; here it is
//! copied as a local file by the operator. In production it would travel over
//! an authenticated, encrypted channel (for example sealed to each node's
//! identity key); this spike does not implement that transport.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use orchard::keys::FullViewingKey;
use rand_09::TryRngCore;
use reddsa::frost::redpallas::keys::dkg::{self, round1, round2};
use reddsa::frost::redpallas::keys::EvenY;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use zcash_keys::keys::UnifiedFullViewingKey;
use zeek_core::seal::{generate_sealing_keypair, SealingPublicKey};
use zeek_vault::frost::{self, Identifier, KeyPackage, PublicKeyPackage};
use zeek_vault::keys;
use zeek_vault::store::{KeyholderFile, SealingKeyFile, SecretsDir, VaultFile};

use crate::config::KeyholderConfig;
use crate::error::KeyholderError;
use zeek_engine::identity::Identity;
use zeek_engine::protocol::{
    self, kind, DkgConfirm, DkgRound1, DkgRound2, Envelope, Expect, SessionId,
};

/// Messages received from peers during a ceremony.
#[derive(Default)]
pub struct Inbox {
    pub session: Option<SessionId>,
    pub round1: BTreeMap<u16, Vec<u8>>,
    pub round2: BTreeMap<u16, Vec<u8>>,
    pub confirms: BTreeMap<u16, DkgConfirm>,
}

/// Shared ceremony state (the API fills it, the ceremony drains it).
pub struct DkgState {
    inbox: Mutex<Inbox>,
}

impl Default for DkgState {
    fn default() -> Self {
        DkgState {
            inbox: Mutex::new(Inbox::default()),
        }
    }
}

impl DkgState {
    /// Opens a ceremony with `session`.
    pub async fn begin(&self, session: SessionId) {
        let mut inbox = self.inbox.lock().await;
        *inbox = Inbox {
            session: Some(session),
            ..Inbox::default()
        };
    }

    async fn accept<T: serde::de::DeserializeOwned>(
        &self,
        config: &KeyholderConfig,
        envelope: &Envelope,
        expected_kind: &str,
    ) -> Result<(u16, T), KeyholderError> {
        let peer = config.peer(envelope.sender).ok_or_else(|| {
            crate::error::protocol(format!("unknown participant {}", envelope.sender))
        })?;
        let inbox = self.inbox.lock().await;
        let session = inbox.session.ok_or_else(|| {
            crate::error::protocol("no DKG ceremony is running on this node".into())
        })?;
        let (_, payload) = protocol::open::<T>(
            envelope,
            &Expect {
                kind: expected_kind,
                sender: peer.identifier,
                sender_key: &peer.identity,
                session: Some(&session),
            },
        )?;
        Ok((peer.identifier, payload))
    }

    /// Accepts a peer's round 1 package.
    pub async fn accept_round1(
        &self,
        config: &KeyholderConfig,
        envelope: &Envelope,
    ) -> Result<(), KeyholderError> {
        let (from, msg): (u16, DkgRound1) = self.accept(config, envelope, kind::DKG_ROUND1).await?;
        round1::Package::deserialize(&msg.package)
            .map_err(|e| crate::error::protocol(format!("round 1 package: {e}")))?;
        self.inbox.lock().await.round1.insert(from, msg.package);
        Ok(())
    }

    /// Accepts a peer's round 2 package addressed to this node.
    pub async fn accept_round2(
        &self,
        config: &KeyholderConfig,
        envelope: &Envelope,
    ) -> Result<(), KeyholderError> {
        let (from, msg): (u16, DkgRound2) = self.accept(config, envelope, kind::DKG_ROUND2).await?;
        round2::Package::deserialize(&msg.package)
            .map_err(|e| crate::error::protocol(format!("round 2 package: {e}")))?;
        self.inbox.lock().await.round2.insert(from, msg.package);
        Ok(())
    }

    /// Accepts a peer's derived-key confirmation.
    pub async fn accept_confirm(
        &self,
        config: &KeyholderConfig,
        envelope: &Envelope,
    ) -> Result<(), KeyholderError> {
        let (from, msg): (u16, DkgConfirm) =
            self.accept(config, envelope, kind::DKG_CONFIRM).await?;
        self.inbox.lock().await.confirms.insert(from, msg);
        Ok(())
    }
}

/// Everything a successful ceremony produced.
pub struct DkgOutput {
    pub key_package: KeyPackage,
    pub public_key_package: PublicKeyPackage,
    pub fvk: FullViewingKey,
    pub ufvk: UnifiedFullViewingKey,
    pub address: String,
    pub sealing_public_key: Option<SealingPublicKey>,
}

/// How long to wait for peers at each step.
const STEP_TIMEOUT: Duration = Duration::from_secs(600);
const RETRY: Duration = Duration::from_secs(2);

/// Runs the ceremony for this node. The node's API server must be running so
/// peers can deliver their packages into `state`.
pub async fn run_ceremony(
    config: &KeyholderConfig,
    identity: &Identity,
    state: &DkgState,
    session: SessionId,
    client: &reqwest::Client,
) -> Result<DkgOutput, KeyholderError> {
    let secrets = SecretsDir::new(&config.secrets_dir);
    if secrets.keyholder_path(config.identifier).exists() {
        return Err(KeyholderError::Dkg(format!(
            "{} already exists; this node already holds a key share",
            secrets.keyholder_path(config.identifier).display()
        )));
    }
    let me = frost::identifier(config.identifier)?;
    state.begin(session).await;
    tracing::info!(session = %hex::encode(session), identifier = config.identifier, "DKG ceremony starting");

    // Round 1: commit to a secret polynomial and broadcast the commitment.
    let (secret1, package1) = dkg::part1(me, config.signers, config.threshold, rand_core::OsRng)?;
    let package1_bytes = package1.serialize()?;
    let round1_env = protocol::seal(
        identity,
        &session,
        config.identifier,
        kind::DKG_ROUND1,
        &DkgRound1 {
            package: package1_bytes.clone(),
        },
    )?;
    for peer in &config.peers {
        deliver(client, &format!("{}/v1/dkg/round1", peer.url), &round1_env).await?;
    }
    let round1_received = wait_for(state, |inbox| inbox.round1.clone(), config.peers.len()).await?;
    let mut round1_packages = BTreeMap::new();
    for (id, bytes) in &round1_received {
        round1_packages.insert(
            frost::identifier(*id)?,
            round1::Package::deserialize(bytes)?,
        );
    }

    // Round 2: send each peer its share of our polynomial.
    let (secret2, round2_packages) = dkg::part2(secret1, &round1_packages)?;
    for (recipient, package) in &round2_packages {
        let number = frost::identifier_number(recipient)?;
        let peer = config.peer(number).ok_or_else(|| {
            KeyholderError::Dkg(format!("round 2 package for unknown participant {number}"))
        })?;
        let env = protocol::seal(
            identity,
            &session,
            config.identifier,
            kind::DKG_ROUND2,
            &DkgRound2 {
                package: package.serialize()?,
            },
        )?;
        deliver(client, &format!("{}/v1/dkg/round2", peer.url), &env).await?;
    }
    let round2_received = wait_for(state, |inbox| inbox.round2.clone(), config.peers.len()).await?;
    let mut round2_for_me = BTreeMap::new();
    for (id, bytes) in &round2_received {
        round2_for_me.insert(
            frost::identifier(*id)?,
            round2::Package::deserialize(bytes)?,
        );
    }

    // Round 3: the key share and the group key, made even-y by reddsa's post_dkg.
    let (key_package, public_key_package) = dkg::part3(&secret2, &round1_packages, &round2_for_me)?;
    if !key_package.has_even_y() || !public_key_package.has_even_y() {
        return Err(KeyholderError::Dkg(
            "group key does not have an even y-coordinate after DKG".into(),
        ));
    }
    let group_key = frost::group_key_bytes(&public_key_package)?;

    // Transcript seed → nk, rivk → viewing keys and address.
    let mut transcript: BTreeMap<u16, Vec<u8>> = round1_received;
    transcript.insert(config.identifier, package1_bytes);
    let seed = transcript_seed(&group_key, &transcript);
    let fvk = keys::derive_full_viewing_key_from_seed(&group_key, seed)?;
    let ufvk = keys::unified_full_viewing_key(&fvk)?;
    let address = keys::encode_address(&keys::orchard_only_address(&ufvk)?);

    // Cross-check with every peer before writing anything.
    let mine = DkgConfirm {
        group_key,
        full_viewing_key: fvk.to_bytes().to_vec(),
        address: address.clone(),
    };
    let confirm_env = protocol::seal(
        identity,
        &session,
        config.identifier,
        kind::DKG_CONFIRM,
        &mine,
    )?;
    for peer in &config.peers {
        deliver(
            client,
            &format!("{}/v1/dkg/confirm", peer.url),
            &confirm_env,
        )
        .await?;
    }
    let confirms = wait_for(state, |inbox| inbox.confirms.clone(), config.peers.len()).await?;
    for (id, theirs) in &confirms {
        if *theirs != mine {
            return Err(KeyholderError::Dkg(format!(
                "participant {id} derived a different vault (address {} vs ours {}); aborting",
                theirs.address, mine.address
            )));
        }
    }

    // Persist: key share, public vault file, and (lead only) the sealing key.
    secrets.write_keyholder(
        config.identifier,
        &KeyholderFile::new(config.identifier, &key_package)?,
    )?;
    let vault = VaultFile::new(
        config.threshold,
        config.signers,
        &public_key_package,
        &fvk,
        &ufvk,
        &address,
    )?;
    secrets.write_vault(&vault)?;
    let sealing_public_key = if config.is_lead() {
        let mut rng = rand_09::rngs::OsRng.unwrap_err();
        let (sk, pk) = generate_sealing_keypair(&mut rng);
        secrets.write_sealing_key(&SealingKeyFile::new(&sk), false)?;
        Some(pk)
    } else {
        None
    };
    tracing::info!(address = %address, "DKG ceremony complete; key share written");
    Ok(DkgOutput {
        key_package,
        public_key_package,
        fvk,
        ufvk,
        address,
        sealing_public_key,
    })
}

/// `SHA-256("zeek/dkg-nk-rivk/v1" || group key || (id BE || round1 package)*)`
/// over the packages in identifier order.
pub fn transcript_seed(group_key: &[u8; 32], round1_packages: &BTreeMap<u16, Vec<u8>>) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"zeek/dkg-nk-rivk/v1");
    hasher.update(group_key);
    for (id, package) in round1_packages {
        hasher.update(id.to_be_bytes());
        hasher.update((package.len() as u32).to_be_bytes());
        hasher.update(package);
    }
    hasher.finalize().into()
}

/// POSTs `envelope` to `url` until the peer accepts it (peers may still be
/// starting), within the step timeout.
async fn deliver(
    client: &reqwest::Client,
    url: &str,
    envelope: &Envelope,
) -> Result<(), KeyholderError> {
    let start = Instant::now();
    loop {
        match client.post(url).json(envelope).send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                // A rejection is final: the peer verified and refused the message.
                if status.is_client_error() {
                    return Err(KeyholderError::Peer(format!("{url}: {status} {body}")));
                }
                tracing::warn!(url, %status, "peer not ready; retrying");
            }
            Err(e) => tracing::warn!(url, error = %e, "peer unreachable; retrying"),
        }
        if start.elapsed() > STEP_TIMEOUT {
            return Err(KeyholderError::Peer(format!(
                "{url}: no answer within {STEP_TIMEOUT:?}"
            )));
        }
        tokio::time::sleep(RETRY).await;
    }
}

/// Waits until `read` yields `needed` entries.
async fn wait_for<T: Clone>(
    state: &DkgState,
    read: impl Fn(&Inbox) -> BTreeMap<u16, T>,
    needed: usize,
) -> Result<BTreeMap<u16, T>, KeyholderError> {
    let start = Instant::now();
    loop {
        let snapshot = read(&*state.inbox.lock().await);
        if snapshot.len() >= needed {
            return Ok(snapshot);
        }
        if start.elapsed() > STEP_TIMEOUT {
            return Err(KeyholderError::Dkg(format!(
                "only {} of {needed} peers answered within {STEP_TIMEOUT:?}",
                snapshot.len()
            )));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Key material of a node that completed the ceremony.
pub struct KeyMaterial {
    pub key_package: KeyPackage,
    pub public_key_package: PublicKeyPackage,
    pub vault: VaultFile,
    pub fvk: FullViewingKey,
    pub sealing: zeek_core::seal::SealingPrivateKey,
}

impl KeyMaterial {
    /// Loads the node's key share, vault file and sealing key.
    pub fn load(config: &KeyholderConfig) -> Result<Self, KeyholderError> {
        let secrets = SecretsDir::new(&config.secrets_dir);
        if !secrets.keyholder_path(config.identifier).exists() {
            return Err(KeyholderError::NoKeyShare);
        }
        let vault = secrets.load_vault()?;
        let key_package = secrets.load_keyholder(config.identifier, &vault)?;
        let public_key_package = vault.public_key_package()?;
        let fvk = vault.orchard_fvk()?;
        let sealing = secrets.load_sealing_key()?.private_key()?;
        Ok(KeyMaterial {
            key_package,
            public_key_package,
            vault,
            fvk,
            sealing,
        })
    }

    /// The identifier of this share.
    pub fn identifier(&self) -> Result<u16, KeyholderError> {
        Ok(frost::identifier_number(self.key_package.identifier())?)
    }

    /// Convenience: the FROST identifier type.
    pub fn frost_identifier(&self) -> Identifier {
        *self.key_package.identifier()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_seed_depends_on_every_package() {
        let mut a = BTreeMap::new();
        a.insert(1u16, vec![1, 2, 3]);
        a.insert(2u16, vec![4, 5]);
        let mut b = a.clone();
        b.insert(2u16, vec![4, 6]);
        let gk = [9u8; 32];
        assert_ne!(transcript_seed(&gk, &a), transcript_seed(&gk, &b));
        assert_ne!(transcript_seed(&gk, &a), transcript_seed(&[8u8; 32], &a));
        assert_eq!(transcript_seed(&gk, &a), transcript_seed(&gk, &a.clone()));
    }

    /// Three in-process participants run the real DKG and derive the same
    /// vault; the result is even-y and signs under the vault's ak.
    #[test]
    fn three_party_dkg_derives_one_vault_and_signs() {
        use rand_core::OsRng;
        let ids: Vec<Identifier> = (1..=3u16)
            .map(|i| frost::identifier(i).expect("id"))
            .collect();
        let mut secrets1 = BTreeMap::new();
        let mut packages1 = BTreeMap::new();
        for id in &ids {
            let (s, p) = dkg::part1(*id, 3, 2, OsRng).expect("part1");
            secrets1.insert(*id, s);
            packages1.insert(*id, p);
        }
        let mut secrets2 = BTreeMap::new();
        let mut packages2: BTreeMap<Identifier, BTreeMap<Identifier, round2::Package>> =
            BTreeMap::new();
        for id in &ids {
            let others: BTreeMap<_, _> = packages1
                .iter()
                .filter(|(k, _)| *k != id)
                .map(|(k, v)| (*k, v.clone()))
                .collect();
            let (s2, out) = dkg::part2(secrets1.remove(id).expect("s1"), &others).expect("part2");
            secrets2.insert(*id, s2);
            for (to, pkg) in out {
                packages2.entry(to).or_default().insert(*id, pkg);
            }
        }
        let mut key_packages = BTreeMap::new();
        let mut pkps = Vec::new();
        for id in &ids {
            let others: BTreeMap<_, _> = packages1
                .iter()
                .filter(|(k, _)| *k != id)
                .map(|(k, v)| (*k, v.clone()))
                .collect();
            let (kp, pkp) = dkg::part3(&secrets2[id], &others, &packages2[id]).expect("part3");
            assert!(kp.has_even_y() && pkp.has_even_y());
            key_packages.insert(*id, kp);
            pkps.push(pkp);
        }
        assert!(pkps.windows(2).all(|w| w[0] == w[1]));
        let group_key = frost::group_key_bytes(&pkps[0]).expect("gk");
        assert_eq!(group_key[31] & 0x80, 0);

        let transcript: BTreeMap<u16, Vec<u8>> = packages1
            .iter()
            .map(|(id, p)| {
                (
                    frost::identifier_number(id).expect("n"),
                    p.serialize().expect("ser"),
                )
            })
            .collect();
        let addresses: Vec<String> = (0..3)
            .map(|_| {
                let fvk = keys::derive_full_viewing_key_from_seed(
                    &group_key,
                    transcript_seed(&group_key, &transcript),
                )
                .expect("fvk");
                keys::encode_address(
                    &keys::orchard_only_address(
                        &keys::unified_full_viewing_key(&fvk).expect("ufvk"),
                    )
                    .expect("addr"),
                )
            })
            .collect();
        assert!(addresses.windows(2).all(|w| w[0] == w[1]));

        // Any two shares sign a rerandomized message that verifies under ak·alpha.
        let alpha = pasta_curves::pallas::Scalar::from(42u64);
        let signers: Vec<&KeyPackage> = [&ids[0], &ids[2]]
            .iter()
            .map(|id| &key_packages[*id])
            .collect();
        let sig = frost::randomized_sign(b"msg", &alpha, &signers, &pkps[0], OsRng).expect("sign");
        let ak = orchard::primitives::redpallas::VerificationKey::<
            orchard::primitives::redpallas::SpendAuth,
        >::try_from(group_key)
        .expect("ak");
        let expected: [u8; 32] = (&ak.randomize(&alpha)).into();
        assert_eq!(expected, sig.randomized_verifying_key);
    }
}
