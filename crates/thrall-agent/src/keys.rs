//! Where Thrall finds keys for services other than its AI provider (fal.ai for video). The
//! system one reads the OS keychain; tests give their own.

/// Looks up a stored key by name.
pub trait KeyStore: Send + Sync {
    /// The key called `name`, if one is stored.
    fn get(&self, name: &str) -> Option<String>;
}

/// Keys in the OS keychain (`thrall key set <name>`).
pub struct SystemKeys;

impl KeyStore for SystemKeys {
    fn get(&self, name: &str) -> Option<String> {
        thrall_core::secrets::get_named(name).ok().flatten()
    }
}
