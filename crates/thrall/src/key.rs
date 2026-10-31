//! `thrall key`: keys in the OS keychain — the AI provider's, and any others your config refers to
//! as `"keychain:<name>"` (MCP servers' keys, for example). Keys never go in a file.

use std::collections::BTreeSet;
use std::io::{BufRead, IsTerminal};
use std::process::ExitCode;

use anyhow::bail;
use thrall_core::secrets;
use thrall_core::{Config, ProviderKind};

use crate::constants::SERVICE_KEYS;

/// The key to use for `provider`: the environment variable if set, otherwise the keychain.
pub fn resolve(provider: ProviderKind) -> anyhow::Result<Option<String>> {
    if let Ok(key) = std::env::var(provider.key_env_var())
        && !key.trim().is_empty()
    {
        return Ok(Some(key.trim().to_owned()));
    }
    Ok(secrets::get_key(provider)?)
}

/// `thrall key set <name>`.
pub fn set(name: &str) -> anyhow::Result<ExitCode> {
    let key = if std::io::stdin().is_terminal() {
        rpassword::prompt_password(format!("Paste your {name} key (it won't be shown): "))?
    } else {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        line
    };
    let key = key.trim();
    if key.is_empty() {
        bail!("No key was entered, so nothing was saved.");
    }
    if key.chars().any(char::is_whitespace) {
        bail!("That doesn't look like a key (it has spaces in it), so nothing was saved.");
    }
    secrets::set_named(name, key)?;
    println!(
        "Saved. Thrall will use {} for {name}.",
        secrets::preview(key)
    );
    Ok(ExitCode::SUCCESS)
}

/// `thrall key remove <name>`.
pub fn remove(name: &str) -> anyhow::Result<ExitCode> {
    if secrets::delete_named(name)? {
        println!("Removed the {name} key.");
    } else {
        println!("There was no {name} key saved.");
    }
    Ok(ExitCode::SUCCESS)
}

/// `thrall key status`: the provider keys, then every key the config refers to.
pub fn status(config: &Config) -> anyhow::Result<ExitCode> {
    for &provider in ProviderKind::ALL {
        let name = provider.as_str();
        let env_var = provider.key_env_var();
        match std::env::var(env_var) {
            Ok(key) if !key.trim().is_empty() => println!(
                "{name}: {} (from the {env_var} environment variable)",
                secrets::preview(&key)
            ),
            _ => match secrets::get_key(provider)? {
                Some(key) => println!("{name}: {} (system keychain)", secrets::preview(&key)),
                None => println!("{name}: not set. Run `thrall key set {name}` to add one."),
            },
        }
    }
    let wanted: BTreeSet<&str> = config
        .mcp
        .servers
        .values()
        .flat_map(|s| s.env.values())
        .filter_map(|v| secrets::keychain_ref(v))
        .collect();
    for name in wanted {
        match secrets::get_named(name)? {
            Some(key) => println!(
                "{name}: {} (system keychain, used by MCP)",
                secrets::preview(&key)
            ),
            None => println!(
                "{name}: not set, but an MCP server needs it. Run `thrall key set {name}`."
            ),
        }
    }
    // Keys for services Thrall's own tools use.
    for (name, used_for) in SERVICE_KEYS {
        match secrets::get_named(name)? {
            Some(key) => println!(
                "{name}: {} (system keychain, used for {used_for})",
                secrets::preview(&key)
            ),
            None => println!(
                "{name}: not set (needed for {used_for}). Run `thrall key set {name}` to add one."
            ),
        }
    }
    Ok(ExitCode::SUCCESS)
}
