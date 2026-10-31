//! The phone app this engine offers as an update. `scripts/release-android.ps1` puts the APK in
//! the data folder's `phone-app` folder with a `release.json` describing it; paired phones ask
//! for it (`app.latest`) and download it in pieces (`app.get`). Nothing outside that folder is
//! ever read.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use thrall_protocol::AppRelease;
use thrall_protocol::constants::FILE_CHUNK_BYTES;

use crate::constants::{PHONE_APP_DIR, PHONE_APP_RELEASE_FILE};

/// `release.json` as the release script writes it.
#[derive(Debug, Deserialize)]
struct ReleaseFile {
    version_code: u32,
    version_name: String,
    /// The APK's name, in the same folder.
    file: String,
    sha256: String,
    #[serde(default)]
    notes: String,
}

/// The release on offer and its APK, if the data folder holds a usable one.
pub fn latest(data_dir: Option<&Path>) -> Option<(AppRelease, PathBuf)> {
    let dir = data_dir?.join(PHONE_APP_DIR);
    let text = std::fs::read_to_string(dir.join(PHONE_APP_RELEASE_FILE)).ok()?;
    let release: ReleaseFile = match serde_json::from_str(&text) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(%e, "the phone app's release.json isn't valid");
            return None;
        }
    };
    // Only a plain file name: never a path out of the folder.
    let plain = Path::new(&release.file)
        .file_name()
        .map(|n| n.to_os_string());
    if plain.as_deref() != Some(std::ffi::OsStr::new(&release.file)) {
        tracing::warn!(file = %release.file, "the phone app's release.json names a path, not a file");
        return None;
    }
    let apk = dir.join(&release.file);
    let size = std::fs::metadata(&apk).ok().filter(|m| m.is_file())?.len();
    Some((
        AppRelease {
            version_code: release.version_code,
            version_name: release.version_name,
            size,
            sha256: release.sha256.to_lowercase(),
            notes: release.notes,
        },
        apk,
    ))
}

/// Up to `FILE_CHUNK_BYTES` of the APK from `offset`: (its name, where the piece starts, the whole
/// size, the bytes).
pub async fn read(
    data_dir: Option<&Path>,
    offset: u64,
) -> Result<(String, u64, u64, Vec<u8>), String> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let none = || "This Thrall has no phone app to offer.".to_owned();
    let (_, apk) = latest(data_dir).ok_or_else(none)?;
    let mut f = tokio::fs::File::open(&apk).await.map_err(|_| none())?;
    let size = f.metadata().await.map_err(|_| none())?.len();
    let start = offset.min(size);
    f.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|_| none())?;
    let want = (size - start).min(FILE_CHUNK_BYTES as u64) as usize;
    let mut buf = vec![0u8; want];
    f.read_exact(&mut buf).await.map_err(|_| none())?;
    let name = apk
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok((name, start, size, buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("thrall-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(PHONE_APP_DIR)).unwrap();
        dir
    }

    fn release(dir: &Path, file: &str) {
        let json = serde_json::json!({"version_code": 3, "version_name": "0.3.0", "file": file,
            "sha256": "ABC123", "notes": "App lock"});
        std::fs::write(
            dir.join(PHONE_APP_DIR).join(PHONE_APP_RELEASE_FILE),
            json.to_string(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn the_release_is_offered_and_read_in_pieces() {
        let dir = data("offer");
        assert!(
            latest(Some(&dir)).is_none(),
            "nothing until there's a release"
        );
        release(&dir, "Thrall-0.3.0.apk");
        assert!(latest(Some(&dir)).is_none(), "nor without its APK");
        let bytes = vec![9u8; FILE_CHUNK_BYTES + 10];
        std::fs::write(dir.join(PHONE_APP_DIR).join("Thrall-0.3.0.apk"), &bytes).unwrap();
        let (offer, _) = latest(Some(&dir)).unwrap();
        assert_eq!((offer.version_code, offer.size), (3, bytes.len() as u64));
        assert_eq!(
            (offer.sha256.as_str(), offer.notes.as_str()),
            ("abc123", "App lock")
        );
        let (name, start, size, first) = read(Some(&dir), 0).await.unwrap();
        assert_eq!(
            (name.as_str(), start, size),
            ("Thrall-0.3.0.apk", 0, bytes.len() as u64)
        );
        assert_eq!(first.len(), FILE_CHUNK_BYTES);
        let (_, _, _, rest) = read(Some(&dir), FILE_CHUNK_BYTES as u64).await.unwrap();
        assert_eq!(rest.len(), 10);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_release_naming_a_path_is_ignored() {
        let dir = data("path");
        std::fs::write(dir.join("secret.db"), b"x").unwrap();
        release(&dir, "../secret.db");
        assert!(latest(Some(&dir)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
