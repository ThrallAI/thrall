//! Read-only access to a session's folder, for clients that aren't on this computer (the phone's
//! Files view): list a folder, read a file in pieces. Only inside the session's own folder, by the
//! same rules Thrall's tools follow (no `..`, no absolute paths, no following links out of it).
//! Nothing here changes a file.

use std::path::{Path, PathBuf};

use thrall_agent::{AgentError, AppState};
use thrall_core::paths::Paths;
use thrall_protocol::constants::FILE_CHUNK_BYTES;
use thrall_protocol::{FolderEntry, Reply, ThrallFolder};
use thrall_tools::paths::{display, inside, resolve};

use crate::constants::{
    FILE_MANAGER, FIND_DEPTH, FIND_SCAN, FIND_SKIP, FOLDER_LIST_MAX, FOLDER_NAME_BAD_CHARS,
    FOLDER_NAME_MAX_CHARS, PLACES_MAX, PLACES_SCAN, RESERVED_NAMES,
};

/// Folders a new session could work in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Places {
    /// The ones sessions already use, most recently active first, only those still there.
    pub recent: Vec<String>,
    /// Thrall's default (`Documents\Thrall`, made when picked).
    pub default: Option<String>,
    /// Where a new folder goes unless another is picked (`folders.new_in`, else `default`).
    pub new_in: Option<String>,
}

/// Where new folders go: the `folders.new_in` setting, else Thrall's default.
fn new_in(state: &AppState) -> Option<PathBuf> {
    Paths::work_folder(&state.config.folders.new_in)
}

/// The folders a new session could work in.
pub async fn places(state: &AppState) -> Result<Places, AgentError> {
    let sessions = state
        .store
        .list_sessions(&state.user_id, PLACES_SCAN)
        .await?;
    let mut recent: Vec<String> = Vec::new();
    for w in sessions.into_iter().filter_map(|s| s.workspace) {
        if recent.len() >= PLACES_MAX {
            break;
        }
        if !recent.contains(&w) && Path::new(&w).is_dir() {
            recent.push(w);
        }
    }
    let shown = |p: PathBuf| p.display().to_string();
    Ok(Places {
        recent,
        default: Paths::default_work_folder().map(shown),
        new_in: new_in(state).map(shown),
    })
}

/// `folders.shown`: the data folder and the working folder.
pub fn shown(state: &AppState) -> Reply {
    let shown = |p: &Path| p.display().to_string();
    Reply::FoldersShown {
        data: state.data_dir.as_deref().map(shown).unwrap_or_default(),
        work: state.work_dir.as_deref().map(shown),
    }
}

/// Open one of Thrall's folders in the file manager on this computer, making the working folder
/// first if it isn't there yet.
pub fn open(state: &AppState, folder: ThrallFolder) -> Result<(), String> {
    let path = match folder {
        ThrallFolder::Data => state.data_dir.as_deref(),
        ThrallFolder::Work => state.work_dir.as_deref(),
    }
    .ok_or_else(|| "This engine has no such folder.".to_owned())?;
    if folder == ThrallFolder::Work {
        std::fs::create_dir_all(path)
            .map_err(|e| format!("Couldn't make `{}`: {e}", path.display()))?;
    }
    // Not waited for: the file manager keeps running (and Explorer's exit code means nothing).
    std::process::Command::new(FILE_MANAGER)
        .arg(path)
        .spawn()
        .map(drop)
        .map_err(|e| format!("Couldn't open `{}`: {e}", path.display()))
}

/// Why `name` can't be a new folder's name, if it can't.
fn bad_name(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("Give the folder a name.".to_owned());
    }
    if name.chars().count() > FOLDER_NAME_MAX_CHARS {
        return Some(format!(
            "That name is too long (at most {FOLDER_NAME_MAX_CHARS} characters)."
        ));
    }
    if name == "." || name == ".." || name.ends_with('.') {
        return Some("A folder name can't be dots or end with one.".to_owned());
    }
    if name
        .chars()
        .any(|c| c.is_control() || FOLDER_NAME_BAD_CHARS.contains(&c))
    {
        return Some(
            "A folder name can't have any of \\ / : * ? \" < > | in it: make one folder at a time."
                .to_owned(),
        );
    }
    let stem = name.split('.').next().unwrap_or(name).trim();
    if RESERVED_NAMES.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        return Some(format!(
            "Windows keeps the name {stem} for itself: pick another."
        ));
    }
    None
}

/// Make the folder `name` inside `parent`, which must be one of `places` (or its new-folder
/// place). Never replaces anything. Returns the new folder's path.
pub async fn create(state: &AppState, parent: &str, name: &str) -> Result<String, String> {
    let name = name.trim();
    if let Some(why) = bad_name(name) {
        return Err(why);
    }
    let offered = places(state)
        .await
        .map_err(|e| format!("Couldn't look up the folders: {e}"))?;
    let parent = parent.trim();
    let own = [offered.default.as_deref(), offered.new_in.as_deref()];
    let allowed = offered.recent.iter().any(|p| p == parent) || own.contains(&Some(parent));
    if !allowed {
        return Err("New folders can only go in one of the folders offered.".to_owned());
    }
    let parent = Path::new(parent);
    // Thrall's own folders are made when first used; any other must already be there.
    if own.contains(&Some(&*parent.display().to_string())) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Couldn't make `{}`: {e}", parent.display()))?;
    } else if !parent.is_dir() {
        return Err(format!(
            "There's no folder `{}` any more.",
            parent.display()
        ));
    }
    let folder = parent.join(name);
    // create_dir (not create_dir_all) fails when it's already there, so nothing is ever replaced.
    match std::fs::create_dir(&folder) {
        Ok(()) => Ok(folder.display().to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(format!(
            "There's already something called {name} in `{}`.",
            parent.display()
        )),
        Err(e) => Err(format!("Couldn't make the folder: {e}")),
    }
}

/// Whether a new session can work in `path`: a folder that exists, or Thrall's default (made now).
pub fn usable(path: &str) -> Result<(), String> {
    let folder = Path::new(path);
    if folder.is_dir() {
        return Ok(());
    }
    if Paths::default_work_folder().as_deref() == Some(folder) {
        return std::fs::create_dir_all(folder)
            .map_err(|e| format!("Couldn't make the folder `{path}`: {e}"));
    }
    Err(format!("There's no folder `{path}` on this computer."))
}

/// A path the client asked for, made real and checked to be inside `workspace`.
fn checked(workspace: &Path, path: &str) -> Result<PathBuf, String> {
    let path = path.trim().trim_start_matches(['/', '\\']);
    if Path::new(path).is_absolute() || path.contains(':') {
        return Err("Only paths inside the session's folder can be opened.".to_owned());
    }
    let full = resolve(workspace, path);
    if !inside(workspace, &full) {
        return Err("That's outside the session's folder.".to_owned());
    }
    Ok(full)
}

fn millis(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

/// The folder at `path` (relative to `workspace`; empty for the folder itself): folders first,
/// then files, each newest first. The bool says whether entries were left out.
pub fn list(workspace: &Path, path: &str) -> Result<(String, Vec<FolderEntry>, bool), String> {
    let dir = checked(workspace, path)?;
    let read = std::fs::read_dir(&dir).map_err(|_| "There's no such folder.".to_owned())?;
    let mut items: Vec<FolderEntry> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            // Links are shown as what they point at, and only when that's inside the folder.
            let full = e.path();
            if !inside(workspace, &full) {
                return None;
            }
            let meta = std::fs::metadata(&full).ok()?;
            Some(FolderEntry {
                name: e.file_name().to_string_lossy().into_owned(),
                folder: meta.is_dir(),
                size: if meta.is_dir() { 0 } else { meta.len() },
                modified: millis(&meta),
            })
        })
        .collect();
    items.sort_by(|a, b| b.folder.cmp(&a.folder).then(b.modified.cmp(&a.modified)));
    let more = items.len() > FOLDER_LIST_MAX;
    items.truncate(FOLDER_LIST_MAX);
    Ok((display(workspace, &dir), items, more))
}

/// Up to `FILE_CHUNK_BYTES` of the file at `path` from `offset`: (shown path, where the piece
/// starts, the whole size, the bytes).
pub async fn read(
    workspace: &Path,
    path: &str,
    offset: u64,
) -> Result<(String, u64, u64, Vec<u8>), String> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let mut file = checked(workspace, path)?;
    let missing = || "There's no such file.".to_owned();
    if !file.exists() {
        file = found_below(workspace, path).await?;
    }
    let mut f = tokio::fs::File::open(&file).await.map_err(|_| missing())?;
    let meta = f.metadata().await.map_err(|_| missing())?;
    if meta.is_dir() {
        return Err("That's a folder: list it instead.".to_owned());
    }
    let size = meta.len();
    let start = offset.min(size);
    f.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|_| missing())?;
    let want = (size - start).min(FILE_CHUNK_BYTES as u64) as usize;
    let mut buf = vec![0u8; want];
    f.read_exact(&mut buf).await.map_err(|_| missing())?;
    Ok((display(workspace, &file), start, size, buf))
}

/// A file that isn't where `path` says: Thrall often names a file it made without the folder it
/// put it in (`notes.md` for `launch/notes.md`). Look a few folders down for the one file whose
/// path ends that way; if there are several, say where they are.
async fn found_below(workspace: &Path, path: &str) -> Result<PathBuf, String> {
    let ws = workspace.to_path_buf();
    let wanted = path
        .trim()
        .trim_start_matches(['/', '\\'])
        .replace('\\', "/");
    let matches = tokio::task::spawn_blocking(move || files_below(&ws, &wanted))
        .await
        .unwrap_or_default();
    match matches.as_slice() {
        [] => Err("There's no such file.".to_owned()),
        [one] => Ok(one.clone()),
        many => {
            let places: Vec<String> = many
                .iter()
                .map(|p| format!("`{}`", display(workspace, p)))
                .collect();
            Err(format!(
                "There's more than one `{path}` in this folder: {}. Open the one you want from Files.",
                places.join(", ")
            ))
        }
    }
}

/// Files under `workspace` (not in it: that was already tried) whose path ends with `wanted`,
/// looking `FIND_DEPTH` folders down and at most `FIND_SCAN` entries, skipping hidden folders and
/// ones full of build output. Links aren't followed.
fn files_below(workspace: &Path, wanted: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut scanned = 0usize;
    let mut level: Vec<PathBuf> = vec![workspace.to_path_buf()];
    for _ in 0..FIND_DEPTH {
        let mut next = Vec::new();
        for dir in &level {
            let Ok(read) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in read.filter_map(Result::ok) {
                scanned += 1;
                if scanned > FIND_SCAN {
                    return found;
                }
                let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
                let name = entry.file_name().to_string_lossy().into_owned();
                if !is_dir || name.starts_with('.') || FIND_SKIP.contains(&name.as_str()) {
                    continue;
                }
                let sub = entry.path();
                let candidate = resolve(&sub, wanted);
                if candidate.is_file()
                    && inside(workspace, &candidate)
                    && !found.contains(&candidate)
                {
                    found.push(candidate);
                }
                next.push(sub);
            }
        }
        level = next;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("thrall-folders-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("docs/notes.md"), "# Notes").unwrap();
        std::fs::write(dir.join("board.dxf"), vec![7u8; 1000]).unwrap();
        dir
    }

    #[test]
    fn a_folder_lists_folders_first() {
        let ws = workspace("list");
        let (shown, items, more) = list(&ws, "").unwrap();
        assert!(!more);
        assert_eq!(items[0].name, "docs");
        assert!(items[0].folder);
        assert_eq!(items[1].name, "board.dxf");
        assert_eq!(items[1].size, 1000);
        assert!(!shown.is_empty());
        let (shown, items, _) = list(&ws, "docs").unwrap();
        assert_eq!(shown, "docs");
        assert_eq!(items[0].name, "notes.md");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn new_folder_names_are_plain_names_windows_accepts() {
        for ok in ["Woodland book", "2026 taxes", "ark.mods", "Ünïcode"] {
            assert_eq!(bad_name(ok), None, "{ok}");
        }
        for bad in [
            "", ".", "..", "dots.", "a/b", "a\\b", "C:", "what?", "pipe|", "con", "Nul.txt", "LPT1",
        ] {
            assert!(bad_name(bad).is_some(), "{bad:?} should be refused");
        }
        assert!(bad_name(&"x".repeat(FOLDER_NAME_MAX_CHARS + 1)).is_some());
    }

    #[test]
    fn nothing_outside_the_folder_can_be_reached() {
        let ws = workspace("outside");
        assert!(list(&ws, "..").is_err());
        assert!(list(&ws, "docs/../..").is_err());
        assert!(list(&ws, "C:/Windows").is_err());
        // A leading slash means the session's folder, not the drive.
        assert!(list(&ws, "/docs").is_ok());
        assert!(list(&ws, "/etc").is_err());
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[tokio::test]
    async fn files_are_read_in_pieces() {
        let ws = workspace("read");
        let (shown, start, size, bytes) = read(&ws, "docs/notes.md", 0).await.unwrap();
        assert_eq!((shown.as_str(), start, size), ("docs/notes.md", 0, 7));
        assert_eq!(bytes, b"# Notes");
        let (_, start, _, bytes) = read(&ws, "board.dxf", 990).await.unwrap();
        assert_eq!((start, bytes.len()), (990, 10));
        assert!(read(&ws, "docs", 0).await.is_err(), "a folder isn't a file");
        assert!(read(&ws, "../secret.txt", 0).await.is_err());
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[tokio::test]
    async fn a_file_named_without_its_folder_is_found_below() {
        let ws = workspace("below");
        // Named without its folder: the one match is opened, and its real path is shown.
        let (shown, _, _, bytes) = read(&ws, "notes.md", 0).await.unwrap();
        assert_eq!(shown, "docs/notes.md");
        assert_eq!(bytes, b"# Notes");
        // Two matches: neither is guessed, and both are named.
        std::fs::create_dir_all(ws.join("old")).unwrap();
        std::fs::write(ws.join("old/notes.md"), "old").unwrap();
        let why = read(&ws, "notes.md", 0).await.unwrap_err();
        assert!(
            why.contains("docs/notes.md") && why.contains("old/notes.md"),
            "{why}"
        );
        // Hidden folders aren't looked in; still nothing outside the folder.
        std::fs::create_dir_all(ws.join(".cache")).unwrap();
        std::fs::write(ws.join(".cache/only.md"), "x").unwrap();
        assert!(read(&ws, "only.md", 0).await.is_err());
        assert!(read(&ws, "../notes.md", 0).await.is_err());
        let _ = std::fs::remove_dir_all(&ws);
    }
}
