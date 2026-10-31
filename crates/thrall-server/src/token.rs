//! The local access token: random, created at first start, kept in the data folder so local
//! clients can read it. It stops other programs and web pages from driving Thrall; it is not an
//! account password and never leaves the computer.

use std::path::Path;

use crate::constants::TOKEN_BYTES;
use crate::error::ServerError;

/// The token in `path`, creating it first if there isn't one.
pub fn load_or_create(path: &Path) -> Result<String, ServerError> {
    if let Ok(text) = std::fs::read_to_string(path) {
        let token = text.trim().to_owned();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    let token = generate()?;
    std::fs::write(path, &token).map_err(|source| ServerError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(token)
}

/// A fresh random token, as hex.
pub fn generate() -> Result<String, ServerError> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| ServerError::Random(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Compare tokens without leaking how much of a wrong one matched through timing.
pub fn matches(given: &str, expected: &str) -> bool {
    let (a, b) = (given.as_bytes(), expected.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_hex() {
        let (a, b) = (generate().unwrap(), generate().unwrap());
        assert_eq!(a.len(), TOKEN_BYTES * 2);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn only_the_exact_token_matches() {
        assert!(matches("abc", "abc"));
        assert!(!matches("abd", "abc"));
        assert!(!matches("ab", "abc"));
    }

    #[test]
    fn token_is_created_once_and_reused() {
        let dir = std::env::temp_dir().join(format!("thrall-token-{}", generate().unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("access-token");
        let first = load_or_create(&path).unwrap();
        assert_eq!(load_or_create(&path).unwrap(), first);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
