//! A player's single-key shielded testnet wallet.
//!
//! Each player lives under `<secrets>/players/<name>/`: a 32-byte seed in
//! `seed.json` (mode 0600) and a `zcash_client_sqlite` wallet database beside
//! it. The spending key is derived from the seed on every run and never
//! written anywhere else. Addresses and seeds are printed only by explicit
//! commands, never logged.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use secrecy::{ExposeSecret, SecretVec};
use serde::{Deserialize, Serialize};
use zcash_client_backend::data_api::{AccountBirthday, WalletWrite};
use zcash_client_sqlite::AccountUuid;
use zcash_keys::keys::{UnifiedAddressRequest, UnifiedSpendingKey};
use zcash_protocol::consensus::BlockHeight;
use zeek_vault::lightwalletd::{self, Client};
use zeek_vault::wallet::{self, Db};
use zeek_vault::VaultError;
use zeek_vault::NETWORK;

/// Errors of the player tool.
#[derive(Debug, thiserror::Error)]
pub enum DigError {
    #[error("player name `{0}` is invalid: use letters, digits, `-` and `_` only")]
    InvalidName(String),
    #[error("player `{0}` already exists; choose another name")]
    PlayerExists(String),
    #[error("player `{0}` does not exist; run `zeek-dig init {0}` first")]
    PlayerMissing(String),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("key derivation failed: {0}")]
    Keys(String),
    #[error(transparent)]
    Vault(#[from] VaultError),
    #[error("engine request failed: {0}")]
    Engine(String),
    #[error("engine returned an unusable response: {0}")]
    EngineResponse(String),
    #[error("round {round} closes in {blocks_remaining} blocks; refusing to dig with fewer than {margin} blocks of margin")]
    ClosingTooSoon {
        round: u32,
        blocks_remaining: u32,
        margin: u32,
    },
    #[error("game error: {0}")]
    Game(String),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> DigError + '_ {
    move |source| DigError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Seed file schema.
#[derive(Serialize, Deserialize)]
struct SeedFile {
    version: u32,
    /// 32 random bytes, hex.
    seed: String,
}

/// A player's on-disk wallet.
pub struct Player {
    name: String,
    dir: PathBuf,
}

impl Player {
    /// Validates `name` and locates its directory.
    pub fn new(secrets_dir: &Path, name: &str) -> Result<Self, DigError> {
        let valid = !name.is_empty()
            && name.len() <= 32
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !valid {
            return Err(DigError::InvalidName(name.to_owned()));
        }
        Ok(Player {
            name: name.to_owned(),
            dir: secrets_dir.join("players").join(name),
        })
    }

    /// The player's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    fn seed_path(&self) -> PathBuf {
        self.dir.join("seed.json")
    }

    /// Whether the player has been initialised.
    pub fn exists(&self) -> bool {
        self.seed_path().exists()
    }

    /// Creates the player directory and a fresh random seed.
    pub fn init(&self) -> Result<(), DigError> {
        if self.exists() {
            return Err(DigError::PlayerExists(self.name.clone()));
        }
        fs::create_dir_all(&self.dir).map_err(io(&self.dir))?;
        set_mode(&self.dir, 0o700)?;
        let mut seed = [0u8; 32];
        rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut seed);
        let file = SeedFile {
            version: 1,
            seed: hex::encode(seed),
        };
        let path = self.seed_path();
        let json = serde_json::to_string_pretty(&file).map_err(|e| DigError::Json {
            path: path.clone(),
            source: e,
        })?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut f = options.open(&path).map_err(io(&path))?;
        f.write_all(json.as_bytes())
            .and_then(|_| f.sync_all())
            .map_err(io(&path))?;
        Ok(())
    }

    fn seed(&self) -> Result<SecretVec<u8>, DigError> {
        if !self.exists() {
            return Err(DigError::PlayerMissing(self.name.clone()));
        }
        let path = self.seed_path();
        let bytes = fs::read(&path).map_err(io(&path))?;
        let file: SeedFile = serde_json::from_slice(&bytes).map_err(|e| DigError::Json {
            path: path.clone(),
            source: e,
        })?;
        let seed = hex::decode(file.seed).map_err(|e| DigError::Keys(format!("seed hex: {e}")))?;
        if seed.len() != 32 {
            return Err(DigError::Keys("seed must be 32 bytes".into()));
        }
        Ok(SecretVec::new(seed))
    }

    /// The player's unified spending key (account 0).
    pub fn spending_key(&self) -> Result<UnifiedSpendingKey, DigError> {
        let seed = self.seed()?;
        UnifiedSpendingKey::from_seed(&NETWORK, seed.expose_secret(), zip32_account())
            .map_err(|e| DigError::Keys(format!("{e:?}")))
    }

    /// The player's default unified address (Orchard receiver only).
    pub fn address(&self) -> Result<String, DigError> {
        let ufvk = self.spending_key()?.to_unified_full_viewing_key();
        let (address, _) = ufvk
            .default_address(UnifiedAddressRequest::ORCHARD)
            .map_err(|e| DigError::Keys(format!("default address: {e}")))?;
        Ok(address.encode(&NETWORK))
    }

    /// Opens the player's wallet database.
    pub fn open_db(&self) -> Result<Db, DigError> {
        if !self.exists() {
            return Err(DigError::PlayerMissing(self.name.clone()));
        }
        Ok(wallet::open(&self.dir)?)
    }

    /// Ensures the wallet has the player's account, creating it with a
    /// birthday at the current chain tip on first use.
    pub async fn ensure_account(
        &self,
        db: &mut Db,
        client: &mut Client,
    ) -> Result<AccountUuid, DigError> {
        let usk = self.spending_key()?;
        let ufvk = usk.to_unified_full_viewing_key();
        if let Some(id) = wallet::find_account(db, &ufvk)? {
            return Ok(id);
        }
        let tip = lightwalletd::latest_height(client).await?;
        let prior: BlockHeight = tip.saturating_sub(1);
        let tree_state = lightwalletd::tree_state(client, prior).await?;
        let birthday = AccountBirthday::from_treestate(tree_state, None)
            .map_err(|e| DigError::Keys(format!("birthday tree state: {e:?}")))?;
        let seed = self.seed()?;
        let (id, _) = db
            .create_account(&self.name, &seed, &birthday, None)
            .map_err(|e| VaultError::Wallet(e.to_string()))?;
        Ok(id)
    }

    /// Looks up the account without creating it.
    pub fn find_account(&self, db: &Db) -> Result<Option<AccountUuid>, DigError> {
        let ufvk = self.spending_key()?.to_unified_full_viewing_key();
        Ok(wallet::find_account(db, &ufvk)?)
    }
}

fn zip32_account() -> zip32::AccountId {
    zip32::AccountId::ZERO
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), DigError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(io(path))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), DigError> {
    Ok(())
}

/// Summarises the player's balance without printing addresses.
pub fn balance_lines(report: &wallet::BalanceReport) -> Vec<String> {
    let mut lines = vec![
        format!(
            "chain tip {} (scanned {})",
            report.chain_tip, report.fully_scanned
        ),
        format!(
            "ironwood spendable {} ZEC, pending {} ZEC, total {} ZEC",
            zeek_vault::send::format_zec(report.ironwood.spendable),
            zeek_vault::send::format_zec(report.ironwood.pending),
            zeek_vault::send::format_zec(report.ironwood.total)
        ),
    ];
    for note in &report.notes {
        lines.push(format!(
            "  [{}] {}:{} {} ZEC at {} ({}){}",
            wallet::pool_name(note.pool),
            note.txid,
            note.output_index,
            zeek_vault::send::format_zec(note.value),
            note.mined_height
                .map(|h| h.to_string())
                .unwrap_or_else(|| "unmined".into()),
            note.confirmations
                .map(|c| format!("{c} confirmations"))
                .unwrap_or_else(|| "unmined".into()),
            if note.spendable { " spendable" } else { "" }
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_creates_a_private_seed_and_refuses_twice() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let player = Player::new(tmp.path(), "alice").expect("name");
        assert!(!player.exists());
        player.init().expect("init");
        assert!(player.exists());
        assert!(matches!(player.init(), Err(DigError::PlayerExists(_))));
        let address = player.address().expect("address");
        assert!(address.starts_with("utest1"));
        assert_eq!(
            player.address().expect("again"),
            address,
            "derivation is deterministic"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(player.seed_path())
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(matches!(
            Player::new(tmp.path(), "../x"),
            Err(DigError::InvalidName(_))
        ));
        assert!(matches!(
            Player::new(tmp.path(), "nobody")
                .expect("name")
                .spending_key(),
            Err(DigError::PlayerMissing(_))
        ));
    }
}
