//! Errors from `thrall-core`. Every variant renders as a sentence a non-technical user can act on.

use std::path::PathBuf;

/// Something went wrong finding folders, reading config or using the keychain.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// The OS didn't tell us where user data belongs.
    #[error(
        "Thrall couldn't find a folder for its data. Set the THRALL_HOME environment variable to a folder you own."
    )]
    NoDataFolder,

    /// Reading or writing a file failed.
    #[error("Thrall couldn't use {path}: {source}")]
    Io {
        /// The file or folder involved.
        path: PathBuf,
        /// What the OS reported.
        source: std::io::Error,
    },

    /// The config file isn't valid.
    #[error("Your config file {path} has a problem: {message}")]
    Config {
        /// The config file.
        path: PathBuf,
        /// What's wrong, from the parser.
        message: String,
    },

    /// A change from the settings screen was refused; nothing was written.
    #[error("Not saved: {0}")]
    SettingRefused(String),

    /// The OS keychain refused or failed.
    #[error("Thrall couldn't use the system keychain: {0}")]
    Keychain(String),

    /// The system refused to change whether Thrall starts at sign-in.
    #[error("Thrall couldn't change whether it starts when you sign in: {0}")]
    Startup(String),
}
