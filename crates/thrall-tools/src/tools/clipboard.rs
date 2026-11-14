//! `clipboard`: read or set the clipboard (text or a picture), and keep named snippets.

use std::collections::BTreeMap;
use std::path::PathBuf;

use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use thrall_gates::Effect;
use thrall_protocol::Risk;

use crate::constants::{CLIPBOARD_MAX_CHARS, SNIPPETS_FILE};
use crate::paths::{display, resolve};
use crate::saved;
use crate::tool::{Tool, ToolContext, ToolOutput, arg_str, cap, parse_args};

/// Use the clipboard.
pub struct Clipboard;

#[derive(Deserialize)]
struct Args {
    action: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

const ACTIONS: &[&str] = &["read", "write", "save", "load", "list", "delete"];

/// What's on the clipboard.
enum Content {
    Text(String),
    Image(u32, u32, Vec<u8>),
    Empty,
}

fn read_clipboard() -> Result<Content, String> {
    let mut board =
        arboard::Clipboard::new().map_err(|e| format!("can't open the clipboard: {e}"))?;
    if let Ok(text) = board.get_text()
        && !text.is_empty()
    {
        return Ok(Content::Text(text));
    }
    match board.get_image() {
        Ok(img) => Ok(Content::Image(
            u32::try_from(img.width).unwrap_or(0),
            u32::try_from(img.height).unwrap_or(0),
            img.bytes.into_owned(),
        )),
        Err(_) => Ok(Content::Empty),
    }
}

fn snippets_path(ctx: &ToolContext) -> Result<PathBuf, String> {
    ctx.data_dir
        .as_ref()
        .map(|d| d.join(SNIPPETS_FILE))
        .ok_or_else(|| "there's no data folder to keep snippets in".to_owned())
}

fn load_snippets(ctx: &ToolContext) -> Result<BTreeMap<String, String>, String> {
    let path = snippets_path(ctx)?;
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|e| format!("the snippets file is damaged: {e}"))
        }
        Err(_) => Ok(BTreeMap::new()),
    }
}

fn store_snippets(ctx: &ToolContext, snippets: &BTreeMap<String, String>) -> Result<(), String> {
    let path = snippets_path(ctx)?;
    let text = serde_json::to_string_pretty(snippets).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("couldn't save snippets: {e}"))
}

impl Tool for Clipboard {
    fn name(&self) -> &'static str {
        "clipboard"
    }
    fn description(&self) -> &'static str {
        "The user's clipboard. action: read (text, or a picture which is saved and shown — look \
         at it with view_image), write (text, or path to a PNG to copy a picture), save (keep \
         the current clipboard text as a named snippet: name), load (put a snippet on the \
         clipboard: name), list (snippet names), delete (a snippet: name)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ACTIONS },
                "text": { "type": "string", "description": "For write: the text to copy" },
                "path": { "type": "string", "description": "For write: a PNG picture to copy instead of text" },
                "name": { "type": "string", "description": "Snippet name, for save, load and delete" }
            },
            "required": ["action"]
        })
    }
    fn title(&self, args: &Value) -> String {
        match arg_str(args, "action") {
            "read" => "Reading the clipboard".to_owned(),
            "write" => "Copying to the clipboard".to_owned(),
            "save" => format!("Saving snippet {}", arg_str(args, "name")),
            "load" => format!("Copying snippet {}", arg_str(args, "name")),
            "delete" => format!("Deleting snippet {}", arg_str(args, "name")),
            _ => "Listing snippets".to_owned(),
        }
    }
    fn precheck(&self, _ctx: &ToolContext, args: &Value) -> Option<String> {
        let action = arg_str(args, "action");
        if !ACTIONS.contains(&action) {
            return Some(format!("action must be one of: {}", ACTIONS.join(", ")));
        }
        if matches!(action, "save" | "load" | "delete") && arg_str(args, "name").trim().is_empty() {
            return Some(format!("{action} needs a snippet name"));
        }
        None
    }
    fn effect(&self, _ctx: &ToolContext, args: &Value) -> Effect {
        // The clipboard can hold passwords and is the user's own: reading or changing it asks
        // (with "always" for the session); snippet lists are Thrall's own.
        match arg_str(args, "action") {
            "list" => Effect::Read,
            "read" | "save" => Effect::Run {
                summary: "Read the clipboard".to_owned(),
                risk: Risk::Low,
                grant: Some("clipboard".to_owned()),
                everyday: false,
            },
            "delete" => Effect::Run {
                summary: format!("Delete snippet {}", arg_str(args, "name")),
                risk: Risk::Low,
                grant: Some("clipboard".to_owned()),
                everyday: false,
            },
            _ => Effect::Run {
                summary: "Change the clipboard".to_owned(),
                risk: Risk::Low,
                grant: Some("clipboard".to_owned()),
                everyday: false,
            },
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: Args = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let name = a.name.clone().unwrap_or_default().trim().to_owned();
            match a.action.as_str() {
                "read" => match tokio::task::spawn_blocking(read_clipboard).await {
                    Ok(Ok(Content::Text(text))) => {
                        let chars = text.chars().count();
                        ToolOutput::ok(
                            format!(
                                "The clipboard holds text:\n```\n{}\n```",
                                cap(text, CLIPBOARD_MAX_CHARS)
                            ),
                            format!("{chars} characters of text"),
                        )
                    }
                    Ok(Ok(Content::Image(w, h, rgba))) => {
                        let encoded = match saved::png(w, h, &rgba) {
                            Ok(b) => b,
                            Err(e) => {
                                return ToolOutput::error(format!(
                                    "couldn't keep the picture: {e}"
                                ));
                            }
                        };
                        match saved::save(ctx, "clipboard.png", &encoded) {
                            Ok((path, file)) => {
                                let mut out = ToolOutput::ok(
                                    format!(
                                        "The clipboard holds a {w}×{h} picture, saved at {} and shown to the user. Use view_image on it to see what's in it.",
                                        path.display()
                                    ),
                                    format!("A {w}×{h} picture"),
                                );
                                out.files = vec![file];
                                out
                            }
                            Err(e) => ToolOutput::error(e),
                        }
                    }
                    Ok(Ok(Content::Empty)) => ToolOutput::ok(
                        "The clipboard is empty (or holds something other than text or a picture).",
                        "Empty",
                    ),
                    Ok(Err(e)) => ToolOutput::error(e),
                    Err(e) => ToolOutput::error(format!("reading stopped: {e}")),
                },
                "write" => {
                    let result = if let Some(raw) = a.path.as_deref().filter(|p| !p.is_empty()) {
                        let path = resolve(&ctx.workspace, raw);
                        let shown = display(&ctx.workspace, &path);
                        let bytes = match std::fs::read(&path) {
                            Ok(b) => b,
                            Err(e) => return ToolOutput::error(format!("can't read {shown}: {e}")),
                        };
                        let (w, h, rgba) = match saved::decode_png(&bytes) {
                            Ok(p) => p,
                            Err(e) => {
                                return ToolOutput::error(format!(
                                    "{shown} isn't a PNG that can be copied: {e}"
                                ));
                            }
                        };
                        tokio::task::spawn_blocking(move || {
                            let mut board = arboard::Clipboard::new().map_err(|e| e.to_string())?;
                            board
                                .set_image(arboard::ImageData {
                                    width: w as usize,
                                    height: h as usize,
                                    bytes: rgba.into(),
                                })
                                .map_err(|e| e.to_string())
                                .map(|()| {
                                    (
                                        format!("Copied the picture {shown} to the clipboard."),
                                        format!("Copied {shown}"),
                                    )
                                })
                        })
                        .await
                    } else {
                        let Some(text) = a.text.clone() else {
                            return ToolOutput::error("write needs text (or path to a PNG)");
                        };
                        let chars = text.chars().count();
                        tokio::task::spawn_blocking(move || {
                            let mut board = arboard::Clipboard::new().map_err(|e| e.to_string())?;
                            board.set_text(text).map_err(|e| e.to_string()).map(|()| {
                                (
                                    "Copied the text to the clipboard.".to_owned(),
                                    format!("Copied {chars} characters"),
                                )
                            })
                        })
                        .await
                    };
                    match result {
                        Ok(Ok((content, summary))) => ToolOutput::ok(content, summary),
                        Ok(Err(e)) => ToolOutput::error(format!("couldn't set the clipboard: {e}")),
                        Err(e) => ToolOutput::error(format!("stopped: {e}")),
                    }
                }
                "save" => {
                    let text = match tokio::task::spawn_blocking(read_clipboard).await {
                        Ok(Ok(Content::Text(t))) => t,
                        Ok(Ok(_)) => {
                            return ToolOutput::error("the clipboard doesn't hold text to save");
                        }
                        Ok(Err(e)) => return ToolOutput::error(e),
                        Err(e) => return ToolOutput::error(format!("stopped: {e}")),
                    };
                    let mut snippets = match load_snippets(ctx) {
                        Ok(s) => s,
                        Err(e) => return ToolOutput::error(e),
                    };
                    let chars = text.chars().count();
                    snippets.insert(name.clone(), text);
                    match store_snippets(ctx, &snippets) {
                        Ok(()) => ToolOutput::ok(
                            format!(
                                "Saved the clipboard as snippet `{name}` ({chars} characters)."
                            ),
                            format!("Saved {name}"),
                        ),
                        Err(e) => ToolOutput::error(e),
                    }
                }
                "load" => {
                    let snippets = match load_snippets(ctx) {
                        Ok(s) => s,
                        Err(e) => return ToolOutput::error(e),
                    };
                    let Some(text) = snippets.get(&name).cloned() else {
                        let names: Vec<&String> = snippets.keys().collect();
                        return ToolOutput::error(format!(
                            "no snippet called {name}. Saved: {names:?}"
                        ));
                    };
                    match tokio::task::spawn_blocking(move || {
                        arboard::Clipboard::new()
                            .and_then(|mut b| b.set_text(text))
                            .map_err(|e| e.to_string())
                    })
                    .await
                    {
                        Ok(Ok(())) => ToolOutput::ok(
                            format!("Snippet `{name}` is on the clipboard."),
                            format!("Copied {name}"),
                        ),
                        Ok(Err(e)) => ToolOutput::error(format!("couldn't set the clipboard: {e}")),
                        Err(e) => ToolOutput::error(format!("stopped: {e}")),
                    }
                }
                "delete" => {
                    let mut snippets = match load_snippets(ctx) {
                        Ok(s) => s,
                        Err(e) => return ToolOutput::error(e),
                    };
                    if snippets.remove(&name).is_none() {
                        return ToolOutput::error(format!("no snippet called {name}"));
                    }
                    match store_snippets(ctx, &snippets) {
                        Ok(()) => ToolOutput::ok(
                            format!("Deleted snippet `{name}`."),
                            format!("Deleted {name}"),
                        ),
                        Err(e) => ToolOutput::error(e),
                    }
                }
                _ => match load_snippets(ctx) {
                    Ok(s) if s.is_empty() => ToolOutput::ok("No saved snippets.", "No snippets"),
                    Ok(s) => {
                        let list: String = s
                            .iter()
                            .map(|(k, v)| {
                                let preview: String = v.chars().take(60).collect();
                                format!("- {k}: {}\n", preview.replace('\n', " "))
                            })
                            .collect();
                        ToolOutput::ok(
                            format!("Saved snippets:\n{list}"),
                            format!("{} snippets", s.len()),
                        )
                    }
                    Err(e) => ToolOutput::error(e),
                },
            }
        })
    }
}
