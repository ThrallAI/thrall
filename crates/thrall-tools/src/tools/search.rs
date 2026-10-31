//! Finding files and text: glob and grep. Both respect .gitignore and skip hidden files.

use std::path::{Path, PathBuf};

use futures::future::BoxFuture;
use globset::GlobBuilder;
use ignore::WalkBuilder;
use regex::RegexBuilder;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::constants::{
    GLOB_MAX_RESULTS, GREP_MAX_FILE_BYTES, GREP_MAX_MATCHES, LINE_MAX_CHARS, RESULT_MAX_CHARS,
};
use crate::paths::{display, resolve};
use crate::tool::{Tool, ToolContext, ToolOutput, arg_str, cap, parse_args};

/// Files under `root`, honouring .gitignore. Runs on a blocking thread (it walks the disk).
fn walk(root: &Path) -> Vec<PathBuf> {
    WalkBuilder::new(root)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(ignore::DirEntry::into_path)
        .collect()
}

// ---------- glob ----------

/// Find files by name pattern.
pub struct Glob;

#[derive(Deserialize)]
struct GlobArgs {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
}

impl Tool for Glob {
    fn name(&self) -> &'static str {
        "glob"
    }
    fn description(&self) -> &'static str {
        "Find files by path pattern, e.g. \"**/*.rs\" or \"src/**/test_*.py\". Skips files \
         ignored by git. Returns paths relative to the search folder."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string" },
                "path": { "type": "string", "description": "Folder to search (default: session folder)" }
            },
            "required": ["pattern"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Finding {}", arg_str(args, "pattern"))
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: GlobArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let matcher = match GlobBuilder::new(&a.pattern).literal_separator(true).build() {
                Ok(g) => g.compile_matcher(),
                Err(e) => return ToolOutput::error(format!("bad pattern: {e}")),
            };
            let root = resolve(&ctx.workspace, a.path.as_deref().unwrap_or("."));
            let walk_root = root.clone();
            let files = match tokio::task::spawn_blocking(move || walk(&walk_root)).await {
                Ok(f) => f,
                Err(e) => return ToolOutput::error(format!("search failed: {e}")),
            };
            let mut hits: Vec<String> = files
                .iter()
                .filter_map(|p| p.strip_prefix(&root).ok())
                .filter(|rel| matcher.is_match(rel))
                .map(|rel| rel.to_string_lossy().replace('\\', "/"))
                .collect();
            hits.sort();
            let total = hits.len();
            let mut out = hits
                .into_iter()
                .take(GLOB_MAX_RESULTS)
                .collect::<Vec<_>>()
                .join("\n");
            if total > GLOB_MAX_RESULTS {
                out.push_str(&format!("\n… {} more", total - GLOB_MAX_RESULTS));
            }
            if total == 0 {
                out = "No files matched.".to_owned();
            }
            ToolOutput::ok(out, format!("{total} files"))
        })
    }
}

// ---------- grep ----------

/// Search file contents with a regular expression.
pub struct Grep;

#[derive(Deserialize)]
struct GrepArgs {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    glob: Option<String>,
    #[serde(default)]
    ignore_case: bool,
}

impl Tool for Grep {
    fn name(&self) -> &'static str {
        "grep"
    }
    fn description(&self) -> &'static str {
        "Search file contents with a regular expression. Returns path:line: text for each match. \
         Optionally limit to files matching a glob (e.g. \"*.rs\"). Skips files ignored by git."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regular expression" },
                "path": { "type": "string", "description": "File or folder (default: session folder)" },
                "glob": { "type": "string", "description": "Only files matching this, e.g. *.rs" },
                "ignore_case": { "type": "boolean" }
            },
            "required": ["pattern"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Searching for {}", arg_str(args, "pattern"))
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: GrepArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let re = match RegexBuilder::new(&a.pattern)
                .case_insensitive(a.ignore_case)
                .build()
            {
                Ok(r) => r,
                Err(e) => return ToolOutput::error(format!("bad regular expression: {e}")),
            };
            let filter = match a.glob.as_deref().map(|g| GlobBuilder::new(g).build()) {
                Some(Ok(g)) => Some(g.compile_matcher()),
                Some(Err(e)) => return ToolOutput::error(format!("bad glob: {e}")),
                None => None,
            };
            let root = resolve(&ctx.workspace, a.path.as_deref().unwrap_or("."));
            let workspace = ctx.workspace.clone();
            let result = tokio::task::spawn_blocking(move || {
                let files = if root.is_file() {
                    vec![root.clone()]
                } else {
                    walk(&root)
                };
                let mut out = String::new();
                let mut matches = 0usize;
                let mut files_hit = 0usize;
                'files: for file in files {
                    if let Some(f) = &filter {
                        let name = file.file_name().map(Path::new).unwrap_or(file.as_path());
                        if !f.is_match(name) && !f.is_match(&file) {
                            continue;
                        }
                    }
                    let Ok(meta) = std::fs::metadata(&file) else {
                        continue;
                    };
                    if meta.len() > GREP_MAX_FILE_BYTES {
                        continue;
                    }
                    let Ok(bytes) = std::fs::read(&file) else {
                        continue;
                    };
                    if bytes.contains(&0) {
                        continue;
                    }
                    let text = String::from_utf8_lossy(&bytes);
                    let shown = display(&workspace, &file);
                    let mut hit = false;
                    for (i, line) in text.lines().enumerate() {
                        if re.is_match(line) {
                            hit = true;
                            matches += 1;
                            let line: String =
                                line.trim_end().chars().take(LINE_MAX_CHARS).collect();
                            out.push_str(&format!("{shown}:{}: {line}\n", i + 1));
                            if matches >= GREP_MAX_MATCHES {
                                out.push_str("… stopped at the match limit; narrow the search");
                                if hit {
                                    files_hit += 1;
                                }
                                break 'files;
                            }
                        }
                    }
                    if hit {
                        files_hit += 1;
                    }
                }
                (out, matches, files_hit)
            })
            .await;
            match result {
                Ok((_, 0, _)) => ToolOutput::ok("No matches.", "No matches"),
                Ok((out, matches, files)) => ToolOutput::ok(
                    cap(out, RESULT_MAX_CHARS),
                    format!("{matches} matches in {files} files"),
                ),
                Err(e) => ToolOutput::error(format!("search failed: {e}")),
            }
        })
    }
}
