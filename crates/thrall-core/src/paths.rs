//! Where Thrall keeps its files: `%APPDATA%\Thrall` on Windows, the OS data folder elsewhere,
//! or wherever `THRALL_HOME` points.

use std::path::{Path, PathBuf};

use crate::constants::{
    APP_DIR_NAME, CONFIG_FILE, DB_FILE, ENGINE_FILE, LOGS_DIR, TOKEN_FILE, WORK_DIR_NAME,
};
use crate::error::CoreError;

/// Every path Thrall uses, worked out once at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// The data folder itself.
    pub home: PathBuf,
    /// `config.toml`.
    pub config_file: PathBuf,
    /// `thrall.db`.
    pub db_file: PathBuf,
    /// `logs/`.
    pub logs_dir: PathBuf,
    /// The engine's access token.
    pub token_file: PathBuf,
    /// `engine.json`: where the running engine listens.
    pub engine_file: PathBuf,
}

impl Paths {
    /// Work out the data folder. `home_override` is the value of `THRALL_HOME`, if set; the caller
    /// reads the environment so this stays a pure function.
    pub fn resolve(home_override: Option<PathBuf>) -> Result<Self, CoreError> {
        let home = match home_override {
            Some(path) if !path.as_os_str().is_empty() => path,
            _ => directories::BaseDirs::new()
                .ok_or(CoreError::NoDataFolder)?
                .data_dir()
                .join(APP_DIR_NAME),
        };
        Ok(Self::under(&home))
    }

    /// The layout inside a given data folder.
    pub fn under(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
            config_file: home.join(CONFIG_FILE),
            db_file: home.join(DB_FILE),
            logs_dir: home.join(LOGS_DIR),
            token_file: home.join(TOKEN_FILE),
            engine_file: home.join(ENGINE_FILE),
        }
    }

    /// The person's home folder. An engine started by the desktop app or the terminal runs from
    /// here, so a session with no folder of its own never works inside whatever project folder
    /// happened to start the engine.
    pub fn user_home() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf())
    }

    /// `Documents\Thrall`: a folder of its own for sessions started without one picked (from the
    /// phone, say). Not created here.
    pub fn default_work_folder() -> Option<PathBuf> {
        directories::UserDirs::new()
            .and_then(|d| d.document_dir().map(Path::to_path_buf))
            .or_else(Self::user_home)
            .map(|d| d.join(WORK_DIR_NAME))
    }

    /// Create the data and log folders if they don't exist yet.
    pub fn ensure_dirs(&self) -> Result<(), CoreError> {
        for dir in [&self.home, &self.logs_dir] {
            std::fs::create_dir_all(dir).map_err(|source| CoreError::Io {
                path: dir.clone(),
                source,
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_wins() {
        let paths = Paths::resolve(Some(PathBuf::from("custom-home"))).unwrap();
        assert_eq!(paths.home, PathBuf::from("custom-home"));
        assert_eq!(paths.db_file, PathBuf::from("custom-home").join(DB_FILE));
    }

    #[test]
    fn empty_override_falls_back_to_os_folder() {
        let paths = Paths::resolve(Some(PathBuf::new())).unwrap();
        assert!(paths.home.ends_with(APP_DIR_NAME));
    }

    #[test]
    fn layout_is_inside_home() {
        let paths = Paths::under(Path::new("h"));
        for p in [&paths.config_file, &paths.db_file, &paths.logs_dir] {
            assert!(p.starts_with("h"));
        }
    }
}
