//! Where Thrall keeps its files: `%APPDATA%\Thrall` on Windows, the OS data folder elsewhere,
//! or wherever `THRALL_HOME` points.

use std::path::{Path, PathBuf};

use crate::constants::{
    APP_DIR_NAME, CONFIG_FILE, DB_FILE, ENGINE_FILE, LOGS_DIR, SYNCED_FOLDER_VARS, TOKEN_FILE,
    WORK_DIR_NAME,
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
    /// phone, say) and for Home. When OneDrive syncs Documents it's `<home folder>\Thrall`
    /// instead, unless `Documents\Thrall` is already there. Not created here.
    pub fn default_work_folder() -> Option<PathBuf> {
        let documents =
            directories::UserDirs::new().and_then(|d| d.document_dir().map(Path::to_path_buf));
        let synced: Vec<PathBuf> = SYNCED_FOLDER_VARS
            .iter()
            .filter_map(std::env::var_os)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .collect();
        pick_work_folder(documents.as_deref(), Self::user_home().as_deref(), &synced)
    }

    /// Thrall's working folder: the `folders.new_in` setting (`set`) when there is one, else
    /// [`Paths::default_work_folder`]. Not created here.
    pub fn work_folder(set: &str) -> Option<PathBuf> {
        let set = set.trim();
        if set.is_empty() {
            Self::default_work_folder()
        } else {
            Some(PathBuf::from(set))
        }
    }

    /// Create the data and log folders if they don't exist yet. On Linux and macOS the data folder
    /// is made private to this account every time (it holds the access token, the phone link's
    /// key and everyone's sessions); Windows already keeps `%APPDATA%` private.
    pub fn ensure_dirs(&self) -> Result<(), CoreError> {
        for dir in [&self.home, &self.logs_dir] {
            std::fs::create_dir_all(dir).map_err(|source| CoreError::Io {
                path: dir.clone(),
                source,
            })?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &self.home,
                std::fs::Permissions::from_mode(crate::constants::DATA_DIR_MODE),
            )
            .map_err(|source| CoreError::Io {
                path: self.home.clone(),
                source,
            })?;
        }
        Ok(())
    }
}

/// `Documents\Thrall`, or `<home>\Thrall` when Documents is inside one of the `synced` folders and
/// `Documents\Thrall` isn't there yet (an existing one keeps being used: sessions already work in
/// it). Without a Documents folder, `<home>\Thrall`.
fn pick_work_folder(
    documents: Option<&Path>,
    home: Option<&Path>,
    synced: &[PathBuf],
) -> Option<PathBuf> {
    let in_home = home.map(|h| h.join(WORK_DIR_NAME));
    let Some(documents) = documents else {
        return in_home;
    };
    let in_documents = documents.join(WORK_DIR_NAME);
    let is_synced = synced.iter().any(|s| documents.starts_with(s));
    match in_home {
        Some(in_home) if is_synced && !in_documents.is_dir() => Some(in_home),
        _ => Some(in_documents),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn the_data_folder_is_private_to_this_account() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("thrall-private-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let paths = Paths::resolve(Some(dir.clone())).unwrap();
        paths.ensure_dirs().unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, crate::constants::DATA_DIR_MODE);
        let _ = std::fs::remove_dir_all(&dir);
    }

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

    #[test]
    fn the_working_folder_stays_out_of_onedrive() {
        let home = Path::new("h");
        let synced = [PathBuf::from("h").join("OneDrive")];
        let documents = home.join("OneDrive").join("Documents");
        assert_eq!(
            pick_work_folder(Some(&documents), Some(home), &synced),
            Some(home.join(WORK_DIR_NAME)),
            "synced Documents: the home folder instead"
        );
        let plain = home.join("Documents");
        assert_eq!(
            pick_work_folder(Some(&plain), Some(home), &synced),
            Some(plain.join(WORK_DIR_NAME))
        );
        assert_eq!(
            pick_work_folder(None, Some(home), &synced),
            Some(home.join(WORK_DIR_NAME))
        );
    }

    #[test]
    fn an_existing_folder_in_synced_documents_is_kept() {
        let dir = std::env::temp_dir().join(format!("thrall-synced-{}", std::process::id()));
        let documents = dir.join("OneDrive").join("Documents");
        std::fs::create_dir_all(documents.join(WORK_DIR_NAME)).unwrap();
        let synced = [dir.join("OneDrive")];
        assert_eq!(
            pick_work_folder(Some(&documents), Some(&dir), &synced),
            Some(documents.join(WORK_DIR_NAME))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_set_working_folder_wins() {
        assert_eq!(
            Paths::work_folder("  /projects  "),
            Some(PathBuf::from("/projects"))
        );
        assert_eq!(Paths::work_folder(""), Paths::default_work_folder());
    }
}
