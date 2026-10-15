//! Runtime configuration, read from environment variables.

use std::env;
use std::path::PathBuf;

use crate::error::VaultError;

/// Environment variable naming the `lightwalletd` gRPC endpoints
/// (comma-separated; default: the verified public testnet servers).
pub const LIGHTWALLETD_URL_VAR: &str = "ZEEK_LIGHTWALLETD_URL";
/// Environment variable overriding where key material is stored.
pub const SECRETS_DIR_VAR: &str = "ZEEK_SECRETS_DIR";
/// Environment variable overriding where the wallet database is stored.
pub const DATA_DIR_VAR: &str = "ZEEK_DATA_DIR";

/// Resolved configuration.
#[derive(Clone, Debug)]
pub struct Config {
    lightwalletd_url: Option<String>,
    secrets_dir: PathBuf,
    data_dir: PathBuf,
}

impl Config {
    /// Reads configuration from the process environment.
    pub fn from_env() -> Result<Self, VaultError> {
        let lightwalletd_url = optional_var(LIGHTWALLETD_URL_VAR)?;
        let secrets_dir = optional_var(SECRETS_DIR_VAR)?
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".secrets"));
        let data_dir = optional_var(DATA_DIR_VAR)?
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data"));
        Ok(Config {
            lightwalletd_url,
            secrets_dir,
            data_dir,
        })
    }

    /// The testnet `lightwalletd` servers to use.
    pub fn lightwalletd_servers(&self) -> Vec<String> {
        match &self.lightwalletd_url {
            Some(list) => crate::pool::parse_servers(list),
            None => crate::pool::default_servers(zeek_core::params::Network::Test),
        }
    }

    /// A pool over [`Config::lightwalletd_servers`].
    pub fn pool(&self) -> Result<crate::pool::Pool, VaultError> {
        crate::pool::Pool::new(
            zeek_core::params::Network::Test,
            self.lightwalletd_servers(),
            0,
        )
    }

    /// Connects to the first server of the pool that answers.
    pub async fn connect(&self) -> Result<crate::lightwalletd::Client, VaultError> {
        Ok(self.pool()?.client().await?.1)
    }

    /// Directory holding keyholder files and the public vault file.
    pub fn secrets_dir(&self) -> &PathBuf {
        &self.secrets_dir
    }

    /// Directory holding the wallet database.
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }
}

fn optional_var(name: &str) -> Result<Option<String>, VaultError> {
    match env::var(name) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => {
            Err(VaultError::Config(format!("{name} contains invalid UTF-8")))
        }
    }
}
