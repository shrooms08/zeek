//! Node configuration (committed JSON, no secrets) and secret file locations.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::KeyholderError;

/// Participant identifier of the coordinator in signed envelopes.
pub const COORDINATOR_ID: u16 = 0;

/// A peer keyholder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    /// FROST identifier, `1..=signers`.
    pub identifier: u16,
    /// Base URL of the peer's API.
    pub url: String,
    /// Pinned ed25519 identity public key, hex.
    pub identity: String,
}

/// Public configuration of one node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyholderConfig {
    /// This node's FROST identifier.
    pub identifier: u16,
    /// Signing threshold.
    pub threshold: u16,
    /// Number of keyholders.
    pub signers: u16,
    /// API bind address.
    pub bind: String,
    /// Directory for this node's databases.
    pub data_dir: PathBuf,
    /// Directory for this node's secrets (0700).
    pub secrets_dir: PathBuf,
    /// The public game config shared with the engine.
    pub game_config: PathBuf,
    /// Ordered lightwalletd endpoints; the first that answers is used.
    pub lightwalletd: Vec<String>,
    /// The other keyholders.
    pub peers: Vec<Peer>,
    /// Pinned ed25519 identity public key of the coordinator (engine), hex.
    pub coordinator_identity: String,
    /// Seconds between chain polls.
    pub poll_secs: u64,
}

impl KeyholderConfig {
    /// Reads and validates a config file.
    pub fn load(path: &Path) -> Result<Self, KeyholderError> {
        let bytes = fs::read(path).map_err(|e| KeyholderError::io(path, e))?;
        let config: KeyholderConfig =
            serde_json::from_slice(&bytes).map_err(|e| KeyholderError::Json {
                path: path.to_path_buf(),
                source: e,
            })?;
        config.validate()?;
        Ok(config)
    }

    /// Checks the config is consistent.
    pub fn validate(&self) -> Result<(), KeyholderError> {
        let bad = |m: String| KeyholderError::Config(m);
        if self.threshold < 2 || self.threshold > self.signers {
            return Err(bad(format!(
                "threshold {} must be between 2 and signers {}",
                self.threshold, self.signers
            )));
        }
        if self.identifier == 0 || self.identifier > self.signers {
            return Err(bad(format!("identifier {} out of range", self.identifier)));
        }
        if self.peers.len() != usize::from(self.signers) - 1 {
            return Err(bad(format!(
                "expected {} peers, got {}",
                self.signers - 1,
                self.peers.len()
            )));
        }
        for peer in &self.peers {
            if peer.identifier == self.identifier
                || peer.identifier == 0
                || peer.identifier > self.signers
            {
                return Err(bad(format!(
                    "peer identifier {} is invalid",
                    peer.identifier
                )));
            }
            decode_key(&peer.identity)
                .map_err(|e| bad(format!("peer {} identity: {e}", peer.identifier)))?;
        }
        let mut ids: Vec<u16> = self.peers.iter().map(|p| p.identifier).collect();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() != self.peers.len() {
            return Err(bad("duplicate peer identifiers".into()));
        }
        decode_key(&self.coordinator_identity)
            .map_err(|e| bad(format!("coordinator identity: {e}")))?;
        if self.lightwalletd.is_empty() {
            return Err(bad("at least one lightwalletd endpoint is required".into()));
        }
        if self.poll_secs == 0 {
            return Err(bad("poll_secs must be positive".into()));
        }
        Ok(())
    }

    /// The peer with `identifier`.
    pub fn peer(&self, identifier: u16) -> Option<&Peer> {
        self.peers.iter().find(|p| p.identifier == identifier)
    }

    /// Whether this node leads the ceremony (lowest identifier).
    pub fn is_lead(&self) -> bool {
        self.peers.iter().all(|p| p.identifier > self.identifier)
    }

    /// Path of the identity key file.
    pub fn identity_path(&self) -> PathBuf {
        self.secrets_dir.join("identity.json")
    }

    /// Path of this node's engine-style chain database.
    pub fn engine_db_path(&self) -> PathBuf {
        self.data_dir.join("chain.sqlite")
    }

    /// Path of this node's own records (signed rounds).
    pub fn node_db_path(&self) -> PathBuf {
        self.data_dir.join("keyholder.sqlite")
    }
}

pub use zeek_engine::identity::decode_key;

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample(identifier: u16) -> KeyholderConfig {
        let peer = |i: u16| Peer {
            identifier: i,
            url: format!("http://127.0.0.1:910{i}"),
            identity: "11".repeat(32),
        };
        KeyholderConfig {
            identifier,
            threshold: 2,
            signers: 3,
            bind: format!("127.0.0.1:910{identifier}"),
            data_dir: PathBuf::from("data"),
            secrets_dir: PathBuf::from(".secrets"),
            game_config: PathBuf::from("config/game.testnet.json"),
            lightwalletd: vec!["https://testnet.zec.rocks:443".into()],
            peers: (1..=3).filter(|i| *i != identifier).map(peer).collect(),
            coordinator_identity: "22".repeat(32),
            poll_secs: 10,
        }
    }

    #[test]
    fn validation_and_lead() {
        let c = sample(1);
        c.validate().expect("valid");
        assert!(c.is_lead());
        assert!(!sample(2).is_lead());
        let mut bad = sample(2);
        bad.peers[0].identifier = 2;
        assert!(bad.validate().is_err());
        let mut bad = sample(3);
        bad.coordinator_identity = "zz".into();
        assert!(bad.validate().is_err());
        let mut bad = sample(1);
        bad.lightwalletd.clear();
        assert!(bad.validate().is_err());
    }
}
