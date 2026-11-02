//! `engine.json`: a running engine says where it listens, so clients (the terminal, the desktop
//! app, a second `thrall engine`) can find it. Written when the engine starts listening, removed
//! when it stops. Kept here so clients can read it without linking the engine.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// What a running engine writes about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineFile {
    /// `ws://127.0.0.1:<port>/v1/ws`.
    pub ws_url: String,
    /// `http://127.0.0.1:<port>/`, the web app.
    pub web_url: String,
    /// The port.
    pub port: u16,
    /// The engine's process id.
    pub pid: u32,
    /// The engine's version.
    pub version: String,
}

impl EngineFile {
    /// Describe this process as an engine listening on `host:port`, with its socket at `ws_path`.
    pub fn new(host: &str, port: u16, ws_path: &str, version: &str) -> Self {
        Self {
            ws_url: format!("ws://{host}:{port}{ws_path}"),
            web_url: format!("http://{host}:{port}/"),
            port,
            pid: std::process::id(),
            version: version.to_owned(),
        }
    }

    /// Write it to `path`.
    pub fn write(&self, path: &Path) -> Result<(), CoreError> {
        let text = serde_json::to_string_pretty(self).unwrap_or_default();
        std::fs::write(path, text).map_err(|source| CoreError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    /// The file at `path`, if there is one and it's readable.
    pub fn read(path: &Path) -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    /// The web app's address as a one-time sign-in link for `code`.
    pub fn signin_url(&self, code: &str) -> String {
        signin_url(&self.web_url, code)
    }

    /// Remove `path` if it still describes this process (a newer engine may have replaced it).
    pub fn remove_if_mine(path: &Path) {
        if Self::read(path).is_some_and(|f| f.pid == std::process::id()) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// `web_url` (with anything after it, such as `?client=desktop`) as a one-time sign-in link.
pub fn signin_url(web_url: &str, code: &str) -> String {
    format!("{web_url}{}{code}", crate::constants::SIGNIN_FRAGMENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_links_carry_the_code_after_the_address() {
        let file = EngineFile::new("127.0.0.1", 7417, "/v1/ws", "0.1.0");
        assert_eq!(
            file.signin_url("ab12"),
            "http://127.0.0.1:7417/#signin=ab12"
        );
        assert_eq!(
            signin_url("http://127.0.0.1:7417/?client=desktop", "ab12"),
            "http://127.0.0.1:7417/?client=desktop#signin=ab12"
        );
    }

    #[test]
    fn round_trips_and_only_its_writer_removes_it() {
        let dir = std::env::temp_dir().join(format!("thrall-engine-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("engine.json");
        let file = EngineFile::new("127.0.0.1", 7417, "/v1/ws", "0.1.0");
        assert_eq!(file.ws_url, "ws://127.0.0.1:7417/v1/ws");
        file.write(&path).unwrap();
        assert_eq!(EngineFile::read(&path), Some(file.clone()));

        let other = EngineFile {
            pid: file.pid + 1,
            ..file
        };
        other.write(&path).unwrap();
        EngineFile::remove_if_mine(&path);
        assert!(path.exists(), "another engine's file stays");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
