//! `edit_spreadsheet`, `edit_word`, `edit_pdf` and `make_pdf`: make and change office documents
//! wherever they're asked. Each shows what it will change before asking, like any file edit.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use thrall_gates::Effect;

use crate::constants::{
    CUSTOM_MARGIN_MM, PDF_LAYOUTS, PDF_PREVIEW_DIR, PDF_PREVIEW_KEEP_SECS, PDF_PREVIEW_MAX_PAGES,
    PDF_PREVIEW_SCALE, PICTURES_MARGIN_MM,
};
use crate::diff;
use crate::office::{embed, layout, markdown, pdf, pictures, print, sheets, word};
use crate::paths::{display, resolve};
use crate::saved;
use crate::tool::{Tool, ToolContext, ToolOutput, arg_str, parse_args};
use crate::tools::files::write_blocked;

fn has_ext(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .is_some_and(|e| exts.contains(&e.as_str()))
}

/// Keep `jpeg` as the preview of the PDF named `stem` in `dir`. Without one, the PDF's old preview
/// goes, so nothing shows a picture of an earlier version as this one.
fn keep_preview(dir: &Path, stem: &str, jpeg: Option<Vec<u8>>) -> Option<PathBuf> {
    let file = dir.join(format!("{stem}-preview.jpg"));
    let Some(jpeg) = jpeg else {
        let _ = std::fs::remove_file(&file);
        return None;
    };
    std::fs::create_dir_all(dir).ok()?;
    std::fs::write(&file, jpeg).ok()?;
    Some(file)
}

/// Remove previews in `dir` made more than `keep` ago: an old one may not match its PDF any more.
fn clear_old_previews(dir: &Path, keep: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .ok()
            .filter(std::fs::Metadata::is_file)
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > keep);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Run blocking document work off the async runtime.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| format!("stopped: {e}"))?
}

// ---------- edit_spreadsheet ----------

/// Set cells in an Excel workbook.
pub struct EditSpreadsheet;

#[derive(Deserialize)]
struct SheetArgs {
    path: String,
    cells: serde_json::Map<String, Value>,
}

fn sheet_changes(
    cells: &serde_json::Map<String, Value>,
) -> Result<Vec<sheets::CellChange>, String> {
    if cells.is_empty() {
        return Err("give at least one cell to set, like {\"B2\": 42}".to_owned());
    }
    cells
        .iter()
        .map(|(address, value)| {
            let (sheet, cell) = sheets::parse_address(address)
                .ok_or_else(|| format!("`{address}` isn't a cell like B2 or Sheet2!C5"))?;
            Ok(sheets::CellChange {
                sheet,
                cell,
                value: sheets::CellValue::from_json(value),
            })
        })
        .collect()
}

impl Tool for EditSpreadsheet {
    fn name(&self) -> &'static str {
        "edit_spreadsheet"
    }
    fn description(&self) -> &'static str {
        "Set cells in an Excel workbook (.xlsx), keeping its formatting; creates the workbook or \
         sheet if missing. cells maps addresses to values: {\"B2\": 42, \"Sheet2!A1\": \"Total\", \
         \"C10\": \"=SUM(C2:C9)\", \"D4\": null} (a leading = makes a formula, null empties the \
         cell). Read the workbook with read_document first to see what's there."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The .xlsx file" },
                "cells": {
                    "type": "object",
                    "description": "Address → value. Sheet optional (first sheet). \"=...\" is a formula; null empties.",
                    "additionalProperties": true
                }
            },
            "required": ["path", "cells"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Editing {}", arg_str(args, "path"))
    }
    fn precheck(&self, ctx: &ToolContext, args: &Value) -> Option<String> {
        let path = resolve(&ctx.workspace, arg_str(args, "path"));
        if !has_ext(&path, &["xlsx", "xlsm"]) {
            return Some(
                "only .xlsx (and .xlsm) workbooks can be edited; an .xls or .ods file must be \
                 saved as .xlsx first"
                    .to_owned(),
            );
        }
        write_blocked(ctx, arg_str(args, "path"))
    }
    fn effect(&self, ctx: &ToolContext, args: &Value) -> Effect {
        let path = resolve(&ctx.workspace, arg_str(args, "path"));
        let shown = display(&ctx.workspace, &path);
        let preview = args
            .get("cells")
            .and_then(Value::as_object)
            .ok_or_else(String::new)
            .and_then(sheet_changes)
            .and_then(|changes| sheets::preview(&path, &changes));
        Effect::Edit {
            diff: preview
                .ok()
                .map(|(old, new)| diff::make(&shown, path.exists().then_some(old.as_str()), &new)),
            path: shown,
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: SheetArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let changes = match sheet_changes(&a.cells) {
                Ok(c) => c,
                Err(e) => return ToolOutput::error(e),
            };
            let path = resolve(&ctx.workspace, &a.path);
            let shown = display(&ctx.workspace, &path);
            let existed = path.exists();
            let preview = sheets::preview(&path, &changes).ok();
            let target = path.clone();
            match blocking(move || sheets::apply(&target, &changes)).await {
                Ok(n) => {
                    ctx.note_read(&path);
                    let verb = if existed { "Updated" } else { "Created" };
                    let mut out = ToolOutput::ok(
                        format!(
                            "{verb} {shown}: set {n} cell(s). Formulas are worked out when the file is opened in Excel."
                        ),
                        format!("{verb} {shown} · {n} cell(s)"),
                    );
                    out.diff = preview.map(|(old, new)| {
                        diff::make(&shown, existed.then_some(old.as_str()), &new)
                    });
                    out
                }
                Err(e) => ToolOutput::error(format!("{shown}: {e}")),
            }
        })
    }
}

// ---------- edit_word ----------

/// Make a Word document, or replace text in one.
pub struct EditWord;

#[derive(Deserialize)]
struct WordArgs {
    path: String,
    #[serde(default)]
    content: Option<String>,
}

fn word_replacements(args: &Value) -> Vec<word::Replacement> {
    args.get("replacements")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|r| {
                    Some(word::Replacement {
                        find: r.get("find")?.as_str()?.to_owned(),
                        replace: r.get("replace")?.as_str()?.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Tool for EditWord {
    fn name(&self) -> &'static str {
        "edit_word"
    }
    fn description(&self) -> &'static str {
        "Word documents (.docx). Either make a new one from content — markdown: headings (the \
         first # is the title), paragraphs, bullet and numbered lists, tables, code, quotes, \
         **bold**, *italic*, links — laid out with real Word styles, tables and numbered lists \
         (replaces the file if it exists); or change an existing one with replacements: \
         [{find, replace}] — exact text, layout and formatting kept (read it with read_document \
         first to copy the text exactly). Layout options for a new document: font_size (body \
         text; headings keep their proportions) or sizes {title, heading, subheading, text} \
         exactly — always with the unit the user used, \"12px\" or \"9pt\"; spacing compact|normal; \
         header_footer false to drop the page title and page numbers; paper A4|Letter; \
         orientation; margin_mm. Restyling an existing document isn't supported: make it again \
         from content. For a PDF, use make_pdf."
    }
    fn parameters(&self) -> Value {
        let mut props = layout::schema_properties();
        props.insert(
            "path".into(),
            json!({ "type": "string", "description": "The .docx file" }),
        );
        props.insert(
            "content".into(),
            json!({ "type": "string", "description": "For a new document: its text as markdown" }),
        );
        props.insert(
            "replacements".into(),
            json!({
                "type": "array",
                "description": "For an existing document: exact text to find and what to put instead",
                "items": {
                    "type": "object",
                    "properties": {
                        "find": { "type": "string" },
                        "replace": { "type": "string" }
                    },
                    "required": ["find", "replace"]
                }
            }),
        );
        json!({ "type": "object", "properties": props, "required": ["path"] })
    }
    fn title(&self, args: &Value) -> String {
        if args.get("content").is_some() {
            format!("Writing {}", arg_str(args, "path"))
        } else {
            format!("Editing {}", arg_str(args, "path"))
        }
    }
    fn precheck(&self, ctx: &ToolContext, args: &Value) -> Option<String> {
        let path = resolve(&ctx.workspace, arg_str(args, "path"));
        if !has_ext(&path, &["docx"]) {
            return Some(
                "only .docx documents can be edited (an old .doc must be saved as .docx)"
                    .to_owned(),
            );
        }
        let has_content = args.get("content").and_then(Value::as_str).is_some();
        let has_replacements = !word_replacements(args).is_empty();
        if has_content == has_replacements {
            return Some(
                "give either content (a new document) or replacements (changes to an existing \
                 one)"
                    .to_owned(),
            );
        }
        if has_replacements && !path.exists() {
            return Some(format!(
                "{} doesn't exist; use content to make it",
                display(&ctx.workspace, &path)
            ));
        }
        if let Err(e) = layout::from_args(args) {
            return Some(e);
        }
        write_blocked(ctx, arg_str(args, "path"))
    }
    fn effect(&self, ctx: &ToolContext, args: &Value) -> Effect {
        let path = resolve(&ctx.workspace, arg_str(args, "path"));
        let shown = display(&ctx.workspace, &path);
        let d = match args.get("content").and_then(Value::as_str) {
            Some(content) => {
                let old = crate::documents::extract(&path).ok().map(|d| d.text);
                Some(diff::make(&shown, old.as_deref(), content))
            }
            None => word::preview(&path, &word_replacements(args))
                .ok()
                .map(|(old, new, _)| diff::make(&shown, Some(&old), &new)),
        };
        Effect::Edit {
            diff: d,
            path: shown,
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let replacements = word_replacements(&args);
            let page_layout = match layout::from_args(&args) {
                Ok(l) => l,
                Err(e) => return ToolOutput::error(e),
            };
            let a: WordArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let path = resolve(&ctx.workspace, &a.path);
            let shown = display(&ctx.workspace, &path);
            let target = path.clone();
            let result = match a.content {
                Some(content) => {
                    let existed = path.exists();
                    blocking(move || word::create(&target, &content, &page_layout))
                        .await
                        .map(|()| {
                            let verb = if existed { "Replaced" } else { "Created" };
                            (format!("{verb} {shown}."), format!("{verb} {shown}"))
                        })
                }
                None => blocking(move || word::replace(&target, &replacements))
                    .await
                    .map(|n| {
                        (
                            format!("Made {n} replacement(s) in {shown}."),
                            format!("Edited {shown} · {n} replacement(s)"),
                        )
                    }),
            };
            match result {
                Ok((content, summary)) => {
                    ctx.note_read(&path);
                    ToolOutput::ok(content, summary)
                }
                Err(e) => ToolOutput::error(format!("{shown}: {e}")),
            }
        })
    }
}

// ---------- edit_pdf ----------

/// Merge, split, rotate, stamp or fill in PDFs.
pub struct EditPdf;

#[derive(Deserialize)]
struct PdfArgs {
    action: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    inputs: Vec<String>,
    #[serde(default)]
    pages: Option<String>,
    #[serde(default)]
    degrees: Option<i64>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    position: Option<String>,
    #[serde(default)]
    fields: BTreeMap<String, Value>,
}

const PDF_ACTIONS: &[&str] = &[
    "merge",
    "extract_pages",
    "delete_pages",
    "rotate",
    "stamp",
    "form_fields",
    "fill_form",
];

/// Where a PDF action writes: `output`, else `<name>-edited.pdf` next to the input.
fn pdf_output(ctx: &ToolContext, args: &Value) -> Option<PathBuf> {
    if let Some(out) = args
        .get("output")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Some(resolve(&ctx.workspace, out));
    }
    let input = resolve(&ctx.workspace, args.get("path").and_then(Value::as_str)?);
    let stem = input.file_stem()?.to_string_lossy().into_owned();
    Some(input.with_file_name(format!("{stem}-edited.pdf")))
}

impl Tool for EditPdf {
    fn name(&self) -> &'static str {
        "edit_pdf"
    }
    fn description(&self) -> &'static str {
        "Change PDFs without retyping them. action: merge (inputs, output), extract_pages / \
         delete_pages (path, pages like \"1-3,5\"), rotate (path, degrees 90/180/270, pages \
         optional), stamp (path, text, position top|bottom, pages optional), form_fields (path; \
         lists the fields), fill_form (path, fields: {name: value}). Writes output, or \
         <name>-edited.pdf next to the original. Text in a PDF can't be rewritten in place: to \
         change wording, read it and make a new document."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": PDF_ACTIONS },
                "path": { "type": "string", "description": "The PDF to change" },
                "inputs": { "type": "array", "items": { "type": "string" }, "description": "For merge: the PDFs, in order" },
                "output": { "type": "string", "description": "Where to save (default: <name>-edited.pdf)" },
                "pages": { "type": "string", "description": "Pages like 1-3,5 or 4- (to the end)" },
                "degrees": { "type": "integer", "description": "For rotate: 90, 180 or 270" },
                "text": { "type": "string", "description": "For stamp: the line of text" },
                "position": { "type": "string", "enum": ["top", "bottom"] },
                "fields": { "type": "object", "description": "For fill_form: field name → value (checkboxes: yes/no)", "additionalProperties": true }
            },
            "required": ["action"]
        })
    }
    fn title(&self, args: &Value) -> String {
        let action = arg_str(args, "action").replace('_', " ");
        match args.get("path").and_then(Value::as_str) {
            Some(p) => format!("PDF {action}: {p}"),
            None => format!("PDF {action}"),
        }
    }
    fn precheck(&self, ctx: &ToolContext, args: &Value) -> Option<String> {
        let action = arg_str(args, "action");
        if !PDF_ACTIONS.contains(&action) {
            return Some(format!("action must be one of: {}", PDF_ACTIONS.join(", ")));
        }
        if action == "form_fields" {
            return None;
        }
        if action == "merge" && args.get("output").and_then(Value::as_str).is_none() {
            return Some("merge needs output (the new PDF's name)".to_owned());
        }
        let out = pdf_output(ctx, args)?;
        if !has_ext(&out, &["pdf"]) {
            return Some("output must end in .pdf".to_owned());
        }
        write_blocked(ctx, &out.to_string_lossy())
    }
    fn effect(&self, ctx: &ToolContext, args: &Value) -> Effect {
        if arg_str(args, "action") == "form_fields" {
            return Effect::Read;
        }
        let path = pdf_output(ctx, args).map_or_else(String::new, |p| display(&ctx.workspace, &p));
        Effect::Edit { path, diff: None }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let output = pdf_output(ctx, &args);
            let a: PdfArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let ws = ctx.workspace.clone();
            let input = a.path.as_deref().map(|p| resolve(&ws, p));
            let need_input = || {
                input
                    .clone()
                    .ok_or_else(|| format!("{} needs path (the PDF)", a.action))
            };
            let shown_out = output.as_ref().map(|p| display(&ws, p)).unwrap_or_default();
            let result: Result<(String, String), String> = match a.action.as_str() {
                "form_fields" => match need_input() {
                    Ok(p) => blocking(move || pdf::form_fields(&p)).await.map(|fields| {
                        if fields.is_empty() {
                            (
                                "This PDF has no form fields.".to_owned(),
                                "No form fields".to_owned(),
                            )
                        } else {
                            let list: String = fields
                                .iter()
                                .map(|f| format!("- {} ({}): {}\n", f.name, f.kind, f.value))
                                .collect();
                            (
                                format!("Form fields:\n{list}"),
                                format!("{} form fields", fields.len()),
                            )
                        }
                    }),
                    Err(e) => Err(e),
                },
                "merge" => {
                    let inputs: Vec<PathBuf> = a.inputs.iter().map(|p| resolve(&ws, p)).collect();
                    let Some(out) = output.clone() else {
                        return ToolOutput::error("merge needs output");
                    };
                    blocking(move || {
                        let refs: Vec<&Path> = inputs.iter().map(PathBuf::as_path).collect();
                        pdf::merge(&refs, &out)
                    })
                    .await
                    .map(|n| {
                        (
                            format!(
                                "Merged {} PDFs into {shown_out} ({n} pages).",
                                a.inputs.len()
                            ),
                            format!("Merged into {shown_out} · {n} pages"),
                        )
                    })
                }
                action => {
                    let (Ok(src), Some(out)) = (need_input(), output.clone()) else {
                        return ToolOutput::error(format!("{action} needs path (the PDF)"));
                    };
                    let pages = a.pages.clone();
                    match action {
                        "extract_pages" | "delete_pages" => {
                            let Some(spec) = pages else {
                                return ToolOutput::error(format!(
                                    "{action} needs pages, like 1-3,5"
                                ));
                            };
                            let remove = action == "delete_pages";
                            blocking(move || pdf::select_pages(&src, &spec, remove, &out))
                                .await
                                .map(|n| {
                                    (
                                        format!("Saved {shown_out} ({n} pages)."),
                                        format!("Saved {shown_out} · {n} pages"),
                                    )
                                })
                        }
                        "rotate" => {
                            let degrees = a.degrees.unwrap_or(90);
                            blocking(move || pdf::rotate(&src, pages.as_deref(), degrees, &out))
                                .await
                                .map(|n| {
                                    (
                                        format!(
                                            "Rotated {n} page(s) by {degrees}°; saved {shown_out}."
                                        ),
                                        format!("Rotated {n} page(s) · {shown_out}"),
                                    )
                                })
                        }
                        "stamp" => {
                            let Some(text) = a.text.clone().filter(|t| !t.trim().is_empty()) else {
                                return ToolOutput::error("stamp needs text");
                            };
                            let top = a.position.as_deref() == Some("top");
                            blocking(move || pdf::stamp(&src, &text, pages.as_deref(), top, &out))
                                .await
                                .map(|n| {
                                    (
                                        format!("Stamped {n} page(s); saved {shown_out}."),
                                        format!("Stamped {n} page(s) · {shown_out}"),
                                    )
                                })
                        }
                        "fill_form" => {
                            let values: BTreeMap<String, String> = a
                                .fields
                                .iter()
                                .map(|(k, v)| {
                                    let v = match v {
                                        Value::String(s) => s.clone(),
                                        Value::Bool(b) => if *b { "yes" } else { "no" }.to_owned(),
                                        other => other.to_string(),
                                    };
                                    (k.clone(), v)
                                })
                                .collect();
                            if values.is_empty() {
                                return ToolOutput::error("fill_form needs fields: {name: value}");
                            }
                            blocking(move || pdf::fill_form(&src, &values, &out))
                                .await
                                .map(|n| {
                                    (
                                        format!("Filled {n} field(s); saved {shown_out}."),
                                        format!("Filled {n} field(s) · {shown_out}"),
                                    )
                                })
                        }
                        other => Err(format!("unknown action {other}")),
                    }
                }
            };
            match result {
                Ok((content, summary)) => {
                    if let Some(out) = &output {
                        ctx.note_read(out);
                    }
                    ToolOutput::ok(content, summary)
                }
                Err(e) => ToolOutput::error(e),
            }
        })
    }
}

// ---------- make_pdf ----------

/// Make a PDF: markdown laid out as a document, pictures one to a page, or Thrall's own page.
pub struct MakePdf;

#[derive(Deserialize)]
struct MakePdfArgs {
    path: String,
    #[serde(default)]
    title: Option<String>,
}

/// What a PDF is made from.
enum PdfSource {
    /// Markdown, laid out as a document.
    Document(String),
    /// Pictures, one to a page.
    Pictures(Vec<pictures::Picture>, pictures::PictureLayout),
    /// A page Thrall wrote (HTML and CSS), printed as it is.
    Custom(String),
}

impl PdfSource {
    /// What it's made from, as text (shown in the approval).
    fn text(&self) -> String {
        match self {
            Self::Document(text) | Self::Custom(text) => text.clone(),
            Self::Pictures(list, how) => {
                let mut lines = Vec::new();
                if let Some(cover) = &how.cover {
                    lines.push(format!("Cover: {cover}"));
                }
                for (i, p) in list.iter().enumerate() {
                    match &p.caption {
                        Some(c) => lines.push(format!("{}. {} ({c})", i + 1, p.path)),
                        None => lines.push(format!("{}. {}", i + 1, p.path)),
                    }
                }
                lines.join("\n")
            }
        }
    }
}

/// The pictures asked for: each a path, or {path, caption}.
fn picture_list(args: &Value) -> Result<Vec<pictures::Picture>, String> {
    let list = args
        .get("images")
        .and_then(Value::as_array)
        .filter(|l| !l.is_empty())
        .ok_or_else(|| "the pictures layout needs images (the pictures, in order)".to_owned())?;
    list.iter()
        .map(|v| match v {
            Value::String(p) => Ok(pictures::Picture {
                path: p.clone(),
                caption: None,
            }),
            Value::Object(o) => Ok(pictures::Picture {
                path: o
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "each picture needs a path".to_owned())?
                    .to_owned(),
                caption: o.get("caption").and_then(Value::as_str).map(str::to_owned),
            }),
            _ => Err("each picture is a path, or {path, caption}".to_owned()),
        })
        .collect()
}

/// What the PDF is made from, by `layout` (default document).
fn pdf_source(ctx: &ToolContext, args: &Value) -> Result<PdfSource, String> {
    match args
        .get("layout")
        .and_then(Value::as_str)
        .unwrap_or("document")
    {
        "document" => {
            if let Some(content) = args.get("content").and_then(Value::as_str) {
                return Ok(PdfSource::Document(content.to_owned()));
            }
            let source = args
                .get("source")
                .and_then(Value::as_str)
                .ok_or_else(|| "give content (markdown) or source (a .md file)".to_owned())?;
            let path = resolve(&ctx.workspace, source);
            std::fs::read_to_string(&path)
                .map(PdfSource::Document)
                .map_err(|e| format!("couldn't read {}: {e}", display(&ctx.workspace, &path)))
        }
        "pictures" => {
            let fit = args.get("fit").and_then(Value::as_str).unwrap_or("whole");
            if !["whole", "fill"].contains(&fit) {
                return Err(format!("fit is whole or fill, not {fit}"));
            }
            let how = pictures::PictureLayout {
                fill: fit == "fill",
                turn_to_fit: !matches!(
                    args.get("orientation").and_then(Value::as_str),
                    Some("portrait" | "landscape")
                ),
                cover: args
                    .get("cover")
                    .and_then(Value::as_str)
                    .filter(|c| !c.trim().is_empty())
                    .map(str::to_owned),
                blank_backs: args
                    .get("blank_backs")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            };
            Ok(PdfSource::Pictures(picture_list(args)?, how))
        }
        "custom" => args
            .get("html")
            .and_then(Value::as_str)
            .filter(|h| !h.trim().is_empty())
            .map(|h| PdfSource::Custom(h.to_owned()))
            .ok_or_else(|| "the custom layout needs html (the page, with its CSS)".to_owned()),
        other => Err(format!(
            "layout is one of {}, not {other}",
            PDF_LAYOUTS.join(", ")
        )),
    }
}

/// The page layout: the document's defaults, except that pictures and custom pages have no
/// running title or page numbers, and their own margins, unless asked for.
fn pdf_layout(source: &PdfSource, args: &Value) -> Result<layout::Layout, String> {
    // "auto" turns each picture's page; the paper itself starts upright.
    let mut plain = args.clone();
    if let Some(o) = plain.as_object_mut()
        && o.get("orientation").and_then(Value::as_str) == Some("auto")
    {
        o.remove("orientation");
    }
    let mut l = layout::from_args(&plain)?;
    let margin = match source {
        PdfSource::Document(_) => return Ok(l),
        PdfSource::Pictures(..) => PICTURES_MARGIN_MM,
        PdfSource::Custom(_) => CUSTOM_MARGIN_MM,
    };
    if args.get("margin_mm").and_then(Value::as_f64).is_none() {
        l.margin_mm = margin;
    }
    if args.get("header_footer").and_then(Value::as_bool).is_none() {
        l.header_footer = false;
    }
    Ok(l)
}

/// Where a PDF's pictures may come from: the working folder and Thrall's saved files.
fn picture_sources(ctx: &ToolContext) -> embed::Sources<'_> {
    embed::Sources {
        workspace: &ctx.workspace,
        also: ctx
            .data_dir
            .iter()
            .map(|d| d.join(thrall_core::constants::UPLOADS_DIR))
            .collect(),
    }
}

/// The page to print, before its pictures go in.
fn pdf_page(
    ctx: &ToolContext,
    source: &PdfSource,
    page_layout: &layout::Layout,
    title: &str,
) -> String {
    match source {
        PdfSource::Document(md) => print::document(md, title, page_layout),
        PdfSource::Pictures(list, how) => {
            pictures::pictures_page(list, how, page_layout, title, |p| {
                resolve(&ctx.workspace, p)
            })
        }
        PdfSource::Custom(html) => pictures::custom_page(html, title),
    }
}

impl Tool for MakePdf {
    fn name(&self) -> &'static str {
        "make_pdf"
    }
    fn description(&self) -> &'static str {
        "Make a PDF. Pick the layout that suits what it's for (default document):\n\
         - document: reports, notes, letters, cheat sheets. Give content (markdown) or source (a \
         .md file): headings, paragraphs, lists, tables, code, quotes, links, pictures \
         (![caption](file.png)); <div class=\"page-break\"></div> starts a new page. The first # \
         heading is the title unless title is given. Options: font_size (body text; headings keep \
         their proportions) or sizes {title, heading, subheading, text} exactly, always with the \
         unit the user used (\"12px\" or \"9pt\"); spacing compact|normal; header_footer false to \
         drop the page title and page numbers; max_pages to shrink everything evenly until it \
         fits. Each list item or form field on its own line: use a list, or end lines with two \
         spaces.\n\
         - pictures: colouring books, photo albums, scans, posters, picture books without text. \
         images: the pictures in order (paths, or {path, caption}), one per page and nothing \
         else: no title or page numbers. fit whole (default: all of the picture shows) or fill \
         (edge to edge, cropping if the shapes differ); margin_mm (default 10; 0 for edge to \
         edge); orientation auto (default: each page turns to match its picture), portrait or \
         landscape; cover: a title for a first page; blank_backs true adds a blank page after \
         each picture, for printing on both sides.\n\
         - custom: anything with its own design (story books with pictures and text, flyers, \
         certificates, cards, menus). html: the whole page with its CSS, printed exactly as \
         written; set the paper with CSS (@page { size: A5; margin: 12mm }) and new pages with \
         break-after: page. The fonts 'Instrument Sans' and 'IBM Plex Mono' are available.\n\
         Every layout: pictures in the working folder (or ones you made or were sent) go into \
         the PDF, and one that can't be found stops it and is named. paper A4|Letter; margin_mm. \
         Replaces path if it exists. Use this whenever someone asks for a PDF."
    }
    fn parameters(&self) -> Value {
        let mut props = layout::schema_properties();
        props.insert(
            "orientation".into(),
            json!({ "type": "string", "enum": ["portrait", "landscape", "auto"], "description": "auto (pictures only, and their default): each page turns to match its picture" }),
        );
        props.insert(
            "path".into(),
            json!({ "type": "string", "description": "The .pdf to write" }),
        );
        props.insert(
            "layout".into(),
            json!({ "type": "string", "enum": PDF_LAYOUTS, "description": "document (default), pictures or custom" }),
        );
        props.insert(
            "content".into(),
            json!({ "type": "string", "description": "document: the document as markdown" }),
        );
        props.insert(
            "source".into(),
            json!({ "type": "string", "description": "document: or a markdown file to lay out" }),
        );
        props.insert("title".into(), json!({ "type": "string", "description": "The PDF's title (document: shown at the top of each page; default the first # heading)" }));
        props.insert("max_pages".into(), json!({ "type": "integer", "minimum": 1, "description": "document: shrink everything evenly (to half size at most) until it fits this many pages; this overrides the sizes, so leave it out when exact sizes matter" }));
        props.insert(
            "images".into(),
            json!({ "type": "array", "description": "pictures: the pictures in order, one per page", "items": {
                "anyOf": [
                    { "type": "string" },
                    { "type": "object", "properties": { "path": { "type": "string" }, "caption": { "type": "string" } }, "required": ["path"] }
                ]
            } }),
        );
        props.insert(
            "fit".into(),
            json!({ "type": "string", "enum": ["whole", "fill"], "description": "pictures: whole (default) shows all of each picture; fill covers the page, cropping" }),
        );
        props.insert(
            "cover".into(),
            json!({ "type": "string", "description": "pictures: a title for a first page" }),
        );
        props.insert(
            "blank_backs".into(),
            json!({ "type": "boolean", "description": "pictures: a blank page after each one, for printing on both sides" }),
        );
        props.insert(
            "html".into(),
            json!({ "type": "string", "description": "custom: the page as HTML with its CSS" }),
        );
        json!({ "type": "object", "properties": props, "required": ["path"] })
    }
    fn title(&self, args: &Value) -> String {
        format!("Making {}", arg_str(args, "path"))
    }
    fn precheck(&self, ctx: &ToolContext, args: &Value) -> Option<String> {
        let path = resolve(&ctx.workspace, arg_str(args, "path"));
        if !has_ext(&path, &["pdf"]) {
            return Some("path must end in .pdf".to_owned());
        }
        let source = match pdf_source(ctx, args) {
            Ok(s) => s,
            Err(e) => return Some(e),
        };
        let page_layout = match pdf_layout(&source, args) {
            Ok(l) => l,
            Err(e) => return Some(e),
        };
        // Pictures that won't go in are caught before anyone is asked.
        let page = pdf_page(ctx, &source, &page_layout, "");
        let problems = embed::check(&picture_sources(ctx), &page);
        if !problems.is_empty() {
            return Some(format!(
                "these pictures can't go into the PDF:\n- {}",
                problems.join("\n- ")
            ));
        }
        write_blocked(ctx, arg_str(args, "path"))
    }
    fn effect(&self, ctx: &ToolContext, args: &Value) -> Effect {
        let path = resolve(&ctx.workspace, arg_str(args, "path"));
        let shown = display(&ctx.workspace, &path);
        Effect::Edit {
            diff: pdf_source(ctx, args)
                .ok()
                .map(|s| diff::make(&shown, None, &s.text())),
            path: shown,
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let source = match pdf_source(ctx, &args) {
                Ok(s) => s,
                Err(e) => return ToolOutput::error(e),
            };
            let page_layout = match pdf_layout(&source, &args) {
                Ok(l) => l,
                Err(e) => return ToolOutput::error(e),
            };
            let is_document = matches!(source, PdfSource::Document(_));
            // Shrinking to fit is for documents; the other layouts are sized on purpose.
            let max_pages = args
                .get("max_pages")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n > 0 && is_document);
            let a: MakePdfArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let path = resolve(&ctx.workspace, &a.path);
            let shown = display(&ctx.workspace, &path);
            let title = a
                .title
                .filter(|t| !t.trim().is_empty())
                .or_else(|| match &source {
                    PdfSource::Document(md) => markdown::title(md),
                    PdfSource::Pictures(_, how) => how.cover.clone(),
                    PdfSource::Custom(_) => None,
                })
                .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "Document".to_owned());
            let page = pdf_page(ctx, &source, &page_layout, &title);
            let embedded = match embed::embed(&picture_sources(ctx), &page) {
                Ok(e) => e,
                Err(e) => return ToolOutput::error(e),
            };
            let mut setup = print::setup(&page_layout, &title);
            setup.css_page_size = !is_document;
            if matches!(source, PdfSource::Pictures(..)) {
                // Each picture page sets its own size, and its margin is inside the page.
                setup.margin_top_in = 0.0;
                setup.margin_bottom_in = 0.0;
                setup.margin_side_in = 0.0;
                setup.header = None;
            }
            let (pdf, pages, scale) = match ctx
                .browser
                .print_pdf(ctx.data_dir.as_deref(), &embedded.html, &setup, max_pages)
                .await
            {
                Ok(done) => done,
                Err(e) => return ToolOutput::error(e),
            };
            let existed = path.exists();
            if let Some(dir) = path.parent()
                && let Err(e) = tokio::fs::create_dir_all(dir).await
            {
                return ToolOutput::error(format!("{shown}: {e}"));
            }
            if let Err(e) = tokio::fs::write(&path, &pdf).await {
                return ToolOutput::error(format!("{shown}: {e}"));
            }
            ctx.note_read(&path);
            let verb = if existed { "Replaced" } else { "Created" };
            let pictures = match embedded.pictures {
                0 => String::new(),
                1 => ", 1 picture".to_owned(),
                n => format!(", {n} pictures"),
            };
            let mut note = String::new();
            if scale < 1.0 {
                // Say what the sizes really are now, so nobody reports the requested ones.
                let px = |pt: f64| pt * scale / 0.75;
                note = format!(
                    " Shrunk to {:.0}% to fit, so the sizes are now: title {:.1}px, headings \
                     {:.1}px, subheadings {:.1}px, text {:.1}px. To keep the sizes asked for, \
                     leave out max_pages (or use spacing compact).",
                    scale * 100.0,
                    px(page_layout.title_pt),
                    px(page_layout.heading_pt),
                    px(page_layout.subheading_pt),
                    px(page_layout.body_pt),
                );
            }
            if let Some(limit) = max_pages
                && pages > limit
            {
                note.push_str(&format!(
                    " Still {pages} pages at half size: shorten the content or allow more pages."
                ));
            }
            // A picture of the pages, so the result can be checked by eye before it's handed over.
            let wide = if matches!(source, PdfSource::Pictures(..)) {
                setup.width_in.max(setup.height_in)
            } else {
                setup.width_in
            };
            let jpeg = ctx
                .browser
                .print_preview(
                    ctx.data_dir.as_deref(),
                    &embedded.html,
                    wide,
                    setup.height_in,
                    setup.height_in * PDF_PREVIEW_MAX_PAGES,
                    PDF_PREVIEW_SCALE,
                )
                .await
                .ok();
            let preview = ctx.data_dir.as_ref().and_then(|d| {
                let dir = d.join(PDF_PREVIEW_DIR);
                clear_old_previews(&dir, std::time::Duration::from_secs(PDF_PREVIEW_KEEP_SECS));
                let stem = path
                    .file_stem()
                    .map_or_else(|| "pdf".to_owned(), |s| s.to_string_lossy().into_owned());
                keep_preview(&dir, &stem, jpeg)
            });
            let check = preview.as_ref().map_or_else(
                || " Before you tell the user it's ready, check it with read_document.".to_owned(),
                |file| {
                    format!(
                        " Before you tell the user it's ready, look at the preview with \
                         view_image ({}): the pages one under the other (up to \
                         {PDF_PREVIEW_MAX_PAGES} pages). Check it's what they asked for, with \
                         every picture showing.",
                        file.display()
                    )
                },
            );
            let mut out = ToolOutput::ok(
                format!(
                    "{verb} {shown} ({pages} pages{pictures}).{note}{check} It's attached in the \
                     chat for the user to open; call it {shown}."
                ),
                format!("{verb} {shown} · {pages} pages{pictures}"),
            );
            // A copy the person can open from the chat.
            let name = path.file_name().map_or_else(
                || "document.pdf".to_owned(),
                |n| n.to_string_lossy().into_owned(),
            );
            if let Ok((_, file)) = saved::save(ctx, &name, &pdf) {
                out.files.push(file);
            }
            out
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::{clear_old_previews, keep_preview};

    #[test]
    fn old_previews_go_and_a_failed_one_leaves_no_stale_picture() {
        let dir = std::env::temp_dir().join(format!("thrall-previews-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // A new preview is kept under the PDF's name.
        let fresh = keep_preview(&dir, "sheet", Some(b"jpeg".to_vec())).unwrap();
        assert_eq!(fresh, dir.join("sheet-preview.jpg"));

        // One from weeks ago is cleared; today's stays.
        let old = dir.join("old-preview.jpg");
        std::fs::write(&old, b"jpeg").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(20 * 24 * 60 * 60))
            .unwrap();
        clear_old_previews(&dir, Duration::from_secs(24 * 60 * 60));
        assert!(!old.exists());
        assert!(fresh.exists());

        // No new preview: the PDF's earlier one goes too.
        assert!(keep_preview(&dir, "sheet", None).is_none());
        assert!(!fresh.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
