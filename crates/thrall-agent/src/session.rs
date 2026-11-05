//! Creating and finding sessions.

use std::path::{Path, PathBuf};

use thrall_core::constants::id_prefix;
use thrall_core::ids::{new_id, now_ms};
use thrall_gates::Mode;
use thrall_store::{NewSession, Session};

use crate::error::AgentError;
use crate::state::AppState;

/// Start a new session, optionally tied to a working folder.
pub async fn create(state: &AppState, workspace: Option<String>) -> Result<Session, AgentError> {
    let session = state
        .store
        .create_session(NewSession {
            id: new_id(id_prefix::SESSION),
            user_id: state.user_id.clone(),
            workspace,
            at: now_ms(),
        })
        .await?;
    Ok(session)
}

/// A session by id; an error the user can act on if it doesn't exist.
pub async fn get(state: &AppState, session_id: &str) -> Result<Session, AgentError> {
    state
        .store
        .get_session(&state.user_id, session_id)
        .await?
        .ok_or_else(|| AgentError::SessionNotFound(session_id.to_owned()))
}

/// Change a session's permission mode.
pub async fn set_mode(state: &AppState, session_id: &str, mode: Mode) -> Result<(), AgentError> {
    state
        .store
        .set_session_mode(&state.user_id, session_id, mode.as_str())
        .await?;
    Ok(())
}

/// The folder a session's turns work in: its own, or the engine's when it has none. Everything
/// that touches a session's files (turns, browsing them from a phone) goes through this.
pub fn folder_of(session: &Session) -> PathBuf {
    session
        .workspace
        .as_deref()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default()
}

/// Move a session to another folder; `path` must be a folder that exists. Returns the folder as
/// stored, or why it can't be used.
pub async fn set_folder(
    state: &AppState,
    session_id: &str,
    path: &str,
) -> Result<Result<String, String>, AgentError> {
    let raw = path.trim().trim_matches('"');
    // `C:\Work\` → `C:\Work`, but a drive or filesystem root keeps its slash.
    let path = match raw.trim_end_matches(['\\', '/']) {
        t if t.is_empty() || t.ends_with(':') => raw,
        t => t,
    };
    let folder = Path::new(path);
    if !folder.is_absolute() {
        return Ok(Err(format!(
            "Give the whole path, like `C:\\Users\\you\\Projects\\{path}`."
        )));
    }
    // A session on a companion PC moves to a folder on that PC: the PC says whether it's there.
    let pc = get(state, session_id).await?.pc;
    match &pc {
        Some(pc) => {
            let look = thrall_tools::companion::Call::List {
                path: String::new(),
            };
            let there = state
                .companions
                .call(pc, &state.user_id, session_id, path, look, None)
                .await;
            if let Err(why) = there {
                return Ok(Err(format!("Couldn't use `{path}` on the PC: {why}")));
            }
        }
        None if !folder.is_dir() => {
            return Ok(Err(format!("There's no folder `{path}` on this computer.")));
        }
        None => {}
    }
    state
        .store
        .set_session_workspace(&state.user_id, session_id, path)
        .await?;
    Ok(Ok(path.to_owned()))
}

/// A session's permission mode.
pub fn mode_of(session: &Session) -> Mode {
    Mode::parse(&session.mode).unwrap_or_default()
}

/// The most recently active session, if there is one.
pub async fn latest(state: &AppState) -> Result<Option<Session>, AgentError> {
    Ok(state
        .store
        .list_sessions(&state.user_id, 1)
        .await?
        .into_iter()
        .next())
}
