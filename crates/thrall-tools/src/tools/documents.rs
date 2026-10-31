//! `read_document`: the text of a PDF, Word, PowerPoint or spreadsheet file.

use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::constants::{DOCUMENT_MAX_BYTES, RESULT_MAX_CHARS};
use crate::documents::{self, Kind};
use crate::paths::{display, resolve};
use crate::tool::{Tool, ToolContext, ToolOutput, arg_str, parse_args};

/// Read a rich document as text.
pub struct ReadDocument;

#[derive(Deserialize)]
struct Args {
    path: String,
    /// Character to start from, for documents too long to read in one go.
    #[serde(default)]
    offset: Option<usize>,
}

impl Tool for ReadDocument {
    fn name(&self) -> &'static str {
        "read_document"
    }
    fn description(&self) -> &'static str {
        "Read the text of a PDF, Word (.docx), PowerPoint (.pptx) or spreadsheet (.xlsx, .xls, .ods) \
         file. Pages, slides and sheets are marked. Long documents come in parts: pass offset to \
         read on. Use read_file for plain text files."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Document to read" },
                "offset": { "type": "integer", "description": "Character to start from (for long documents)" }
            },
            "required": ["path"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Reading {}", arg_str(args, "path"))
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: Args = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let path = resolve(&ctx.workspace, &a.path);
            let shown = display(&ctx.workspace, &path);
            if Kind::of(&path).is_none() {
                return ToolOutput::error(format!(
                    "{shown} isn't a PDF, Word, PowerPoint or spreadsheet file; use read_file"
                ));
            }
            match tokio::fs::metadata(&path).await {
                Ok(m) if m.len() > DOCUMENT_MAX_BYTES => {
                    return ToolOutput::error(format!(
                        "{shown} is {} MB, too big to read",
                        m.len() / (1024 * 1024)
                    ));
                }
                Ok(_) => {}
                Err(e) => return ToolOutput::error(format!("can't read {shown}: {e}")),
            }
            let owned = path.clone();
            let doc = match tokio::task::spawn_blocking(move || documents::extract(&owned)).await {
                Ok(Ok(doc)) => doc,
                Ok(Err(e)) => return ToolOutput::error(format!("{shown}: {e}")),
                Err(e) => return ToolOutput::error(format!("{shown}: reading stopped ({e})")),
            };
            ctx.note_read(&path);
            let total = doc.text.chars().count();
            let start = a.offset.unwrap_or(0).min(total);
            let part: String = doc
                .text
                .chars()
                .skip(start)
                .take(RESULT_MAX_CHARS)
                .collect();
            let end = start + part.chars().count();
            let mut out = format!("{} · {}\n\n{part}", doc.kind.label(), doc.detail);
            if doc.scanned {
                out.push_str(
                    "\n\n(This PDF is mostly pictures of pages, so its text couldn't be pulled out. If the user \
                     attaches it to a message, Thrall's picture reader transcribes it.)",
                );
            }
            if end < total {
                out.push_str(&format!(
                    "\n\n… {} more characters (read on with offset {end})",
                    total - end
                ));
            }
            ToolOutput::ok(out, doc.detail)
        })
    }
}
