//! Long-term ed25519 identity of a node (or the coordinator).

use std::fs;
use std::io::Write;
use std::path::Path;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::error::EngineError;

/// Decodes a 32-byte hex public key.
pub fn decode_key(hex_key: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(hex_key).map_err(|e| e.to_string())?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| "key must be 32 bytes".to_owned())
}

/// Identity file schema. The private key is hex; the file is 0600.
#[derive(Serialize, Deserialize)]
struct IdentityFile {
    version: u32,
    public_key: String,
    private_key: String,
}

/// An identity signing key. No `Debug`: the private key must never be printed.
pub struct Identity {
    key: SigningKey,
}

impl Identity {
    /// Generates a fresh identity.
    pub fn generate() -> Self {
        Identity {
            key: SigningKey::generate(&mut rand_core::OsRng),
        }
    }

    /// Loads an identity file.
    pub fn load(path: &Path) -> Result<Self, EngineError> {
        let bytes = fs::read(path).map_err(|e| EngineError::io(path, e))?;
        let file: IdentityFile = serde_json::from_slice(&bytes).map_err(|e| EngineError::Json {
            path: path.to_path_buf(),
            source: e,
        })?;
        let secret = hex::decode(&file.private_key)
            .map_err(|e| EngineError::Config(format!("identity private key hex: {e}")))?;
        let secret = <[u8; 32]>::try_from(secret.as_slice())
            .map_err(|_| EngineError::Config("identity private key must be 32 bytes".into()))?;
        let key = SigningKey::from_bytes(&secret);
        if hex::encode(key.verifying_key().to_bytes()) != file.public_key {
            return Err(EngineError::Config(
                "identity file: public key does not match the private key".into(),
            ));
        }
        Ok(Identity { key })
    }

    /// Writes the identity file (0600); refuses to overwrite unless `force`.
    pub fn save(&self, path: &Path, force: bool) -> Result<(), EngineError> {
        if path.exists() && !force {
            return Err(EngineError::Config(format!(
                "{} exists; pass --force to replace it",
                path.display()
            )));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| EngineError::io(parent, e))?;
            set_mode(parent, 0o700)?;
        }
        let file = IdentityFile {
            version: 1,
            public_key: self.public_key_hex(),
            private_key: hex::encode(self.key.to_bytes()),
        };
        let json = serde_json::to_string_pretty(&file).map_err(|e| EngineError::Json {
            path: path.to_path_buf(),
            source: e,
        })?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut f = options.open(path).map_err(|e| EngineError::io(path, e))?;
        f.write_all(json.as_bytes())
            .and_then(|_| f.sync_all())
            .map_err(|e| EngineError::io(path, e))?;
        Ok(())
    }

    /// The public key, hex.
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.key.verifying_key().to_bytes())
    }

    /// Signs `message`.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.key.sign(message).to_bytes()
    }
}

/// Verifies `signature` over `message` under the hex-encoded public key.
pub fn verify(public_key_hex: &str, message: &[u8], signature: &[u8]) -> Result<(), EngineError> {
    let key = crate::identity::decode_key(public_key_hex)
        .and_then(|bytes| VerifyingKey::from_bytes(&bytes).map_err(|e| e.to_string()))
        .map_err(|e| EngineError::Protocol(format!("bad identity key: {e}")))?;
    let signature = <[u8; 64]>::try_from(signature)
        .map(|bytes| Signature::from_bytes(&bytes))
        .map_err(|_| EngineError::Protocol("signature must be 64 bytes".into()))?;
    key.verify(message, &signature)
        .map_err(|_| EngineError::Protocol("invalid identity signature".into()))
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), EngineError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|e| EngineError::io(path, e))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), EngineError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_and_file_round_trip() {
        let id = Identity::generate();
        let sig = id.sign(b"hello");
        verify(&id.public_key_hex(), b"hello", &sig).expect("verifies");
        assert!(verify(&id.public_key_hex(), b"hellp", &sig).is_err());
        assert!(verify(&Identity::generate().public_key_hex(), b"hello", &sig).is_err());

        let tmp = tempfile::tempdir().expect("tmp");
        let path = tmp.path().join("secrets").join("identity.json");
        id.save(&path, false).expect("save");
        assert!(id.save(&path, false).is_err(), "refuses to overwrite");
        let loaded = Identity::load(&path).expect("load");
        assert_eq!(loaded.public_key_hex(), id.public_key_hex());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).expect("meta").permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
