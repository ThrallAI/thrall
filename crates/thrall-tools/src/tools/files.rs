//! File tools: read, write, edit, list. Paths are relative to the workspace unless absolute.

use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::constants::{
    EDIT_MAX_CHANGES, EDIT_SHOWN_MAX_LINES, LINE_MAX_CHARS, LIST_MAX_ENTRIES, READ_DEFAULT_LINES,
    READ_MAX_BYTES, RESULT_MAX_CHARS,
};
use thrall_gates::Effect;

use crate::diff;
use crate::paths::{display, resolve};
use crate::tool::{Tool, ToolContext, ToolOutput, arg_str, cap, parse_args};

// ---------- read_file ----------

/// Read a text file, with line numbers.
pub struct ReadFile;

#[derive(Deserialize)]
struct ReadArgs {
    path: String,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "read_file"
    }
    fn description(&self) -> &'static str {
        "Read a text file. Returns numbered lines. Use offset (1-based line) and limit to read \
         part of a long file. Paths are relative to the session folder unless absolute."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File to read" },
                "offset": { "type": "integer", "description": "First line to read (1-based)" },
                "limit": { "type": "integer", "description": "Most lines to read" }
            },
            "required": ["path"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Reading {}", arg_str(args, "path"))
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: ReadArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let path = resolve(&ctx.workspace, &a.path);
            let shown = display(&ctx.workspace, &path);
            match tokio::fs::metadata(&path).await {
                Ok(m) if m.is_dir() => {
                    return ToolOutput::error(format!("{shown} is a folder; use list_dir"));
                }
                Ok(m) if m.len() > READ_MAX_BYTES => {
                    return ToolOutput::error(format!(
                        "{shown} is {} MB, too big to read whole; use grep or offset/limit",
                        m.len() / (1024 * 1024)
                    ));
                }
                Ok(_) => {}
                Err(e) => return ToolOutput::error(format!("can't read {shown}: {e}")),
            }
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => b,
                Err(e) => return ToolOutput::error(format!("can't read {shown}: {e}")),
            };
            let Some(text) = text_of(&bytes) else {
                return ToolOutput::error(format!("{shown} looks like a binary file"));
            };
            let total = text.lines().count();
            let start = a.offset.unwrap_or(1).max(1);
            let limit = a.limit.unwrap_or(READ_DEFAULT_LINES).max(1);
            let mut out = String::new();
            let mut shown_lines = 0;
            for (i, line) in text.lines().enumerate().skip(start - 1).take(limit) {
                let line: String = line.chars().take(LINE_MAX_CHARS).collect();
                out.push_str(&format!("{:>6}\t{line}\n", i + 1));
                shown_lines += 1;
            }
            if total == 0 {
                out.push_str("(empty file)");
            } else if start - 1 + shown_lines < total {
                out.push_str(&format!(
                    "… {} more lines (read with offset {})",
                    total - (start - 1 + shown_lines),
                    start + shown_lines
                ));
            }
            ctx.note_read(&path);
            ToolOutput::ok(
                cap(out, RESULT_MAX_CHARS),
                format!("{shown_lines} of {total} lines"),
            )
        })
    }
}

/// A file's text: UTF-8, or UTF-16 with its byte-order mark (what Windows PowerShell's `>`
/// writes). `None` for anything else holding zero bytes (a binary file).
fn text_of(bytes: &[u8]) -> Option<String> {
    let utf16 = |rest: &[u8], from: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| from(*c)).collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => Some(utf16(rest, u16::from_le_bytes)),
        [0xFE, 0xFF, rest @ ..] => Some(utf16(rest, u16::from_be_bytes)),
        _ if bytes.contains(&0) => None,
        [0xEF, 0xBB, 0xBF, rest @ ..] => Some(String::from_utf8_lossy(rest).into_owned()),
        _ => Some(String::from_utf8_lossy(bytes).into_owned()),
    }
}

/// Why a write or edit to `path` can't happen (it wasn't read, or changed since), checked before
/// anyone is asked about it. Any folder is fine: the gate and the person's rules decide where.
pub(crate) fn write_blocked(ctx: &ToolContext, raw_path: &str) -> Option<String> {
    let path = resolve(&ctx.workspace, raw_path);
    let shown = display(&ctx.workspace, &path);
    ctx.edit_guard(&path, &shown)
}

/// The new content an edit would produce and how many replacements it makes, or why it can't be
/// made. Changes apply in order, each to the text the ones before it left; if any can't, none do.
fn apply_edit(old: &str, a: &EditArgs, shown: &str) -> Result<(String, usize), String> {
    let changes = a.changes()?;
    let mut text = old.to_owned();
    let mut total = 0;
    for (index, c) in changes.iter().enumerate() {
        // Name the change only when there are several, so a single edit reads as it always has.
        let which = if changes.len() > 1 {
            format!("edits[{index}]: ")
        } else {
            String::new()
        };
        if c.old_string.is_empty() {
            return Err(format!(
                "{which}old_string is empty; use write_file to create files"
            ));
        }
        let count = text.matches(c.old_string.as_str()).count();
        if count == 0 {
            let order = if index > 0 {
                " Edits apply in order, so match the text as the earlier ones left it."
            } else {
                ""
            };
            return Err(format!(
                "{which}old_string wasn't found in {shown}. Read the file and copy the text \
                 exactly.{order} Nothing was changed."
            ));
        }
        if count > 1 && !c.replace_all {
            return Err(format!(
                "{which}old_string appears {count} times in {shown}. Include more surrounding \
                 text to make it unique, or set replace_all. Nothing was changed."
            ));
        }
        text = if c.replace_all {
            text.replace(c.old_string.as_str(), &c.new_string)
        } else {
            text.replacen(c.old_string.as_str(), &c.new_string, 1)
        };
        total += count;
    }
    Ok((text, total))
}

// ---------- write_file ----------

/// Create or overwrite a file.
pub struct WriteFile;

#[derive(Deserialize)]
struct WriteArgs {
    path: String,
    content: String,
}

impl Tool for WriteFile {
    fn name(&self) -> &'static str {
        "write_file"
    }
    fn description(&self) -> &'static str {
        "Create a file, or replace a file's whole content. Creates missing folders. Prefer \
         edit_file for changes to existing files."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string", "description": "The complete new content" }
            },
            "required": ["path", "content"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Writing {}", arg_str(args, "path"))
    }
    fn precheck(&self, ctx: &ToolContext, args: &Value) -> Option<String> {
        write_blocked(ctx, arg_str(args, "path"))
    }
    fn effect(&self, ctx: &ToolContext, args: &Value) -> Effect {
        let path = resolve(&ctx.workspace, arg_str(args, "path"));
        let shown = display(&ctx.workspace, &path);
        let old = std::fs::read_to_string(&path).ok();
        let new = arg_str(args, "content");
        Effect::Edit {
            diff: Some(diff::make(&shown, old.as_deref(), new)),
            path: shown,
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: WriteArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            if let Some(why) = write_blocked(ctx, &a.path) {
                return ToolOutput::error(why);
            }
            let path = resolve(&ctx.workspace, &a.path);
            let shown = display(&ctx.workspace, &path);
            let old = tokio::fs::read_to_string(&path).await.ok();
            if let Some(parent) = path.parent()
                && let Err(e) = tokio::fs::create_dir_all(parent).await
            {
                return ToolOutput::error(format!("can't create the folder for {shown}: {e}"));
            }
            if let Err(e) = tokio::fs::write(&path, &a.content).await {
                return ToolOutput::error(format!("can't write {shown}: {e}"));
            }
            ctx.note_read(&path);
            let d = diff::make(&shown, old.as_deref(), &a.content);
            let verb = if old.is_some() { "Updated" } else { "Created" };
            let summary = format!("{verb} {shown} (+{} −{})", d.added, d.removed);
            let mut out = ToolOutput::ok(format!("{verb} {shown}."), summary);
            out.diff = Some(d);
            out
        })
    }
}

// ---------- edit_file ----------

/// Replace exact text in a file: one change, or several in one call.
pub struct EditFile;

#[derive(Deserialize)]
struct EditArgs {
    path: String,
    #[serde(default)]
    old_string: Option<String>,
    #[serde(default)]
    new_string: Option<String>,
    #[serde(default)]
    replace_all: bool,
    #[serde(default)]
    edits: Vec<Change>,
}

/// One replacement in an edit.
#[derive(Deserialize, Clone)]
struct Change {
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

impl EditArgs {
    /// The changes asked for: the `edits` list, or the single change given directly.
    fn changes(&self) -> Result<Vec<Change>, String> {
        if !self.edits.is_empty() {
            if self.old_string.is_some() || self.new_string.is_some() {
                return Err(
                    "give either old_string and new_string, or a list in edits, not both"
                        .to_owned(),
                );
            }
            if self.edits.len() > EDIT_MAX_CHANGES {
                return Err(format!(
                    "edits has {} changes; at most {EDIT_MAX_CHANGES} go in one call (use \
                     write_file to rewrite the whole file)",
                    self.edits.len()
                ));
            }
            return Ok(self.edits.clone());
        }
        match (&self.old_string, &self.new_string) {
            (Some(old), Some(new)) => Ok(vec![Change {
                old_string: old.clone(),
                new_string: new.clone(),
                replace_all: self.replace_all,
            }]),
            _ => Err("give old_string and new_string, or a list of changes in edits".to_owned()),
        }
    }
}

impl Tool for EditFile {
    fn name(&self) -> &'static str {
        "edit_file"
    }
    fn description(&self) -> &'static str {
        "Replace exact text in a file. Put every change to the file in one call: list them in \
         edits (applied in order; if any can't be made, none are), or give a single \
         old_string/new_string. Each old_string must match the file exactly (including \
         indentation) and be unique unless replace_all is true. Read the file first. The result \
         shows the changed lines as they now are, so there's no need to read the file again."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "edits": {
                    "type": "array",
                    "description": "Several changes to this file, made together",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_string": { "type": "string", "description": "Exact text to replace" },
                            "new_string": { "type": "string", "description": "Replacement text" },
                            "replace_all": { "type": "boolean", "description": "Replace every occurrence" }
                        },
                        "required": ["old_string", "new_string"]
                    }
                },
                "old_string": { "type": "string", "description": "Exact text to replace (one change)" },
                "new_string": { "type": "string", "description": "Replacement text (one change)" },
                "replace_all": { "type": "boolean", "description": "Replace every occurrence (one change)" }
            },
            "required": ["path"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Editing {}", arg_str(args, "path"))
    }
    fn precheck(&self, ctx: &ToolContext, args: &Value) -> Option<String> {
        if let Some(why) = write_blocked(ctx, arg_str(args, "path")) {
            return Some(why);
        }
        // Refuse an edit that can't apply before asking anyone to approve it.
        let a: EditArgs = serde_json::from_value(args.clone()).ok()?;
        let path = resolve(&ctx.workspace, &a.path);
        let shown = display(&ctx.workspace, &path);
        match std::fs::read_to_string(&path) {
            Ok(old) => apply_edit(&old, &a, &shown).err(),
            Err(e) => Some(format!("can't read {shown}: {e}")),
        }
    }
    fn effect(&self, ctx: &ToolContext, args: &Value) -> Effect {
        let path = resolve(&ctx.workspace, arg_str(args, "path"));
        let shown = display(&ctx.workspace, &path);
        let diff = serde_json::from_value::<EditArgs>(args.clone())
            .ok()
            .and_then(|a| {
                let old = std::fs::read_to_string(&path).ok()?;
                let (new, _) = apply_edit(&old, &a, &shown).ok()?;
                Some(diff::make(&shown, Some(&old), &new))
            });
        Effect::Edit { path: shown, diff }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: EditArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            if let Some(why) = write_blocked(ctx, &a.path) {
                return ToolOutput::error(why);
            }
            let path = resolve(&ctx.workspace, &a.path);
            let shown = display(&ctx.workspace, &path);
            let old = match tokio::fs::read_to_string(&path).await {
                Ok(t) => t,
                Err(e) => return ToolOutput::error(format!("can't read {shown}: {e}")),
            };
            let (new, count) = match apply_edit(&old, &a, &shown) {
                Ok(r) => r,
                Err(e) => return ToolOutput::error(e),
            };
            if let Err(e) = tokio::fs::write(&path, &new).await {
                return ToolOutput::error(format!("can't write {shown}: {e}"));
            }
            ctx.note_read(&path);
            let d = diff::make(&shown, Some(&old), &new);
            let summary = format!("Edited {shown} (+{} −{})", d.added, d.removed);
            // The changed lines as they now read, so the model needn't read the file back.
            let mut out = ToolOutput::ok(
                format!(
                    "Edited {shown} ({count} replacement{}). The changed lines now read:\n{}",
                    if count == 1 { "" } else { "s" },
                    diff::new_lines(&d.unified, EDIT_SHOWN_MAX_LINES)
                ),
                summary,
            );
            out.diff = Some(d);
            out
        })
    }
}

// ---------- list_dir ----------

/// List a folder.
pub struct ListDir;

#[derive(Deserialize)]
struct ListArgs {
    #[serde(default)]
    path: Option<String>,
}

impl Tool for ListDir {
    fn name(&self) -> &'static str {
        "list_dir"
    }
    fn description(&self) -> &'static str {
        "List the files and folders in a folder (the session folder if no path is given)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "path": { "type": "string" } }
        })
    }
    fn title(&self, args: &Value) -> String {
        match arg_str(args, "path") {
            "" => "Listing the session folder".to_owned(),
            p => format!("Listing {p}"),
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: ListArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let path = resolve(&ctx.workspace, a.path.as_deref().unwrap_or("."));
            let shown = display(&ctx.workspace, &path);
            let mut dir = match tokio::fs::read_dir(&path).await {
                Ok(d) => d,
                Err(e) => return ToolOutput::error(format!("can't list {shown}: {e}")),
            };
            let mut entries: Vec<(bool, String, u64)> = Vec::new();
            while let Ok(Some(entry)) = dir.next_entry().await {
                let meta = entry.metadata().await.ok();
                let is_dir = meta.as_ref().is_some_and(std::fs::Metadata::is_dir);
                let size = meta.map(|m| m.len()).unwrap_or(0);
                entries.push((
                    is_dir,
                    entry.file_name().to_string_lossy().into_owned(),
                    size,
                ));
            }
            entries.sort_by(|a, b| {
                b.0.cmp(&a.0)
                    .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
            });
            let total = entries.len();
            let mut out = String::new();
            for (is_dir, name, size) in entries.iter().take(LIST_MAX_ENTRIES) {
                if *is_dir {
                    out.push_str(&format!("{name}/\n"));
                } else {
                    out.push_str(&format!("{name}  ({size} bytes)\n"));
                }
            }
            if total > LIST_MAX_ENTRIES {
                out.push_str(&format!("… {} more\n", total - LIST_MAX_ENTRIES));
            }
            if total == 0 {
                out.push_str("(empty folder)");
            }
            ToolOutput::ok(out, format!("{total} entries"))
        })
    }
}

// ---------- show_file ----------

/// Put a file that's already there in front of the person: a card under the step to open it, show
/// it in its folder, download it or open it on the phone.
pub struct ShowFile;

#[derive(Deserialize)]
struct ShowArgs {
    path: String,
}

impl Tool for ShowFile {
    fn name(&self) -> &'static str {
        "show_file"
    }
    fn description(&self) -> &'static str {
        "Show the person a file that's already there (one made earlier, or one they ask to see): \
         it appears as a card in the chat that they can open, show in its folder, download, or \
         open on their phone. Use this whenever they ask to see, open or have a file; never open \
         it in the browser, serve it or screenshot it. Paths are relative to the session folder \
         unless absolute."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File to show" }
            },
            "required": ["path"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Showing {}", arg_str(args, "path"))
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: ShowArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            // A chat that works on a companion PC: the file is there, not on this computer.
            if let Some(pc) = &ctx.pc {
                return ToolOutput::ok(
                    format!(
                        "This chat works on {pc}, so the file is on that PC, not here. Tell the \
                         person where it is ({}) so they can open it there.",
                        a.path
                    ),
                    "On the person's PC".to_owned(),
                );
            }
            let path = resolve(&ctx.workspace, &a.path);
            let shown = display(&ctx.workspace, &path);
            match tokio::fs::metadata(&path).await {
                Ok(m) if m.is_dir() => {
                    ToolOutput::error(format!("{shown} is a folder; show the files in it"))
                }
                Ok(_) => {
                    let mut out = ToolOutput::ok(
                        format!(
                            "{shown} is shown to the person as a card under this step: they can \
                             open it from there. Don't open it any other way."
                        ),
                        format!("Showing {shown}"),
                    );
                    out.saved.push(path);
                    out
                }
                Err(e) => ToolOutput::error(format!("can't show {shown}: {e}")),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::text_of;

    #[test]
    fn reads_utf16_with_its_mark_and_refuses_other_zero_bytes() {
        let le: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("hi\r\nthere".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert_eq!(text_of(&le).unwrap(), "hi\r\nthere");
        let be: Vec<u8> = [0xFE, 0xFF]
            .into_iter()
            .chain("hé".encode_utf16().flat_map(u16::to_be_bytes))
            .collect();
        assert_eq!(text_of(&be).unwrap(), "hé");
        assert_eq!(text_of(b"\xEF\xBB\xBFplain").unwrap(), "plain");
        assert_eq!(text_of(b"plain").unwrap(), "plain");
        assert!(text_of(b"PK\x03\x04\x00\x00").is_none());
    }
}
