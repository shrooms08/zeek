//! Log output for the engine and the keyholders.
//!
//! With `ZEEK_LOG_DIR` set, logs go to `<dir>/<name>.<date>.log`, rotated
//! daily and kept for [`KEEP_DAYS`] days (launchd keeps a service's stdout
//! open, so files it owns could not be rotated). Without it they go to
//! stderr. `RUST_LOG` filters as usual (default `info`).

use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::EnvFilter;

use crate::error::EngineError;

/// Environment variable naming the log directory.
pub const LOG_DIR_VAR: &str = "ZEEK_LOG_DIR";
/// Daily files kept.
pub const KEEP_DAYS: usize = 14;

fn filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
}

/// Installs the global subscriber for the process `name`. Keep the returned
/// guard alive for the whole process: it flushes the file writer on drop.
pub fn init(name: &str) -> Result<Option<WorkerGuard>, EngineError> {
    let Some(dir) = std::env::var_os(LOG_DIR_VAR).map(PathBuf::from) else {
        tracing_subscriber::fmt()
            .with_env_filter(filter())
            .with_target(false)
            .with_writer(std::io::stderr)
            .init();
        return Ok(None);
    };
    std::fs::create_dir_all(&dir).map_err(|e| EngineError::io(&dir, e))?;
    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix(name)
        .filename_suffix("log")
        .max_log_files(KEEP_DAYS)
        .build(&dir)
        .map_err(|e| EngineError::Config(format!("log directory {}: {e}", dir.display())))?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .with_env_filter(filter())
        .with_target(false)
        .with_ansi(false)
        .with_writer(writer)
        .init();
    Ok(Some(guard))
}
