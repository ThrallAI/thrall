//! Provider keys in the OS keychain (Windows Credential Manager, macOS Keychain, Secret Service).
//! Keys are never written to a file, a log or the database.

use crate::config::ProviderKind;
use crate::constants::{
    KEY_PREVIEW_HEAD, KEY_PREVIEW_TAIL, KEYCHAIN_REF_PREFIX, KEYCHAIN_SERVICE,
    LONG_SECRET_CHUNK_CHARS, LONG_SECRET_MAX_PARTS,
};
use crate::error::CoreError;

fn entry(name: &str) -> Result<keyring::Entry, CoreError> {
    keyring::Entry::new(KEYCHAIN_SERVICE, name).map_err(|e| CoreError::Keychain(e.to_string()))
}

/// Whether `name` is usable as a key name: lower-case letters, digits, `-` and `_`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// A stored key by name (a provider like `openrouter`, or any other, like `youtube`).
pub fn get_named(name: &str) -> Result<Option<String>, CoreError> {
    match entry(name)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(CoreError::Keychain(e.to_string())),
    }
}

/// A pasted key, cleaned: a whole `.env` line (`FAL_KEY="abc"`, `export X=abc`) becomes just the
/// key, and surrounding spaces and quotes go.
pub fn clean_pasted(raw: &str) -> String {
    let mut key = raw.trim();
    key = key.strip_prefix("export ").map_or(key, str::trim);
    if let Some((name, value)) = key.split_once('=')
        && !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        key = value.trim();
    }
    key.trim_matches(|c| c == '"' || c == '\'')
        .trim()
        .to_owned()
}

/// Store (or replace) a key by name.
pub fn set_named(name: &str, key: &str) -> Result<(), CoreError> {
    entry(name)?
        .set_password(&clean_pasted(key))
        .map_err(|e| CoreError::Keychain(e.to_string()))
}

/// Remove a key by name. Returns `false` if there was none.
pub fn delete_named(name: &str) -> Result<bool, CoreError> {
    match entry(name)?.delete_credential() {
        Ok(()) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(CoreError::Keychain(e.to_string())),
    }
}

/// The names of every key Thrall has stored, when this system can list them (Windows Credential
/// Manager can; elsewhere `None`, and the keychain app lists them under "Thrall").
pub fn stored_names() -> Option<Vec<String>> {
    if !cfg!(windows) {
        return None;
    }
    let out = std::process::Command::new(crate::constants::CREDENTIAL_LIST_EXE)
        .arg("/list")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| names_in_listing(&String::from_utf8_lossy(&out.stdout)))
}

/// Thrall's key names in `cmdkey /list` output: targets ending `.Thrall` exactly (other programs'
/// entries, like the GitHub CLI's `gh:github.com:…`, are left alone).
fn names_in_listing(listing: &str) -> Vec<String> {
    let suffix = format!(".{KEYCHAIN_SERVICE}");
    listing
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("Target:")?
                .trim()
                .strip_prefix(crate::constants::CREDENTIAL_TARGET_PREFIX)?
                .strip_suffix(&suffix)
                .filter(|name| valid_name(name))
                .map(str::to_owned)
        })
        .collect()
}

/// The stored key for a provider, if there is one.
pub fn get_key(provider: ProviderKind) -> Result<Option<String>, CoreError> {
    get_named(provider.as_str())
}

/// Store (or replace) a provider's key.
pub fn set_key(provider: ProviderKind, key: &str) -> Result<(), CoreError> {
    set_named(provider.as_str(), key)
}

/// Remove a provider's key. Returns `false` if there was none.
pub fn delete_key(provider: ProviderKind) -> Result<bool, CoreError> {
    delete_named(provider.as_str())
}

/// `text` in pieces of at most `size` characters (at least one piece).
fn split_long(text: &str, size: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    chars
        .chunks(size.max(1))
        .map(|c| c.iter().collect())
        .collect()
}

/// The keychain name of a long secret's `part` (the first part is `name` itself).
fn part_name(name: &str, part: usize) -> String {
    if part == 0 {
        name.to_owned()
    } else {
        format!("{name}-{}", part + 1)
    }
}

/// Store (or replace) a secret too long for one keychain entry, such as a sign-in's tokens. The
/// first entry, `name`, starts with how many parts there are; the rest are `name-2`, `name-3`….
/// Stored as is (not cleaned like a pasted key).
pub fn set_long(name: &str, text: &str) -> Result<(), CoreError> {
    let parts = split_long(text, LONG_SECRET_CHUNK_CHARS);
    if parts.len() > LONG_SECRET_MAX_PARTS {
        return Err(CoreError::Keychain(format!(
            "{name} is too long to keep in the keychain"
        )));
    }
    // Clear an older, longer copy first so no stale parts are left behind.
    delete_long(name)?;
    for (i, part) in parts.iter().enumerate() {
        let value = if i == 0 {
            format!("{}\n{part}", parts.len())
        } else {
            part.clone()
        };
        entry(&part_name(name, i))?
            .set_password(&value)
            .map_err(|e| CoreError::Keychain(e.to_string()))?;
    }
    Ok(())
}

/// A secret stored with [`set_long`], if there is one (and all its parts are there).
pub fn get_long(name: &str) -> Result<Option<String>, CoreError> {
    let Some(first) = get_named(name)? else {
        return Ok(None);
    };
    let Some((count, head)) = first.split_once('\n') else {
        return Ok(None);
    };
    let count: usize = count.parse().unwrap_or(0);
    if count == 0 || count > LONG_SECRET_MAX_PARTS {
        return Ok(None);
    }
    let mut text = head.to_owned();
    for i in 1..count {
        match get_named(&part_name(name, i))? {
            Some(part) => text.push_str(&part),
            None => return Ok(None),
        }
    }
    Ok(Some(text))
}

/// Remove a secret stored with [`set_long`]. Returns `false` if there was none.
pub fn delete_long(name: &str) -> Result<bool, CoreError> {
    let count = get_named(name)?
        .and_then(|first| {
            first
                .split_once('\n')
                .and_then(|(n, _)| n.parse::<usize>().ok())
        })
        .unwrap_or(1)
        .clamp(1, LONG_SECRET_MAX_PARTS);
    let mut any = false;
    for i in 0..count {
        any |= delete_named(&part_name(name, i))?;
    }
    Ok(any)
}

/// The keychain name a config value refers to: `"keychain:youtube"` → `youtube`.
pub fn keychain_ref(value: &str) -> Option<&str> {
    value.strip_prefix(KEYCHAIN_REF_PREFIX).map(str::trim)
}

/// A safe preview of a key: `sk-…a1b2`. Short keys show only `…`.
pub fn preview(key: &str) -> String {
    let chars: Vec<char> = key.trim().chars().collect();
    if chars.len() <= KEY_PREVIEW_HEAD + KEY_PREVIEW_TAIL + 4 {
        return "…".to_owned();
    }
    let head: String = chars.iter().take(KEY_PREVIEW_HEAD).collect();
    let tail: String = chars.iter().skip(chars.len() - KEY_PREVIEW_TAIL).collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_thralls_own_entries_are_listed() {
        let listing = "Currently stored credentials:\n\n    \
            Target: LegacyGeneric:target=gh:github.com:someone\n    Type: Generic\n    \
            Target: LegacyGeneric:target=openrouter.Thrall\n    \
            Target: LegacyGeneric:target=mcp-notion-1.Thrall\n    \
            Target: LegacyGeneric:target=Some App.Thrall\n    \
            Target: Domain:target=fal.Thrall\n";
        assert_eq!(
            names_in_listing(listing),
            vec!["openrouter", "mcp-notion-1"]
        );
    }

    #[test]
    fn preview_hides_the_middle() {
        assert_eq!(preview("sk-or-v1-abcdef0123456789a1b2"), "sk-…a1b2");
    }

    #[test]
    fn key_names_and_references() {
        assert!(valid_name("youtube") && valid_name("my-key_2"));
        assert!(!valid_name("YouTube") && !valid_name("a b") && !valid_name(""));
        assert_eq!(keychain_ref("keychain:youtube"), Some("youtube"));
        assert_eq!(keychain_ref("plain-value"), None);
    }

    #[test]
    fn pasted_env_lines_become_the_key() {
        assert_eq!(clean_pasted("FAL_KEY=abc:def"), "abc:def");
        assert_eq!(clean_pasted("  FAL_KEY=\"abc:def\"  "), "abc:def");
        assert_eq!(
            clean_pasted("export OPENROUTER_API_KEY='sk-or-1'"),
            "sk-or-1"
        );
        assert_eq!(clean_pasted("sk-or-v1-abc"), "sk-or-v1-abc");
        // A key that itself contains = (base64 padding) stays whole.
        assert_eq!(clean_pasted("abc=="), "abc==");
    }

    #[test]
    fn long_secrets_split_into_parts_and_join_back() {
        let text = "a\u{e9}".repeat(1_500);
        let parts = split_long(&text, LONG_SECRET_CHUNK_CHARS);
        assert_eq!(parts.len(), 3);
        assert!(
            parts
                .iter()
                .all(|p| p.chars().count() <= LONG_SECRET_CHUNK_CHARS)
        );
        assert_eq!(parts.concat(), text);
        assert_eq!(split_long("", 10), vec![String::new()]);
        assert_eq!(part_name("mcp-docs", 0), "mcp-docs");
        assert_eq!(part_name("mcp-docs", 2), "mcp-docs-3");
        assert!(valid_name(&part_name("mcp-docs", 2)));
    }

    #[test]
    fn preview_hides_short_keys_entirely() {
        assert_eq!(preview("short"), "…");
    }
}
