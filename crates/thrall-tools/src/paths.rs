//! Turning the model's paths into real ones, and keeping edits inside the workspace.

use std::path::{Component, Path, PathBuf};

/// `path` as an absolute path: relative paths are taken from the workspace.
pub fn resolve(workspace: &Path, path: &str) -> PathBuf {
    let raw = Path::new(path.trim());
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        workspace.join(raw)
    };
    normalize(&joined)
}

/// Remove `.` and `..` without touching the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The real location of `path`, following links in the part that exists, so a link inside the
/// workspace can't be used to write outside it.
fn real(path: &Path) -> PathBuf {
    let mut existing = path.to_path_buf();
    let mut rest = Vec::new();
    while !existing.exists() {
        match (
            existing.file_name().map(ToOwned::to_owned),
            existing.parent(),
        ) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
    let mut out = std::fs::canonicalize(&existing).unwrap_or(existing);
    for name in rest.into_iter().rev() {
        out.push(name);
    }
    out
}

/// Whether `path` is inside `workspace` (or is it).
pub fn inside(workspace: &Path, path: &Path) -> bool {
    real(path).starts_with(real(workspace))
}

/// How a path is shown to people and the model: relative to the workspace when inside it.
pub fn display(workspace: &Path, path: &Path) -> String {
    let shown = match path.strip_prefix(workspace) {
        Ok(rel) if !rel.as_os_str().is_empty() => rel.to_path_buf(),
        _ => path.to_path_buf(),
    };
    shown.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_join_the_workspace() {
        let ws = Path::new("/work/project");
        assert_eq!(
            resolve(ws, "src/main.rs"),
            Path::new("/work/project/src/main.rs")
        );
        assert_eq!(
            resolve(ws, "./a/../b.txt"),
            Path::new("/work/project/b.txt")
        );
    }

    #[test]
    fn escapes_are_outside() {
        let ws = std::env::temp_dir().join("thrall-paths-test");
        assert!(inside(&ws, &resolve(&ws, "a/b.txt")));
        assert!(!inside(&ws, &resolve(&ws, "../elsewhere.txt")));
    }

    #[test]
    fn display_is_relative_inside_the_workspace() {
        let ws = Path::new("/work/project");
        assert_eq!(display(ws, Path::new("/work/project/src/a.rs")), "src/a.rs");
        assert_eq!(display(ws, Path::new("/other/x")), "/other/x");
    }
}
