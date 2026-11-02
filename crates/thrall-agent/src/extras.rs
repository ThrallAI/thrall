//! What Thrall reads besides the conversation: standing instructions (a personal `THRALL.md` in the
//! data folder, the project's `THRALL.md` or `AGENTS.md`), the list of skills, and files the user
//! mentions with `@path`.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use thrall_tools::documents::{self, Kind};
use thrall_tools::paths::{display, resolve};
use thrall_tools::tool::modified;

use crate::constants::{
    HUMANISE_COMMAND, HUMANISE_COMMAND_ALT, HUMANISE_DETECTOR_NOTE, HUMANISE_RUN_NOTE,
    INSTRUCTIONS_MAX_CHARS, MENTION_MAX_BYTES, MENTION_MAX_CHARS, MENTIONS_MAX, PERSONAL_FILE,
    PROJECT_FILES, SKILL_RUN_NOTE,
};
use crate::state::AppState;

fn read_capped(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(INSTRUCTIONS_MAX_CHARS).collect())
}

/// Extra system-prompt text: personal and project instructions, and the available skills.
pub fn standing_instructions(state: &AppState, workspace: &Path) -> String {
    let mut out = String::new();
    if let Some(text) = state
        .my_dir()
        .and_then(|d| read_capped(&d.join(PERSONAL_FILE)))
    {
        out.push_str(&format!("\n\n# The user's standing instructions\n\n{text}"));
    }
    if let Some((name, text)) = PROJECT_FILES
        .iter()
        .find_map(|f| read_capped(&workspace.join(f)).map(|t| (*f, t)))
    {
        out.push_str(&format!("\n\n# Project instructions ({name})\n\n{text}"));
    }
    let skills = thrall_tools::skills::discover(
        state.my_dir().as_deref(),
        workspace,
        &state.config.skills.disabled,
    );
    if !skills.is_empty() {
        out.push_str(
            "\n\n# Skills\n\nUse one with the skill tool when the task matches its description:",
        );
        for skill in skills {
            out.push_str(&format!("\n- {}: {}", skill.name, skill.description));
        }
    }
    out
}

/// The helper types Thrall can hand work to (the `agent` tool's `type`), for its own system
/// prompt; helpers don't start helpers, so theirs leaves it out.
pub fn helper_types(state: &AppState, workspace: &Path) -> String {
    let types = thrall_tools::agents::discover(state.my_dir().as_deref(), workspace);
    let mut out = format!(
        "\n\n# Helper types\n\nWith the agent tool, pick the type that fits. Up to {} helpers work \
         at the same time when you call agent several times in one reply:",
        state.config.agent.helpers_at_once()
    );
    for t in types {
        let looks = if t.read_only { " (looks only)" } else { "" };
        out.push_str(&format!("\n- {}{looks}: {}", t.name, t.description));
    }
    out
}

/// A message starting `/name` where `name` is a skill (turned on, here) runs that skill: the
/// model is told to use it, with the rest of the message as the task. Empty otherwise.
pub fn skill_run(state: &AppState, workspace: &Path, text: &str) -> String {
    let Some(rest) = text.trim_start().strip_prefix('/') else {
        return String::new();
    };
    let name = rest.split_whitespace().next().unwrap_or_default();
    let found = thrall_tools::skills::discover(
        state.my_dir().as_deref(),
        workspace,
        &state.config.skills.disabled,
    )
    .into_iter()
    .find(|s| s.name.eq_ignore_ascii_case(name));
    match found {
        Some(skill) => format!("\n\n{}", SKILL_RUN_NOTE.replace("{name}", &skill.name)),
        // The person's own skill of the same name wins over the built-in.
        None if name.eq_ignore_ascii_case(HUMANISE_COMMAND)
            || name.eq_ignore_ascii_case(HUMANISE_COMMAND_ALT) =>
        {
            humanise_note(state.config.writing.detector_aware)
        }
        None => String::new(),
    }
}

/// What `/humanise` adds for the model.
fn humanise_note(detector_aware: bool) -> String {
    let mut note = format!("\n\n{HUMANISE_RUN_NOTE}");
    if detector_aware {
        note.push('\n');
        note.push_str(HUMANISE_DETECTOR_NOTE);
    }
    note
}

/// The picture type of a file, from its extension, for formats models accept.
pub fn image_mime(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

/// Files the user attached (uploaded to the engine), as the model sees them: pictures are named
/// here and sent as pictures with the message; documents (PDF, Word, PowerPoint, spreadsheets)
/// and small text files go in as text (and count as read); anything else is described with where
/// it's saved, so tools can open it.
pub async fn attach_files(
    state: &AppState,
    session_id: &str,
    files: &[PathBuf],
) -> (String, Vec<PathBuf>) {
    let mut out = String::new();
    // PDFs that are mostly pictures of pages: the picture reader transcribes them.
    let mut scanned = Vec::new();
    for path in files {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let saved = path.display();
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if image_mime(path).is_some() {
            out.push_str(&format!(
                "\n\n[The user attached a picture: {name}, saved at {saved}]"
            ));
            continue;
        }
        if crate::audio::audio_format(path).is_some() {
            out.push_str(&format!(
                "\n\n[The user attached a recording: {name}, saved at {saved}]"
            ));
            continue;
        }
        if let Some(kind) = Kind::of(path) {
            let owned = path.clone();
            let read = tokio::task::spawn_blocking(move || documents::extract(&owned)).await;
            match read {
                Ok(Ok(doc)) => {
                    note_read(state, session_id, path);
                    if doc.scanned {
                        scanned.push(path.clone());
                    }
                    let text: String = doc.text.chars().take(MENTION_MAX_CHARS).collect();
                    let cut = if doc.text.chars().count() > MENTION_MAX_CHARS {
                        format!(
                            "\n(Only the start is shown; read the rest with read_document on {saved}.)"
                        )
                    } else {
                        String::new()
                    };
                    out.push_str(&format!(
                        "\n\n[The user attached this file: {name} ({}, {}), saved at {saved}]\n```\n{text}\n```{cut}",
                        kind.label(),
                        doc.detail
                    ));
                }
                Ok(Err(e)) => out.push_str(&format!(
                    "\n\n[The user attached this file: {name} ({}), saved at {saved}. It couldn't be read: {e}]",
                    kind.label(),
                )),
                Err(e) => out.push_str(&format!(
                    "\n\n[The user attached this file: {name}, saved at {saved}. Reading it stopped: {e}]",
                )),
            }
            continue;
        }
        let text = (size <= MENTION_MAX_BYTES)
            .then(|| std::fs::read_to_string(path).ok())
            .flatten();
        match text {
            Some(content) => {
                note_read(state, session_id, path);
                let content: String = content.chars().take(MENTION_MAX_CHARS).collect();
                out.push_str(&format!(
                    "\n\n[The user attached this file: {name}, saved at {saved}]\n```\n{content}\n```"
                ));
            }
            None => out.push_str(&format!(
                "\n\n[The user attached this file: {name} ({size} bytes), saved at {saved}. It isn't \
                 text Thrall can read or is too large to include here; open it with your tools if \
                 you need it.]"
            )),
        }
    }
    (out, scanned)
}
/// How clients refer to an uploaded file: its upload id (the folder it's saved in), name, kind
/// and size.
pub fn file_ref(path: &Path) -> thrall_protocol::FileRef {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let file = path
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let kind = if image_mime(path).is_some() {
        "image"
    } else if crate::audio::audio_format(path).is_some() {
        "audio"
    } else if path.extension().is_some_and(|e| {
        matches!(
            e.to_string_lossy().to_lowercase().as_str(),
            "mp4" | "mov" | "m4v"
        )
    }) {
        "video"
    } else if Kind::of(path).is_some() {
        "document"
    } else if std::fs::read_to_string(path).is_ok() {
        "text"
    } else {
        "other"
    };
    thrall_protocol::FileRef {
        file,
        name,
        kind: kind.to_owned(),
        size: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
    }
}

fn note_read(state: &AppState, session_id: &str, path: &Path) {
    if let Ok(mut reads) = state.reads_for(session_id).lock() {
        reads.insert(path.to_path_buf(), modified(path));
    }
}
static MENTION: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"(?:^|\s)@([^\s@]+)").ok());

/// The user's message as the model sees it: files mentioned with `@path` are attached. Attached
/// files count as read for the edit guard.
pub fn expand_mentions(state: &AppState, session_id: &str, workspace: &Path, text: &str) -> String {
    let Some(re) = MENTION.as_ref() else {
        return text.to_owned();
    };
    let mut attached = Vec::new();
    for caps in re.captures_iter(text) {
        let Some(raw) = caps.get(1) else { continue };
        let raw = raw
            .as_str()
            .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '"', '\'']);
        let path = resolve(workspace, raw);
        let small = std::fs::metadata(&path)
            .map(|m| m.is_file() && m.len() <= MENTION_MAX_BYTES)
            .unwrap_or(false);
        if !small || attached.iter().any(|(p, _)| p == &path) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(mut reads) = state.reads_for(session_id).lock() {
            reads.insert(path.clone(), modified(&path));
        }
        attached.push((path, content));
        if attached.len() >= MENTIONS_MAX {
            break;
        }
    }
    if attached.is_empty() {
        return text.to_owned();
    }
    let mut out = text.to_owned();
    for (path, content) in attached {
        let content: String = content.chars().take(MENTION_MAX_CHARS).collect();
        out.push_str(&format!(
            "\n\n[Attached: {}]\n```\n{content}\n```",
            display(workspace, &path)
        ));
    }
    out
}
