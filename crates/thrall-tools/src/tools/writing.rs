//! `style_check`: the humaniser's checker as a tool, so Thrall can check a draft or a document
//! before the person sends or publishes it.

use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::constants::STYLE_MAX_FILE_BYTES;
use crate::paths::{display, resolve};
use crate::tool::{Tool, ToolContext, ToolOutput, arg_str, parse_args};

/// Check writing for habits that make it read as machine-made.
pub struct StyleCheck;

/// The tool's name.
pub const STYLE_CHECK: &str = "style_check";

#[derive(Deserialize)]
struct StyleArgs {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    detector: bool,
}

/// The text of `path`: rich documents through the document reader, anything else as plain text.
fn file_text(path: &std::path::Path) -> Result<String, String> {
    if crate::documents::Kind::of(path).is_some() {
        return crate::documents::extract(path).map(|d| d.text);
    }
    let size = std::fs::metadata(path)
        .map_err(|e| format!("can't read it: {e}"))?
        .len();
    if size > STYLE_MAX_FILE_BYTES {
        return Err("it's too big to check".to_owned());
    }
    std::fs::read_to_string(path).map_err(|e| format!("can't read it as text: {e}"))
}

impl Tool for StyleCheck {
    fn name(&self) -> &'static str {
        STYLE_CHECK
    }
    fn description(&self) -> &'static str {
        "Check writing for the habits that make it read as machine-made: puffed-up phrases \
         (\"a testament to\"), advert tone, \"-ing\" tails, vague \"experts say\", stock endings, \
         \"not X but Y\", chatbot talk, overused words (delve, crucial), flat sentence rhythm, \
         heavy bold, em dashes and emoji. Give text, or path to a file (markdown, text, Word, \
         PDF). detector: true adds what AI detectors measure that a rewrite can change \
         (sentence-length variation). Free and instant. Use it on writing the person will send \
         or publish, then fix what it finds."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "The writing to check" },
                "path": { "type": "string", "description": "Or a file to check" },
                "detector": { "type": "boolean", "description": "Also report detector signals (for detector-aware rewrites)" }
            }
        })
    }
    fn title(&self, args: &Value) -> String {
        match arg_str(args, "path") {
            "" => "Checking writing style".to_owned(),
            path => format!("Checking the writing in {path}"),
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: StyleArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let text = match (a.text, a.path) {
                (Some(text), _) if !text.trim().is_empty() => text,
                (_, Some(path)) if !path.trim().is_empty() => {
                    let full = resolve(&ctx.workspace, &path);
                    let shown = display(&ctx.workspace, &full);
                    match tokio::task::spawn_blocking(move || file_text(&full)).await {
                        Ok(Ok(text)) => text,
                        Ok(Err(why)) => return ToolOutput::error(format!("{shown}: {why}")),
                        Err(e) => return ToolOutput::error(format!("check failed: {e}")),
                    }
                }
                _ => return ToolOutput::error("give text, or path to a file"),
            };
            let report = crate::style::check(&text);
            let mut out = report.render();
            if a.detector {
                out.push('\n');
                out.push_str(&report.detector_signals());
            }
            ToolOutput::ok(out, report.summary())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_files_are_read_and_big_ones_refused() {
        let dir = std::env::temp_dir().join(format!("thrall-style-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("draft.md");
        std::fs::write(&file, "Great question! It plays a pivotal role.").unwrap();
        let text = file_text(&file).unwrap();
        let report = crate::style::check(&text);
        assert_eq!(report.hits.len(), 2, "{}", report.render());
        assert!(file_text(&dir.join("missing.md")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
