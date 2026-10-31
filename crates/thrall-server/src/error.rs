//! Server errors: the engine couldn't start listening, or couldn't write its files.

use std::path::PathBuf;

/// Why the server couldn't start.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// A file in the data folder couldn't be read or written.
    #[error("Couldn't use {path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        source: std::io::Error,
    },

    /// The port is taken (usually by another Thrall engine).
    #[error(
        "Port {port} is already in use. Another Thrall engine may be running; if not, close the \
         program using that port or choose another port."
    )]
    PortInUse {
        /// The port.
        port: u16,
    },

    /// Listening failed for another reason.
    #[error("Couldn't start listening: {0}")]
    Listen(std::io::Error),

    /// The system wouldn't supply random bytes for the token.
    #[error("Couldn't create an access token: {0}")]
    Random(String),
}
