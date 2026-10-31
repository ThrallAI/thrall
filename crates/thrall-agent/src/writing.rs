//! The humaniser (`writing.humanise`): writing rules in the system prompt when it's on, and on
//! full, a style check of every document Thrall writes, added to that tool's result so Thrall can
//! fix what it finds. The check itself is `thrall_tools::style` (no model call).

use std::path::Path;

use serde_json::Value;
use thrall_core::config::Humanise;

use crate::constants::{
    CHECKED_WRITING, PROSE_EXTENSIONS, WRITING_CHECK_NOTE, WRITING_RULES, WRITING_RULES_FULL,
};

/// What the humaniser adds to the system prompt (nothing when it's off).
pub fn rules(humanise: Humanise) -> Option<String> {
    match humanise {
        Humanise::Off => None,
        Humanise::Light => Some(WRITING_RULES.to_owned()),
        Humanise::Full => Some(format!("{WRITING_RULES}\n{WRITING_RULES_FULL}")),
    }
}

/// The prose a document tool call is about to write, if it writes prose: the text of a markdown,
/// text, Word or PDF document (never code).
pub fn written_text(tool: &str, args: &Value, workspace: &Path) -> Option<String> {
    let (_, field) = CHECKED_WRITING.iter().find(|(name, _)| *name == tool)?;
    if matches!(tool, "write_file" | "edit_file") {
        let path = args.get("path").and_then(Value::as_str)?;
        let ext = Path::new(path)
            .extension()?
            .to_string_lossy()
            .to_lowercase();
        if !PROSE_EXTENSIONS.contains(&ext.as_str()) {
            return None;
        }
    }
    if let Some(text) = args.get(*field).and_then(Value::as_str) {
        return Some(text.to_owned());
    }
    // make_pdf can lay out a markdown file instead.
    let source = args.get("source").and_then(Value::as_str)?;
    std::fs::read_to_string(thrall_tools::paths::resolve(workspace, source)).ok()
}

/// The note to add to the tool's result, when the check finds something.
pub fn review(text: &str) -> Option<String> {
    let report = thrall_tools::style::check(text);
    (!report.is_clean()).then(|| format!("{WRITING_CHECK_NOTE}\n{}", report.render()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rules_follow_the_setting() {
        assert!(rules(Humanise::Off).is_none());
        let light = rules(Humanise::Light).unwrap();
        let full = rules(Humanise::Full).unwrap();
        assert!(!light.contains("on full") && full.contains("on full"));
    }

    #[test]
    fn only_prose_is_checked() {
        let ws = Path::new(".");
        let md = json!({ "path": "notes/Plan.MD", "content": "A pivotal moment." });
        assert_eq!(
            written_text("write_file", &md, ws).unwrap(),
            "A pivotal moment."
        );
        let code = json!({ "path": "src/main.rs", "content": "// a pivotal moment" });
        assert!(written_text("write_file", &code, ws).is_none());
        let edit = json!({ "path": "README.md", "old_string": "a", "new_string": "b" });
        assert_eq!(written_text("edit_file", &edit, ws).unwrap(), "b");
        let word = json!({ "path": "r.docx", "content": "Hi" });
        assert_eq!(written_text("edit_word", &word, ws).unwrap(), "Hi");
        assert!(written_text("shell", &json!({ "command": "ls" }), ws).is_none());
    }

    #[test]
    fn clean_writing_adds_nothing() {
        assert!(review("The bridge opened in 1932.").is_none());
        let note = review("Great question! The bridge stands as a testament to grit.").unwrap();
        assert!(note.starts_with(WRITING_CHECK_NOTE) && note.contains("chatbot talk"));
    }
}
