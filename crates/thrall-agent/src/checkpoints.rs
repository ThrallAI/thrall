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

/// Where a session's files are: here, or on its companion PC (then they're read and written
/// through the PC's runner).
#[derive(Clone)]
pub(crate) struct Place {
    pc: Option<OnPc>,
}

#[derive(Clone)]
struct OnPc {
    companions: std::sync::Arc<crate::companions::Companions>,
    device: String,
    person: String,
    session: String,
    workspace: String,
}

/// What reading a file came to.
enum Got {
    Missing,
    TooBig,
    Bytes(Vec<u8>),
    Unreadable(String),
}

impl Place {
    /// Files on this computer.
    pub(crate) fn here() -> Self {
        Self { pc: None }
    }

    /// `session`'s files: on its PC (`pc`), else here.
    pub(crate) fn of(state: &AppState, session: &str, workspace: &Path, pc: Option<&str>) -> Self {
        Self {
            pc: pc.map(|device| OnPc {
                companions: state.companions.clone(),
                device: device.to_owned(),
                person: state.user_id.clone(),
                session: session.to_owned(),
                workspace: workspace.to_string_lossy().into_owned(),
            }),
        }
    }

    async fn call(
        pc: &OnPc,
        job: thrall_tools::companion::Call,
    ) -> Result<serde_json::Value, String> {
        pc.companions
            .call(
                &pc.device,
                &pc.person,
                &pc.session,
                &pc.workspace,
                job,
                None,
            )
            .await
    }

    async fn read(&self, path: &Path) -> Got {
        let Some(pc) = &self.pc else {
            return match tokio::fs::metadata(path).await {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Got::Missing,
                Err(e) => Got::Unreadable(e.to_string()),
                Ok(meta) if !meta.is_file() => Got::Unreadable("not a file".to_owned()),
                Ok(meta) if meta.len() > CHECKPOINT_MAX_FILE_BYTES => Got::TooBig,
                Ok(_) => match tokio::fs::read(path).await {
                    Ok(bytes) => Got::Bytes(bytes),
                    Err(e) => Got::Unreadable(e.to_string()),
                },
            };
        };
        let job = thrall_tools::companion::Call::Read {
            path: path.to_string_lossy().into_owned(),
        };
        match Self::call(pc, job).await {
            Err(why) => Got::Unreadable(why),
            Ok(v) if v["missing"] == true => Got::Missing,
            Ok(v) if v["too_big"] == true => Got::TooBig,
            Ok(v) => {
                use base64::Engine as _;
                match base64::engine::general_purpose::STANDARD
                    .decode(v["data"].as_str().unwrap_or_default())
                {
                    Ok(bytes) if bytes.len() as u64 > CHECKPOINT_MAX_FILE_BYTES => Got::TooBig,
                    Ok(bytes) => Got::Bytes(bytes),
                    Err(e) => Got::Unreadable(e.to_string()),
                }
            }
        }
    }

    async fn write(&self, path: &Path, bytes: Vec<u8>) -> Result<(), String> {
        let Some(pc) = &self.pc else {
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            return tokio::fs::write(path, bytes)
                .await
                .map_err(|e| e.to_string());
        };
        use base64::Engine as _;
        let job = thrall_tools::companion::Call::Write {
            path: path.to_string_lossy().into_owned(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
        };
        Self::call(pc, job).await.map(|_| ())
    }

    async fn delete(&self, path: &Path) -> Result<(), String> {
        let Some(pc) = &self.pc else {
            return match tokio::fs::remove_file(path).await {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        };
        let job = thrall_tools::companion::Call::Delete {
            path: path.to_string_lossy().into_owned(),
        };
        Self::call(pc, job).await.map(|_| ())
    }
}

/// Keep `path` as it is now, if this turn hasn't already. Failing to keep it never stops the
/// edit; it's logged.
pub(crate) async fn before_edit(
    state: &AppState,
    place: &Place,
    session_id: &str,
    turn_id: &str,
    path: &Path,
) {
    let (existed, content, too_big) = match place.read(path).await {
        Got::Missing => (false, None, false),
        Got::TooBig => (true, None, true),
        Got::Bytes(bytes) => (true, Some(bytes), false),
        Got::Unreadable(e) => {
            tracing::warn!(path = %path.display(), %e, "couldn't keep a checkpoint");
            return;
        }
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
pub(crate) async fn after_edit(
    state: &AppState,
    place: &Place,
    session_id: &str,
    turn_id: &str,
    path: &Path,
) {
    let Got::Bytes(bytes) = place.read(path).await else {
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
    /// Here, or on the session's PC.
    place: Place,
}

/// What rewinding to before `turn_id` would do to files, in the order they were first changed.
/// Files the turns never managed to change are left out. A session on a companion PC (`pc`) has
/// its files looked at there.
pub async fn plan(
    state: &AppState,
    session_id: &str,
    turn_id: &str,
    workspace: Option<&Path>,
    pc: Option<&str>,
) -> Result<Vec<Planned>, AgentError> {
    let place = match (pc, workspace) {
        (Some(pc), Some(ws)) => Place::of(state, session_id, ws, Some(pc)),
        _ => Place::here(),
    };
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
        let now = match place.read(&path).await {
            Got::Bytes(bytes) => Some(bytes),
            _ => None,
        };
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
            place: place.clone(),
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
            RewindAction::Delete => p.place.delete(&p.path).await,
            RewindAction::Restore => p.place.write(&p.path, p.content.unwrap_or_default()).await,
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
