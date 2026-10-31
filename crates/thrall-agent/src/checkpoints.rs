//! Checkpoints: before a turn first changes a file, its content is kept, so the person can rewind
//! to before any of their messages. Only changes made by Thrall's file tools are kept; what shell
//! commands do to files isn't.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use thrall_core::ids::now_ms;
use thrall_protocol::{RewindAction, RewindFile};
use thrall_store::Checkpoint;

use crate::constants::CHECKPOINT_MAX_FILE_BYTES;
use crate::error::AgentError;
use crate::state::AppState;

/// SHA-256 of `bytes`, as hex.
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Keep `path` as it is now, if this turn hasn't already. Failing to keep it never stops the
/// edit; it's logged.
pub(crate) async fn before_edit(state: &AppState, session_id: &str, turn_id: &str, path: &Path) {
    let (existed, content, too_big) = match tokio::fs::metadata(path).await {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (false, None, false),
        Err(e) => {
            tracing::warn!(path = %path.display(), %e, "couldn't keep a checkpoint");
            return;
        }
        Ok(meta) if !meta.is_file() => return,
        Ok(meta) if meta.len() > CHECKPOINT_MAX_FILE_BYTES => (true, None, true),
        Ok(_) => match tokio::fs::read(path).await {
            Ok(bytes) => (true, Some(bytes), false),
            Err(e) => {
                tracing::warn!(path = %path.display(), %e, "couldn't keep a checkpoint");
                return;
            }
        },
    };
    let checkpoint = Checkpoint {
        session_id: session_id.to_owned(),
        turn_id: turn_id.to_owned(),
        user_id: state.user_id.clone(),
        path: path.to_string_lossy().into_owned(),
        existed,
        content,
        too_big,
        after_hash: None,
        at: now_ms(),
    };
    if let Err(e) = state.store.save_checkpoint(checkpoint).await {
        tracing::warn!(path = %path.display(), %e, "couldn't keep a checkpoint");
    }
}

/// Note what the turn left in `path` after changing it.
pub(crate) async fn after_edit(state: &AppState, session_id: &str, turn_id: &str, path: &Path) {
    let Ok(bytes) = tokio::fs::read(path).await else {
        return;
    };
    let shown = path.to_string_lossy();
    if let Err(e) = state
        .store
        .set_checkpoint_after(&state.user_id, session_id, turn_id, &shown, &hash(&bytes))
        .await
    {
        tracing::warn!(path = %shown, %e, "couldn't note a checkpoint");
    }
}

/// One file a rewind would put back.
pub struct Planned {
    /// Where it is.
    pub path: PathBuf,
    /// What's shown and sent to clients.
    pub file: RewindFile,
    /// Its content before (for `Restore`).
    content: Option<Vec<u8>>,
}

/// What rewinding to before `turn_id` would do to files, in the order they were first changed.
/// Files the turns never managed to change are left out.
pub async fn plan(
    state: &AppState,
    session_id: &str,
    turn_id: &str,
    workspace: Option<&Path>,
) -> Result<Vec<Planned>, AgentError> {
    let checkpoints = state
        .store
        .checkpoints_from(&state.user_id, session_id, turn_id)
        .await?;
    let mut paths: Vec<&str> = Vec::new();
    for c in &checkpoints {
        if !paths.contains(&c.path.as_str()) {
            paths.push(&c.path);
        }
    }
    let mut out = Vec::new();
    for p in paths {
        let of_path = || checkpoints.iter().filter(move |c| c.path == p);
        // The earliest is how it was before the turn; the latest change is what Thrall left.
        let Some(first) = of_path().next() else {
            continue;
        };
        let Some(left) = of_path()
            .filter_map(|c| c.after_hash.as_deref())
            .next_back()
        else {
            continue;
        };
        let path = PathBuf::from(p);
        let now = tokio::fs::read(&path).await.ok();
        let changed = now.as_deref().map(hash).as_deref() != Some(left);
        let action = if !first.existed {
            RewindAction::Delete
        } else if first.too_big || first.content.is_none() {
            RewindAction::Unavailable
        } else {
            RewindAction::Restore
        };
        // A file Thrall made that's gone already needs nothing.
        if action == RewindAction::Delete && now.is_none() {
            continue;
        }
        let shown = match workspace {
            Some(ws) => thrall_tools::paths::display(ws, &path),
            None => p.replace('\\', "/"),
        };
        out.push(Planned {
            path,
            file: RewindFile {
                path: shown,
                action,
                changed,
            },
            content: first.content.clone(),
        });
    }
    Ok(out)
}

/// Put the planned files back, except those named in `keep`. Returns what was put back and what
/// couldn't be (with the reason).
pub async fn restore(planned: Vec<Planned>, keep: &[String]) -> (Vec<String>, Vec<String>) {
    let mut restored = Vec::new();
    let mut failed = Vec::new();
    for p in planned {
        if keep.contains(&p.file.path) {
            continue;
        }
        let done = match p.file.action {
            RewindAction::Unavailable => Err("it was too big to keep a copy of".to_owned()),
            RewindAction::Delete => match tokio::fs::remove_file(&p.path).await {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            },
            RewindAction::Restore => {
                if let Some(parent) = p.path.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                tokio::fs::write(&p.path, p.content.unwrap_or_default())
                    .await
                    .map_err(|e| e.to_string())
            }
        };
        match done {
            Ok(()) => restored.push(p.file.path),
            Err(why) => failed.push(format!("{}: {why}", p.file.path)),
        }
    }
    (restored, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_hex_sha256() {
        assert_eq!(
            hash(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
