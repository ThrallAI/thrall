//! Tools that run on a companion PC: which ones, what crosses the link, and the PC's side.
//!
//! A session started on a companion PC (or a phone of someone whose work runs on their PC) does its
//! file, document and command work on that PC. The engine sends each such call there twice: once
//! to check it (why it can't work, and what it would do, which the permission card shows), then,
//! if allowed, to run it. The PC runs the same tool code as Thrall does here, so it behaves the
//! same; only the folder is the PC's.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thrall_gates::Effect;
use thrall_protocol::{Diff, TaskItem};
use tokio::sync::mpsc;

use crate::constants::{COMPANION_FILE_MAX_BYTES, COMPANION_TOOLS};
use crate::tool::{Cancel, Interrupter, ReadLog, Tool, ToolContext, ToolOutput};

/// Whether `tool` runs on the companion PC (for a session that works there).
pub fn runs_on_pc(tool: &str) -> bool {
    COMPANION_TOOLS.contains(&tool)
}

/// What the engine asks a companion PC to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Call {
    /// Why this call can't work, and what it would do.
    Check {
        /// The tool.
        tool: String,
        /// Its arguments.
        args: Value,
    },
    /// Do it.
    Run {
        /// The tool.
        tool: String,
        /// Its arguments.
        args: Value,
    },
    /// Stop a call that's running (the person pressed Stop).
    Stop {
        /// The running call's id.
        call: String,
    },
    /// A file in the folder, for Thrall to keep or look at here.
    Read {
        /// Path, relative to the folder or absolute.
        path: String,
    },
    /// Put a file in the folder (something made here, like a picture).
    Write {
        /// Path, relative to the folder or absolute.
        path: String,
        /// Its bytes, base64.
        data: String,
    },
    /// The folder's entries at `path` (relative; "" is the folder), for the Files view. Only
    /// inside the folder, read only.
    List {
        /// Relative path.
        path: String,
    },
    /// A piece of a file in the folder, for the Files view.
    Part {
        /// Relative path.
        path: String,
        /// Where to start, in bytes.
        offset: u64,
    },
    /// Folders a new session on this PC could work in: those of `recent` still there, and the
    /// PC's Thrall folder.
    Places {
        /// Folders this PC's sessions used, most recent first.
        recent: Vec<String>,
    },
    /// A new, empty folder in `parent` (the PC's Thrall folder or one of its sessions' folders).
    MakeFolder {
        /// Where.
        parent: String,
        /// Its name (a plain folder name).
        name: String,
    },
}

impl Call {
    /// Whether the folder tools of the PC's link answer this (not the tool runner).
    pub fn is_folder_call(&self) -> bool {
        matches!(
            self,
            Self::List { .. } | Self::Part { .. } | Self::Places { .. } | Self::MakeFolder { .. }
        )
    }
}

/// The answer to [`Call::Check`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checked {
    /// Why the call can't work, if it can't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precheck: Option<String>,
    /// What it would do.
    pub effect: Effect,
}

/// A file a tool made for the person to see, carried across the link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Carried {
    /// Its name.
    pub name: String,
    /// Its bytes, base64.
    pub data: String,
}

/// The answer to [`Call::Run`]: the tool's output, with any files it made carried along.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ran {
    /// Whether it worked.
    pub ok: bool,
    /// What the model reads.
    pub content: String,
    /// One line for the person.
    pub summary: String,
    /// The change, for file edits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<Diff>,
    /// The new task list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tasks: Option<Vec<TaskItem>>,
    /// Ran to the end without working (a command's exit code wasn't 0).
    #[serde(default)]
    pub warn: bool,
    /// Files it made for the person to see.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<Carried>,
}

impl Ran {
    /// What a tool produced on the PC, ready to send. Files it saved for the person (in the PC's
    /// data folder) go along as bytes; one too big to carry is left out.
    pub fn from_output(output: ToolOutput, data_dir: Option<&Path>) -> Self {
        let files = output
            .files
            .iter()
            .filter_map(|f| {
                let path = data_dir?
                    .join(thrall_core::constants::UPLOADS_DIR)
                    .join(&f.file)
                    .join(&f.name);
                let bytes = std::fs::read(path).ok()?;
                (bytes.len() as u64 <= COMPANION_FILE_MAX_BYTES).then(|| Carried {
                    name: f.name.clone(),
                    data: BASE64.encode(bytes),
                })
            })
            .collect();
        Self {
            ok: output.ok,
            content: output.content,
            summary: output.summary,
            diff: output.diff,
            tasks: output.tasks,
            warn: output.warn,
            files,
        }
    }

    /// The output again, here: carried files are saved like the engine's own (so every client
    /// can show them).
    pub fn into_output(self, ctx: &ToolContext) -> ToolOutput {
        let mut output = if self.ok {
            ToolOutput::ok(self.content, self.summary)
        } else {
            let mut o = ToolOutput::error("");
            o.content = self.content;
            o.summary = self.summary;
            o
        };
        output.diff = self.diff;
        output.tasks = self.tasks;
        output.warn = self.warn;
        for file in self.files {
            let Ok(bytes) = BASE64.decode(&file.data) else {
                continue;
            };
            // Only a plain name: a PC can't place files elsewhere in the data folder.
            let name = Path::new(&file.name)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            match crate::saved::save(ctx, &name, &bytes) {
                Ok((_, saved)) => output.files.push(saved),
                Err(e) => tracing::warn!(%e, "couldn't keep a file a companion PC made"),
            }
        }
        output
    }
}

/// What every call a companion PC runs is given, besides the call itself.
pub struct Base {
    /// The tools it runs (only those that run on a PC are ever asked for).
    pub tools: Vec<Arc<dyn Tool>>,
    /// Storage the tools may use (the PC's own, kept in memory).
    pub store: Arc<dyn thrall_store::Store>,
    /// HTTP client.
    pub http: reqwest::Client,
    /// The PC's Thrall data folder (pictures to show, the browser's profile).
    pub data_dir: Option<std::path::PathBuf>,
    /// The PC's browser (making PDFs).
    pub browser: Arc<crate::browser::Browsers>,
    /// The PC's Thrall folder: a session's folder inside it is made when first used (Home).
    pub home: Option<std::path::PathBuf>,
}

/// The companion PC's side: runs the calls the engine sends, one context per session.
pub struct Runner {
    base: Base,
    /// Files read per session, for the edit guard.
    reads: Mutex<HashMap<String, ReadLog>>,
    /// Calls running now, so Stop reaches them.
    running: Mutex<HashMap<String, Interrupter>>,
}

impl Runner {
    /// A runner with `base`.
    pub fn new(base: Base) -> Self {
        Self {
            base,
            reads: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
        }
    }

    fn reads_for(&self, session: &str) -> ReadLog {
        let mut reads = self.reads.lock().unwrap_or_else(|p| p.into_inner());
        reads.entry(session.to_owned()).or_default().clone()
    }

    fn context(
        &self,
        workspace: &Path,
        person: &str,
        session: &str,
        output: mpsc::UnboundedSender<String>,
        cancel: Cancel,
    ) -> ToolContext {
        ToolContext {
            workspace: workspace.to_path_buf(),
            store: self.base.store.clone(),
            user_id: person.to_owned(),
            session_id: session.to_owned(),
            http: self.base.http.clone(),
            output,
            reads: self.reads_for(session),
            cancel,
            data_dir: self.base.data_dir.clone(),
            mcp: None,
            memory: None,
            browser: self.base.browser.clone(),
            disabled_skills: Vec::new(),
        }
    }

    fn tool(&self, name: &str) -> Result<Arc<dyn Tool>, String> {
        if !runs_on_pc(name) {
            return Err(format!("{name} doesn't run on a companion PC"));
        }
        self.base
            .tools
            .iter()
            .find(|t| t.name() == name)
            .cloned()
            .ok_or_else(|| format!("this PC's Thrall doesn't have {name}; update it"))
    }

    /// Handle call `id` for `person`'s `session`, working in `workspace` (on this PC). Live
    /// output (a command's lines) goes to `output`. The answer is JSON for the engine.
    pub async fn handle(
        &self,
        id: &str,
        call: Call,
        workspace: &Path,
        person: &str,
        session: &str,
        output: mpsc::UnboundedSender<String>,
    ) -> Result<Value, String> {
        if call.is_folder_call() {
            return Err("the PC's link answers folder calls".to_owned());
        }
        if !workspace.is_dir()
            && let Some(home) = &self.base.home
            && workspace.starts_with(home)
        {
            let _ = std::fs::create_dir_all(workspace);
        }
        if !matches!(call, Call::Stop { .. }) && !workspace.is_dir() {
            return Err(format!(
                "the folder {} isn't on this PC (any more)",
                workspace.display()
            ));
        }
        match call {
            Call::Check { tool, args } => {
                let tool = self.tool(&tool)?;
                let ctx = self.context(workspace, person, session, output, Cancel::never());
                let precheck = tool.precheck(&ctx, &args);
                tool.prepare(&ctx, &args).await;
                let effect = tool.effect(&ctx, &args);
                serde_json::to_value(Checked { precheck, effect }).map_err(|e| e.to_string())
            }
            Call::Run { tool, args } => {
                let tool = self.tool(&tool)?;
                let (stop, cancel) = Cancel::pair();
                self.running
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(id.to_owned(), stop);
                let ctx = self.context(workspace, person, session, output, cancel);
                let out = tool.run(&ctx, args).await;
                self.running
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(id);
                let ran = Ran::from_output(out, self.base.data_dir.as_deref());
                serde_json::to_value(ran).map_err(|e| e.to_string())
            }
            Call::Stop { call } => {
                if let Some(stop) = self
                    .running
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .get(&call)
                {
                    stop.interrupt();
                }
                Ok(Value::Null)
            }
            Call::Read { path } => {
                let path = crate::paths::resolve(workspace, &path);
                match tokio::fs::metadata(&path).await {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        Ok(serde_json::json!({ "missing": true }))
                    }
                    Err(e) => Err(e.to_string()),
                    Ok(meta) if meta.len() > COMPANION_FILE_MAX_BYTES => {
                        Ok(serde_json::json!({ "too_big": true }))
                    }
                    Ok(_) => {
                        let bytes = tokio::fs::read(&path).await.map_err(|e| e.to_string())?;
                        Ok(serde_json::json!({ "data": BASE64.encode(bytes) }))
                    }
                }
            }
            Call::List { .. }
            | Call::Part { .. }
            | Call::Places { .. }
            | Call::MakeFolder { .. } => Err("the PC's link answers folder calls".to_owned()),
            Call::Write { path, data } => {
                let bytes = BASE64.decode(data).map_err(|e| e.to_string())?;
                let path = crate::paths::resolve(workspace, &path);
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                tokio::fs::write(&path, bytes)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(Value::Null)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_and_effects_cross_the_link_unchanged() {
        let call = Call::Run {
            tool: "shell".into(),
            args: serde_json::json!({ "command": "dir" }),
        };
        let text = serde_json::to_string(&call).unwrap_or_default();
        assert!(text.contains(r#""phase":"run""#), "{text}");
        assert_eq!(serde_json::from_str::<Call>(&text).ok(), Some(call));
        let checked = Checked {
            precheck: None,
            effect: Effect::Run {
                summary: "Run `dir`".into(),
                risk: thrall_protocol::Risk::Medium,
                grant: Some("shell:dir".into()),
            },
        };
        let text = serde_json::to_string(&checked).unwrap_or_default();
        assert_eq!(serde_json::from_str::<Checked>(&text).ok(), Some(checked));
    }

    #[test]
    fn only_pc_tools_run_on_a_pc() {
        assert!(runs_on_pc("edit_file") && runs_on_pc("shell") && runs_on_pc("read_document"));
        assert!(
            !runs_on_pc("web_search") && !runs_on_pc("memory_save") && !runs_on_pc("make_image")
        );
    }
}
