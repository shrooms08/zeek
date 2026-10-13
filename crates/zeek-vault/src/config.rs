//! Runtime configuration, read from environment variables.

use std::env;
use std::path::PathBuf;

use crate::error::VaultError;

/// Environment variable naming the `lightwalletd` gRPC endpoint.
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
    ///
    /// The lightwalletd URL is optional here so that offline commands
    /// (`keygen`, `address`) work without it; network commands call
    /// [`Config::lightwalletd_url`], which reports a clear error if unset.
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

    /// The lightwalletd endpoint, or an error naming the variable to set.
    pub fn lightwalletd_url(&self) -> Result<&str, VaultError> {
        self.lightwalletd_url.as_deref().ok_or_else(|| {
            VaultError::Config(format!(
                "{LIGHTWALLETD_URL_VAR} is not set; copy .env.example to .env or export it"
            ))
        })
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
