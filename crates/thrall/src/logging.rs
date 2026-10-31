//! Logging to a daily file in the data folder. Nothing goes to the terminal: stdout is the reply.

use anyhow::Context;
use thrall_core::constants::LOG_FILE_PREFIX;
use thrall_core::{Config, Paths};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

use crate::constants::LOG_LEVEL_ENV_VAR;

/// Start logging. Keep the returned guard alive until exit so the last lines are written.
pub fn init(paths: &Paths, config: &Config) -> anyhow::Result<WorkerGuard> {
    let appender = tracing_appender::rolling::daily(&paths.logs_dir, LOG_FILE_PREFIX);
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let level = std::env::var(LOG_LEVEL_ENV_VAR).unwrap_or_else(|_| config.logging.level.clone());
    let filter = EnvFilter::try_new(&level)
        .with_context(|| format!("`{level}` isn't a log level Thrall understands"))?;
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(filter)
        .try_init()
        .map_err(|e| anyhow::anyhow!("Thrall couldn't start logging: {e}"))?;
    Ok(guard)
}
