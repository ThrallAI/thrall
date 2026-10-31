//! The `Tool` trait and what every tool receives and returns.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use futures::future::BoxFuture;
use serde::de::DeserializeOwned;
use serde_json::Value;
use thrall_gates::Effect;
use thrall_protocol::{Diff, TaskItem};
use thrall_store::Store;
use tokio::sync::{mpsc, watch};

/// Files read in a session and when they last changed, for the edit guard: Thrall may only edit a
/// file it has read, and only if nobody changed it since.
pub type ReadLog = Arc<Mutex<HashMap<PathBuf, Option<SystemTime>>>>;

/// When a file last changed, if it exists.
pub fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Lets a running tool notice that the user pressed Esc.
#[derive(Clone)]
pub struct Cancel(watch::Receiver<bool>);

/// The other end: whoever can interrupt the turn.
pub struct Interrupter(watch::Sender<bool>);

impl Interrupter {
    /// Stop the turn.
    pub fn interrupt(&self) {
        let _ = self.0.send(true);
    }
}

impl Cancel {
    /// A connected pair.
    pub fn pair() -> (Interrupter, Cancel) {
        let (tx, rx) = watch::channel(false);
        (Interrupter(tx), Cancel(rx))
    }

    /// One that never fires (tests, tools run outside a turn).
    pub fn never() -> Cancel {
        Self::pair().1
    }

    /// Whether the user has interrupted.
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }

    /// Wait until the user interrupts (forever if they can't any more).
    pub async fn cancelled(&mut self) {
        loop {
            if *self.0.borrow() {
                return;
            }
            if self.0.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }
}

/// What a tool can see and use. Built per call by the agent.
pub struct ToolContext {
    /// The session's working folder. Edits only happen inside it.
    pub workspace: PathBuf,
    /// Storage (tasks, memory).
    pub store: Arc<dyn Store>,
    /// Owner.
    pub user_id: String,
    /// Session.
    pub session_id: String,
    /// Shared HTTP client for web tools.
    pub http: reqwest::Client,
    /// Live output for clients (shell). Tools may ignore it.
    pub output: mpsc::UnboundedSender<String>,
    /// Files read this session (edit guard).
    pub reads: ReadLog,
    /// Fires when the user interrupts.
    pub cancel: Cancel,
    /// Thrall's data folder (personal skills), if known.
    pub data_dir: Option<PathBuf>,
    /// The user's MCP servers, when any are set up.
    pub mcp: Option<Arc<thrall_mcp::McpHub>>,
    /// Long-term memory (facts and past conversations).
    pub memory: Option<Arc<thrall_memory::Memory>>,
    /// The engine's browser (one process, a tab per session).
    pub browser: Arc<crate::browser::Browsers>,
    /// Skills the person turned off (config `skills.disabled`).
    pub disabled_skills: Vec<String>,
}

impl ToolContext {
    /// Note that `path` was read (or just written) as it is now.
    pub fn note_read(&self, path: &Path) {
        if let Ok(mut reads) = self.reads.lock() {
            reads.insert(path.to_path_buf(), modified(path));
        }
    }

    /// The edit guard: why `path` mustn't be changed yet, if it mustn't.
    pub fn edit_guard(&self, path: &Path, shown: &str) -> Option<String> {
        if !path.exists() {
            return None; // New files are fine.
        }
        let reads = self.reads.lock().ok()?;
        match reads.get(path) {
            None => Some(format!(
                "read {shown} before changing it (read_file), so the edit is based on what's \
                 really there."
            )),
            Some(seen) if *seen != modified(path) => Some(format!(
                "{shown} changed since you last read it (someone else edited it). Read it again \
                 before changing it."
            )),
            Some(_) => None,
        }
    }
}

/// What a tool call produced.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    /// Whether it worked.
    pub ok: bool,
    /// What the model reads.
    pub content: String,
    /// One line for the user.
    pub summary: String,
    /// The change, for file edits.
    pub diff: Option<Diff>,
    /// The new task list, for `todo_write`.
    pub tasks: Option<Vec<TaskItem>>,
    /// The user approved a restart (`thrall_restart`).
    pub restart: bool,
    /// Files the step made, shown to the user (pictures).
    pub files: Vec<thrall_protocol::FileRef>,
    /// A fact it saved to long-term memory, so the person is told (and can forget it).
    pub remembered: Option<SavedFact>,
    /// Not `ok`, but it ran to the end (a command whose exit code wasn't 0): shown as a warning,
    /// not a failure, since the output may still be what was asked.
    pub warn: bool,
}

/// A fact saved to long-term memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedFact {
    /// Its id.
    pub id: String,
    /// Its topic.
    pub topic: String,
    /// The fact.
    pub content: String,
}

impl ToolOutput {
    /// A success: `content` for the model, `summary` for the user.
    pub fn ok(content: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            ok: true,
            content: content.into(),
            summary: summary.into(),
            diff: None,
            tasks: None,
            restart: false,
            files: Vec::new(),
            remembered: None,
            warn: false,
        }
    }

    /// A failure the model can read and recover from.
    pub fn error(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            ok: false,
            summary: message.lines().next().unwrap_or_default().to_owned(),
            content: format!("Error: {message}"),
            diff: None,
            tasks: None,
            restart: false,
            files: Vec::new(),
            remembered: None,
            warn: false,
        }
    }
}

/// A native tool. One file per tool group under `tools/`.
pub trait Tool: Send + Sync {
    /// Name the model calls it by.
    fn name(&self) -> &'static str;

    /// What it does and when to use it (sent to the model).
    fn description(&self) -> &'static str;

    /// JSON Schema of the arguments.
    fn parameters(&self) -> Value;

    /// Plain-language title of a call: "Reading src/main.rs".
    fn title(&self, args: &Value) -> String;

    /// Why this call can't work, found before anyone is asked to approve it (`None` = fine).
    fn precheck(&self, _ctx: &ToolContext, _args: &Value) -> Option<String> {
        None
    }

    /// Anything to find out before `effect` can be judged (e.g. whether an MCP tool only reads,
    /// which needs its server's tool list). Runs before the gate; default: nothing.
    fn prepare<'a>(&'a self, _ctx: &'a ToolContext, _args: &'a Value) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// What this call would do; the tool gate decides from it (and the permission mode).
    fn effect(&self, _ctx: &ToolContext, _args: &Value) -> Effect {
        Effect::Read
    }

    /// Do it.
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput>;
}

/// Decode a tool's arguments, or say (for the model) what was wrong with them. Models often send
/// a list or object as text (`"[{...}]"`); when the arguments don't fit as sent, such text is
/// unpacked and they're tried once more.
pub fn parse_args<T: DeserializeOwned>(tool: &str, args: Value) -> Result<T, String> {
    match serde_json::from_value::<T>(args.clone()) {
        Ok(v) => Ok(v),
        Err(first) => {
            let mut fixed = args;
            let mut changed = false;
            if let Value::Object(map) = &mut fixed {
                for value in map.values_mut() {
                    let Value::String(text) = value else { continue };
                    let trimmed = text.trim_start();
                    if !(trimmed.starts_with('[') || trimmed.starts_with('{')) {
                        continue;
                    }
                    if let Ok(inner) = serde_json::from_str::<Value>(text) {
                        *value = inner;
                        changed = true;
                    }
                }
            }
            if changed {
                serde_json::from_value(fixed).map_err(|e| format!("bad arguments for {tool}: {e}"))
            } else {
                Err(format!("bad arguments for {tool}: {first}"))
            }
        }
    }
}

/// A string argument for titles, or "" when missing.
pub fn arg_str<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Shorten text for the model, saying how much was cut.
pub fn cap(text: String, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text;
    }
    let kept: String = text.chars().take(max_chars).collect();
    let dropped = text.chars().count() - max_chars;
    format!("{kept}\n… [{dropped} more characters cut]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Args {
        #[serde(alias = "todos")]
        items: Vec<Item>,
    }
    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Item {
        text: String,
    }

    #[test]
    fn lists_sent_as_text_are_unpacked() {
        // Exactly what mimo-v2.5-pro sent in the user's test: another name, and the list as text.
        let args = serde_json::json!({ "todos": "[{\"status\": \"pending\", \"text\": \"Count lines\"}]" });
        let parsed: Args = parse_args("todo_write", args).unwrap();
        assert_eq!(
            parsed.items,
            vec![Item {
                text: "Count lines".into()
            }]
        );
    }

    #[test]
    fn text_that_isnt_json_still_fails_clearly() {
        let args = serde_json::json!({ "items": "[not json" });
        let err = parse_args::<Args>("todo_write", args).unwrap_err();
        assert!(err.starts_with("bad arguments for todo_write"), "{err}");
    }
}
