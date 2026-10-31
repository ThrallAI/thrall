//! Shared building blocks: where files live, the config file, ids and time, errors, and provider
//! keys in the OS keychain.

pub mod config;
pub mod constants;
pub mod discovery;
pub mod error;
pub mod ids;
pub mod paths;
pub mod process;
pub mod secrets;
pub mod startup;

pub use config::{Config, ProviderKind};
pub use discovery::EngineFile;
pub use error::CoreError;
pub use paths::Paths;
