//! On-disk key files.
//!
//! The secrets directory holds:
//!
//! * `vault.json` — public data: group verifying key, public key package,
//!   Orchard full viewing key, unified full viewing key and address.
//! * `keyholder-N.json` — keyholder `N`'s FROST key package (which contains
//!   its secret signing share) plus the group key so the file can be matched
//!   to a vault.
//!
//! Files are written with mode `0600` inside a `0700` directory and are never
//! overwritten unless the caller passes `force`. Secret material is stored as
//! hex of the `frost-core` postcard encoding and is deliberately kept out of
//! every `Debug` and `Display` implementation.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use orchard::keys::FullViewingKey;
use serde::{Deserialize, Serialize};
use zcash_keys::keys::UnifiedFullViewingKey;

use zeek_core::seal::{SealingPrivateKey, SealingPublicKey};

use crate::error::VaultError;
use crate::frost::{self, KeyPackage, PublicKeyPackage};
use crate::keys::FVK_LEN;

/// Schema version of the JSON files.
pub const FILE_VERSION: u32 = 1;
/// Name of the public vault file.
pub const VAULT_FILE: &str = "vault.json";
/// Name of the payout sealing key file.
pub const SEALING_FILE: &str = "payout-sealing.json";

/// Public description of the vault. Safe to share.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultFile {
    /// Schema version.
    pub version: u32,
    /// Network name: `test` or `main`.
    pub network: String,
    /// Signing threshold.
    pub threshold: u16,
    /// Number of keyholders.
    pub signers: u16,
    /// FROST group verifying key (`ak`), hex.
    pub group_verifying_key: String,
    /// FROST public key package (postcard encoding), hex.
    pub public_key_package: String,
    /// Raw Orchard full viewing key `ak || nk || rivk`, hex.
    pub orchard_full_viewing_key: String,
    /// Unified full viewing key, bech32m.
    pub unified_full_viewing_key: String,
    /// Default unified address with only an Orchard receiver.
    pub address: String,
    /// Chain height the wallet was told to start scanning from, once known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birthday_height: Option<u32>,
    /// Unified incoming viewing key (external scope only), bech32m. Absent in
    /// files written before it was introduced; derived on demand from the UFVK.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unified_incoming_viewing_key: Option<String>,
}

impl VaultFile {
    /// Builds the vault file from freshly derived material.
    pub fn new(
        network: zeek_core::params::Network,
        threshold: u16,
        signers: u16,
        public_key_package: &PublicKeyPackage,
        fvk: &FullViewingKey,
        ufvk: &UnifiedFullViewingKey,
        address: &str,
    ) -> Result<Self, VaultError> {
        let group_verifying_key = frost::group_key_bytes(public_key_package)?;
        let public_key_package = public_key_package.serialize()?;
        let params = crate::params(network);
        Ok(VaultFile {
            version: FILE_VERSION,
            network: crate::network_name(network).to_owned(),
            threshold,
            signers,
            group_verifying_key: hex::encode(group_verifying_key),
            public_key_package: hex::encode(public_key_package),
            orchard_full_viewing_key: hex::encode(fvk.to_bytes()),
            unified_full_viewing_key: crate::keys::encode_ufvk(ufvk, &params),
            address: address.to_owned(),
            birthday_height: None,
            unified_incoming_viewing_key: Some(crate::keys::encode_uivk(
                &crate::keys::unified_incoming_viewing_key(ufvk),
                &params,
            )),
        })
    }

    /// The vault's network.
    pub fn network(&self) -> Result<zeek_core::params::Network, VaultError> {
        crate::parse_network(&self.network)
    }

    /// The vault's consensus parameters.
    pub fn params(&self) -> Result<crate::Params, VaultError> {
        Ok(crate::params(self.network()?))
    }

    /// The encoded unified incoming viewing key, derived from the UFVK.
    pub fn uivk(&self) -> Result<String, VaultError> {
        Ok(crate::keys::encode_uivk(
            &crate::keys::unified_incoming_viewing_key(&self.ufvk()?),
            &self.params()?,
        ))
    }

    /// Decodes the group verifying key.
    pub fn group_key(&self) -> Result<[u8; 32], VaultError> {
        decode_array::<32>(&self.group_verifying_key, "group_verifying_key")
    }

    /// Decodes the FROST public key package.
    pub fn public_key_package(&self) -> Result<PublicKeyPackage, VaultError> {
        let bytes = decode_hex(&self.public_key_package, "public_key_package")?;
        let package = PublicKeyPackage::deserialize(&bytes)?;
        if frost::group_key_bytes(&package)? != self.group_key()? {
            return Err(VaultError::InvalidGroupKey);
        }
        Ok(package)
    }

    /// Decodes the Orchard full viewing key and checks its `ak` is the group key.
    pub fn orchard_fvk(&self) -> Result<FullViewingKey, VaultError> {
        let raw =
            decode_array::<FVK_LEN>(&self.orchard_full_viewing_key, "orchard_full_viewing_key")?;
        let fvk = FullViewingKey::from_bytes(&raw).ok_or(VaultError::InvalidFullViewingKey)?;
        if crate::keys::ak_bytes(&fvk) != self.group_key()? {
            return Err(VaultError::InvalidFullViewingKey);
        }
        Ok(fvk)
    }

    /// Decodes the unified full viewing key.
    pub fn ufvk(&self) -> Result<UnifiedFullViewingKey, VaultError> {
        UnifiedFullViewingKey::decode(&self.params()?, &self.unified_full_viewing_key)
            .map_err(VaultError::KeyDerivation)
    }
}

/// One keyholder's secret material. No `Debug`: the share must never be printed.
#[derive(Clone, Serialize, Deserialize)]
pub struct KeyholderFile {
    /// Schema version.
    pub version: u32,
    /// 1-based keyholder number.
    pub identifier: u16,
    /// FROST group verifying key this share belongs to, hex.
    pub group_verifying_key: String,
    /// FROST key package including the secret share (postcard encoding), hex.
    key_package: String,
}

impl KeyholderFile {
    /// Builds the keyholder file for `identifier`.
    pub fn new(identifier: u16, key_package: &KeyPackage) -> Result<Self, VaultError> {
        let group_verifying_key = key_package.verifying_key().serialize()?;
        Ok(KeyholderFile {
            version: FILE_VERSION,
            identifier,
            group_verifying_key: hex::encode(group_verifying_key),
            key_package: hex::encode(key_package.serialize()?),
        })
    }

    /// Decodes the key package and checks that it belongs to `vault`.
    pub fn key_package(&self, vault: &VaultFile, path: &Path) -> Result<KeyPackage, VaultError> {
        let bytes = decode_hex(&self.key_package, "key_package")?;
        let package = KeyPackage::deserialize(&bytes)?;
        let own_group = package.verifying_key().serialize()?;
        let claimed_group = decode_hex(&self.group_verifying_key, "group_verifying_key")?;
        if own_group != claimed_group || own_group != vault.group_key()? {
            return Err(VaultError::ForeignKeyholder {
                path: path.to_path_buf(),
            });
        }
        if frost::identifier_number(package.identifier())? != self.identifier {
            return Err(VaultError::ForeignKeyholder {
                path: path.to_path_buf(),
            });
        }
        Ok(package)
    }
}

/// The vault's payout sealing key pair (zeek-core HPKE). The private key is
/// hex in this file only; it is never printed. No `Debug`.
#[derive(Clone, Serialize, Deserialize)]
pub struct SealingKeyFile {
    /// Schema version.
    pub version: u32,
    /// X25519 public key, hex. Public configuration.
    pub public_key: String,
    /// X25519 private key, hex. Secret.
    private_key: String,
}

impl SealingKeyFile {
    /// Builds the file from a fresh key pair.
    pub fn new(private_key: &SealingPrivateKey) -> Self {
        SealingKeyFile {
            version: FILE_VERSION,
            public_key: hex::encode(private_key.public_key().to_bytes()),
            private_key: hex::encode(private_key.to_bytes()),
        }
    }

    /// Decodes the private key and checks it matches the stored public key.
    pub fn private_key(&self) -> Result<SealingPrivateKey, VaultError> {
        let bytes = decode_hex(&self.private_key, "private_key")?;
        let key = SealingPrivateKey::from_bytes(&bytes)
            .map_err(|e| VaultError::KeyDerivation(format!("payout sealing key: {e}")))?;
        if hex::encode(key.public_key().to_bytes()) != self.public_key {
            return Err(VaultError::KeyDerivation(
                "payout sealing file: public key does not match the private key".into(),
            ));
        }
        Ok(key)
    }

    /// Decodes the public key.
    pub fn public_key(&self) -> Result<SealingPublicKey, VaultError> {
        let bytes = decode_hex(&self.public_key, "public_key")?;
        SealingPublicKey::from_bytes(&bytes)
            .map_err(|e| VaultError::KeyDerivation(format!("payout sealing key: {e}")))
    }
}

/// Path helpers for the secrets directory.
#[derive(Clone, Debug)]
pub struct SecretsDir(PathBuf);

impl SecretsDir {
    /// Wraps a directory path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        SecretsDir(path.into())
    }

    /// The directory itself.
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Path of the public vault file.
    pub fn vault_path(&self) -> PathBuf {
        self.0.join(VAULT_FILE)
    }

    /// Path of keyholder `n`'s file.
    pub fn keyholder_path(&self, n: u16) -> PathBuf {
        self.0.join(format!("keyholder-{n}.json"))
    }

    /// Path of the payout sealing key file.
    pub fn sealing_path(&self) -> PathBuf {
        self.0.join(SEALING_FILE)
    }

    /// Writes the payout sealing key file; refuses to overwrite unless `force`.
    pub fn write_sealing_key(&self, file: &SealingKeyFile, force: bool) -> Result<(), VaultError> {
        let path = self.sealing_path();
        if path.exists() && !force {
            return Err(VaultError::KeyFileExists(path));
        }
        self.ensure_dir()?;
        write_json(&path, file)
    }

    /// Loads the payout sealing key file.
    pub fn load_sealing_key(&self) -> Result<SealingKeyFile, VaultError> {
        let path = self.sealing_path();
        if !path.exists() {
            return Err(VaultError::SealingKeyNotFound(path));
        }
        read_json(&path)
    }

    /// Returns an error if any file the vault would write already exists.
    pub fn check_absent(&self, signers: u16) -> Result<(), VaultError> {
        let mut paths = vec![self.vault_path()];
        paths.extend((1..=signers).map(|n| self.keyholder_path(n)));
        for path in paths {
            if path.exists() {
                return Err(VaultError::KeyFileExists(path));
            }
        }
        Ok(())
    }

    /// Writes the vault file, creating the directory if needed.
    pub fn write_vault(&self, vault: &VaultFile) -> Result<(), VaultError> {
        self.ensure_dir()?;
        write_json(&self.vault_path(), vault)
    }

    /// Writes keyholder `n`'s file.
    pub fn write_keyholder(&self, n: u16, file: &KeyholderFile) -> Result<(), VaultError> {
        self.ensure_dir()?;
        write_json(&self.keyholder_path(n), file)
    }

    /// Loads the vault file.
    pub fn load_vault(&self) -> Result<VaultFile, VaultError> {
        let path = self.vault_path();
        if !path.exists() {
            return Err(VaultError::VaultNotFound(path));
        }
        let vault: VaultFile = read_json(&path)?;
        vault.network()?;
        Ok(vault)
    }

    /// Loads keyholder `n`'s key package and checks it belongs to `vault`.
    pub fn load_keyholder(&self, n: u16, vault: &VaultFile) -> Result<KeyPackage, VaultError> {
        if n == 0 || n > vault.signers {
            return Err(VaultError::IdentifierOutOfRange(n, vault.signers));
        }
        let path = self.keyholder_path(n);
        if !path.exists() {
            return Err(VaultError::KeyholderNotFound(path));
        }
        let file: KeyholderFile = read_json(&path)?;
        file.key_package(vault, &path)
    }

    fn ensure_dir(&self) -> Result<(), VaultError> {
        fs::create_dir_all(&self.0).map_err(|e| VaultError::io(&self.0, e))?;
        set_mode(&self.0, 0o700)
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), VaultError> {
    let json = serde_json::to_string_pretty(value).map_err(|e| VaultError::Json {
        path: path.to_path_buf(),
        source: e,
    })?;
    let tmp = path.with_extension("json.tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(|e| VaultError::io(&tmp, e))?;
    file.write_all(json.as_bytes())
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all())
        .map_err(|e| VaultError::io(&tmp, e))?;
    fs::rename(&tmp, path).map_err(|e| VaultError::io(path, e))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, VaultError> {
    let bytes = fs::read(path).map_err(|e| VaultError::io(path, e))?;
    serde_json::from_slice(&bytes).map_err(|e| VaultError::Json {
        path: path.to_path_buf(),
        source: e,
    })
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), VaultError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|e| VaultError::io(path, e))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), VaultError> {
    Ok(())
}

fn decode_hex(value: &str, field: &'static str) -> Result<Vec<u8>, VaultError> {
    hex::decode(value).map_err(|source| VaultError::Hex { field, source })
}

fn decode_array<const N: usize>(value: &str, field: &'static str) -> Result<[u8; N], VaultError> {
    let bytes = decode_hex(value, field)?;
    <[u8; N]>::try_from(bytes.as_slice()).map_err(|_| VaultError::BadLength {
        field,
        expected: N,
        actual: bytes.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{frost, keys};
    use rand_core::OsRng;

    fn make_vault(dir: &SecretsDir) -> (VaultFile, frost::KeygenOutput) {
        let out = frost::dealer_keygen(2, 3, OsRng).expect("keygen");
        let group = frost::group_key_bytes(&out.public_key_package).expect("group");
        let fvk = keys::derive_full_viewing_key(&group, OsRng).expect("fvk");
        let ufvk = keys::unified_full_viewing_key(&fvk).expect("ufvk");
        let address = keys::encode_address(
            &keys::orchard_only_address(&ufvk).expect("address"),
            &crate::Params::TestNetwork,
        );
        let vault = VaultFile::new(
            zeek_core::params::Network::Test,
            2,
            3,
            &out.public_key_package,
            &fvk,
            &ufvk,
            &address,
        )
        .expect("vault");
        dir.write_vault(&vault).expect("write vault");
        for (id, kp) in &out.key_packages {
            let n = frost::identifier_number(id).expect("n");
            dir.write_keyholder(n, &KeyholderFile::new(n, kp).expect("file"))
                .expect("write keyholder");
        }
        (vault, out)
    }

    #[test]
    fn round_trip_and_membership_checks() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = SecretsDir::new(tmp.path().join("secrets"));
        let (vault, out) = make_vault(&dir);

        let loaded = dir.load_vault().expect("load vault");
        assert_eq!(loaded.address, vault.address);
        assert_eq!(
            loaded.public_key_package().expect("pkp"),
            out.public_key_package
        );
        assert_eq!(
            keys::ak_bytes(&loaded.orchard_fvk().expect("fvk")),
            loaded.group_key().expect("group")
        );
        loaded.ufvk().expect("ufvk decodes");

        for n in 1..=3u16 {
            let kp = dir.load_keyholder(n, &loaded).expect("keyholder");
            assert_eq!(&kp, &out.key_packages[&frost::identifier(n).expect("id")]);
        }
        assert!(matches!(
            dir.load_keyholder(4, &loaded),
            Err(VaultError::IdentifierOutOfRange(4, 3))
        ));
        assert!(matches!(
            dir.load_keyholder(0, &loaded),
            Err(VaultError::IdentifierOutOfRange(0, 3))
        ));

        // A keyholder file from a different vault is rejected.
        let other_tmp = tempfile::tempdir().expect("tempdir");
        let other_dir = SecretsDir::new(other_tmp.path());
        make_vault(&other_dir);
        let other_vault = other_dir.load_vault().expect("other vault");
        assert!(matches!(
            dir.load_keyholder(1, &other_vault),
            Err(VaultError::ForeignKeyholder { .. })
        ));
    }

    #[test]
    fn refuses_to_overwrite_without_force() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = SecretsDir::new(tmp.path());
        dir.check_absent(3).expect("nothing there yet");
        make_vault(&dir);
        assert!(matches!(
            dir.check_absent(3),
            Err(VaultError::KeyFileExists(_))
        ));
    }

    #[test]
    fn sealing_key_file_round_trips_and_refuses_overwrite() {
        use rand_09::rngs::StdRng;
        use rand_09::SeedableRng;
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = SecretsDir::new(tmp.path().join("secrets"));
        assert!(matches!(
            dir.load_sealing_key(),
            Err(VaultError::SealingKeyNotFound(_))
        ));
        let (sk, pk) = zeek_core::seal::generate_sealing_keypair(&mut StdRng::seed_from_u64(1));
        let file = SealingKeyFile::new(&sk);
        dir.write_sealing_key(&file, false).expect("write");
        assert!(matches!(
            dir.write_sealing_key(&file, false),
            Err(VaultError::KeyFileExists(_))
        ));
        dir.write_sealing_key(&file, true).expect("force");
        let loaded = dir.load_sealing_key().expect("load");
        assert_eq!(loaded.public_key().expect("pk"), pk);
        assert_eq!(loaded.private_key().expect("sk").to_bytes(), sk.to_bytes());
        let text = fs::read_to_string(dir.sealing_path()).expect("read");
        assert!(
            text.contains(&hex::encode(sk.to_bytes())),
            "private key is stored"
        );
    }

    #[cfg(unix)]
    #[test]
    fn key_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = SecretsDir::new(tmp.path().join("secrets"));
        make_vault(&dir);
        let mode = fs::metadata(dir.keyholder_path(1))
            .expect("meta")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let dir_mode = fs::metadata(dir.path()).expect("meta").permissions().mode();
        assert_eq!(dir_mode & 0o777, 0o700);
    }

    #[test]
    fn vault_file_contains_no_secret_share() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = SecretsDir::new(tmp.path());
        let (_, out) = make_vault(&dir);
        let vault_text = fs::read_to_string(dir.vault_path()).expect("read vault");
        for kp in out.key_packages.values() {
            let share_hex = hex::encode(kp.signing_share().serialize());
            assert!(
                !vault_text.contains(&share_hex),
                "vault.json leaks a signing share"
            );
        }
        let keyholder_text = fs::read_to_string(dir.keyholder_path(2)).expect("read");
        assert!(keyholder_text.contains("\"identifier\": 2"));
    }
}
